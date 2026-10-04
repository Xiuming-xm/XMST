use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use walkdir::WalkDir;

/// 备份目录相对服务器根目录的名称
pub const BACKUP_DIR: &str = ".mcsrv_backups";
/// 快照根目录：每份快照一个子目录（未变文件用硬链接指向上一份，不占新空间）
pub const SNAPSHOTS_DIR: &str = ".mcsrv_backups/snapshots";
/// 旧版镜像目录（老配置留下的状态，仅用于清理）
pub const SNAPSHOT_DIR: &str = ".mcsrv_backups/snapshot";
/// 旧版全量基线清单（老配置留下的状态，仅用于清理）
pub const MANIFEST_FILE: &str = ".mcsrv_backups/manifest.json";
/// 旧版增量 zip 内的删除清单文件名
pub const DELTA_META: &str = ".mcsrv_delta.json";
/// 快照格式版本
pub const SNAPSHOT_VERSION: u32 = 1;
/// 快照内清单文件名
const SNAPSHOT_MANIFEST: &str = "manifest.json";
/// 快照内元信息文件名
const SNAPSHOT_META: &str = "meta.json";
/// 超过该大小视为大文件：额外记录头尾采样哈希，避免"只改了时间"就整份重拷
const LARGE_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// 大文件采样哈希读取的头/尾字节数
const SAMPLE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupKind {
    /// 旧格式：全量 zip
    Full,
    /// 旧格式：增量 zip
    Incremental,
    /// 新格式：硬链接快照目录
    Snapshot,
}

impl BackupKind {
    pub fn label(&self) -> &'static str {
        match self {
            BackupKind::Full => "全量",
            BackupKind::Incremental => "增量",
            BackupKind::Snapshot => "快照",
        }
    }
}

/// 快照触发原因
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupReason {
    /// 手动点「立即备份一次」
    Manual,
    /// 正常关服后自动
    Stop,
    /// 定时 / 其它自动
    Auto,
}

impl BackupReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackupReason::Manual => "manual",
            BackupReason::Stop => "stop",
            BackupReason::Auto => "auto",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            BackupReason::Manual => "手动",
            BackupReason::Stop => "关服",
            BackupReason::Auto => "自动",
        }
    }

    /// 从 meta.json 里的字符串还原（未知值一律按自动处理）
    pub fn from_meta(s: &str) -> BackupReason {
        match s {
            "manual" => BackupReason::Manual,
            "stop" => BackupReason::Stop,
            _ => BackupReason::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupResult {
    pub kind: BackupKind,
    /// 快照目录 / zip 文件路径
    pub path: PathBuf,
    /// 快照内文件总数（zip 为打包文件数）
    pub files: usize,
    /// 本次实际复制写入的字节数
    pub bytes: u64,
    /// false 表示与上一份相比无变化，未生成新快照
    pub changed: bool,
    /// 因被占用/锁定/读取失败而跳过的文件数
    pub skipped: usize,
    pub reason: BackupReason,
    /// 硬链接自上一份快照的文件数（不占新空间）
    pub linked: usize,
    /// 本次实际复制的文件数
    pub copied: usize,
    /// 快照逻辑总大小（清单内所有文件 size 之和）
    pub total_bytes: u64,
}

/// 单份快照的元信息（meta.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// 展示用时间，如 2026-10-03 12:00:00
    pub time: String,
    /// manual / stop / auto
    pub reason: String,
    #[serde(default)]
    pub server_name: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub files_total: usize,
    #[serde(default)]
    pub files_linked: usize,
    #[serde(default)]
    pub files_copied: usize,
    #[serde(default)]
    pub bytes_copied: u64,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default = "default_snapshot_version")]
    pub version: u32,
    /// 是否已锁定：锁定后快照内所有文件都是独立副本，内容不再随后续写入/其它快照变化
    #[serde(default)]
    pub pinned: bool,
}

fn default_snapshot_version() -> u32 {
    SNAPSHOT_VERSION
}

/// 清单里的一条文件记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// 相对服务器根目录（正斜杠）
    pub path: String,
    pub size: u64,
    /// 修改时间（100ns 精度，Unix 纪元起的纳秒）
    #[serde(default)]
    pub mtime_ns: i64,
    /// 大文件头尾采样哈希（0 = 未计算）
    #[serde(default)]
    pub hash: u64,
    /// "link"（硬链接自上一份快照）或 "copy"（本次复制）
    #[serde(default)]
    pub mode: String,
}

/// 快照清单（manifest.json）：用于下次对比与完整性校验
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    #[serde(default = "default_snapshot_version")]
    pub version: u32,
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub entries: Vec<ManifestEntry>,
}

/// 列出备份时用的展示项（快照与旧版 zip 统一表示）
#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub name: String,
    /// 快照逻辑总大小 / zip 文件大小
    pub size: u64,
    pub mtime: String,
    pub kind: BackupKind,
    /// 展示用触发原因（手动/关服/自动/旧版备份）
    pub reason: String,
    /// 快照内文件总数
    pub files_total: usize,
    /// 与上一份相比变化的文件数（本次实际复制）
    pub files_copied: usize,
    /// 本次新增字节
    pub bytes_copied: u64,
    pub is_snapshot: bool,
}

/// list_snapshots 的返回项
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub dir: PathBuf,
    pub name: String,
    pub meta: SnapshotMeta,
    /// 清单内所有文件 size 之和
    pub total_bytes: u64,
}

/// 创建快照的请求参数
pub struct SnapshotRequest<'a> {
    pub server_dir: &'a Path,
    pub server_name: &'a str,
    pub folders: &'a [String],
    pub exclude: &'a [String],
    /// 限速 MB/s，0 = 不限速
    pub throttle_mbps: f64,
    pub reason: BackupReason,
    /// true = 即使与上一份无差异也生成（手动备份）；false = 无变化直接跳过
    pub force: bool,
}

/// 回退结果：失败文件清单一并返回，便于界面提示
#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    pub restored: usize,
    /// 恢复失败的文件（相对服务器根目录）
    pub failed: Vec<String>,
    /// 被移动到 .mcsrv_trash 的旧目录
    pub trashed: Vec<PathBuf>,
    /// 恢复前自动生成的快照目录
    pub pre_snapshot: Option<PathBuf>,
    /// 非致命警告（如恢复前快照失败）
    pub warning: Option<String>,
}

/// 保留策略
#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    /// 保留最近 N 份
    pub keep_recent: usize,
    /// 每天保留 N 份
    pub keep_daily: usize,
    /// 每周保留 N 份
    pub keep_weekly: usize,
}

/// 存储总览
#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    pub snapshot_count: usize,
    pub legacy_zip_count: usize,
    pub legacy_zip_bytes: u64,
    /// 快照实际写入字节（各份 bytes_copied 之和）+ 旧版 zip 大小
    pub used_bytes: u64,
    /// 最新快照的逻辑总大小
    pub latest_total_bytes: u64,
    pub latest_files: usize,
    pub oldest: Option<String>,
    pub newest: Option<String>,
    /// 统计跨度（天）
    pub span_days: f64,
    /// 平均每天新增字节
    pub bytes_per_day: u64,
    /// 留存磁盘剩余空间预计可记录的天数
    pub estimated_days: u64,
    /// 保留策略覆盖的天数
    pub retention_days: u64,
    /// 备份所在卷剩余空间（取不到为 0）
    pub free_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeltaMeta {
    /// 删除清单（旧版增量 zip；相对服务器根目录，正斜杠）
    #[serde(default)]
    deleted: Vec<String>,
}

/// 快照清单总字节缓存：目录 → (清单文件大小 + mtime, 清单内文件 size 之和)。
/// 清单动辄上百 KB～数 MB，界面每次刷新都重新解析会明显卡顿，这里进程内复用。
static SNAP_TOTAL_CACHE: std::sync::OnceLock<
    std::sync::Mutex<HashMap<PathBuf, ((u64, u128), u64)>>,
> = std::sync::OnceLock::new();

/// 排除表默认值：日志/崩溃报告/锁文件/缓存/调试/备份自身目录，
/// 以及服务器自带临时目录 `tmp`（JNA/SQLite 会把 dll 解压到这里，不排除会让快照"变化数"虚高）
/// 和它们可能在服务器根目录留下的 `hsperfdata_*` / `jna-*` / `sqlite-*`。
pub fn default_excludes() -> Vec<String> {
    [
        "logs",
        "crash-reports",
        "session.lock",
        "*.lock",
        "cache",
        "debug",
        ".mcsrv_backups",
        ".mcsrv_trash",
        "tmp",
        "hsperfdata_*",
        "jna-*",
        "sqlite-*",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 给**已有**用户配置的排除表补齐新增的默认项（只补，不删、不改顺序）。
/// 返回是否有改动（调用方据此决定是否落盘）。若传入列表为空，则直接用 default_excludes() 填满。
///
/// 比较时忽略首尾空白与大小写（Windows 路径不区分大小写），
/// 用户已有的自定义项（哪怕与默认项同名但大小写不同）不会被重复追加。
pub fn migrate_excludes(list: &mut Vec<String>) -> bool {
    let defaults = default_excludes();
    if list.is_empty() {
        if defaults.is_empty() {
            return false;
        }
        *list = defaults;
        return true;
    }
    // 归一化：去首尾空白 + 反斜杠转正斜杠 + 小写
    let norm = |s: &str| s.trim().replace('\\', "/").to_ascii_lowercase();
    let existing: HashSet<String> = list.iter().map(|s| norm(s)).collect();
    let mut changed = false;
    for d in defaults {
        if !existing.contains(&norm(&d)) {
            list.push(d);
            changed = true;
        }
    }
    changed
}

// ---------- 创建快照 ----------

/// 生成一份硬链接快照：
/// 与上一份快照相比内容未变的文件用硬链接指过去（不占新空间），变化/新增的才实际复制；
/// 每份快照在文件层面都是完整全量（可独立回退），删除其它快照只会减少链接数。
/// 写完后做完整性校验，失败则删除该份快照并返回 Err（上一份不受影响）。
pub fn create_snapshot(req: &SnapshotRequest) -> Result<BackupResult, String> {
    let started = Instant::now();
    let server_dir = req.server_dir;
    let snap_root = server_dir.join(SNAPSHOTS_DIR);
    fs::create_dir_all(&snap_root).map_err(|e| format!("创建快照目录失败: {e}"))?;

    let excludes: Vec<String> = if req.exclude.is_empty() {
        default_excludes()
    } else {
        req.exclude.to_vec()
    };

    // 1) 采集源文件（大小 + 100ns 精度 mtime；大文件额外取头尾采样哈希）
    let mut src: Vec<(String, PathBuf, u64, i64, u64)> = Vec::new();
    for folder in req.folders {
        for (rel, path) in collect_files(server_dir, folder, &excludes) {
            if let Some((size, mtime_ns)) = file_sig(&path) {
                let hash = if size > LARGE_FILE_BYTES {
                    sample_hash(&path, size).unwrap_or(0)
                } else {
                    0
                };
                src.push((rel, path, size, mtime_ns, hash));
            }
        }
    }
    if src.is_empty() {
        return Err("没有找到可备份的文件（请检查「备份内容」与排除表）".to_string());
    }

    // 2) 上一份快照清单：本份的对比基线
    let prev = latest_snapshot(server_dir);
    let prev_dir = prev.as_ref().map(|(d, _)| d.clone());
    let prev_entries: HashMap<String, ManifestEntry> = prev
        .as_ref()
        .map(|(_, m)| {
            m.entries
                .iter()
                .map(|e| (e.path.clone(), e.clone()))
                .collect()
        })
        .unwrap_or_default();

    // 3) 先规划：哪些能硬链接、哪些必须复制（不落盘，便于"无变化直接跳过"）
    struct Planned {
        rel: String,
        path: PathBuf,
        size: u64,
        mtime_ns: i64,
        hash: u64,
        link_from: Option<PathBuf>,
    }
    let mut plan: Vec<Planned> = Vec::with_capacity(src.len());
    let mut copy_count = 0usize;
    for (rel, path, size, mtime_ns, hash) in src {
        let mut link_from = None;
        if let (Some(pd), Some(pe)) = (prev_dir.as_ref(), prev_entries.get(rel.as_str())) {
            if same_as_prev(pe, size, mtime_ns, hash) {
                let cand = pd.join(sanitize_rel(&rel));
                if cand.is_file() {
                    link_from = Some(cand);
                }
            }
        }
        if link_from.is_none() {
            copy_count += 1;
        }
        plan.push(Planned { rel, path, size, mtime_ns, hash, link_from });
    }
    let total_bytes = plan.iter().map(|p| p.size).sum::<u64>();

    // 与上一份完全一致且非强制：跳过并只回报一条"无变化"
    if copy_count == 0 && !req.force {
        return Ok(BackupResult {
            kind: BackupKind::Snapshot,
            path: snap_root,
            files: plan.len(),
            bytes: 0,
            changed: false,
            skipped: 0,
            reason: req.reason,
            linked: plan.len(),
            copied: 0,
            total_bytes,
        });
    }

    // 4) 建立快照目录（同秒触发时追加序号，避免互相覆盖）
    let now = Local::now();
    let stamp = now.format("%Y%m%d_%H%M%S").to_string();
    let mut dir = snap_root.join(&stamp);
    let mut seq = 1u32;
    while dir.exists() {
        seq += 1;
        dir = snap_root.join(format!("{stamp}_{seq}"));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("创建快照目录失败: {e}"))?;

    // 5) 执行：能链接的链接，其余复制（链接失败退化为复制并计数）
    let mut throttle = Throttle::new(req.throttle_mbps);
    let mut entries: Vec<ManifestEntry> = Vec::with_capacity(plan.len());
    let mut files_linked = 0usize;
    let mut files_copied = 0usize;
    let mut bytes_copied = 0u64;
    let mut skipped = 0usize;
    for p in &plan {
        let dst = dir.join(sanitize_rel(&p.rel));
        if let Some(parent) = dst.parent() {
            if fs::create_dir_all(parent).is_err() {
                skipped += 1;
                continue;
            }
        }
        let mut mode = "copy";
        let mut done = false;
        if let Some(from) = &p.link_from {
            match fs::hard_link(from, &dst) {
                Ok(()) => {
                    mode = "link";
                    files_linked += 1;
                    done = true;
                }
                // 跨卷 / 文件系统不支持硬链接：退化为复制（下面继续走复制分支）
                Err(_) => {}
            }
        }
        if !done {
            match copy_throttled(&p.path, &dst, &mut throttle) {
                Ok(n) => {
                    files_copied += 1;
                    bytes_copied = bytes_copied.saturating_add(n);
                }
                Err(_) => {
                    skipped += 1;
                    let _ = fs::remove_file(&dst);
                    continue;
                }
            }
        }
        entries.push(ManifestEntry {
            path: p.rel.clone(),
            size: p.size,
            mtime_ns: p.mtime_ns,
            hash: p.hash,
            mode: mode.to_string(),
        });
    }

    // 6) 写清单与元信息
    let manifest = SnapshotManifest {
        version: SNAPSHOT_VERSION,
        time: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        entries,
    };
    if let Err(e) = write_json(&dir.join(SNAPSHOT_MANIFEST), &manifest) {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("写入快照清单失败，已删除该份快照：{e}"));
    }
    let meta = SnapshotMeta {
        time: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        reason: req.reason.as_str().to_string(),
        server_name: req.server_name.to_string(),
        duration_ms: started.elapsed().as_millis() as u64,
        files_total: manifest.entries.len(),
        files_linked,
        files_copied,
        bytes_copied,
        folders: req.folders.to_vec(),
        version: SNAPSHOT_VERSION,
        pinned: false,
    };
    if let Err(e) = write_json(&dir.join(SNAPSHOT_META), &meta) {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("写入快照元信息失败，已删除该份快照：{e}"));
    }

    // 7) 完整性校验：清单里每个文件都必须存在且大小一致，否则整份作废
    if let Err(e) = verify_snapshot(&dir, &manifest) {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("快照完整性校验失败，已删除该份快照：{e}"));
    }

    Ok(BackupResult {
        kind: BackupKind::Snapshot,
        path: dir,
        files: manifest.entries.len(),
        bytes: bytes_copied,
        changed: true,
        skipped,
        reason: req.reason,
        linked: files_linked,
        copied: files_copied,
        total_bytes,
    })
}

/// 校验快照：清单中每个文件存在于快照目录且大小一致
fn verify_snapshot(dir: &Path, manifest: &SnapshotManifest) -> Result<(), String> {
    if manifest.entries.is_empty() {
        return Err("清单为空".to_string());
    }
    let mut bad = 0usize;
    for e in &manifest.entries {
        let p = dir.join(sanitize_rel(&e.path));
        match fs::metadata(&p) {
            Ok(m) if m.is_file() && m.len() == e.size => {}
            _ => bad += 1,
        }
        if bad > 0 {
            return Err(format!("文件缺失或大小不符：{}", e.path));
        }
    }
    Ok(())
}

/// 一个待备份文件是否可以直接硬链接上一份：size 相同，且 mtime 相同
/// （mtime 不同时，大文件用头尾采样哈希兜底：只被 touch 过就不必重拷）
fn same_as_prev(pe: &ManifestEntry, size: u64, mtime_ns: i64, hash: u64) -> bool {
    if pe.size != size {
        return false;
    }
    if pe.mtime_ns == mtime_ns {
        return true;
    }
    hash != 0 && pe.hash == hash
}

/// 取最近一份快照（按目录名倒序 = 时间倒序）及其清单
fn latest_snapshot(server_dir: &Path) -> Option<(PathBuf, SnapshotManifest)> {
    let mut list = list_snapshots(server_dir);
    list.sort_by(|a, b| b.name.cmp(&a.name));
    for s in list {
        if let Some(m) = read_manifest(&s.dir) {
            return Some((s.dir, m));
        }
    }
    None
}

pub fn list_snapshots(server_dir: &Path) -> Vec<SnapshotInfo> {
    let snap_root = server_dir.join(SNAPSHOTS_DIR);
    let mut out = Vec::new();
    let rd = match fs::read_dir(&snap_root) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for e in rd.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let meta = read_meta(&dir);
        // 清单可能很大（上万文件）：进程内按"清单大小 + 修改时间"缓存总字节，避免每次刷新都重新解析
        let total_bytes = cached_total_bytes(&dir);
        // meta.json 缺失（中断/手工改动）：用目录时间退化出可用元信息，保证仍能列表与回退
        let meta = meta.unwrap_or_else(|| fallback_meta(&dir));
        out.push(SnapshotInfo { dir, name, meta, total_bytes });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 清单总字节缓存：清单文件大小 + 修改时间不变即复用（快照写入后不再变化）
fn cached_total_bytes(dir: &Path) -> u64 {
    let mf = dir.join(SNAPSHOT_MANIFEST);
    let stamp = fs::metadata(&mf).ok().and_then(|m| {
        m.modified()
            .ok()
            .map(|t| (m.len(), t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)))
    });
    let cache = SNAP_TOTAL_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Some(stamp) = stamp {
        let g = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_stamp, total)) = g.get(dir) {
            if *cached_stamp == stamp {
                return *total;
            }
        }
        drop(g);
        let total = read_manifest(dir)
            .map(|m| m.entries.iter().map(|e| e.size).sum::<u64>())
            .unwrap_or(0);
        let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() > 512 {
            g.clear();
        }
        g.insert(dir.to_path_buf(), (stamp, total));
        return total;
    }
    // 清单不存在：按 0 处理（调用方会走 meta 缺失的退化分支）
    0
}

fn read_manifest(dir: &Path) -> Option<SnapshotManifest> {
    let s = fs::read_to_string(dir.join(SNAPSHOT_MANIFEST)).ok()?;
    serde_json::from_str::<SnapshotManifest>(&s).ok()
}

fn read_meta(dir: &Path) -> Option<SnapshotMeta> {
    let s = fs::read_to_string(dir.join(SNAPSHOT_META)).ok()?;
    serde_json::from_str::<SnapshotMeta>(&s).ok()
}

/// meta.json 缺失/损坏时的退化元信息：时间取目录修改时间，文件总数取清单条目数。
/// 字段缺失一律走默认值（缺 pinned 视为未锁定），保证老快照仍可用。
fn fallback_meta(dir: &Path) -> SnapshotMeta {
    let time = fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            let dt: chrono::DateTime<Local> = t.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_default();
    let files_total = read_manifest(dir).map(|m| m.entries.len()).unwrap_or(0);
    SnapshotMeta {
        time,
        reason: BackupReason::Auto.as_str().to_string(),
        server_name: String::new(),
        duration_ms: 0,
        files_total,
        files_linked: 0,
        files_copied: 0,
        bytes_copied: 0,
        folders: Vec::new(),
        version: SNAPSHOT_VERSION,
        pinned: false,
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v).map_err(|e| format!("序列化失败: {e}"))?;
    fs::write(path, s.as_bytes()).map_err(|e| format!("写入失败: {e}"))
}

/// 列出全部备份（新快照 + 旧版 zip），时间升序
pub fn list_backups(server_dir: &Path) -> Vec<BackupInfo> {
    let mut out = Vec::new();
    for s in list_snapshots(server_dir) {
        let reason = BackupReason::from_meta(&s.meta.reason).label().to_string();
        out.push(BackupInfo {
            path: s.dir,
            name: s.name,
            size: s.total_bytes,
            mtime: s.meta.time.clone(),
            kind: BackupKind::Snapshot,
            reason,
            files_total: s.meta.files_total,
            files_copied: s.meta.files_copied,
            bytes_copied: s.meta.bytes_copied,
            is_snapshot: true,
        });
    }
    // 旧版 zip：不能被破坏，仍需在列表里可见可回退
    let backup_root = server_dir.join(BACKUP_DIR);
    if let Ok(rd) = fs::read_dir(&backup_root) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.extension().map(|x| x == "zip").unwrap_or(false) {
                continue;
            }
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let md = fs::metadata(&p);
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = md
                .and_then(|m| m.modified())
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<Local> = t.into();
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                })
                .unwrap_or_default();
            let kind = if name.contains("_full.zip") {
                BackupKind::Full
            } else {
                BackupKind::Incremental
            };
            out.push(BackupInfo {
                path: p,
                name,
                size,
                mtime,
                kind,
                reason: "旧版备份".to_string(),
                files_total: 0,
                files_copied: 0,
                bytes_copied: size,
                is_snapshot: false,
            });
        }
    }
    out.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.name.cmp(&b.name)));
    out
}

/// 删除单份备份：快照删除整目录（被其它快照硬链接引用的数据仍由其它链接保留），
/// 旧版 zip 删除文件。目录/文件位置必须落在备份目录内。
pub fn delete_backup(server_dir: &Path, target: &Path) -> Result<(), String> {
    let backup_root = server_dir.join(BACKUP_DIR);
    let snap_root = server_dir.join(SNAPSHOTS_DIR);
    if target.is_dir() {
        if target.parent() != Some(snap_root.as_path()) {
            return Err("拒绝删除快照目录之外的路径".to_string());
        }
        fs::remove_dir_all(target).map_err(|e| format!("删除快照失败: {e}"))
    } else {
        if target.parent() != Some(backup_root.as_path()) {
            return Err("拒绝删除备份目录外的文件".to_string());
        }
        fs::remove_file(target).map_err(|e| format!("删除备份失败: {e}"))
    }
}

// ---------- 回退 ----------

/// 回退：把目标备份（新快照目录 或 旧版 zip）恢复到服务器根目录。
/// 只恢复 `scope` 指定的顶层目录；恢复前把现有内容整体改名到 `.mcsrv_trash/`，
/// **改名失败立即返回 Err 并中止**（绝不带着未备份的原数据继续覆盖）。
/// 返回失败文件清单，供界面提示"N 个文件恢复失败"。
pub fn restore_backup(
    server_dir: &Path,
    target: &Path,
    scope: &[String],
) -> Result<RestoreReport, String> {
    if !target.exists() {
        return Err("目标备份不存在".to_string());
    }
    let scope: Vec<String> = scope
        .iter()
        .map(|s| s.trim().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .collect();
    if scope.is_empty() {
        return Err("未选择恢复范围".to_string());
    }

    // 0) 先确认目标备份可读（清单/压缩包能解析），再动原数据：
    //    否则会出现"原数据已移走、备份却打不开"的最坏情况
    if target.is_dir() {
        if read_manifest(target).is_none() {
            return Err("快照清单缺失或损坏，已中止回退（原数据未改动）".to_string());
        }
    } else {
        let f = fs::File::open(target).map_err(|e| format!("打开备份失败: {e}"))?;
        zip::ZipArchive::new(f)
            .map_err(|e| format!("备份压缩包损坏或无法读取，已中止回退（原数据未改动）: {e}"))?;
    }

    // 1) 先留档：现有内容改名到 .mcsrv_trash/<目录>_<时间戳_纳秒>_<序号>
    let trash_root = server_dir.join(".mcsrv_trash");
    fs::create_dir_all(&trash_root).map_err(|e| format!("创建回收目录失败: {e}"))?;
    let mut report = RestoreReport::default();
    let stamp = Local::now().format("%Y%m%d_%H%M%S_%f").to_string();
    let mut seq = 0u32;
    for folder in &scope {
        let src = server_dir.join(sanitize_rel(folder));
        if !src.exists() {
            continue;
        }
        let dir_name = sanitize_rel(folder)
            .to_string_lossy()
            .replace(['\\', '/'], "_");
        let dst = loop {
            seq += 1;
            let cand = trash_root.join(format!("{dir_name}_{stamp}_{seq}"));
            if !cand.exists() {
                break cand;
            }
        };
        // 关键：改名必须成功；失败说明原数据没被保住，立即中止，不再覆盖
        fs::rename(&src, &dst).map_err(|e| {
            format!("移动原数据到回收目录失败（已中止回退，未覆盖任何文件）: {folder} -> {e}")
        })?;
        report.trashed.push(dst);
    }

    // 2) 应用备份内容
    if target.is_dir() {
        apply_snapshot(target, server_dir, &scope, &mut report)?;
    } else {
        // 旧版 zip：按 全量基线 + 其后增量 的顺序应用
        let chain = build_zip_chain(server_dir, target)?;
        for zp in &chain {
            apply_zip(server_dir, zp, &scope, &mut report)?;
        }
        // 旧版镜像状态已失效：清掉，避免下次对比继续用旧基线
        let _ = fs::remove_dir_all(server_dir.join(SNAPSHOT_DIR));
        let _ = fs::remove_file(server_dir.join(MANIFEST_FILE));
    }

    Ok(report)
}

/// 旧版 zip 恢复链：目标之前的最近一份全量 + 其后到目标（含）的所有增量
fn build_zip_chain(server_dir: &Path, target: &Path) -> Result<Vec<PathBuf>, String> {
    let mut zips: Vec<BackupInfo> = list_backups(server_dir)
        .into_iter()
        .filter(|b| !b.is_snapshot)
        .collect();
    zips.sort_by(|a, b| a.name.cmp(&b.name));
    let Some(idx) = zips.iter().position(|b| b.path == target) else {
        return Err("目标备份不在备份目录中".to_string());
    };
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut found_full = false;
    for b in zips.iter().take(idx + 1) {
        if b.kind == BackupKind::Full {
            chain.clear();
            found_full = true;
        }
        chain.push(b.path.clone());
    }
    if !found_full {
        return Err("找不到该增量备份对应的全量基线，无法回退".to_string());
    }
    Ok(chain)
}

/// 应用新格式快照：按清单把文件复制回服务器根目录
fn apply_snapshot(
    snapshot_dir: &Path,
    server_dir: &Path,
    scope: &[String],
    report: &mut RestoreReport,
) -> Result<(), String> {
    let manifest = read_manifest(snapshot_dir).ok_or("快照清单缺失或损坏，无法回退")?;
    for e in &manifest.entries {
        if !in_scope(&e.path, scope) {
            continue;
        }
        let src = snapshot_dir.join(sanitize_rel(&e.path));
        let dst = server_dir.join(sanitize_rel(&e.path));
        if !src.is_file() {
            report.failed.push(e.path.clone());
            continue;
        }
        if let Some(parent) = dst.parent() {
            if fs::create_dir_all(parent).is_err() {
                report.failed.push(e.path.clone());
                continue;
            }
        }
        match fs::copy(&src, &dst) {
            Ok(_) => report.restored += 1,
            Err(_) => report.failed.push(e.path.clone()),
        }
    }
    Ok(())
}

/// 应用单个旧版 zip：先处理删除清单，再流式解压覆盖（只处理 scope 内的路径）
fn apply_zip(
    server_dir: &Path,
    zip_path: &Path,
    scope: &[String],
    report: &mut RestoreReport,
) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("打开备份失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("读取备份失败: {e}"))?;

    // 第一遍：删除清单（先删后覆盖，保证删除的文件确实消失）
    for i in 0..archive.len() {
        let mut f = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if f.name() != DELTA_META {
            continue;
        }
        let mut json = String::new();
        if f.read_to_string(&mut json).is_err() {
            continue;
        }
        if let Ok(meta) = serde_json::from_str::<DeltaMeta>(&json) {
            for rel in meta.deleted {
                if !in_scope(&rel, scope) {
                    continue;
                }
                let del_path = server_dir.join(sanitize_rel(&rel));
                if del_path.starts_with(server_dir) {
                    let _ = fs::remove_file(&del_path);
                }
            }
        }
    }

    // 第二遍：流式解压（不再整文件读入内存）
    for i in 0..archive.len() {
        let mut f = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let name = f.name().to_string();
        if name == DELTA_META || !in_scope(&name, scope) {
            continue;
        }
        let out_path = server_dir.join(sanitize_rel(&name));
        if !out_path.starts_with(server_dir) {
            continue;
        }
        if let Some(parent) = out_path.parent() {
            if fs::create_dir_all(parent).is_err() {
                report.failed.push(name);
                continue;
            }
        }
        let mut out = match fs::File::create(&out_path) {
            Ok(o) => o,
            Err(_) => {
                report.failed.push(name);
                continue;
            }
        };
        match std::io::copy(&mut f, &mut out) {
            Ok(_) => report.restored += 1,
            Err(_) => report.failed.push(name),
        }
    }

    Ok(())
}

// ---------- 保留策略 / 存储总览 ----------

/// 按保留策略清理快照：保留最近 N 份 + 每天 N 份 + 每周 N 份，其余删除。
/// 硬链接下删除某份快照只减少链接数，其它快照的数据仍然完整。
/// 永远保留最新一份；旧版 zip 不在清理范围内。
pub fn apply_retention(server_dir: &Path, policy: &RetentionPolicy) -> Result<usize, String> {
    let mut snaps = list_snapshots(server_dir);
    if snaps.len() <= 1 {
        return Ok(0);
    }
    // 新的在前
    snaps.sort_by(|a, b| b.name.cmp(&a.name));

    let mut keep: HashSet<String> = HashSet::new();
    // 最新一份永远保留
    if let Some(first) = snaps.first() {
        keep.insert(first.name.clone());
    }
    // 最近 N 份
    for s in snaps.iter().take(policy.keep_recent.max(1)) {
        keep.insert(s.name.clone());
    }
    // 每天 N 份（按目录名前 8 位日期，每天最新一份）
    let mut days = 0usize;
    let mut seen_days: HashSet<String> = HashSet::new();
    for s in &snaps {
        let day: String = s.name.chars().take(8).collect();
        if day.len() != 8 {
            continue;
        }
        if seen_days.contains(&day) {
            continue;
        }
        if days >= policy.keep_daily {
            break;
        }
        seen_days.insert(day);
        keep.insert(s.name.clone());
        days += 1;
    }
    // 每周 N 份（ISO 周，每周最新一份）
    let mut weeks = 0usize;
    let mut seen_weeks: HashSet<(i32, u32)> = HashSet::new();
    for s in &snaps {
        let day: String = s.name.chars().take(8).collect();
        let Ok(date) = chrono::NaiveDate::parse_from_str(&day, "%Y%m%d") else {
            continue;
        };
        let iw = date.iso_week();
        let key = (iw.year(), iw.week());
        if seen_weeks.contains(&key) {
            continue;
        }
        if weeks >= policy.keep_weekly {
            break;
        }
        seen_weeks.insert(key);
        keep.insert(s.name.clone());
        weeks += 1;
    }

    let mut removed = 0usize;
    for s in &snaps {
        if keep.contains(&s.name) {
            continue;
        }
        // ★ 已锁定的快照是"防篡改归档"（见 pin_snapshot）：保留策略一律跳过，不参与清理。
        //    否则归档随时可能被自动清理删掉，锁定就失去意义。
        if s.meta.pinned {
            continue;
        }
        match fs::remove_dir_all(&s.dir) {
            Ok(()) => removed += 1,
            Err(e) => return Err(format!("清理快照 {} 失败: {e}", s.name)),
        }
    }
    Ok(removed)
}

// ---------- 快照锁定（防篡改归档） ----------

/// 复制 `src` 到 `dst`：先写同目录的 `<dst>.tmp`，保持修改时间后再改名覆盖。
/// 用于①锁定快照时把硬链接实化为独立副本（此时 dst 就是快照内的原文件）
/// ②同步到目标目录。改名覆盖而不是"删除 + 重建"，可保证任一瞬间目标路径上都是一个完整文件。
fn copy_replace(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let mut tmp_name = dst.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    // Windows 上目标带只读属性时"改名覆盖"会直接失败（ACCESS_DENIED），先临时去掉
    let dst_was_readonly = readonly_attr(dst);
    if dst_was_readonly {
        set_readonly_attr(dst, false);
    }
    let n = match fs::copy(src, &tmp) {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            if dst_was_readonly {
                set_readonly_attr(dst, true);
            }
            return Err(e);
        }
    };
    // 保持 100ns 精度的修改时间：清单/远端同步都依赖这个时间戳判定"未变"
    if let Ok(m) = fs::metadata(src) {
        let ft = filetime::FileTime::from_last_modification_time(&m);
        let _ = filetime::set_file_mtime(&tmp, ft);
    }
    if let Err(e) = fs::rename(&tmp, dst) {
        let _ = fs::remove_file(&tmp);
        if dst_was_readonly {
            set_readonly_attr(dst, true);
        }
        return Err(e);
    }
    // 覆盖后与源文件保持一致的只读属性（硬链接共享属性，实化后必须显式对齐）
    set_readonly_attr(dst, readonly_attr(src));
    Ok(n)
}

/// 是否带只读属性（取不到按 false）
fn readonly_attr(p: &Path) -> bool {
    fs::metadata(p)
        .map(|m| m.permissions().readonly())
        .unwrap_or(false)
}

/// 设置只读属性。非 Windows 平台上 readonly() 表示"缺少写权限位"，
/// 改写它会连带改变权限语义，因此只在 Windows 上实际执行
#[cfg(windows)]
fn set_readonly_attr(p: &Path, ro: bool) {
    if let Ok(m) = fs::metadata(p) {
        let mut perm = m.permissions();
        if perm.readonly() != ro {
            perm.set_readonly(ro);
            let _ = fs::set_permissions(p, perm);
        }
    }
}

#[cfg(not(windows))]
fn set_readonly_attr(_p: &Path, _ro: bool) {}

/// 读取某份快照是否已锁定（meta.json 不存在/损坏 → false）
pub fn is_snapshot_pinned(snapshot_dir: &Path) -> bool {
    read_meta(snapshot_dir).map(|m| m.pinned).unwrap_or(false)
}

/// 锁定快照：把快照目录中所有文件实化为独立副本（先写 `<name>.tmp` 再 rename 覆盖），
/// 从而与源文件/其它快照解耦；已锁定则直接返回 Ok(0)。返回实际实化的文件数。
///
/// 说明：硬链接快照与源文件共享数据，源文件被"原地改写"（长度不变）时旧快照内容会跟着变；
/// 锁定后每份文件都是独立数据块，快照从此成为防篡改归档（代价是占用空间）。
pub fn pin_snapshot(snapshot_dir: &Path) -> Result<u64, String> {
    if !snapshot_dir.is_dir() {
        return Err(format!("快照目录不存在: {}", snapshot_dir.display()));
    }
    if is_snapshot_pinned(snapshot_dir) {
        return Ok(0);
    }
    // 先把文件清单收集完再逐个实化：实化过程会在同目录生成 `*.tmp`，
    // 边遍历边处理会把这些临时文件也当成快照内容。
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(snapshot_dir).follow_links(false) {
        match entry {
            Ok(e) if e.file_type().is_file() => files.push(e.path().to_path_buf()),
            Ok(_) => {}
            Err(e) => return Err(format!("遍历快照目录失败: {e}")),
        }
    }
    let mut done = 0u64;
    for f in &files {
        if let Err(e) = copy_replace(f, f) {
            let rel = f.strip_prefix(snapshot_dir).unwrap_or(f);
            // 失败时 pinned 仍为 false，重新调用即可续做（已实化的文件再实化一次也无害）
            return Err(format!(
                "锁定失败（{}）: {e}",
                rel.to_string_lossy().replace('\\', "/")
            ));
        }
        done += 1;
    }
    // meta.json 必须放在**文件实化之后**更新：否则刚写好的 pinned 可能又被当成普通文件实化一遍
    let mut meta = read_meta(snapshot_dir).unwrap_or_else(|| fallback_meta(snapshot_dir));
    meta.pinned = true;
    write_json(&snapshot_dir.join(SNAPSHOT_META), &meta)
        .map_err(|e| format!("快照内容已实化，但写入 meta.json 失败: {e}"))?;
    Ok(done)
}

/// 取消锁定：仅把 meta.json 的 pinned 置 false（不回收空间；
/// 说明：已实化的文件保持实化，直到该快照被删除或保留策略清理）。
pub fn unpin_snapshot(snapshot_dir: &Path) -> Result<(), String> {
    if !snapshot_dir.is_dir() {
        return Err(format!("快照目录不存在: {}", snapshot_dir.display()));
    }
    let mut meta = read_meta(snapshot_dir)
        .ok_or_else(|| "meta.json 缺失或损坏，无法取消锁定".to_string())?;
    if !meta.pinned {
        return Ok(());
    }
    meta.pinned = false;
    write_json(&snapshot_dir.join(SNAPSHOT_META), &meta)
        .map_err(|e| format!("写入 meta.json 失败: {e}"))
}

// ---------- 快照远端转存（本地目录 / UNC） ----------

/// 把一份快照目录同步到目标根目录下：`<dest_root>\<snapshot_name>\`
/// 逐文件按 "相对路径 + size + 100ns mtime" 判断是否需要复制（已存在且一致则跳过）。
/// 返回本次复制的字节数。目标可为本地目录或 UNC（\\nas\share）；WebDAV 不在本函数范围内。
///
/// 不做超时/重试：网络盘断连等情况由调用方决定是否重跑（本函数可安全重复执行）。
pub fn sync_snapshot_dir(snapshot_dir: &Path, dest_root: &Path) -> Result<u64, String> {
    if !snapshot_dir.is_dir() {
        return Err(format!("快照目录不存在: {}", snapshot_dir.display()));
    }
    let name = snapshot_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return Err("无法确定快照目录名".to_string());
    }
    let dest = dest_root.join(&name);
    fs::create_dir_all(&dest).map_err(|e| format!("创建目标目录失败（{}）: {e}", dest.display()))?;

    let mut copied = 0u64;
    for entry in WalkDir::new(snapshot_dir).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(format!("遍历快照目录失败: {e}")),
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let src = entry.path();
        let rel = src.strip_prefix(snapshot_dir).unwrap_or(src);
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        let dst = dest.join(rel);
        // 已存在且 size + 100ns mtime 一致 → 跳过（重跑时只补差量）
        if let (Some((ssz, smt)), Some((dsz, dmt))) = (file_sig(src), file_sig(&dst)) {
            if ssz == dsz && smt == dmt {
                continue;
            }
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败（{rel_s}）: {e}"))?;
        }
        if let Err(e) = copy_replace(src, &dst) {
            return Err(format!("同步失败（{rel_s}）: {e}"));
        }
        copied = copied.saturating_add(fs::metadata(src).map(|m| m.len()).unwrap_or(0));
    }
    Ok(copied)
}

/// 存储总览：占用、份数、可回滚范围、按变化速度估算的可保留天数
pub fn storage_stats(server_dir: &Path, policy: &RetentionPolicy) -> StorageStats {
    let snaps = list_snapshots(server_dir);
    let mut st = StorageStats {
        snapshot_count: snaps.len(),
        ..Default::default()
    };
    for s in &snaps {
        st.used_bytes = st.used_bytes.saturating_add(s.meta.bytes_copied);
    }
    // 旧版 zip 占用
    let backup_root = server_dir.join(BACKUP_DIR);
    if let Ok(rd) = fs::read_dir(&backup_root) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.extension().map(|x| x == "zip").unwrap_or(false) {
                continue;
            }
            let len = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            st.legacy_zip_count += 1;
            st.legacy_zip_bytes = st.legacy_zip_bytes.saturating_add(len);
            st.used_bytes = st.used_bytes.saturating_add(len);
        }
    }
    if let Some(newest) = snaps.last() {
        st.latest_total_bytes = newest.total_bytes;
        st.latest_files = newest.meta.files_total;
        st.newest = Some(newest.meta.time.clone());
    }
    if let Some(oldest) = snaps.first() {
        st.oldest = Some(oldest.meta.time.clone());
    }
    // 跨度与日均变化量（用写入时间最新的两份推速度，避免首次快照的巨量影响）
    if snaps.len() >= 2 {
        let last = snaps.last().map(|s| s.meta.time.clone()).unwrap_or_default();
        let first = snaps.first().map(|s| s.meta.time.clone()).unwrap_or_default();
        let t_last = parse_meta_time(&last);
        let t_first = parse_meta_time(&first);
        if let (Some(a), Some(b)) = (t_first, t_last) {
            let days = (b - a).num_seconds() as f64 / 86400.0;
            st.span_days = days;
            let written: u64 = snaps
                .iter()
                .skip(1)
                .map(|s| s.meta.bytes_copied)
                .fold(0u64, |a, b| a.saturating_add(b));
            st.bytes_per_day = if days > 0.05 {
                (written as f64 / days) as u64
            } else {
                written
            };
        }
    }
    // 保留策略覆盖天数：按最近若干份的平均间隔推算
    if snaps.len() >= 2 {
        let t_last = snaps.last().and_then(|s| parse_meta_time(&s.meta.time));
        let t_first = snaps.first().and_then(|s| parse_meta_time(&s.meta.time));
        if let (Some(a), Some(b)) = (t_first, t_last) {
            let span = (b - a).num_seconds() as f64 / 86400.0;
            let per = span / (snaps.len() - 1) as f64;
            st.retention_days = (per * policy.keep_recent.max(1) as f64).round() as u64;
        }
    } else {
        st.retention_days = policy.keep_recent.max(1) as u64;
    }
    st.free_bytes = free_space_bytes(server_dir);
    st.estimated_days = if st.bytes_per_day > 0 && st.free_bytes > 0 {
        (st.free_bytes / st.bytes_per_day).min(3650)
    } else {
        st.retention_days
    };
    if st.retention_days == 0 {
        st.retention_days = policy.keep_recent.max(1) as u64;
    }
    st
}

fn parse_meta_time(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
}

/// 备份所在卷剩余空间；取不到返回 0
fn free_space_bytes(path: &Path) -> u64 {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let mut avail: u64 = 0;
        // 第 2/3 个出参不需要：传空指针，避免多写两个临时变量
        let ok = unsafe {
            winapi::um::fileapi::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut avail as *mut u64 as *mut winapi::um::winnt::ULARGE_INTEGER,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            return avail;
        }
        0
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        0
    }
}

// ---------- 退出判定（关服快照） ----------

/// 日志尾部是否出现正常关服标志（"Stopping server" / "Saving worlds"）
pub fn log_shows_normal_stop(log_tail: &str) -> bool {
    let n = log_tail.chars().count();
    let tail: String = log_tail.chars().skip(n.saturating_sub(4000)).collect();
    tail.contains("Stopping server") || tail.contains("Saving worlds") || tail.contains("Saving chunks")
}

/// 三重信号判定"正常关闭"：日志出现关服标志 或 退出码为 0，且没有新的崩溃报告。
/// 异常退出/强杀/有新崩溃报告 → false（默认不备份）
pub fn looks_normal_shutdown(log_tail: &str, exit_code: Option<i32>, new_crash_report: bool) -> bool {
    if new_crash_report {
        return false;
    }
    log_shows_normal_stop(log_tail) || exit_code == Some(0)
}

/// 是否有比 `since` 更新的崩溃报告（crash-reports/*.txt 与服务器根目录的 hs_err_pid*.log）
pub fn has_new_crash_report(server_dir: &Path, since: SystemTime) -> bool {
    let crash_dir = server_dir.join("crash-reports");
    if let Ok(rd) = fs::read_dir(&crash_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            if let Ok(m) = fs::metadata(&p) {
                if let Ok(t) = m.modified() {
                    if t > since {
                        return true;
                    }
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir(server_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if name.starts_with("hs_err_pid") && name.ends_with(".log") {
                if let Ok(m) = fs::metadata(&p) {
                    if let Ok(t) = m.modified() {
                        if t > since {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

// ---------- 收集 / 对比 ----------

/// 收集某个顶层目录（或单个文件）下应备份的文件：(相对服务器根目录, 绝对路径)
fn collect_files(server_dir: &Path, folder: &str, excludes: &[String]) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let rel_root = folder.trim().replace('\\', "/");
    if rel_root.is_empty() {
        return out;
    }
    let src = server_dir.join(sanitize_rel(&rel_root));
    let meta = match fs::symlink_metadata(&src) {
        Ok(m) => m,
        Err(_) => return out,
    };
    if meta.is_file() {
        if !is_excluded(&rel_root, file_name_of(&src).as_str(), excludes) {
            out.push((rel_root, src));
        }
        return out;
    }
    if !meta.is_dir() {
        return out;
    }
    for entry in WalkDir::new(&src)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let p = e.path();
            if p == src {
                return true;
            }
            // 备份目录自身、备份/回收目录一律不遍历
            if e.file_type().is_dir() && has_backup_dir_segment(p) {
                return false;
            }
            let rel = p.strip_prefix(server_dir).unwrap_or(p);
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            !is_excluded(&rel_s, file_name_of(p).as_str(), excludes)
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(server_dir).unwrap_or(path);
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        if is_excluded(&rel_s, file_name_of(path).as_str(), excludes) {
            continue;
        }
        out.push((rel_s, path.to_path_buf()));
    }
    out
}

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 排除表匹配：
/// - 以 `*` 开头（`*.lock`）：按文件名/相对路径结尾匹配；
/// - 以 `*` 结尾（`hsperfdata_*` / `jna-*` / `sqlite-*`）：按文件名或任一路径段**前缀**匹配；
/// - 含 `/` 的模式按相对路径匹配；
/// - 其余按"任一路径段精确相等"匹配（目录名，如 logs / tmp / session.lock）
fn is_excluded(rel: &str, name: &str, excludes: &[String]) -> bool {
    let rel_l = rel.to_ascii_lowercase();
    let name_l = name.to_ascii_lowercase();
    for pat in excludes {
        let pat = pat.trim().replace('\\', "/").to_ascii_lowercase();
        if pat.is_empty() {
            continue;
        }
        if let Some(suffix) = pat.strip_prefix('*') {
            if !suffix.is_empty() && (name_l.ends_with(suffix) || rel_l.ends_with(suffix)) {
                return true;
            }
            continue;
        }
        if let Some(prefix) = pat.strip_suffix('*') {
            let prefix = prefix.trim_end_matches('/');
            if !prefix.is_empty()
                && (name_l.starts_with(prefix) || rel_l.split('/').any(|s| s.starts_with(prefix)))
            {
                return true;
            }
            continue;
        }
        if pat.contains('/') {
            if rel_l == pat || rel_l.starts_with(&format!("{pat}/")) || rel_l.contains(&format!("/{pat}/")) {
                return true;
            }
            continue;
        }
        if name_l == pat || rel_l.split('/').any(|s| s == pat) {
            return true;
        }
    }
    false
}

/// 相对路径是否落在恢复范围（scope 为空 = 全部）
fn in_scope(rel: &str, scope: &[String]) -> bool {
    if scope.is_empty() {
        return true;
    }
    let rel_l = rel.replace('\\', "/").to_ascii_lowercase();
    scope.iter().any(|s| {
        let s = s.trim().replace('\\', "/").to_ascii_lowercase();
        !s.is_empty() && (rel_l == s || rel_l.starts_with(&format!("{s}/")))
    })
}

/// 防止路径穿越
fn sanitize_rel(name: &str) -> PathBuf {
    let mut p = PathBuf::new();
    for comp in Path::new(name).components() {
        use std::path::Component;
        if let Component::Normal(c) = comp {
            p.push(c);
        }
    }
    p
}

/// 文件签名：(长度, 100ns 精度 mtime 的纳秒值)。
/// 旧实现用小文件内容哈希 + 大文件"秒级 mtime"，秒级精度会让同一秒内的改写漏检；
/// 现在统一用 `filetime::FileTime` 的 100ns 精度，配合大文件头尾采样哈希。
fn file_sig(p: &Path) -> Option<(u64, i64)> {
    let m = fs::metadata(p).ok()?;
    if !m.is_file() {
        return None;
    }
    let ft = filetime::FileTime::from_last_modification_time(&m);
    // Windows 的 FILETIME 本身是 100ns 粒度：unix_seconds() 去掉纪元偏移，
    // nanoseconds() 给出亚秒部分，两者相加得到 100ns 精度的修改时间
    let ns = (ft.unix_seconds() as i128)
        .saturating_mul(1_000_000_000)
        .saturating_add(ft.nanoseconds() as i128);
    let ns = if ns > i64::MAX as i128 {
        i64::MAX
    } else if ns < i64::MIN as i128 {
        i64::MIN
    } else {
        ns as i64
    };
    Some((m.len(), ns))
}

/// 大文件头尾各取 64KB 计算 FNV-1a（内容未变但被 touch 时避免整份重拷）
fn sample_hash(p: &Path, len: u64) -> Option<u64> {
    let mut f = fs::File::open(p).ok()?;
    let mut head = vec![0u8; SAMPLE_BYTES.min(len as usize)];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let mut h = fnv1a64(&head);
    if len > (SAMPLE_BYTES as u64) * 2 {
        use std::io::Seek;
        if f.seek(std::io::SeekFrom::End(-(SAMPLE_BYTES as i64))).is_ok() {
            let mut tail = vec![0u8; SAMPLE_BYTES];
            if let Ok(n) = f.read(&mut tail) {
                tail.truncate(n);
                h = h.wrapping_mul(0x100000001b3) ^ fnv1a64(&tail);
            }
        }
    }
    Some(h)
}

fn fnv1a64(buf: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in buf {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 判断路径中是否包含名为 backup 的目录段（不区分大小写）。
/// 用于备份遍历时整棵跳过：避免把模组/工具自身生成的 backup 文件夹再次打进备份。
fn has_backup_dir_segment(p: &Path) -> bool {
    // ★ 只检查路径的**最后两段**，且只认精确的保留目录名。
    //
    // 旧实现用 `seg.to_ascii_lowercase().contains("backup")` 扫描**绝对路径的所有段**，
    // 而快照目录本身叫 `.mcsrv_backups`（含 "backup"）→ 遍历快照时把所有子目录剪掉，
    // 镜像里只剩第一层文件，对比时每个嵌套文件都算"新增" →
    // **每次备份都退化成全量**（用户实测"存储暴增"的直接原因）；
    // 另外服务器路径里任一级含 "backup"（如 D:\Backups\srv）会导致正式备份把
    // world 数据整棵剪掉、只打包顶层文件，却报告成功（静默丢数据）。
    //
    // 只看结尾两段即可正确区分：
    //   .../.mcsrv_backups/snapshots/x/world/region → ["world","region"] 不是保留目录 ✓ 不剪
    //   D:\Backups\srv\world\region                 → ["world","region"] ✓ 不剪
    //   <server>/world/backup/                      → ["world","backup"] ✓ 正确剪掉
    let mut segs: Vec<String> = p
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let keep_from = segs.len().saturating_sub(2);
    let tail = segs.split_off(keep_from);
    tail.iter()
        .any(|s| s == ".mcsrv_backups" || s == "backup" || s == "backups" || s == ".mcsrv_trash")
}

/// 简单节流器：按 MB/s 限速
struct Throttle {
    bytes_per_sec: u64,
    window_bytes: u64,
    last_total: u64,
    window_start: Instant,
}

impl Throttle {
    fn new(mbps: f64) -> Self {
        let bps = if mbps > 0.0 && mbps.is_finite() {
            (mbps * 1048576.0) as u64
        } else {
            0
        };
        Self {
            bytes_per_sec: bps,
            window_bytes: 1048576, // 1MB 窗口
            last_total: 0,
            window_start: Instant::now(),
        }
    }

    fn wait(&mut self, total: u64) {
        if self.bytes_per_sec == 0 {
            return;
        }
        let delta = total.saturating_sub(self.last_total);
        if delta >= self.window_bytes {
            let elapsed = self.window_start.elapsed();
            let expect = Duration::from_secs_f64(delta as f64 / self.bytes_per_sec as f64);
            if elapsed < expect {
                std::thread::sleep(expect - elapsed);
            }
            self.last_total = total;
            self.window_start = Instant::now();
        }
    }
}

/// 流式复制并限速（读 -> 写）
fn stream_copy<R: Read, W: Write>(r: &mut R, w: &mut W, throttle: &mut Throttle) -> std::io::Result<u64> {
    let mut buf = [0u8; 65536];
    let mut copied: u64 = 0;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        copied += n as u64;
        throttle.wait(copied);
    }
    Ok(copied)
}

/// 复制文件并限速（保留 mtime，供下次对比与硬链接判定）
fn copy_throttled(src: &Path, dst: &Path, throttle: &mut Throttle) -> std::io::Result<u64> {
    let mut r = fs::File::open(src)?;
    let mut w = fs::File::create(dst)?;
    let n = stream_copy(&mut r, &mut w, throttle)?;
    if let Ok(m) = fs::metadata(src) {
        let ft = filetime::FileTime::from_last_modification_time(&m);
        let _ = filetime::set_file_mtime(dst, ft);
    }
    Ok(n)
}

/// 统计目录大小（字节）
pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| fs::metadata(e.path()).ok())
        .map(|m| m.len())
        .sum()
}

/// 备份线程设为低优先级，避免与服务器进程抢资源
pub fn set_thread_low_priority() {
    #[cfg(windows)]
    unsafe {
        use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
        use winapi::um::winbase::THREAD_PRIORITY_BELOW_NORMAL;
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL as i32);
    }
}
