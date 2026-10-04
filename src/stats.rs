//! 仪表盘成就 / 里程碑统计（本地、可关闭、不联网、不上传任何数据）。
//!
//! 落盘：`<data 目录>\stats.json`（与 `tool.log` 同级；原子写 tmp → `.bak` → rename）。
//! 解析失败先把坏文件留档成 `stats.json.broken_<时间戳>`，再从 `.bak` 恢复，最后才用默认值
//! ——与配置（`xmst_config.json`）同一套做法，绝不静默覆盖用户数据。
//!
//! 设计约束：
//! - **只在事件发生时累加**（启动成功 / 停止 / 下载完成 / 快照创建 / 回退完成 / 隧道连通 /
//!   诊断包导出 / 崩溃分析出结论 / 服务器数量变化），渲染路径只读 `snapshot()` 的内存克隆，
//!   不读文件、不起线程。
//! - 开关 `enabled`（默认 true）：关闭后**不再累加、不再判定解锁**，但**不清空**已有数据。
//! - 累计值只增不减；解锁状态一旦写入就不会因为服务器被删除等原因回退。
//! - 锁一律 `lock().unwrap_or_else(|e| e.into_inner())`（release 为 `panic=abort`）。
//! - 不新增依赖：序列化用 serde_json，日期用 chrono。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

/// 统计文件名（位于 data 目录）
pub const FILE_NAME: &str = "stats.json";
/// 上一份统计文件名（原子写时保留）
pub const BAK_NAME: &str = "stats.json.bak";
/// 「有服务器在线的日期」保留上限（条）
pub const ONLINE_DAYS_CAP: usize = 400;
/// 每天在线时长换算（秒/小时），用于时长类成就的进度
pub const SECS_PER_HOUR: u64 = 3600;

/// 本地累计统计 + 解锁状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// 累计启动服务器次数（与每台服务器的 `start_count` 同口径：每次成功接管进程 +1）
    pub launches: u64,
    /// 累计运行时长（秒，只在服务器停止/退出时累加一次）
    pub play_secs: u64,
    /// 累计下载文件数（建服服务端 jar、下载页自定义下载、模组/插件/数据包下载）
    pub downloads: u64,
    /// 累计安装模组数（模组页下载成功数）
    pub mods_installed: u64,
    /// 累计成功快照数（任意原因）
    pub backups: u64,
    /// 关服自动快照成功数（`BackupReason::Stop`，用于「善始善终」成就）
    pub stop_backups: u64,
    /// 累计成功回退次数
    pub restore_ok: u64,
    /// 历史最多同时拥有的服务器数（服务器数量类成就按此判定，删除后不回退）
    pub servers_peak: u64,
    /// 是否用过内网穿透并成功连接
    pub tunnel_first_seen: bool,
    /// 是否导出过诊断包
    pub diag_pack_exported: bool,
    /// 是否用崩溃分析定位过问题（成功分析出结论即可）
    pub crashscan_solved: bool,
    /// 有服务器在线的日期（`yyyy-MM-dd`，去重、升序，上限 `ONLINE_DAYS_CAP` 条）
    pub online_days: Vec<String>,
    /// 已解锁成就 id（只增不减）
    pub unlocked: Vec<String>,
    /// 成就总开关（默认 true；关闭后不记录、不通知，但不清空数据）
    pub enabled: bool,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            launches: 0,
            play_secs: 0,
            downloads: 0,
            mods_installed: 0,
            backups: 0,
            stop_backups: 0,
            restore_ok: 0,
            servers_peak: 0,
            tunnel_first_seen: false,
            diag_pack_exported: false,
            crashscan_solved: false,
            online_days: Vec::new(),
            unlocked: Vec::new(),
            enabled: true,
        }
    }
}

/// 可以累加的统计类型（`bump` 的入参）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatKind {
    /// 启动成功一次
    Launches,
    /// 本次运行时长（秒）
    PlaySecs,
    /// 下载成功一个文件
    Downloads,
    /// 安装成功一个模组
    Mods,
    /// 成功创建一份快照
    Backups,
    /// 成功创建一份**关服自动**快照
    StopBackups,
    /// 成功回退一次
    RestoreOk,
    /// 内网穿透连接成功（一次性）
    TunnelOk,
    /// 导出诊断包成功（一次性）
    DiagPack,
    /// 崩溃分析得出有效结论（一次性）
    CrashScanOk,
}

/// 成就对应的指标（决定进度怎么算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metric {
    /// 历史最多拥有的服务器数
    Servers,
    Launches,
    /// 累计运行小时数（`play_secs / 3600`）
    PlayHours,
    Downloads,
    Mods,
    Backups,
    StopBackups,
    RestoreOk,
    /// 内网穿透连接成功（0 / 1）
    Tunnel,
    /// 导出过诊断包（0 / 1）
    DiagPack,
    /// 崩溃分析出结论（0 / 1）
    CrashScan,
    /// 最长连续在线天数
    OnlineStreak,
}

/// 一条成就（含当前进度，未解锁也返回，便于界面灰显）
#[derive(Debug, Clone, PartialEq)]
pub struct Achievement {
    /// 稳定 id（英文短横线小写）
    pub id: &'static str,
    pub name: &'static str,
    /// 达成条件（界面 hover 文案）
    pub desc: &'static str,
    /// 当前值（时长类为小时）
    pub cur: u64,
    /// 目标值
    pub target: u64,
    /// 进度 0.0..=1.0（超额封顶 1.0）
    pub progress: f32,
    /// 是否已解锁
    pub unlocked: bool,
}

/// 成就定义表（阈值来自 `docs\UI-审计表.md` §8.2）
struct Def {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    target: u64,
    metric: Metric,
}

/// 全部成就：服务器数 5 + 启动 3 + 时长 3 + 下载 4 + 模组 3 + 行为 8 = 26 项
static DEFS: &[Def] = &[
    Def { id: "servers-1", name: "初创", desc: "拥有 1 台服务器", target: 1, metric: Metric::Servers },
    Def { id: "servers-3", name: "三台并管", desc: "拥有 3 台服务器", target: 3, metric: Metric::Servers },
    Def { id: "servers-5", name: "五台在手", desc: "拥有 5 台服务器", target: 5, metric: Metric::Servers },
    Def { id: "servers-10", name: "十台机房", desc: "拥有 10 台服务器", target: 10, metric: Metric::Servers },
    Def { id: "servers-20", name: "二十台集群", desc: "拥有 20 台服务器", target: 20, metric: Metric::Servers },
    Def { id: "launches-10", name: "常客", desc: "累计启动服务器 10 次", target: 10, metric: Metric::Launches },
    Def { id: "launches-100", name: "启动百次", desc: "累计启动服务器 100 次", target: 100, metric: Metric::Launches },
    Def { id: "launches-1000", name: "启动千次", desc: "累计启动服务器 1000 次", target: 1000, metric: Metric::Launches },
    Def { id: "play-10h", name: "十小时", desc: "累计运行 10 小时", target: 10, metric: Metric::PlayHours },
    Def { id: "play-100h", name: "百小时", desc: "累计运行 100 小时", target: 100, metric: Metric::PlayHours },
    Def { id: "play-1000h", name: "千小时", desc: "累计运行 1000 小时", target: 1000, metric: Metric::PlayHours },
    Def { id: "downloads-1", name: "初次下载", desc: "累计下载 1 个文件", target: 1, metric: Metric::Downloads },
    Def { id: "downloads-10", name: "下载十次", desc: "累计下载 10 个文件", target: 10, metric: Metric::Downloads },
    Def { id: "downloads-50", name: "下载五十次", desc: "累计下载 50 个文件", target: 50, metric: Metric::Downloads },
    Def { id: "downloads-100", name: "下载百次", desc: "累计下载 100 个文件", target: 100, metric: Metric::Downloads },
    Def { id: "mods-1", name: "初装模组", desc: "累计安装 1 个模组", target: 1, metric: Metric::Mods },
    Def { id: "mods-10", name: "模组十枚", desc: "累计安装 10 个模组", target: 10, metric: Metric::Mods },
    Def { id: "mods-100", name: "模组百枚", desc: "累计安装 100 个模组", target: 100, metric: Metric::Mods },
    Def { id: "backup-first", name: "首次快照", desc: "首次成功创建备份快照", target: 1, metric: Metric::Backups },
    Def { id: "restore-first", name: "有备无患", desc: "首次成功回退备份", target: 1, metric: Metric::RestoreOk },
    Def { id: "tunnel-first", name: "开门迎客", desc: "首次用内网穿透连接成功", target: 1, metric: Metric::Tunnel },
    Def { id: "diag-first", name: "对症下药", desc: "首次导出诊断包", target: 1, metric: Metric::DiagPack },
    Def { id: "crashscan-first", name: "排障入门", desc: "首次用崩溃分析定位问题", target: 1, metric: Metric::CrashScan },
    Def { id: "online-7", name: "七日在线", desc: "连续 7 天有服务器在线", target: 7, metric: Metric::OnlineStreak },
    Def { id: "online-30", name: "满月在线", desc: "连续 30 天有服务器在线", target: 30, metric: Metric::OnlineStreak },
    Def { id: "stopbackup-100", name: "善始善终", desc: "关服自动快照累计 100 次", target: 100, metric: Metric::StopBackups },
];

/// 全局状态：统计快照 + 落盘路径
struct State {
    data: Stats,
    path: PathBuf,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// 默认落盘路径：`<exe 目录>\data\stats.json`（与 `tool.log` 同目录）
fn default_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data")
        .join(FILE_NAME)
}

/// 取得（首次调用时建立）全局状态；`dir_hint` 只在首次生效。
fn state(dir_hint: Option<&Path>) -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        let path = match dir_hint {
            Some(d) => d.join(FILE_NAME),
            None => default_path(),
        };
        let data = load_from(&path);
        Mutex::new(State { data, path })
    })
}

/// 启动时初始化（指定 data 目录并读取一次文件）。幂等：重复调用只生效第一次。
pub fn init(data_dir: &Path) {
    let _ = state(Some(data_dir));
}

/// 读盘：解析失败先留档坏文件 → 尝试 `.bak` → 最后默认值。
fn load_from(path: &Path) -> Stats {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Stats::default();
    };
    match serde_json::from_str::<Stats>(&text) {
        Ok(s) => {
            let mut s = s;
            s.online_days = normalize_days(&s.online_days);
            s
        }
        Err(_) => {
            // 坏文件留档（不删、不覆盖），再从 .bak 恢复
            let bad = path.with_file_name(format!(
                "{FILE_NAME}.broken_{}",
                Local::now().format("%Y%m%d_%H%M%S")
            ));
            let _ = std::fs::rename(path, &bad);
            if let Ok(bs) = std::fs::read_to_string(path.with_file_name(BAK_NAME)) {
                if let Ok(mut s) = serde_json::from_str::<Stats>(&bs) {
                    s.online_days = normalize_days(&s.online_days);
                    return s;
                }
            }
            Stats::default()
        }
    }
}

/// 原子写（best-effort）：tmp → 保留旧文件为 `.bak` → rename；任何失败静默返回，不 panic。
fn save_locked(st: &State) {
    let Ok(json) = serde_json::to_string_pretty(&st.data) else {
        return;
    };
    if let Some(parent) = st.path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = st.path.with_extension("json.tmp");
    if std::fs::write(&tmp, json.as_bytes()).is_err() {
        return;
    }
    if st.path.exists() {
        let _ = std::fs::copy(&st.path, st.path.with_extension("json.bak"));
    }
    let _ = std::fs::rename(&tmp, &st.path);
}

/// 立即落盘（best-effort）；一般不需要手动调用（`bump` / `set_enabled` 内部已写）。
pub fn save() {
    let g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    save_locked(&g);
}

/// 读一份内存克隆（给渲染读：不读文件、不加长锁）
pub fn snapshot() -> Stats {
    state(None)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .data
        .clone()
}

/// 按类型累加并落盘。**只在事件真的发生时调用一次**，不得放进每帧路径；
/// 开关关闭或 `n == 0` 时直接返回（不记录）。
pub fn bump(kind: StatKind, n: u64) {
    let mut g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    if !g.data.enabled || n == 0 {
        return;
    }
    {
        let d = &mut g.data;
        apply(d, kind, n);
    }
    save_locked(&g);
}

/// 累加的纯逻辑（不落盘、不碰全局状态，便于单测）
fn apply(d: &mut Stats, kind: StatKind, n: u64) {
    if n == 0 {
        return;
    }
    match kind {
        StatKind::Launches => d.launches = d.launches.saturating_add(n),
        StatKind::PlaySecs => d.play_secs = d.play_secs.saturating_add(n),
        StatKind::Downloads => d.downloads = d.downloads.saturating_add(n),
        StatKind::Mods => d.mods_installed = d.mods_installed.saturating_add(n),
        StatKind::Backups => d.backups = d.backups.saturating_add(n),
        StatKind::StopBackups => d.stop_backups = d.stop_backups.saturating_add(n),
        StatKind::RestoreOk => d.restore_ok = d.restore_ok.saturating_add(n),
        StatKind::TunnelOk => d.tunnel_first_seen = true,
        StatKind::DiagPack => d.diag_pack_exported = true,
        StatKind::CrashScanOk => d.crashscan_solved = true,
    }
}

/// 记录「当前拥有的服务器数」峰值（服务器数量类成就按峰值判定，删除服务器不回退）。
pub fn note_server_count(n: u64) {
    let mut g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    if !g.data.enabled || n <= g.data.servers_peak {
        return;
    }
    g.data.servers_peak = n;
    save_locked(&g);
}

/// 记录「今天有服务器在线」：每个自然日最多记一次（启动成功时调用）。
pub fn note_online_today() {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    if !g.data.enabled {
        return;
    }
    if !note_day_into(&mut g.data, &today) {
        return;
    }
    save_locked(&g);
}

/// 写入在线日期的纯逻辑；返回是否发生了变化
fn note_day_into(d: &mut Stats, day: &str) -> bool {
    if !is_valid_day(day) || d.online_days.iter().any(|x| x == day) {
        return false;
    }
    d.online_days.push(day.to_string());
    d.online_days = normalize_days(&d.online_days);
    true
}

/// 成就总开关（关闭只停止记录与通知，不清空已有数据）
pub fn set_enabled(v: bool) {
    let mut g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    if g.data.enabled == v {
        return;
    }
    g.data.enabled = v;
    save_locked(&g);
}

/// 判定并落盘，返回**本次新解锁**的成就（由调用方弹通知 / 记工具日志）。
/// 开关关闭时直接返回空（不判定、不写入）。
pub fn evaluate() -> Vec<Achievement> {
    let mut g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    if !g.data.enabled {
        return Vec::new();
    }
    let newly = evaluate_into(&mut g.data);
    if !newly.is_empty() {
        save_locked(&g);
    }
    newly
}

/// 判定的纯逻辑（只改传入的 `Stats`，不落盘）
fn evaluate_into(s: &mut Stats) -> Vec<Achievement> {
    let mut newly: Vec<Achievement> = Vec::new();
    for d in DEFS {
        if d.target == 0 || s.unlocked.iter().any(|u| u == d.id) {
            continue;
        }
        let cur = metric_value(s, d.metric);
        if cur < d.target {
            continue;
        }
        s.unlocked.push(d.id.to_string());
        newly.push(Achievement {
            id: d.id,
            name: d.name,
            desc: d.desc,
            cur,
            target: d.target,
            progress: 1.0,
            unlocked: true,
        });
    }
    newly
}

/// 全部成就（含未解锁，供界面灰显 + 进度条）
pub fn all() -> Vec<Achievement> {
    let g = state(None).lock().unwrap_or_else(|e| e.into_inner());
    all_of(&g.data)
}

/// 全部成就的纯逻辑
fn all_of(s: &Stats) -> Vec<Achievement> {
    DEFS.iter()
        .map(|d| {
            let cur = metric_value(s, d.metric);
            Achievement {
                id: d.id,
                name: d.name,
                desc: d.desc,
                cur,
                target: d.target,
                progress: progress_of(cur, d.target),
                unlocked: s.unlocked.iter().any(|u| u == d.id),
            }
        })
        .collect()
}

/// 某项成就的当前值（时长类换算成小时）
fn metric_value(s: &Stats, m: Metric) -> u64 {
    match m {
        Metric::Servers => s.servers_peak,
        Metric::Launches => s.launches,
        Metric::PlayHours => s.play_secs / SECS_PER_HOUR,
        Metric::Downloads => s.downloads,
        Metric::Mods => s.mods_installed,
        Metric::Backups => s.backups,
        Metric::StopBackups => s.stop_backups,
        Metric::RestoreOk => s.restore_ok,
        Metric::Tunnel => u64::from(s.tunnel_first_seen),
        Metric::DiagPack => u64::from(s.diag_pack_exported),
        Metric::CrashScan => u64::from(s.crashscan_solved),
        Metric::OnlineStreak => longest_streak(&s.online_days),
    }
}

/// 进度 0.0..=1.0（超额封顶 1.0；`target == 0` 视为已完成）
pub fn progress_of(cur: u64, target: u64) -> f32 {
    if target == 0 {
        return 1.0;
    }
    (cur as f64 / target as f64).clamp(0.0, 1.0) as f32
}

/// `yyyy-MM-dd` 是否合法（且必须是零填充的规范写法）
pub fn is_valid_day(s: &str) -> bool {
    match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        Ok(d) => d.format("%Y-%m-%d").to_string() == s,
        Err(_) => false,
    }
}

/// 规范化在线日期：丢掉非法项、升序、去重、只保留最近 `ONLINE_DAYS_CAP` 条
pub fn normalize_days(days: &[String]) -> Vec<String> {
    let mut v: Vec<String> = days
        .iter()
        .filter(|d| is_valid_day(d))
        .cloned()
        .collect();
    v.sort();
    v.dedup();
    if v.len() > ONLINE_DAYS_CAP {
        let drop = v.len() - ONLINE_DAYS_CAP;
        v.drain(..drop);
    }
    v
}

/// 解析在线日期（已规范化列表，非法项跳过）
fn parse_days(days: &[String]) -> Vec<NaiveDate> {
    normalize_days(days)
        .iter()
        .filter_map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .collect()
}

/// 最长连续在线天数（成就判定用这个；空列表为 0）
pub fn longest_streak(days: &[String]) -> u64 {
    let mut best = 0u64;
    let mut run = 0u64;
    let mut prev: Option<NaiveDate> = None;
    for d in parse_days(days) {
        run = match prev {
            Some(p) if d.signed_duration_since(p).num_days() == 1 => run + 1,
            _ => 1,
        };
        if run > best {
            best = run;
        }
        prev = Some(d);
    }
    best
}

/// 当前连续在线天数（截至 `today`；今天与昨天都没在线则为 0）
pub fn current_streak(days: &[String], today: &str) -> u64 {
    let Ok(mut d) = NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
        return 0;
    };
    let set = parse_days(days);
    let has = |x: NaiveDate| set.iter().any(|s| *s == x);
    if !has(d) {
        let Some(prev) = d.pred_opt() else { return 0 };
        d = prev;
        if !has(d) {
            return 0;
        }
    }
    let mut n = 0u64;
    while has(d) {
        n += 1;
        let Some(prev) = d.pred_opt() else { break };
        d = prev;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn progress_accumulates_and_caps_at_one() {
        assert_eq!(progress_of(0, 100), 0.0);
        assert!((progress_of(12, 100) - 0.12).abs() < 1e-6);
        assert_eq!(progress_of(100, 100), 1.0);
        // 超额封顶
        assert_eq!(progress_of(999, 100), 1.0);
        // 目标为 0 视为已完成（防御，避免除零）
        assert_eq!(progress_of(0, 0), 1.0);
    }

    #[test]
    fn apply_accumulates_and_flags_are_one_shot() {
        let mut s = Stats::default();
        apply(&mut s, StatKind::Downloads, 3);
        apply(&mut s, StatKind::Downloads, 2);
        assert_eq!(s.downloads, 5);
        // 0 不写入
        apply(&mut s, StatKind::Downloads, 0);
        assert_eq!(s.downloads, 5);
        // 一次性标记：重复写仍是 true，不影响累加值
        apply(&mut s, StatKind::TunnelOk, 1);
        apply(&mut s, StatKind::TunnelOk, 1);
        assert!(s.tunnel_first_seen);
        assert_eq!(s.downloads, 5);
        // 饱和累加不 panic
        s.backups = u64::MAX;
        apply(&mut s, StatKind::Backups, 1);
        assert_eq!(s.backups, u64::MAX);
    }

    #[test]
    fn evaluate_returns_only_newly_unlocked() {
        let mut s = Stats::default();
        s.downloads = 1;
        let first = evaluate_into(&mut s);
        // 下载 1 个文件同时解开「初次下载」
        assert!(first.iter().any(|a| a.id == "downloads-1"));
        assert!(s.unlocked.iter().any(|u| u == "downloads-1"));
        assert_eq!(first.len(), s.unlocked.len());
        // 再判一次：没有新解锁
        let second = evaluate_into(&mut s);
        assert!(second.is_empty());
        assert_eq!(first.len(), s.unlocked.len());
        // 进度继续涨到下一个阈值才会再解锁
        s.downloads = 10;
        let third = evaluate_into(&mut s);
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].id, "downloads-10");
    }

    #[test]
    fn evaluate_with_disabled_switch_is_noop() {
        // 明确约定：开关关闭时 evaluate 不判定、不写入解锁列表（已有数据保留）
        let mut s = Stats::default();
        s.enabled = false;
        s.downloads = 1000;
        let got = evaluate_into_disabled_guard(&mut s);
        assert!(got.is_empty());
        assert!(s.unlocked.is_empty());
        assert_eq!(s.downloads, 1000);
    }

    /// 复刻 `evaluate()` 的开关分支（不落盘版本）：关闭时直接返回空
    fn evaluate_into_disabled_guard(s: &mut Stats) -> Vec<Achievement> {
        if !s.enabled {
            return Vec::new();
        }
        evaluate_into(s)
    }

    #[test]
    fn streak_longest_handles_gaps_dupes_and_month_boundary() {
        assert_eq!(longest_streak(&[]), 0);
        // 跨月连续：1/31 与 2/1 是连续的
        assert_eq!(longest_streak(&days(&["2026-01-31", "2026-02-01"])), 2);
        // 重复日期只算一天
        assert_eq!(
            longest_streak(&days(&["2026-01-31", "2026-01-31", "2026-02-01"])),
            2
        );
        // 有断档：最长段是 3 天
        assert_eq!(
            longest_streak(&days(&[
                "2026-03-01",
                "2026-03-02",
                "2026-03-03",
                "2026-03-05",
                "2026-03-06",
            ])),
            3
        );
        // 乱序输入先排序
        assert_eq!(
            longest_streak(&days(&["2026-03-03", "2026-03-01", "2026-03-02"])),
            3
        );
        // 非法项被丢弃
        assert_eq!(longest_streak(&days(&["不是日期", "2026-03-01"])), 1);
    }

    #[test]
    fn current_streak_counts_back_from_today() {
        let d = days(&["2026-10-01", "2026-10-02", "2026-10-03"]);
        assert_eq!(current_streak(&d, "2026-10-03"), 3);
        // 今天还没记：从昨天往回数
        assert_eq!(current_streak(&d, "2026-10-04"), 3);
        // 断了两天以上：归零
        assert_eq!(current_streak(&d, "2026-10-06"), 0);
        assert_eq!(current_streak(&[], "2026-10-06"), 0);
        assert_eq!(current_streak(&d, "非法日期"), 0);
    }

    #[test]
    fn normalize_days_sorts_dedupes_and_caps() {
        let v = normalize_days(&days(&["2026-01-02", "2026-01-01", "2026-01-02", "x"]));
        assert_eq!(v, days(&["2026-01-01", "2026-01-02"]));
        // 上限裁剪：只留最近的
        let mut many: Vec<String> = Vec::new();
        let base = match NaiveDate::from_ymd_opt(2020, 1, 1) {
            Some(d) => d,
            None => return,
        };
        for i in 0..(ONLINE_DAYS_CAP as u64 + 20) {
            if let Some(d) = base.checked_add_days(chrono::Days::new(i)) {
                many.push(d.format("%Y-%m-%d").to_string());
            }
        }
        let capped = normalize_days(&many);
        assert_eq!(capped.len(), ONLINE_DAYS_CAP);
        assert_eq!(capped.last(), many.last());
    }

    #[test]
    fn all_achievements_have_unique_ids_and_clear_progress() {
        let s = Stats::default();
        let list = all_of(&s);
        assert_eq!(list.len(), DEFS.len());
        assert!(list.len() >= 25);
        let mut ids: Vec<&str> = list.iter().map(|a| a.id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "id 不得重复");
        // 默认状态：全部未解锁、进度 0
        assert!(list.iter().all(|a| !a.unlocked));
        assert!(list.iter().all(|a| a.progress >= 0.0 && a.progress <= 1.0));
        assert!(list.iter().all(|a| a.target > 0));
        // 时长类按小时计进度
        let mut s2 = Stats::default();
        s2.play_secs = SECS_PER_HOUR * 50;
        let list2 = all_of(&s2);
        let h10 = list2.iter().find(|a| a.id == "play-10h").expect("应存在");
        assert_eq!(h10.cur, 50);
        assert_eq!(h10.progress, 1.0);
        let h1000 = list2.iter().find(|a| a.id == "play-1000h").expect("应存在");
        assert_eq!(h1000.cur, 50);
        assert!((h1000.progress - 0.05).abs() < 1e-6);
    }

    #[test]
    fn server_peak_and_day_notes_drive_metrics() {
        let mut s = Stats::default();
        s.servers_peak = 5;
        let got = evaluate_into(&mut s);
        let ids: Vec<&str> = got.iter().map(|a| a.id).collect();
        assert!(ids.contains(&"servers-1"));
        assert!(ids.contains(&"servers-3"));
        assert!(ids.contains(&"servers-5"));
        assert!(!ids.contains(&"servers-10"));
        // 在线日：连续 7 天解开「七日在线」，30 天解开「满月在线」
        let mut s2 = Stats::default();
        let start = match NaiveDate::from_ymd_opt(2026, 5, 1) {
            Some(d) => d,
            None => return,
        };
        for i in 0..30 {
            let Some(d) = start.checked_add_days(chrono::Days::new(i)) else {
                return;
            };
            assert!(note_day_into(&mut s2, &d.format("%Y-%m-%d").to_string()));
        }
        // 同一天重复记不再变化
        assert!(!note_day_into(&mut s2, "2026-05-30"));
        assert_eq!(longest_streak(&s2.online_days), 30);
        let got2 = evaluate_into(&mut s2);
        let ids2: Vec<&str> = got2.iter().map(|a| a.id).collect();
        assert!(ids2.contains(&"online-7"));
        assert!(ids2.contains(&"online-30"));
    }
}
