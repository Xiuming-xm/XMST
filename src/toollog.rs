//! 工具自身运行日志（左侧「日志」页的数据源）。
//!
//! 只记录 **XMST 工具自己做的事**（启动/停止、备份、隧道、下载、插件、配置、更新、诊断）。
//! 服务器控制台日志属于「服务器 → 日志」（`logdb`），穿透输出属于隧道页，两者都不进这里。
//!
//! 落盘：`<data 目录>\tool.log`，每行
//! `[yyyy-MM-dd HH:mm:ss] [级别] [类别] 消息`（级别：信息/警告/错误）。
//! 文件超过 1 MB 时滚成 `tool.log.1` 重新开始（与 `crash.log` 同一套做法）。
//! 内存里同时保留一个环形缓冲（上限 `RING_CAP` 条）供页面渲染，避免每帧读文件。
//!
//! 全局可用：UI 线程、后台线程、tick 都能直接调 `tool_log`；写盘为 best-effort，
//! 任何失败都静默忽略（不 panic）。锁一律 `unwrap_or_else(|e| e.into_inner())`。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// 日志文件名（位于 data 目录，与 `launch.log` / `integrity_check.log` 同级）
pub const FILE_NAME: &str = "tool.log";
/// 轮转后保留的上一份文件名
pub const ROTATED_NAME: &str = "tool.log.1";
/// 内存环形缓冲上限（条）
pub const RING_CAP: usize = 2000;
/// 文件轮转阈值（字节）
pub const ROTATE_BYTES: u64 = 1024 * 1024;
/// 启动时从文件尾部装载的行数上限
pub const STARTUP_TAIL_LINES: usize = 1000;

/// 日志级别：信息 / 警告 / 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLevel {
    Info,
    Warn,
    Error,
}

impl ToolLevel {
    /// 行内中文标签
    pub fn label(self) -> &'static str {
        match self {
            ToolLevel::Info => "信息",
            ToolLevel::Warn => "警告",
            ToolLevel::Error => "错误",
        }
    }

    /// 由行内标签还原级别（未知标签返回 None）
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "信息" => Some(ToolLevel::Info),
            "警告" => Some(ToolLevel::Warn),
            "错误" => Some(ToolLevel::Error),
            _ => None,
        }
    }
}

/// 一条工具日志
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLogEntry {
    /// `yyyy-MM-dd HH:mm:ss`
    pub ts: String,
    pub level: ToolLevel,
    /// 短词类别：服务器 / 备份 / 隧道 / 下载 / 插件 / 配置 / 更新 / 诊断 / 启动 / 进程
    pub category: String,
    pub msg: String,
}

/// 单行文本化：换行会破坏"一条日志一行"，统一压成空格。
fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// 生成一行日志文本（纯函数，便于往返测试）
pub fn format_line(ts: &str, level: ToolLevel, category: &str, msg: &str) -> String {
    format!(
        "[{ts}] [{}] [{}] {}\n",
        level.label(),
        one_line(category),
        one_line(msg)
    )
}

/// 解析一行日志文本；不是本模块写出的格式（或级别未知）返回 None。
pub fn parse_line(line: &str) -> Option<ToolLogEntry> {
    let rest = line.trim_end_matches(['\r', '\n']).strip_prefix('[')?;
    let (ts, rest) = rest.split_once("] [")?;
    let (level, rest) = rest.split_once("] [")?;
    let (category, msg) = rest.split_once("] ")?;
    Some(ToolLogEntry {
        ts: ts.to_string(),
        level: ToolLevel::parse(level)?,
        category: category.to_string(),
        msg: msg.to_string(),
    })
}

/// 环形缓冲裁剪：超过上限时丢最旧的（纯函数）
pub fn push_capped(buf: &mut VecDeque<ToolLogEntry>, e: ToolLogEntry, cap: usize) {
    if cap == 0 {
        return;
    }
    while buf.len() >= cap {
        buf.pop_front();
    }
    buf.push_back(e);
}

/// 文件是否达到轮转阈值（纯函数）
pub fn should_rotate(len: u64) -> bool {
    len > ROTATE_BYTES
}

/// 过滤判定（纯函数）：级别（"全部" 表示不限）、类别（"全部" 表示不限）、
/// 关键字（大小写不敏感，空串表示不限；匹配消息与类别）。
pub fn matches(level: &str, category: &str, query_lower: &str, e: &ToolLogEntry) -> bool {
    if level != "全部" && level != e.level.label() {
        return false;
    }
    if category != "全部" && !category.is_empty() && category != e.category {
        return false;
    }
    if !query_lower.is_empty()
        && !e.msg.to_lowercase().contains(query_lower)
        && !e.category.to_lowercase().contains(query_lower)
    {
        return false;
    }
    true
}

/// 取文本尾部最多 `max` 条可解析日志（纯函数）
pub fn tail_entries(text: &str, max: usize) -> Vec<ToolLogEntry> {
    let mut out: Vec<ToolLogEntry> = text.lines().filter_map(parse_line).collect();
    if out.len() > max {
        let drop = out.len() - max;
        out.drain(..drop);
    }
    out
}

/// 内存状态：环形缓冲 + 落盘路径 + 已写字节数（用于轮转判断，避免每条都 stat）
struct State {
    buf: VecDeque<ToolLogEntry>,
    path: PathBuf,
    file_len: u64,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
/// 版本号：每次写入 / 清空自增，供页面判断"要不要重算过滤"（读它不需要加锁）
static REV: AtomicU64 = AtomicU64::new(0);

/// 默认日志路径：`<exe 目录>\data\tool.log`
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
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut buf: VecDeque<ToolLogEntry> = VecDeque::new();
        // 只在启动时读一次文件尾部（之后全靠内存缓冲，渲染路径不碰盘）
        if let Ok(text) = std::fs::read_to_string(&path) {
            for e in tail_entries(&text, STARTUP_TAIL_LINES) {
                push_capped(&mut buf, e, RING_CAP);
            }
        }
        Mutex::new(State { buf, path, file_len })
    })
}

/// 启动时初始化（指定 data 目录并装载文件尾部）。幂等：重复调用只生效第一次。
pub fn init(data_dir: &Path) {
    let guard = state(Some(data_dir)).lock().unwrap_or_else(|e| e.into_inner());
    let _ = guard.path.clone();
}

/// 日志文件路径（"打开日志文件"按钮用）
pub fn file_path() -> PathBuf {
    state(None)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .path
        .clone()
}

/// 当前版本号（数据变化后自增）
pub fn rev() -> u64 {
    REV.load(Ordering::Relaxed)
}

/// 记录一条工具日志：写内存环形缓冲 + 追加落盘（best-effort，失败静默）。
pub fn tool_log(level: ToolLevel, category: &str, msg: impl AsRef<str>) {
    let ts = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let line = format_line(&ts, level, category, msg.as_ref());
    let entry = ToolLogEntry {
        ts,
        level,
        category: one_line(category),
        msg: one_line(msg.as_ref()),
    };
    let mut guard = state(None).lock().unwrap_or_else(|e| e.into_inner());
    push_capped(&mut guard.buf, entry, RING_CAP);
    REV.fetch_add(1, Ordering::Relaxed);
    // 轮转：超过 1MB 先把现有文件改名，再从空文件续写
    if should_rotate(guard.file_len) {
        let rotated = guard.path.with_file_name(ROTATED_NAME);
        if std::fs::rename(&guard.path, &rotated).is_ok() {
            guard.file_len = 0;
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&guard.path)
    {
        use std::io::Write;
        if f.write_all(line.as_bytes()).is_ok() {
            guard.file_len = guard.file_len.saturating_add(line.len() as u64);
        }
        let _ = f.flush();
    }
}

/// 取全部内存缓冲（仅在版本号变化时调用，避免每帧克隆）
pub fn snapshot() -> Vec<ToolLogEntry> {
    state(None)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .buf
        .iter()
        .cloned()
        .collect()
}

/// 清空内存缓冲（**不删日志文件**）
pub fn clear() {
    let mut guard = state(None).lock().unwrap_or_else(|e| e.into_inner());
    guard.buf.clear();
    REV.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: ToolLevel, category: &str, msg: &str) -> ToolLogEntry {
        ToolLogEntry {
            ts: "2026-10-05 12:00:00".to_string(),
            level,
            category: category.to_string(),
            msg: msg.to_string(),
        }
    }

    #[test]
    fn line_roundtrip() {
        let text = format_line("2026-10-05 12:00:00", ToolLevel::Warn, "备份", "快照创建：变化 3 个");
        let back = parse_line(&text).expect("应当能解析");
        assert_eq!(back.ts, "2026-10-05 12:00:00");
        assert_eq!(back.level, ToolLevel::Warn);
        assert_eq!(back.category, "备份");
        assert_eq!(back.msg, "快照创建：变化 3 个");
        // 反向：解析结果再格式化，仍然一致
        assert_eq!(
            format_line(&back.ts, back.level, &back.category, &back.msg),
            text
        );
    }

    #[test]
    fn line_msg_keeps_brackets_and_flattens_newlines() {
        let text = format_line("2026-10-05 12:00:00", ToolLevel::Error, "服务器", "启动失败 [code 1]\n第二行");
        let back = parse_line(&text).expect("应当能解析");
        assert_eq!(back.msg, "启动失败 [code 1] 第二行");
        assert_eq!(back.category, "服务器");
    }

    #[test]
    fn parse_rejects_foreign_lines() {
        assert!(parse_line("").is_none());
        assert!(parse_line("普通控制台日志").is_none());
        assert!(parse_line("[2026-10-05 12:00:00] [DEBUG] [服务器] x").is_none());
    }

    #[test]
    fn filter_by_level_and_category() {
        let e = entry(ToolLevel::Warn, "备份", "与上一份无变化");
        assert!(matches("全部", "全部", "", &e));
        assert!(matches("警告", "全部", "", &e));
        assert!(!matches("信息", "全部", "", &e));
        assert!(!matches("错误", "全部", "", &e));
        assert!(matches("全部", "备份", "", &e));
        assert!(!matches("全部", "隧道", "", &e));
    }

    #[test]
    fn filter_keyword_is_case_insensitive() {
        let e = entry(ToolLevel::Info, "下载", "开始下载 Paper-1.21.JAR");
        assert!(matches("全部", "全部", "paper", &e));
        assert!(matches("全部", "全部", ".jar", &e));
        assert!(!matches("全部", "全部", "fabric", &e));
        // 关键字也能命中类别
        let e2 = entry(ToolLevel::Info, "隧道", "frpc 已启动");
        assert!(matches("全部", "全部", "隧道", &e2));
    }

    #[test]
    fn ring_buffer_trims_oldest() {
        let mut buf: VecDeque<ToolLogEntry> = VecDeque::new();
        for i in 0..5 {
            push_capped(&mut buf, entry(ToolLevel::Info, "启动", &i.to_string()), 3);
        }
        let msgs: Vec<String> = buf.iter().map(|e| e.msg.clone()).collect();
        assert_eq!(msgs, vec!["2", "3", "4"]);
        // 上限为 0 时不写入（避免脏参数把缓冲清空后越界）
        push_capped(&mut buf, entry(ToolLevel::Info, "启动", "x"), 0);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn rotate_threshold() {
        assert!(!should_rotate(0));
        assert!(!should_rotate(ROTATE_BYTES));
        assert!(should_rotate(ROTATE_BYTES + 1));
    }

    #[test]
    fn tail_entries_keeps_last_lines_only() {
        let mut text = String::new();
        for i in 0..10 {
            text.push_str(&format_line(
                "2026-10-05 12:00:00",
                ToolLevel::Info,
                "启动",
                &format!("第{i}条"),
            ));
        }
        text.push_str("不是日志的一行\n");
        let got = tail_entries(&text, 3);
        let msgs: Vec<String> = got.iter().map(|e| e.msg.clone()).collect();
        assert_eq!(msgs, vec!["第7条", "第8条", "第9条"]);
    }
}
