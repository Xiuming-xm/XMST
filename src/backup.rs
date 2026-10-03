use chrono::Local;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// 备份目录相对服务器根目录的名称
pub const BACKUP_DIR: &str = ".mcsrv_backups";
/// 镜像目录：保存最近一次备份的完整文件状态，用于增量对比
pub const SNAPSHOT_DIR: &str = ".mcsrv_backups/snapshot";
/// 备份清单：记录最近一次全量备份的文件名
pub const MANIFEST_FILE: &str = ".mcsrv_backups/manifest.json";
/// 增量 zip 内删除清单文件名
pub const DELTA_META: &str = ".mcsrv_delta.json";

#[derive(Debug, Clone, PartialEq)]
pub enum BackupKind {
    Full,
    Incremental,
}

impl BackupKind {
    pub fn label(&self) -> &'static str {
        match self {
            BackupKind::Full => "全量",
            BackupKind::Incremental => "增量",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupResult {
    pub kind: BackupKind,
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// true 表示生成了 zip；false 表示无变化未生成
    pub changed: bool,
    /// 因被占用/锁定而跳过的文件数（服务器运行时常见）
    pub skipped: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    /// 最近一次全量备份 zip 文件名
    pub base_full: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DeltaMeta {
    /// 删除清单（相对服务器根目录，正斜杠）
    deleted: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mtime: String,
    pub kind: BackupKind,
}

/// 生成一次备份：首次全量，后续增量；检测到全量基线丢失则补做全量。
/// 自动备份只在服务器运行时由上层触发；无数据变化时返回 changed=false 不生成 zip。
/// 不做自动清理：增量备份依赖完整链路，删除中间备份会导致恢复出错。
/// throttle_mbps: 限速 MB/s，0 表示不限速。
pub fn create_backup(
    server_dir: &Path,
    folders: &[String],
    throttle_mbps: f64,
) -> Result<BackupResult, String> {
    let backup_root = server_dir.join(BACKUP_DIR);
    fs::create_dir_all(&backup_root).map_err(|e| format!("创建备份目录失败: {e}"))?;

    let manifest = load_manifest(server_dir);
    let base_missing = match &manifest.base_full {
        Some(name) => !backup_root.join(name).exists(),
        None => true,
    };
    // 无全量基线（首次 / 全量被删 / 镜像缺失）→ 做全量
    let need_full = base_missing || !server_dir.join(SNAPSHOT_DIR).exists();

    if need_full {
        do_full_backup(server_dir, folders, throttle_mbps)
    } else {
        do_incremental_backup(server_dir, folders, throttle_mbps)
    }
}

/// 全量备份：遍历所有源文件，写全量 zip，并重建镜像目录
fn do_full_backup(
    server_dir: &Path,
    folders: &[String],
    throttle_mbps: f64,
) -> Result<BackupResult, String> {
    let backup_root = server_dir.join(BACKUP_DIR);
    let snapshot = server_dir.join(SNAPSHOT_DIR);
    // 重建镜像，避免残留文件干扰后续增量
    let _ = fs::remove_dir_all(&snapshot);
    fs::create_dir_all(&snapshot).map_err(|e| format!("创建镜像目录失败: {e}"))?;

    let ts = Local::now().format("%Y%m%d_%H%M%S");
    let zip_path = backup_root.join(format!("backup_{ts}_full.zip"));

    let file = fs::File::create(&zip_path).map_err(|e| format!("创建备份文件失败: {e}"))?;
    let mut zip = ZipWriter::new(file);
    let options: SimpleFileOptions = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .unix_permissions(0o755);

    let mut throttle = Throttle::new(throttle_mbps);
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;
    let mut skipped = 0usize;

    for folder in folders {
        let src = server_dir.join(folder);
        if !src.exists() {
            continue;
        }
        if !src.is_dir() {
            // 单文件也支持
            let name = format!("{folder}");
            let ok = (|| -> std::io::Result<()> {
                let mut f = fs::File::open(&src)?;
                zip.start_file(name.clone(), options)?;
                let n = stream_copy(&mut f, &mut zip, &mut throttle)?;
                // 同步写入镜像
                let mdir = snapshot.join(folder);
                if let Some(parent) = mdir.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = copy_throttled(&src, &mdir, &mut throttle);
                total_bytes += n;
                Ok(())
            })();
            match ok {
                Ok(()) => total_files += 1,
                Err(_) => skipped += 1,
            }
            continue;
        }
        for entry in WalkDir::new(&src).follow_links(false).into_iter().filter_entry(|e| {
            let p = e.path();
            if p == src {
                return true;
            }
            // 目录路径任一段含 backup 则整棵跳过
            if e.file_type().is_dir() && has_backup_dir_segment(p) {
                return false;
            }
            // 文件级跳过运行时临时/锁文件
            if e.file_type().is_file() && is_transient_backup_file(p) {
                return false;
            }
            true
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path == src {
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = path.strip_prefix(server_dir).unwrap_or(path);
            let name = rel.to_string_lossy().replace('\\', "/");
            let ok = (|| -> std::io::Result<()> {
                let mut f = fs::File::open(path)?;
                zip.start_file(name.clone(), options)?;
                let n = stream_copy(&mut f, &mut zip, &mut throttle)?;
                // 同步写入镜像（保留 mtime 以便增量对比）
                let mdir = snapshot.join(rel);
                if let Some(parent) = mdir.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = copy_throttled(path, &mdir, &mut throttle);
                total_bytes += n;
                Ok(())
            })();
            match ok {
                Ok(()) => total_files += 1,
                Err(_) => skipped += 1,
            }
        }
    }

    zip.finish().map_err(|e| format!("写入 zip 失败: {e}"))?;

    // 写入清单
    let manifest = BackupManifest {
        base_full: Some(zip_path.file_name().unwrap_or_default().to_string_lossy().to_string()),
    };
    let _ = fs::write(
        server_dir.join(MANIFEST_FILE),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    );

    Ok(BackupResult {
        kind: BackupKind::Full,
        path: zip_path,
        files: total_files,
        bytes: total_bytes,
        changed: true,
        skipped,
    })
}

/// 增量备份：对比源目录与镜像，仅打包变化/新增文件；源中已删除的文件记入删除清单
fn do_incremental_backup(
    server_dir: &Path,
    folders: &[String],
    throttle_mbps: f64,
) -> Result<BackupResult, String> {
    let backup_root = server_dir.join(BACKUP_DIR);
    let snapshot = server_dir.join(SNAPSHOT_DIR);

    // 收集源文件签名: rel -> (len, mtime_sec)
    let mut src_files: Vec<(String, (u64, u64))> = Vec::new();
    for folder in folders {
        let src = server_dir.join(folder);
        if !src.exists() {
            continue;
        }
        if !src.is_dir() {
            if let Some(sig) = file_sig(&src) {
                src_files.push((folder.clone(), sig));
            }
            continue;
        }
        for entry in WalkDir::new(&src).follow_links(false).into_iter().filter_entry(|e| {
            let p = e.path();
            if p == src {
                return true;
            }
            if e.file_type().is_dir() && has_backup_dir_segment(p) {
                return false;
            }
            if e.file_type().is_file() && is_transient_backup_file(p) {
                return false;
            }
            true
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path == src || !entry.file_type().is_file() {
                continue;
            }
            let rel = path.strip_prefix(server_dir).unwrap_or(path);
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if let Some(sig) = file_sig(path) {
                src_files.push((rel_s, sig));
            }
        }
    }

    // 收集镜像文件签名
    let mut mirror_files: Vec<(String, (u64, u64))> = Vec::new();
    if snapshot.exists() {
        for entry in WalkDir::new(&snapshot).follow_links(false).into_iter().filter_entry(|e| {
            let p = e.path();
            if p == snapshot {
                return true;
            }
            !(e.file_type().is_dir() && has_backup_dir_segment(p))
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path == snapshot || !entry.file_type().is_file() {
                continue;
            }
            let rel = path.strip_prefix(&snapshot).unwrap_or(path);
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if let Some(sig) = file_sig(path) {
                mirror_files.push((rel_s, sig));
            }
        }
    }

    let src_map: std::collections::HashMap<&str, (u64, u64)> =
        src_files.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let mirror_map: std::collections::HashMap<&str, (u64, u64)> =
        mirror_files.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    // 变化/新增文件
    let mut changed: Vec<String> = Vec::new();
    for (rel, sig) in &src_files {
        match mirror_map.get(rel.as_str()) {
            Some(msig) if *msig == *sig => {}
            _ => changed.push(rel.clone()),
        }
    }
    // 源中已删除的文件
    let mut deleted: Vec<String> = Vec::new();
    for rel in mirror_map.keys() {
        if !src_map.contains_key(rel) {
            deleted.push(rel.to_string());
        }
    }

    if changed.is_empty() && deleted.is_empty() {
        return Ok(BackupResult {
            kind: BackupKind::Incremental,
            path: backup_root.join("none.zip"),
            files: 0,
            bytes: 0,
            changed: false,
            skipped: 0,
        });
    }

    let ts = Local::now().format("%Y%m%d_%H%M%S");
    let zip_path = backup_root.join(format!("backup_{ts}_inc.zip"));

    let file = fs::File::create(&zip_path).map_err(|e| format!("创建备份文件失败: {e}"))?;
    let mut zip = ZipWriter::new(file);
    let options: SimpleFileOptions = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .unix_permissions(0o755);

    let mut throttle = Throttle::new(throttle_mbps);
    let mut total_bytes = 0u64;
    let mut skipped = 0usize;

    // 删除清单（先写入 zip，恢复时先删后覆盖）
    if !deleted.is_empty() {
        let meta = DeltaMeta { deleted: deleted.clone() };
        let json = serde_json::to_string(&meta).unwrap_or_default();
        zip.start_file(DELTA_META, options)
            .map_err(|e| format!("写入删除清单失败: {e}"))?;
        zip.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    }

    for rel in &changed {
        let src_path = server_dir.join(sanitize_rel(rel));
        if !src_path.exists() {
            continue;
        }
        let ok = (|| -> std::io::Result<()> {
            let mut f = fs::File::open(&src_path)?;
            zip.start_file(rel.clone(), options)?;
            let n = stream_copy(&mut f, &mut zip, &mut throttle)?;
            // 更新镜像
            let mdir = snapshot.join(sanitize_rel(rel));
            if let Some(parent) = mdir.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = copy_throttled(&src_path, &mdir, &mut throttle);
            total_bytes += n;
            Ok(())
        })();
        if ok.is_err() {
            skipped += 1;
        }
    }
    // 从镜像删除已消失的文件
    for rel in &deleted {
        let mdir = snapshot.join(sanitize_rel(rel));
        let _ = fs::remove_file(&mdir);
    }

    zip.finish().map_err(|e| format!("写入 zip 失败: {e}"))?;

    Ok(BackupResult {
        kind: BackupKind::Incremental,
        path: zip_path,
        files: changed.len(),
        bytes: total_bytes,
        changed: true,
        skipped,
    })
}

/// 列出备份文件（时间升序）
pub fn list_backups(server_dir: &Path) -> Vec<BackupInfo> {
    let backup_root = server_dir.join(BACKUP_DIR);
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(&backup_root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "zip").unwrap_or(false) {
                let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                let mtime = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Local> = t.into();
                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_default();
                let kind = if name.contains("_full.zip") {
                    BackupKind::Full
                } else {
                    BackupKind::Incremental
                };
                out.push(BackupInfo { path: p, name, size, mtime, kind });
            }
        }
    }
    out.sort_by_key(|b| b.name.clone());
    out
}

/// 删除单个备份文件；若删除的是全量基线，下次备份会自动补全量
pub fn delete_backup(server_dir: &Path, zip_path: &Path) -> Result<(), String> {
    let backup_root = server_dir.join(BACKUP_DIR);
    if zip_path.parent() != Some(backup_root.as_path()) {
        return Err("拒绝删除备份目录外的文件".to_string());
    }
    fs::remove_file(zip_path).map_err(|e| format!("删除备份失败: {e}"))
}

/// 回退：把目标备份（含其全量基线及之间的增量）按顺序恢复到服务器根目录。
/// 旧内容移动到 .mcsrv_trash；恢复完成后镜像失效，下次备份自动补全量。
pub fn restore_backup(server_dir: &Path, zip_path: &Path, folders: &[String]) -> Result<usize, String> {
    let all = list_backups(server_dir);
    if all.is_empty() {
        return Err("备份目录为空".to_string());
    }
    let Some(target_idx) = all.iter().position(|b| b.path == zip_path) else {
        return Err("目标备份不在备份目录中".to_string());
    };

    // 构造恢复链：全量基线 + 之后到目标（含目标）的所有增量
    let mut chain: Vec<PathBuf> = Vec::new();
    let target_full = all[target_idx].kind == BackupKind::Full;
    if target_full {
        chain.push(zip_path.to_path_buf());
    } else {
        let mut found_full = false;
        for b in all.iter().take(target_idx + 1) {
            if b.kind == BackupKind::Full {
                chain.clear();
                found_full = true;
            }
            chain.push(b.path.clone());
        }
        if !found_full {
            return Err("找不到该增量备份对应的全量基线，无法回退".to_string());
        }
    }

    // 1) 先备份现有内容到 .mcsrv_trash（可恢复）
    let trash_root = server_dir.join(".mcsrv_trash");
    fs::create_dir_all(&trash_root).map_err(|e| format!("创建回收目录失败: {e}"))?;
    for folder in folders {
        let src = server_dir.join(folder);
        if src.exists() {
            let ts = Local::now().format("%Y%m%d_%H%M%S");
            let dst = trash_root.join(format!("{folder}_{ts}"));
            let _ = fs::rename(&src, &dst);
        }
    }

    // 2) 按序应用恢复链
    let mut restored = 0usize;
    for zp in &chain {
        restored += apply_zip(server_dir, zp, folders)?;
    }

    // 3) 镜像失效，下次备份自动全量重建
    let _ = fs::remove_dir_all(server_dir.join(SNAPSHOT_DIR));
    let _ = fs::remove_file(server_dir.join(MANIFEST_FILE));

    Ok(restored)
}

/// 应用单个 zip：先处理删除清单，再解压覆盖
fn apply_zip(server_dir: &Path, zip_path: &Path, folders: &[String]) -> Result<usize, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("打开备份失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("读取备份失败: {e}"))?;

    let mut restored = 0usize;
    // 第一遍：处理删除清单
    for i in 0..archive.len() {
        let mut f = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if f.name() != DELTA_META {
            continue;
        }
        let mut json = String::new();
        let _ = f.read_to_string(&mut json);
        if let Ok(meta) = serde_json::from_str::<DeltaMeta>(&json) {
            for rel in meta.deleted {
                let del_path = server_dir.join(sanitize_rel(&rel));
                if del_path.starts_with(server_dir) {
                    let _ = fs::remove_file(&del_path);
                }
            }
        }
    }

    // 第二遍：解压
    for i in 0..archive.len() {
        let mut f = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let name = f.name().to_string();
        if name == DELTA_META {
            continue;
        }
        // 只解压指定顶层文件夹下的内容
        let in_target = folders.iter().any(|fd| {
            name == *fd || name.starts_with(&format!("{fd}/")) || name.starts_with(&format!("{fd}\\"))
        });
        if !in_target {
            continue;
        }
        let out_path = server_dir.join(sanitize_rel(&name));
        if let Some(parent) = out_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut out = match fs::File::create(&out_path) {
            Ok(o) => o,
            Err(_) => continue,
        };
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_ok() {
            if out.write_all(&buf).is_ok() {
                restored += 1;
            }
        }
    }

    Ok(restored)
}

/// 读取备份清单
fn load_manifest(server_dir: &Path) -> BackupManifest {
    match fs::read_to_string(server_dir.join(MANIFEST_FILE)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(BackupManifest { base_full: None }),
        Err(_) => BackupManifest { base_full: None },
    }
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

/// 文件签名：(长度, 哈希/修改时间秒)。
/// 小文件（<= 16MB）用内容哈希：抗 mtime 秒级抖动与“内容未变但时间被 touch”的误判，
/// 是“无数据更新却每次多出 1.99MB”问题的关键修复；大文件用 (长度, 修改时间秒) 控制开销。
fn file_sig(p: &Path) -> Option<(u64, u64)> {
    let m = fs::metadata(p).ok()?;
    let len = m.len();
    if len <= 16 * 1024 * 1024 {
        let h = fnv1a64_file(p).ok()?;
        Some((len, h))
    } else {
        let mt = m
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some((len, mt))
    }
}

/// FNV-1a 64 位内容哈希
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
    //   .../.mcsrv_backups/snap/x/world/region  → ["world","region"] 不是保留目录 ✓ 不剪
    //   D:\Backups\srv\world\region             → ["world","region"] ✓ 不剪
    //   <server>/world/backup/                  → ["world","backup"] ✓ 正确剪掉
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
        .any(|s| s == ".mcsrv_backups" || s == "backup" || s == "backups")
}

/// 判断是否为运行时临时/锁文件（服务器运行期间高频变化或处于锁定状态，不应进入备份）：
/// session.lock、level.dat_new、level.dat_old、*.tmp、*.lock、*.pid、*.lck、*.part
/// 参考 FTB-Backups 的默认排除规则：session.lock 等锁文件随运行状态反复改写，
/// 打进备份既无恢复价值又会让增量每次都误判为"变化"而重打包。
fn is_transient_backup_file(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    if name.eq_ignore_ascii_case("session.lock")
        || name.eq_ignore_ascii_case("level.dat_new")
        || name.eq_ignore_ascii_case("level.dat_old")
    {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tmp")
        || lower.ends_with(".lock")
        || lower.ends_with(".pid")
        || lower.ends_with(".lck")
        || lower.ends_with(".part")
}

fn fnv1a64_file(p: &Path) -> std::io::Result<u64> {
    use std::io::Read;
    let mut f = fs::File::open(p)?;
    let mut buf = [0u8; 65536];
    let mut h: u64 = 0xcbf29ce484222325;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    Ok(h)
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
        let bps = if mbps > 0.0 { (mbps * 1048576.0) as u64 } else { 0 };
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

/// 复制文件并限速（保留 mtime）
fn copy_throttled(src: &Path, dst: &Path, throttle: &mut Throttle) -> std::io::Result<u64> {
    let mut r = fs::File::open(src)?;
    let mut w = fs::File::create(dst)?;
    let n = stream_copy(&mut r, &mut w, throttle)?;
    if let Ok(m) = fs::metadata(src) {
        if let Ok(modified) = m.modified() {
            let _ = set_file_mtime(dst, filetime_from_system_time(modified));
        }
    }
    Ok(n)
}

/// 系统时间 -> (sec, nsec)
fn filetime_from_system_time(t: std::time::SystemTime) -> (i64, u32) {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
        Err(e) => {
            let d = e.duration();
            (-(d.as_secs() as i64), d.subsec_nanos())
        }
    }
}

/// 设置文件修改时间（Windows）
fn set_file_mtime(path: &Path, ft: (i64, u32)) -> std::io::Result<()> {
    let t = filetime::FileTime::from_unix_time(ft.0, ft.1);
    filetime::set_file_mtime(path, t)
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
