#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod backdrop;
mod config;
mod crashscan;
mod download;
mod logdb;
mod features;
mod mcmod_db;
mod modrinth;
mod perf;
mod plugins;
mod process;
mod server_download;
mod serverinfo;
mod spark_analysis;
mod theme;

use chrono::Local;
use config::{GlobalConfig, JavaHome, ServerConfig, TunnelConfig, CONFIG_FILE, OLD_CONFIG_FILE};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use rhai::Dynamic;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ---------- Java 解析辅助 ----------

/// �?run.bat 提取 Java 路径：支�?`set JAVA=...` / `set JAVA_PATH=...` /
/// `set JAVA_HOME=...` 以及行内出现 `"C:\...\java.exe"` / `javaw.exe` 的形式�?
fn extract_java_from_bat(bat: &Path) -> Option<String> {
    let content = std::fs::read_to_string(bat).ok()?;
    for line in content.lines() {
        let line = line.trim();
        let lower = line.to_lowercase();
        if lower.starts_with("set ") {
            let rest = &line[4..];
            if let Some(eq) = rest.find('=') {
                let key = rest[..eq].trim().to_lowercase();
                let val = rest[eq + 1..].trim().trim_matches('"').to_string();
                if !val.is_empty()
                    && (key == "java" || key == "java_path" || key == "java_home")
                {
                    return Some(val);
                }
            }
        } else if lower.contains("java.exe") || lower.contains("javaw.exe") {
            // 提取行内第一个引号包裹的 java 可执行文�?
            if let Some(start) = line.find('"') {
                if let Some(end) = line[start + 1..].find('"') {
                    let p = &line[start + 1..start + 1 + end];
                    if p.to_lowercase().contains("java") {
                        return Some(p.to_string());
                    }
                }
            }
        }
    }
    None
}

/// �?MC 版本自动匹配全局 Java 列表：先完整版本前缀，再主版本（�?"21"）�?
fn match_java_by_version(homes: &[JavaHome], mc_version: Option<&str>) -> Option<String> {
    let ver = mc_version?.trim();
    if ver.is_empty() {
        return None;
    }
    // 完整版本前缀匹配：JavaHome.version �?"1.21.1" / "21.1" / "1.21" 均可
    for h in homes {
        let v = h.version.trim();
        if !v.is_empty() && (ver.starts_with(v) || v.starts_with(ver)) {
            return Some(h.path.clone());
        }
    }
    // 主版本匹配：ver="1.21.1" -> "21"；JavaHome.version �?"21" 即命�?
    if let Some(maj) = ver.split('.').nth(1) {
        for h in homes {
            if h.version.trim() == maj {
                return Some(h.path.clone());
            }
        }
    }
    None
}

/// 解析服务器实际使用的 Java 可执行文件路径（�?run.bat 时调用）�?
/// 服务器指定列表项 -> 服务器自定义路径 -> run.bat 提取 -> 按版本自动匹�?-> 全局兜底 -> PATH�?
fn resolve_java_for_server(cfg: &GlobalConfig, sc: &ServerConfig) -> String {
    if let Some(id) = sc.java_home_id.as_ref() {
        if let Some(h) = cfg.java_homes.iter().find(|h| &h.name == id) {
            if !h.path.trim().is_empty() {
                return h.path.clone();
            }
        }
    }
    if let Some(p) = sc.java_path.as_ref() {
        if !p.trim().is_empty() {
            return p.clone();
        }
    }
    let bat = sc.dir.join("run.bat");
    if bat.exists() {
        if let Some(p) = extract_java_from_bat(&bat) {
            if !p.trim().is_empty() {
                return p;
            }
        }
    }
    if let Some(p) = match_java_by_version(&cfg.java_homes, sc.mc_version.as_deref()) {
        return p;
    }
    if !cfg.java_path.trim().is_empty() {
        return cfg.java_path.clone();
    }
    "java".to_string()
}

/// 公式化启动：扫描服务器目录，自动识别 MC 服务端核�?jar�?
/// 识别优先级：Fabric/Quilt -> NeoForge/Forge -> Paper/Purpur/Spigot/Bukkit -> vanilla server.jar / minecraft_server.*.jar�?
/// 找不到任何核心时回退 "server.jar"（保持模板原样）�?
fn detect_core_jar(dir: &std::path::Path) -> String {
    let mut vanilla: Option<String> = None;
    let mut best: Option<String> = None;
    let mut best_rank: u8 = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return "server.jar".to_string();
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("jar") {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
        // 跳过安装�?库文�?
        if name.contains("installer") || name.contains("sources") || name.contains("javadoc") {
            continue;
        }
        let rank = if name.starts_with("fabric-server-launch") || name.starts_with("quilt-server-launch") {
            6
        } else if name.starts_with("neoforge-") || (name.starts_with("forge-") && !name.contains("universal")) {
            5
        } else if name.starts_with("paper-") || name.starts_with("purpur-") {
            4
        } else if name.starts_with("spigot-") || name.starts_with("bukkit-") {
            3
        } else if name == "server.jar" || name.starts_with("minecraft_server.") {
            2
        } else {
            0
        };
        if rank == 0 {
            continue;
        }
        if rank == 2 && vanilla.is_none() {
            vanilla = p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
        }
        if rank > best_rank {
            best_rank = rank;
            best = p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
        }
    }
    best.or(vanilla).unwrap_or_else(|| "server.jar".to_string())
}

/// ===== 服务端核心检测：目录/JAR/ZIP 整合包（拖入自动识别） =====
/// 从源路径（JAR / ZIP / 文件夹）检测服务端核心候选列表。
/// 检测策略：文件名前缀 > JAR 内 MANIFEST.MF > version.json，三层兜底。
fn detect_server_source(path: &std::path::Path) -> Result<Vec<DetectedCore>, String> {
    if path.is_dir() {
        let mut out: Vec<DetectedCore> = Vec::new();
        let root_depth = path.components().count();
        // 递归 3 层扫描 jar，跳过常见噪音目录（libraries/versions/mods 等）
        for entry in walkdir::WalkDir::new(path)
            .max_depth(3)
            .into_iter()
            .filter_entry(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                !matches!(
                    n.as_str(),
                    "libraries" | "cache" | "crash-reports" | "logs" | "world" | "world_nether"
                        | "world_the_end" | "backups" | ".mcsrv_backups" | "mods" | "plugins"
                        | "config" | "resourcepacks" | "saves" | "versions" | "assets" | "bin"
                )
            })
        {
            let Ok(entry) = entry else { continue };
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase())
                    == Some("jar".to_string())
            {
                let depth = entry.path().components().count().saturating_sub(root_depth);
                if let Some(c) = inspect_jar(entry.path(), depth) {
                    out.push(c);
                }
            }
        }
        if out.is_empty() {
            return Err(
                "未在文件夹中找到服务端核心 JAR（server.jar / fabric-server-launch / forge / paper 等）"
                    .to_string(),
            );
        }
        out.sort_by(|a, b| b.score.cmp(&a.score));
        Ok(out)
    } else if let Some(ext) = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
    {
        match ext.as_str() {
            "jar" => match inspect_jar(path, 0) {
                Some(c) => Ok(vec![c]),
                None => Err("无法识别该 JAR（非服务端核心或已损坏）".to_string()),
            },
            "zip" => scan_zip(path),
            "rar" | "7z" | "gz" | "tar" | "xz" => {
                Err("暂不支持直接读取压缩包，请先解压后拖入文件夹".to_string())
            }
            _ => Err(format!(
                "不支持的文件类型 .{ext}，请拖入 JAR / ZIP 整合包 / 服务器文件夹"
            )),
        }
    } else {
        Err("未知文件，请拖入 JAR / ZIP 整合包 / 服务器文件夹".to_string())
    }
}

/// 检测单个 JAR：文件名前缀优先，其次读 MANIFEST.MF / version.json
fn inspect_jar(path: &std::path::Path, depth: usize) -> Option<DetectedCore> {
    let file_name = path.file_name()?.to_string_lossy().to_string();
    let lower = file_name.to_lowercase();
    // 排除明显非服务端 jar
    if lower.contains("installer")
        || lower.contains("sources")
        || lower.contains("javadoc")
        || lower.contains("-dev")
        || lower.contains("-api")
        || lower.contains("-client")
    {
        return None;
    }
    let mut kind = "Unknown".to_string();
    let mut mc: Option<String> = None;
    // 1) 文件名前缀判断
    if lower.starts_with("fabric-server-launch") || lower.starts_with("quilt-server-launch") {
        kind = "Fabric".to_string();
        mc = extract_mc_version(&file_name);
    } else if lower.starts_with("neoforge-") {
        kind = "NeoForge".to_string();
        mc = extract_mc_version(&file_name).map(|v| normalize_mc_version(&v, "NeoForge"));
    } else if lower.starts_with("forge-") {
        kind = "Forge".to_string();
        mc = extract_mc_version(&file_name);
    } else if lower.starts_with("paper-") || lower.starts_with("purpur-") {
        kind = "Paper".to_string();
        mc = extract_mc_version(&file_name);
    } else if lower.starts_with("spigot-") || lower.starts_with("bukkit-") {
        kind = "Spigot".to_string();
        mc = extract_mc_version(&file_name);
    } else if lower.starts_with("minecraft_server")
        || lower == "server.jar"
        || lower == "sponge.jar"
    {
        kind = "Vanilla".to_string();
        mc = extract_mc_version(&file_name);
    }
    // 2) MANIFEST.MF / version.json 兜底（压缩格式损坏则跳过）
    if kind == "Unknown" || mc.is_none() {
        if let Ok(file) = std::fs::File::open(path) {
            if let Ok(mut z) = zip::ZipArchive::new(file) {
                if kind == "Unknown" {
                    if let Ok(mut mf) = z.by_name("META-INF/MANIFEST.MF") {
                        let mut s = Vec::new();
                        if std::io::Read::read_to_end(&mut mf, &mut s).is_ok() {
                            let ss = String::from_utf8_lossy(&s);
                            if ss.contains("net.fabricmc") {
                                kind = "Fabric".to_string();
                            } else if ss.contains("cpw.mods.bootstraplauncher")
                                || ss.contains("net.minecraftforge")
                            {
                                kind = "Forge".to_string();
                            } else if ss.contains("net.neoforged") {
                                kind = "NeoForge".to_string();
                            } else if ss.contains("io.papermc.paperclip") {
                                kind = "Paper".to_string();
                            } else if ss.contains("net.minecraft.bundler.Main") {
                                kind = "Vanilla".to_string();
                            }
                        }
                    }
                }
                if mc.is_none() {
                    if let Ok(mut vf) = z.by_name("version.json") {
                        let mut s = Vec::new();
                        if std::io::Read::read_to_end(&mut vf, &mut s).is_ok() {
                            if let Ok(v) =
                                serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&s))
                            {
                                if let Some(id) = v["id"].as_str() {
                                    mc = Some(id.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if kind == "Unknown" && mc.is_none() {
        return None;
    }
    // 3) 推荐分：类型权重 + 版本完整度 + 深度（越靠近根目录越高）
    let rank = match kind.as_str() {
        "Fabric" => 6,
        "NeoForge" => 5,
        "Forge" => 5,
        "Paper" => 4,
        "Spigot" => 3,
        "Vanilla" => 2,
        _ => 1,
    };
    let mut score = rank * 10 + if mc.is_some() { 10 } else { 0 };
    score += if depth <= 1 { 20 } else if depth <= 2 { 10 } else { 5 };
    Some(DetectedCore {
        path: path.to_path_buf(),
        file_name,
        kind,
        mc_version: mc,
        score,
        in_zip: false,
    })
}

/// 扫描 ZIP 整合包内的服务端核心候选（含嵌套 jar 的 MANIFEST 识别）
fn scan_zip(path: &std::path::Path) -> Result<Vec<DetectedCore>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打开压缩包失败: {e}"))?;
    let mut z = zip::ZipArchive::new(file).map_err(|e| format!("不是有效的 ZIP 文件: {e}"))?;
    let mut candidates: Vec<DetectedCore> = Vec::new();
    for i in 0..z.len() {
        let Ok(entry) = z.by_index(i) else { continue };
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if entry.is_dir() || !lower.ends_with(".jar") {
            continue;
        }
        if lower.contains("libraries/")
            || lower.contains("versions/")
            || lower.contains("mods/")
            || lower.contains("plugins/")
            || lower.contains("installer")
            || lower.contains("sources")
            || lower.contains("javadoc")
            || lower.contains("-api")
            || lower.contains("-dev")
        {
            continue;
        }
        let file_name = name.rsplit('/').next().unwrap_or(&name).to_string();
        let mut kind = "Unknown".to_string();
        let mut mc: Option<String> = None;
        if lower.contains("fabric-server-launch") || lower.contains("quilt-server-launch") {
            kind = "Fabric".to_string();
            mc = extract_mc_version(&file_name);
        } else if lower.contains("neoforge-") {
            kind = "NeoForge".to_string();
            mc = extract_mc_version(&file_name).map(|v| normalize_mc_version(&v, "NeoForge"));
        } else if lower.contains("forge-") {
            kind = "Forge".to_string();
            mc = extract_mc_version(&file_name);
        } else if lower.contains("paper-") || lower.contains("purpur-") {
            kind = "Paper".to_string();
            mc = extract_mc_version(&file_name);
        } else if lower.contains("minecraft_server") || file_name.eq_ignore_ascii_case("server.jar")
        {
            kind = "Vanilla".to_string();
            mc = extract_mc_version(&file_name);
        }
        if kind == "Unknown" || mc.is_none() {
            // 读 jar 内容（拷入内存再嵌套解压）
            let mut buf = Vec::new();
            let mut r = std::io::Read::take(entry, u64::MAX);
            if std::io::Read::read_to_end(&mut r, &mut buf).is_ok() {
                if let Ok(mut iz) = zip::ZipArchive::new(std::io::Cursor::new(buf)) {
                    if kind == "Unknown" {
                        if let Ok(mut mf) = iz.by_name("META-INF/MANIFEST.MF") {
                            let mut s = Vec::new();
                            if std::io::Read::read_to_end(&mut mf, &mut s).is_ok() {
                                let ss = String::from_utf8_lossy(&s);
                                if ss.contains("net.fabricmc") {
                                    kind = "Fabric".to_string();
                                } else if ss.contains("cpw.mods.bootstraplauncher")
                                    || ss.contains("net.minecraftforge")
                                {
                                    kind = "Forge".to_string();
                                } else if ss.contains("net.neoforged") {
                                    kind = "NeoForge".to_string();
                                } else if ss.contains("io.papermc.paperclip") {
                                    kind = "Paper".to_string();
                                } else if ss.contains("net.minecraft.bundler.Main") {
                                    kind = "Vanilla".to_string();
                                }
                            }
                        }
                    }
                    if mc.is_none() {
                        if let Ok(mut vf) = iz.by_name("version.json") {
                            let mut s = Vec::new();
                            if std::io::Read::read_to_end(&mut vf, &mut s).is_ok() {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(
                                    &String::from_utf8_lossy(&s),
                                ) {
                                    if let Some(id) = v["id"].as_str() {
                                        mc = Some(id.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if kind == "Unknown" && mc.is_none() {
            continue;
        }
        let rank = match kind.as_str() {
            "Fabric" => 6,
            "NeoForge" => 5,
            "Forge" => 5,
            "Paper" => 4,
            "Spigot" => 3,
            "Vanilla" => 2,
            _ => 1,
        };
        let depth = name.matches('/').count();
        let mut score = rank * 10 + if mc.is_some() { 10 } else { 0 };
        score += if depth <= 1 { 20 } else if depth <= 3 { 10 } else { 5 };
        candidates.push(DetectedCore {
            path: path.to_path_buf(),
            file_name: name.clone(),
            kind,
            mc_version: mc,
            score,
            in_zip: true,
        });
    }
    if candidates.is_empty() {
        return Err("压缩包内未找到服务端核心 JAR（请确认是服务端整合包）".to_string());
    }
    candidates.sort_by(|a, b| b.score.cmp(&a.score));
    Ok(candidates)
}

/// 从文件名中尽力提取 MC 版本（如 1.21.1 / 1.20.1 / 21.1 等）
fn extract_mc_version(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut j = i;
            let mut dots = 0;
            while j < chars.len()
                && (chars[j].is_ascii_digit()
                    || (chars[j] == '.'
                        && j + 1 < chars.len()
                        && chars[j + 1].is_ascii_digit()
                        && dots < 2))
            {
                if chars[j] == '.' {
                    dots += 1;
                }
                j += 1;
            }
            if dots >= 1 {
                let cand: String = chars[start..j].iter().collect();
                let segs: Vec<&str> = cand.split('.').collect();
                let ok = segs.len() >= 2
                    && segs.iter().all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));
                if ok {
                    let major: u32 = segs[0].parse().unwrap_or(0);
                    if major == 1 || major >= 20 {
                        return Some(cand);
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// 归一化 MC 版本：NeoForge 新式版本号 21.1.x 对应 MC 1.21.x
fn normalize_mc_version(v: &str, kind: &str) -> String {
    if kind == "NeoForge" {
        if let Some((a, b, _c)) = parse_ver(v) {
            if a >= 20 && b >= 1 {
                return format!("1.{}.x", b + 20);
            }
        }
    }
    v.to_string()
}

/// 解析版本号为三元组（不足位补 0）
fn parse_ver(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let c = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((a, b, c))
}

/// 根据 MC 版本给出 Java 需求建议
fn java_required(mc: Option<&str>) -> &'static str {
    let Some(v) = mc else { return "Java 21（建议）" };
    if let Some((a, b, c)) = parse_ver(v) {
        if a >= 2 || (a == 1 && b >= 20 && c >= 5) {
            return "Java 21";
        }
        if a == 1 && b >= 18 {
            return "Java 17";
        }
        if a == 1 && b >= 17 {
            return "Java 16";
        }
        return "Java 8";
    }
    "Java 21（建议）"
}

/// 解压 ZIP 整合包到目标目录（zip slip 防护 + 返回解压文件数）
fn extract_zip_to(zip_path: &std::path::Path, dest: &std::path::Path) -> Result<usize, String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("打开压缩包失败: {e}"))?;
    let mut z = zip::ZipArchive::new(file).map_err(|e| format!("读取压缩包失败: {e}"))?;
    std::fs::create_dir_all(dest).map_err(|e| format!("创建目录失败: {e}"))?;
    let mut n = 0usize;
    for i in 0..z.len() {
        let mut entry = z.by_index(i).map_err(|e| format!("读取压缩条目失败: {e}"))?;
        let name = entry.name().replace('\\', "/");
        let clean = name.trim_start_matches('/');
        // zip slip 防护：禁止跳出目标目录
        let out = dest.join(clean);
        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&out);
            continue;
        }
        if let Some(p) = out.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let mut f = std::fs::File::create(&out)
            .map_err(|e| format!("创建文件失败（{}）: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut f)
            .map_err(|e| format!("解压文件失败（{}）: {e}", out.display()))?;
        n += 1;
    }
    Ok(n)
}


/// 检测到的服务端核心候选（带推荐分）
#[derive(Clone, Debug)]
struct DetectedCore {
    /// 实际路径（zip 整合包内条目时为 zip 文件路径）
    path: std::path::PathBuf,
    /// 文件名（zip 整合包内为 zip 内条目路径，如 subdir/server.jar）
    file_name: String,
    /// 核心类型：Vanilla / Paper / Fabric / Forge / NeoForge / Spigot / Unknown
    kind: String,
    /// MC 版本（尽力提取，可能为 None）
    mc_version: Option<String>,
    /// 推荐分（类型权重 + 版本完整度 + 深度）
    score: i32,
    /// 是否来自 zip 整合包内部条目（选择后需解压）
    in_zip: bool,
}


/// ===== B3 强停二次确认：类型定义 =====
/// 确认弹窗请求：显示 pid + 影响 → 生成一次性 token → 带 token 执行强杀
#[derive(Clone, Debug)]
struct ForceStopReq {
    /// 服务器下标
    idx: usize,
    /// 主进程 PID（弹窗展示）
    pid: u32,
    /// 一次性确认令牌（确认时校验，过期自动失效）
    token: u64,
    /// 创建时刻（30s 过期）
    created_at: std::time::Instant,
}

/// 强停确认有效期（秒）
const FORCE_STOP_TOKEN_TTL: u64 = 30;

/// 左侧导航
#[derive(PartialEq, Clone, Copy)]
enum Nav {
    Dashboard,
    Servers,
    Tunnel,
    /// 工具自身日志（XMST 日志库 + 相关设置；与服务器日志无关，不按服务器拆分）
    Logs,
    Settings,
    Download,
    Plugins,
}

/// 图2 模组社区左侧导航（Modrinth 分类 + 收藏栏）
#[derive(PartialEq, Clone, Copy)]
enum ModCommunityNav {
    Mod,
    Plugin,
    Datapack,
    Favorites,
}

impl ModCommunityNav {
    /// 对应 Modrinth project_type（安装包类返回空）
    fn project_type(self) -> &'static str {
        match self {
            ModCommunityNav::Mod => "mod",
            ModCommunityNav::Plugin => "plugin",
            ModCommunityNav::Datapack => "datapack",
            ModCommunityNav::Favorites => "",
        }
    }
}

/// 模组卡片操作结果（图2 卡片按钮）
enum ModCardAction {
    None,
    Install,
    FavToggle,
    /// 点击卡片主体 -> 展开详情/版本列表
    Detail,
}

/// 模组图标占位色（按 project_id 哈希选色板）
fn mod_icon_color(id: &str) -> Color32 {
    let palette: &[Color32] = &[
        Color32::from_rgb(86, 124, 228),
        Color32::from_rgb(236, 117, 105),
        Color32::from_rgb(92, 183, 120),
        Color32::from_rgb(240, 168, 82),
        Color32::from_rgb(147, 112, 219),
        Color32::from_rgb(77, 182, 172),
        Color32::from_rgb(222, 93, 131),
        Color32::from_rgb(96, 156, 212),
    ];
    let h = id.bytes().fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(b as u64));
    palette[(h as usize) % palette.len()]
}

/// 下载量格式化（12345 -> 12.3k）
fn fmt_count(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// 详情页版本列表分组：按 game_versions 展开分组（一个版本可属于多个 MC 版本），
/// 无法识别版本归入"其他"；组按 MC 版本降序，组内按发布时间降序。
/// loader 为空或"全部"时不筛选。
fn group_mod_versions<'a>(
    versions: &'a [modrinth::ModrinthVersion],
    loader: &str,
) -> Vec<(String, Vec<&'a modrinth::ModrinthVersion>)> {
    let filtered: Vec<&'a modrinth::ModrinthVersion> = versions
        .iter()
        .filter(|v| {
            loader.is_empty()
                || loader == "全部"
                || v.loaders.iter().any(|l| l == loader)
        })
        .collect();
    let mut groups: Vec<(String, Vec<&'a modrinth::ModrinthVersion>)> = Vec::new();
    for v in &filtered {
        let gvs: Vec<String> = if v.game_versions.is_empty() {
            vec!["其他".to_string()]
        } else {
            v.game_versions.clone()
        };
        for gv in gvs {
            if let Some(g) = groups.iter_mut().find(|(k, _)| *k == gv) {
                g.1.push(v);
            } else {
                groups.push((gv, vec![*v]));
            }
        }
    }
    groups.sort_by(|a, b| match (parse_ver(&a.0), parse_ver(&b.0)) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.0.cmp(&a.0),
    });
    for (_, vs) in &mut groups {
        vs.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    }
    groups
}

/// 剥离简易 HTML 标签与实体（用于 Modrinth changelog 展示）
fn strip_html(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // 跳过标签；</p> </li> 等块级闭合标签换行
                let mut tag = String::new();
                for cc in chars.by_ref() {
                    if cc == '>' {
                        break;
                    }
                    tag.push(cc);
                }
                let t = tag.trim().to_ascii_lowercase();
                if t == "/p" || t == "/li" || t == "/h1" || t == "/h2" || t == "/h3"
                    || t == "/h4" || t == "/ul" || t == "/ol" || t == "br" || t == "/div"
                {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
            '&' => {
                // 常见实体
                let rest: String = chars.clone().take(8).collect();
                let lower = rest.to_ascii_lowercase();
                if lower.starts_with("amp;") {
                    out.push('&');
                    for _ in 0..4 {
                        chars.next();
                    }
                } else if lower.starts_with("lt;") {
                    out.push('<');
                    for _ in 0..3 {
                        chars.next();
                    }
                } else if lower.starts_with("gt;") {
                    out.push('>');
                    for _ in 0..3 {
                        chars.next();
                    }
                } else if lower.starts_with("nbsp;") {
                    out.push(' ');
                    for _ in 0..5 {
                        chars.next();
                    }
                } else if lower.starts_with("quot;") {
                    out.push('"');
                    for _ in 0..5 {
                        chars.next();
                    }
                } else if lower.starts_with("apos;") {
                    out.push('\'');
                    for _ in 0..5 {
                        chars.next();
                    }
                } else {
                    out.push('&');
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// 网络下载模块 UI 状态（仅在 BETA_DOWNLOAD 启用时初始化，默认关零占用）
struct DlUiState {
    // 自定义下载（任意 URL）
    custom_url: String,
    custom_file_name: String,
    custom_dest_dir: String,
    custom_threads: String,
    custom_busy: bool,
    custom_progress: download::DlProgress,
    custom_shared: Option<std::sync::Arc<std::sync::Mutex<download::DlProgress>>>,
    // 模组下载
    mod_query: String,
    mod_mc_version: String,
    mod_loader: String,
    /// 项目类型：mod / plugin / datapack / resourcepack / shader / 全部
    mod_project_type: String,
    /// 标签过滤（逗号分隔 categories）
    mod_tags: String,
    /// 排序：relevance(默认/相关) / downloads / follows / newest / updated(最近)
    mod_sort: String,
    /// 每页显示模组数量（1-20，默认 20）
    mod_page_size: i32,
    /// 当前搜索结果页码（0-based）
    mod_page: i64,
    /// 搜索结果总命中数（分页用，来自 API total_hits）
    mod_total_hits: i64,
    /// 目标位置：true=跟随所选服务器自动归类，false=自选目录
    mod_target_use_server: bool,
    mod_results: Vec<modrinth::ModrinthHit>,
    mod_search_busy: bool,
    /// 搜索参数在搜索进行中再次变化时置位：完成后立即用最新参数重搜（修复调数量后只显示一页/一个）
    mod_search_pending: bool,
    /// 本页是否已做过首次自动搜索（进入搜索页时空关键词拉一批热门，避免空列表）
    mod_search_started: bool,

    mod_search_error: Option<String>,
    mod_download_name: String,
    mod_download_target: String,
    mod_dl_busy: bool,
    mod_dl_progress: download::DlProgress,
    // 日志
    dl_log: Vec<String>,
    mod_dl_shared: Option<std::sync::Arc<std::sync::Mutex<download::DlProgress>>>,
    mod_search_shared:
        Option<std::sync::Arc<std::sync::Mutex<Option<Result<modrinth::SearchResult, String>>>>>,
    /// 模组安装目标服务器目录
    mod_target_dir: String,
    /// 图2 社区左侧导航选中项
    mod_nav: ModCommunityNav,
    /// 收藏夹详情加载中
    mod_fav_busy: bool,
    mod_fav_error: Option<String>,
    /// 收藏夹详情缓存（tick 回传）
    mod_fav_hits: Vec<modrinth::ModrinthHit>,
    /// 收藏夹详情线程回传
    mod_fav_shared:
        Option<std::sync::Arc<std::sync::Mutex<Option<Result<Vec<modrinth::ModrinthHit>, String>>>>>,
    /// 返回顶部标记（渲染时消费）
    mod_scroll_top: bool,
    /// 当前展开详情的项目 ID（None=收起）
    mod_detail_id: Option<String>,
    /// 当前展开详情的项目信息（详情头展示）
    mod_detail_hit: Option<modrinth::ModrinthHit>,
    /// 已拉取的版本列表（详情页）
    mod_versions: Vec<modrinth::ModrinthVersion>,
    /// 版本列表加载中
    mod_ver_busy: bool,
    mod_ver_error: Option<String>,
    mod_ver_shared:
        Option<std::sync::Arc<std::sync::Mutex<Option<Result<Vec<modrinth::ModrinthVersion>, String>>>>>,
    /// 详情页加载器筛选
    mod_ver_loader: String,
    /// 详情打开时记录的搜索 MC 版本（反馈③：版本列表置顶高亮）
    mod_detail_mc: String,
    /// 详情页简介翻译（None=未翻译/收起清理）
    mod_translate_result: Option<String>,
    /// 简介译文是否被用户隐藏（隐藏仅置位、不清缓存；再次点翻译直接复用缓存）
    mod_translate_hidden: bool,
    /// 简介翻译加载中
    mod_translate_busy: bool,
    mod_translate_error: Option<String>,
    /// 简介翻译线程回传
    mod_translate_shared:
        Option<std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>>,
    /// 详情页当前展开更新日志的版本 id（None=未展开）
    mod_ver_log_open: Option<String>,
    /// 详情页右侧预览选中的版本（None=未选中；点击版本行时更新）
    mod_ver_preview: Option<modrinth::ModrinthVersion>,
    /// 详情页是否显示快照版本（阶段16②：默认 false=隐藏 snapshot- 开头版本）
    mod_show_snapshot: bool,
    /// 打开页面回传（拉取 source_url 后打开系统浏览器）
    mod_open_shared: Option<std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>>,
    /// 模组图标缓存：project_id -> 已解码纹理（图2 从网站获取）
    mod_icon_tex: HashMap<String, egui::TextureHandle>,
    /// 正在下载图标的 project_id 集合（避免重复请求）
    mod_icon_pending: HashSet<String>,
    /// 图标下载线程回传：project_id -> 字节结果
    mod_icon_shared: HashMap<
        String,
        std::sync::Arc<std::sync::Mutex<Option<Result<Vec<u8>, String>>>>,
    >,
    /// 图标下载/解码失败的 project_id 集合（避免每帧重复发起线程风暴）
    mod_icon_failed: HashSet<String>,
    /// 模组中文名缓存：英文 title -> 中文名（title 本身为中文时等于原文）
    mod_cn_name: HashMap<String, String>,
    /// 正在翻译标题的 title 集合（避免重复请求）
    mod_cn_pending: HashSet<String>,
    /// 标题翻译线程回传：title -> 结果
    mod_cn_shared: HashMap<
        String,
        std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
    >,
    /// 标题自动翻译失败的 title 集合（显示"译"按钮供手动重试）
    mod_cn_failed: HashSet<String>,
    /// MC 百科命中缓存：title -> 百科页 URL（中文名复用 mod_cn_name）
    mod_mcmod_url: HashMap<String, String>,
    /// 正在查询 MC 百科的 title 集合（避免重复请求）
    mod_mcmod_pending: HashSet<String>,
    /// MC 百科查询线程回传：title -> (中文名, 百科URL)
    mod_mcmod_shared: HashMap<
        String,
        std::sync::Arc<std::sync::Mutex<Option<Result<(String, String), String>>>>,
    >,
    /// 内置 MC 百科离线库（mcmod.raw，按 slug 精确查表，与 PCL-CE 同源）
    mcmod_db: mcmod_db::McmodDb,
}

impl Default for DlUiState {
    fn default() -> Self {
        Self {
            custom_url: String::new(),
            custom_file_name: String::new(),
            custom_dest_dir: String::new(),
            custom_threads: "32".to_string(),
            custom_busy: false,
            custom_progress: download::DlProgress::default(),
            custom_shared: None,
            mod_query: String::new(),
            mod_mc_version: String::new(),
            mod_loader: "fabric".to_string(),
            mod_project_type: "mod".to_string(),
            mod_tags: String::new(),
            mod_sort: "relevance".to_string(),
            mod_page_size: 20,
            mod_page: 0,
            mod_total_hits: 0,
            mod_target_use_server: false,
            mod_results: Vec::new(),
            mod_search_busy: false,
            mod_search_pending: false,
            mod_search_started: false,

            mod_search_error: None,
            mod_download_name: String::new(),
            mod_download_target: String::new(),
            mod_dl_busy: false,
            mod_dl_progress: download::DlProgress::default(),
            dl_log: Vec::new(),
            mod_dl_shared: None,
            mod_search_shared: None,
            mod_target_dir: String::new(),
            mod_nav: ModCommunityNav::Mod,
            mod_fav_busy: false,
            mod_fav_error: None,
            mod_fav_hits: Vec::new(),
            mod_fav_shared: None,
            mod_scroll_top: false,
            mod_detail_id: None,
            mod_detail_hit: None,
            mod_versions: Vec::new(),
            mod_ver_busy: false,
            mod_ver_error: None,
            mod_ver_shared: None,
            mod_ver_loader: "全部".to_string(),
            mod_detail_mc: String::new(),
            mod_translate_result: None,
            mod_translate_hidden: false,
            mod_translate_busy: false,
            mod_translate_error: None,
            mod_translate_shared: None,
            mod_ver_log_open: None,
            mod_ver_preview: None,
            mod_show_snapshot: false,
            mod_open_shared: None,
            mod_icon_tex: HashMap::new(),
            mod_icon_pending: HashSet::new(),
            mod_icon_shared: HashMap::new(),
            mod_icon_failed: HashSet::new(),
            mod_cn_name: HashMap::new(),
            mod_cn_pending: HashSet::new(),
            mod_cn_shared: HashMap::new(),
            mod_cn_failed: HashSet::new(),
            mod_mcmod_url: HashMap::new(),
            mod_mcmod_pending: HashSet::new(),
            mod_mcmod_shared: HashMap::new(),
            mcmod_db: mcmod_db::McmodDb::new(),
        }
    }
}

/// 创建服务器弹窗状态（B7：选类型 -> 自动拉取版本 -> 搜索过滤 -> 下载 -> 建服）
struct CreateServerState {
    kind: server_download::ServerKind,
    /// 已拉取的版本列表
    versions: Vec<String>,
    version_busy: bool,
    version_error: Option<String>,
    /// 版本搜索过滤词
    version_filter: String,
    /// 是否显示快照版（默认隐藏：快照版不适合长期开服）
    show_snapshot: bool,
    /// 已选版本（显示名）
    selected_version: String,
    /// 服务器文件夹名（data/servers/<folder_name>）
    folder_name: String,
    /// 下载中
    downloading: bool,
    progress: download::DlProgress,
    /// 下载线程共享进度（线程写，tick 读后拷入 progress）
    progress_shared: Option<std::sync::Arc<std::sync::Mutex<download::DlProgress>>>,
    /// 版本列表线程回传（None 进行中，Some 为结果）
    version_shared: Option<std::sync::Arc<std::sync::Mutex<Option<Result<Vec<String>, String>>>>>,
    /// 下载完成且已建服（显示"启动服务器"按钮）
    finished: bool,
    /// 建服成功后的服务器下标
    created_idx: Option<usize>,
    last_error: Option<String>,
}

impl CreateServerState {
    fn new() -> Self {
        Self {
            kind: server_download::ServerKind::Vanilla,
            versions: Vec::new(),
            version_busy: false,
            version_error: None,
            version_filter: String::new(),
            show_snapshot: false,
            selected_version: String::new(),
            folder_name: String::new(),
            downloading: false,
            progress: download::DlProgress::default(),
            progress_shared: None,
            version_shared: None,
            finished: false,
            created_idx: None,
            last_error: None,
        }
    }

    /// 过滤后的版本列表（搜索框 + 快照开关）
    fn filtered_versions(&self) -> Vec<String> {
        let f = self.version_filter.trim();
        self.versions
            .iter()
            .filter(|v| self.show_snapshot || !is_snapshot_version(v))
            .filter(|v| f.is_empty() || v.contains(f))
            .cloned()
            .collect()
    }
}

/// 判断版本号是否为快照/预发布版（Mojang 版本清单里快照形如 `24w45a`、预发布形如 `1.21.4-pre1`/`-rc1`）。
fn is_snapshot_version(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    if l.contains("-pre") || l.contains("-rc") || l.contains("snapshot") || l.contains("experimental") {
        return true;
    }
    // 快照周版本：24w45a / 1.21.2-pre1 之外还有 25w03a 这种
    let b = l.as_bytes();
    b.len() >= 5
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2] == b'w'
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
}

/// 服务器页子页
#[derive(PartialEq, Clone, Copy)]
enum ServerTab {
    Overview,
    Scripts,
    Files,
    /// 自动功能（备�?+ 自动重启�?
    Backup,
    /// 独立性能�?
    Perf,
    /// 状态页（玩家列表日志解析 + 性能/网络）
    Status,
    /// 玩家管理页（B1 四 Tab：在线/白名单/封禁/OP，BETA_PLAYERS 启用时显示）
    Players,
    /// 特殊功能页（Spark 性能分析：总开关 BETA_SPECIAL + Spark 子开关 FEATURE_SPARK，2026-10-02 回归测试功能，默认禁用）
    Special,
}

/// 玩家管理页子页签（B1）
#[derive(PartialEq, Clone, Copy)]
enum PlayerTab {
    Online,
    Whitelist,
    Banned,
    Ops,
    Props,
}

/// 玩家列表快照状态（B5 防竞态：状态模型补 Unknown）
#[derive(PartialEq, Clone, Copy)]
enum PlayersState {
    /// 数据已加载（最近一次快照成功）
    Ready,
    /// 刷新中（操作成功后的延迟刷新窗口）
    Refreshing,
    /// 未知（服务器目录缺失 / 文件不可读 / 数据源暂不可用）
    Unknown,
}

/// 玩家快照（白名单 / 封禁玩家 / 封禁 IP / OP 列表），异步线程读取后经通道回传
#[derive(Default, Clone)]
struct PlayersSnapshot {
    wl: Vec<serde_json::Value>,
    bp: Vec<serde_json::Value>,
    bi: Vec<serde_json::Value>,
    ops: Vec<serde_json::Value>,
}

/// 玩家快照异步回传消息：(server_idx, 请求序号, 快照)
type PlayersMsg = (usize, u64, PlayersSnapshot);

struct ServerRuntime {
    proc: Option<process::ManagedProcess>,
    log_buf: String,
    /// 自上次渲染以来新增的日志行数（用于贴底判断；缓冲被裁剪后长度不再变化，须按行数判断）
    log_pending: usize,
    /// 日志文件尾随器：MC 服务器优先从 logs/latest.log 增量读日志，
    /// 绕开 stdout 管道�?Java 侧缓冲导致日志停滞的问题；无效时回退 stdout�?
    log_tail: Option<process::LogFileTail>,
    // 脚本编辑状�? (run.bat 内容, user_jvm_args 内容)
    run_bat_edit: Option<String>,
    jvm_args_edit: Option<String>,
    // server.properties 编辑状�? (键值对缓存, 高级文本缓冲, 是否高级模式)
    server_props_map: Option<std::collections::BTreeMap<String, String>>,
    server_props_text: Option<String>,
    server_props_advanced: bool,
    // 文件浏览
    file_tab: String,
    /// file_sub 对应的页签，切换页签时清空子目录�?
    file_tab_sub: String,
    /// 当前浏览的子目录栈（相对页签根目录）
    file_sub: Vec<String>,
    file_list: Vec<(String, u64, bool)>, // (name, size, is_dir)
    file_content_edit: Option<(String, String)>, // (path, content)
    /// 文件浏览搜索关键字（过滤文件名）
    file_search: String,
    /// 收藏的文件名（置顶显示）
    file_favs: Vec<String>,
    /// 客户端模组排查结果（文件名列表；None=未排查）。仅临时 UI 状态，切走页面/切换服务器即清除，不持久化
    file_client_mods: Option<Vec<String>>,
    /// 排查客户端模组按钮的二次确认弹窗是否显示
    file_client_mods_confirm: bool,
    /// 右侧预览目标: (文件名, 大小, 是否目录, 目录内文件数)，None=未选中
    file_preview: Option<(String, u64, bool, usize)>,
    /// 文件重命名弹窗: Some(旧文件名)
    file_rename: Option<String>,
    /// 文件重命名弹窗: 编辑中的新文件名（跨帧持久，避免每帧重建导致输入丢失）
    file_rename_draft: String,
    /// 文件删除二次确认弹窗: Some(待删除文件名)；确认后删到回收站（禁止永久删除）
    file_delete_confirm: Option<String>,
    backup_working: bool,
    last_msg: String,
    /// 是否正在后台执行优雅停止（进程已移交后台线程，等待保存退出）
    stopping: bool,
    /// 概览�?日志页的命令输入框内�?
    cmd_input: String,
    /// 已发送过的命令历史（用于上下键回看）
    cmd_history: Vec<String>,
    /// 历史浏览位置（None=不在历史浏览中）
    cmd_hist_pos: Option<usize>,
    /// 点击日志区域后请求聚焦命令输入框（渲染后复位�?
    cmd_focus: bool,
    /// 切回概览/点击"回到底部"后请求强制贴底（渲染后复位）
    force_scroll_bottom: bool,
    /// 进程性能采样（独立性能页图表）
    perf: perf::PerfMonitor,
    /// 是否已发送过"启动完成"Windows 通知（防重复�?
    startup_notified: bool,
    /// 是否已向插件事件总线发送过 server_started（防重复，进程存活期间仅一次）
    plugin_start_emitted: bool,
    /// 本次启动时刻（用�?就绪超时"提示�?
    started_at: Option<std::time::Instant>,
    /// 是否已提示过"未检测到就绪标志"（防重复�?
    startup_warned: bool,
    /// 崩溃重启：当前窗口内连续崩溃次数
    crash_count: u32,
    /// 崩溃重启：熔断窗口起点（None=窗口未开始）
    crash_first_at: Option<std::time::Instant>,
    /// 崩溃重启：计划重启时刻（None=无待执行重启�?
    crash_restart_at: Option<std::time::Instant>,
    // 白名单/黑名单管理：添加输入框缓存（egui TextEdit 需要跨帧持久化文本）
    wl_add_name: String,
    ban_add_name: String,
    ban_add_reason: String,
    banip_add_ip: String,
    banip_add_reason: String,
    /// 崩溃报告分析结果缓存（None=尚未分析）
    crash_analysis: Option<String>,
    /// 崩溃来源列表：(kind, filename)，kind: 0=latest.log, 1=crash-reports/*.txt, 2=hs_err_pid*.log
    crash_list: Vec<(u8, String)>,
    /// 当前选中的单文件分析目标
    crash_sel: Option<(u8, String)>,
    // ---------- 玩家管理（B1/B5，BETA_PLAYERS） ----------
    /// 玩家管理页当前子页签
    players_tab: PlayerTab,
    /// 玩家列表快照状态（Ready/Refreshing/Unknown）
    players_state: PlayersState,
    /// 请求序号：每次快照刷新递增；延迟刷新执行时序号不匹配则丢弃（防 A/B 服数据错位）
    players_seq: u64,
    /// 最近一次成功快照关联的请求序号
    players_snapshot_seq: u64,
    /// 操作成功后的计划延迟刷新时刻（1-2s）
    players_refresh_at: Option<std::time::Instant>,
    /// 计划延迟刷新时记录的请求序号（执行时校验）
    players_refresh_seq: u64,
    /// 前台 5s 轮询：上次快照刷新时刻（None=立即刷新一次）
    players_last_tick: Option<std::time::Instant>,
    /// 在线玩家快照缓存（前台 5s 轮询 + 日志解析）
    players_online: Vec<String>,
    /// 最近一次成功快照（白名单/封禁/OP；异步回传后写入）
    players_snapshot: Option<PlayersSnapshot>,
    /// 仪表盘：目录占用字节（后台线程刷新）
    dir_size: std::sync::Arc<std::sync::Mutex<u64>>,
    /// 仪表盘：目录大小上次计算时刻
    dir_size_at: Option<std::time::Instant>,
    /// OP 添加输入框缓存
    op_add_name: String,
    /// mods 内 .jar 更新中（防重复触发）
    mod_update_busy: bool,
    /// 更新状态消息（显示在文件行旁）
    mod_update_msg: String,
    /// mods 内 .jar 更新回传（成功=Ok(新文件名)，失败=Err(原因)）
    mod_update_shared: Option<std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>>,
    /// 待执行更新：(下载URL, 新文件名, 旧文件名, 旧文件是否 .jar.disabled, 新版本号)
    mod_update_pending: Option<(String, String, String, bool, String)>,
    /// 当前检查/更新目标文件名（用于行内状态显示）
    mod_update_target: String,
    /// 当前检查/更新目标是否为 .jar.disabled
    mod_update_target_disabled: bool,
    /// 特殊功能：Spark 检测结果（None=未检测）；Some((可用, 命中的 spark jar 文件名列表))
    spark_detect: Option<(bool, Vec<String>)>,
    /// 特殊功能：Spark 输出文件列表：(所在目录, 文件名, 大小字节, 修改时间秒)
    spark_files: Vec<(String, String, u64, i64)>,
    /// 特殊功能：是否已扫描过输出文件列表（空目录也置位，避免每帧重扫）
    spark_scanned: bool,
    /// 特殊功能：Spark 分析状态（某文件解析结果缓存；None=尚未解析）
    spark_analysis: Option<spark_analysis::SparkAnalysisState>,
    /// 特殊功能：正在解析（防重复触发）
    spark_parsing: bool,
    /// 特殊功能：Spark 性能分析进行中状态（None=未在分析）
    spark_prof: Option<SparkProfiling>,
    /// 特殊功能：发送 stop --save-to-file 后延迟刷新输出文件列表的时刻
    spark_prof_refresh_at: Option<std::time::Instant>,
    /// 特殊功能：分析时长选择（秒）
    spark_prof_secs: u64,
    /// 特殊功能：分析结果预览子页签（概览/波动曲线/模组性能/卡顿热点，仿服务器顶栏可切换）
    spark_view_tab: spark_analysis::SparkViewTab,
    /// 特殊功能：模组详情展开目标（None=未展开）
    spark_mod_detail: Option<String>,
    /// 特殊功能：mods 目录 jar 定位索引缓存（jar 名 + 元数据别名；None=未扫描）
    mod_jar_index: Option<Vec<spark_analysis::ModJarInfo>>,
    /// 文件浏览：滚动定位目标（命中后滚动到该行并清空）
    file_scroll_to: Option<String>,
}

/// Spark 性能分析进行中状态（问题2：开始分析后本地计时推进，到期自动发送 stop --save-to-file）
struct SparkProfiling {
    /// 总时长（秒）
    total_secs: u64,
    /// 开始时刻
    start: std::time::Instant,
    /// 是否已发送 stop --save-to-file（到期触发一次）
    sent_stop: bool,
}

/// Frp 后台任务回传（检查更�?/ 下载更新 / 进度日志�?
enum FrpTaskMsg {
    /// 检查更新结果：Ok((本地版本, 远程版本))
    CheckUpdate(Result<(String, String), String>),
    /// 下载/更新结果：Ok(新版本号)
    Download(Result<String, String>),
    /// 下载过程进度文本（写�?frp 日志，避免用户不知道当前在哪一步）
    Progress(String),
}

/// 设置页可折叠区块状态（平滑高度动画�?
struct SettingsSection {
    id: &'static str,
    open: bool,
    /// 展开比例 0..=1（动画插值）
    anim: f32,
    /// 上一帧内容实际高度（用于裁剪动画�?
    last_h: f32,
}

/// 内网穿透页左侧栏分页
#[derive(PartialEq, Clone, Copy)]
enum TunnelSide {
    Dashboard,
    Create,
    Manage,
    Logs,
    Tutorial,
}

/// 设置页左侧栏分组
#[derive(PartialEq, Clone, Copy)]
enum SettingsSide {
    General,
    Logs,
    Ui,
    Java,
    Notify,
    Beta,
}

/// 右下角自绘通知（侧�?淡入，滞留后自动收回�?
struct ToastMsg {
    id: u64,
    title: String,
    body: String,
    born: std::time::Instant,
}

impl Default for ServerRuntime {
    fn default() -> Self {
        Self {
            proc: None,
            log_buf: String::new(),
            log_pending: 0,
            log_tail: None,
            run_bat_edit: None,
            jvm_args_edit: None,
            server_props_map: None,
            server_props_text: None,
            server_props_advanced: false,
            file_tab: "mods".to_string(),
            file_tab_sub: String::new(),
            file_sub: Vec::new(),
            file_list: Vec::new(),
            file_content_edit: None,
            file_search: String::new(),
            file_favs: Vec::new(),
            file_client_mods: None,
            file_client_mods_confirm: false,
            file_preview: None,
            file_rename: None,
            file_rename_draft: String::new(),
            file_delete_confirm: None,
            backup_working: false,
            last_msg: String::new(),
            stopping: false,
            cmd_input: String::new(),
            cmd_history: Vec::new(),
            cmd_hist_pos: None,
            cmd_focus: false,
            force_scroll_bottom: false,
            perf: perf::PerfMonitor::new(),
            startup_notified: false,
            plugin_start_emitted: false,
            started_at: None,
            startup_warned: false,
            crash_count: 0,
            crash_first_at: None,
            crash_restart_at: None,
            wl_add_name: String::new(),
            ban_add_name: String::new(),
            ban_add_reason: String::new(),
            banip_add_ip: String::new(),
            banip_add_reason: String::new(),
            crash_analysis: None,
            crash_list: Vec::new(),
            crash_sel: None,
            players_tab: PlayerTab::Online,
            players_state: PlayersState::Unknown,
            players_seq: 0,
            players_snapshot_seq: 0,
            players_refresh_at: None,
            players_refresh_seq: 0,
            players_last_tick: None,
            players_online: Vec::new(),
            players_snapshot: None,
            dir_size: std::sync::Arc::new(std::sync::Mutex::new(0)),
            dir_size_at: None,
            op_add_name: String::new(),
            mod_update_busy: false,
            mod_update_msg: String::new(),
            mod_update_shared: None,
            mod_update_pending: None,
            mod_update_target: String::new(),
            mod_update_target_disabled: false,
            spark_detect: None,
            spark_files: Vec::new(),
            spark_scanned: false,
            spark_analysis: None,
            spark_parsing: false,
            spark_prof: None,
            spark_prof_refresh_at: None,
            spark_prof_secs: 60,
            spark_view_tab: spark_analysis::SparkViewTab::Overview,
            spark_mod_detail: None,
            mod_jar_index: None,
            file_scroll_to: None,
        }
    }
}

struct TunnelRuntime {
    proc: Option<process::ManagedProcess>,
    log_buf: String,
    /// 自上次渲染以来新增的日志行数（用于贴底判断）
    log_pending: usize,
    /// 是否正在手动停止（抑制异常退出通知�?
    stopping: bool,
    /// 连接异常已通知（防重复弹提示，启动时重置）
    err_notified: bool,
    /// 流量统计：frpc admin API 上次采样的进程内累计字节
    traffic_last_in: u64,
    traffic_last_out: u64,
    /// 上次流量采样时间（秒时间戳）
    traffic_last_at: Option<f64>,
    /// 速率采样历史（字节/秒，用于迷你波动图，最多 30 点）
    traffic_hist_in: VecDeque<f32>,
    traffic_hist_out: VecDeque<f32>,
}

impl Default for TunnelRuntime {
    fn default() -> Self {
        Self {
            proc: None,
            log_buf: String::new(),
            log_pending: 0,
            stopping: false,
            err_notified: false,
            traffic_last_in: 0,
            traffic_last_out: 0,
            traffic_last_at: None,
            traffic_hist_in: VecDeque::new(),
            traffic_hist_out: VecDeque::new(),
        }
    }
}

struct App {
    cfg: GlobalConfig,
    nav: Nav,
    server_tab: ServerTab,
    selected_server: Option<usize>,
    runtimes: Vec<ServerRuntime>,
    tunnel_runtimes: Vec<TunnelRuntime>,
    /// Frp 集中管理：单进程（frpc -c frpc.toml）句�?
    frp_proc: Option<process::ManagedProcess>,
    /// Frp 集中管理日志（下�?启动/异常�?
    frp_log: String,
    /// Frp 集中管理流量：上次采样时间 / 上次累计字节 / 实时速率 / 累计总量
    frp_traffic_last_at: Option<f64>,
    frp_traffic_last_in: u64,
    frp_traffic_last_out: u64,
    frp_traffic_in_rate: f32,
    frp_traffic_out_rate: f32,
    frp_traffic_in_total: u64,
    frp_traffic_out_total: u64,
    /// frpc 是否正在下载中（防止重复触发�?
    frp_busy: bool,
    /// Frp 后台下载+启动回传通道
    frp_tx: std::sync::mpsc::Sender<Result<process::ManagedProcess, String>>,
    frp_rx: std::sync::mpsc::Receiver<Result<process::ManagedProcess, String>>,
    /// Frp 后台任务（检查更�?/ 下载）回传通道
    frp_task_tx: std::sync::mpsc::Sender<FrpTaskMsg>,
    frp_task_rx: std::sync::mpsc::Receiver<FrpTaskMsg>,
    /// 正在检�?frpc 更新
    frp_ver_busy: bool,
    /// 正在下载/更新 frpc
    frp_dl_busy: bool,
    /// 待确认的 frpc 更新�?(本地版本, 远程版本)
    frp_update_pending: Option<(String, String)>,
    /// 本地 frpc 版本缓存（显示用�?
    frp_local_ver: String,
    /// 设置页折叠区块状�?
    settings_sections: Vec<SettingsSection>,
    /// [TEST-HOOK] 自动选中测试计时（验证后删除�?
    test_hook_t: std::time::Instant,
    backup_inflight: HashSet<usize>,
    backup_tx: std::sync::mpsc::Sender<(usize, Result<backup::BackupResult, String>)>,
    backup_rx: std::sync::mpsc::Receiver<(usize, Result<backup::BackupResult, String>)>,
    /// 优雅停止后台线程回传�?(服务器下�? 是否优雅完成)
    stop_tx: std::sync::mpsc::Sender<(usize, bool)>,
    stop_rx: std::sync::mpsc::Receiver<(usize, bool)>,
    stop_inflight: HashSet<usize>,
    config_path: PathBuf,
    /// 当前 exe 绝对路径（开机自启注册表写入用）
    exe_path: PathBuf,
    add_tunnel_name: String,
    add_tunnel_kind: String,
    add_tunnel_exe: String,
    add_tunnel_cfg: String,
    add_tunnel_local_port: u16,
    add_tunnel_remote_port: u16,
    add_tunnel_proxy_type: String,
    add_tunnel_server_addr: String,
    add_tunnel_server_port: u16,
    add_tunnel_user: String,
    add_tunnel_token: String,
    // 设置页：新增 Java 表单
    add_java_name: String,
    add_java_version: String,
    add_java_path: String,
    /// 插件管理页当前选中插件名（文件浏览式右侧预览）
    plugin_sel: Option<String>,
    /// 插件页「添加配置项」的新 key 输入框
    plugin_cfg_new_key: String,
    /// B7 创建服务器弹窗（None=关闭）
    create_server: Option<CreateServerState>,
    /// B3 强停二次确认请求（显示 pid + 影响，带一次性 token，30s 过期）
    force_stop: Option<ForceStopReq>,
    /// 网络下载模块状态（BETA_DOWNLOAD 启用时 Some，默认 None 零占用）
    dl: Option<Box<DlUiState>>,
    /// 插件系统（BETA_PLUGINS 启用时 Some：rhai 宿主 + 事件总线 + zip 热加载；
    /// 默认 None，关闭时零加载零轮询零内存）
    plugins: Option<plugins::PluginManager>,
    /// 插件请求的窗口背景模式（Default=不透明；Translucent/Frosted/Acrylic 见 plugins::BgStyle）
    plugin_bg_style: plugins::BgStyle,
    /// 当前背景模式的不透明度/着色浓度 0.0-1.0
    /// （半透明=中央覆盖层 alpha；毛玻璃/亚克力=DWM accent 的 GradientColor alpha）
    plugin_bg_opacity: f32,
    /// 一次性：本次运行是否已按插件保存的 bg_style 恢复过背景效果
    plugin_bg_restored: bool,
    /// 当前生效背景效果的**归属插件**（D2）：禁用/卸载该插件时自动回收效果，
    /// 避免「插件已禁用但窗口还是半透明」的残留
    plugin_bg_owner: Option<String>,
    /// 诊断（只取一次）：GL 默认帧缓冲的 (红位数, alpha 位数)；None = 取不到 GL 上下文。
    /// alpha=0 说明像素格式根本没有 alpha 通道 —— 逐像素窗口透明不可能生效。
    gl_fb_bits: Option<(i32, i32)>,
    confirm_restore: Option<(usize, PathBuf, String)>, // (server_idx, zip, name)
    confirm_delete_backup: Option<(usize, PathBuf, String)>, // (server_idx, zip, name)
    /// 右键重命名弹窗： (server_idx, 当前�?
    rename_server: Option<(usize, String)>,
    /// 完全禁用 Defender 的确认弹窗开�?
    confirm_defender_disable: bool,
    /// �?× 且配置为彻底关闭时，若有服务器运行则弹出确认（存运行中数量）
    confirm_close: Option<usize>,
    /// 已确认退出：静默停止所有服务器，全部停止后自动退�?
    closing_exit: bool,
    /// 退出放行标志：所有服务器已停止，下次关闭请求直接放行
    ctx_close_pending: bool,
    /// 自定义标题栏拖拽起始位置（窗口外框左上角�?
    // (已改�?ViewportCommand::StartDrag 系统拖拽，此字段保留备用)
    title_drag_start: Option<egui::Pos2>,
    /// 系统托盘图标（最小化到托�?/ 关闭行为=tray 时使用）
    tray: Option<tray_icon::TrayIcon>,
    /// 托盘菜单「显示主窗口」项 ID
    tray_show_id: Option<tray_icon::menu::MenuId>,
    /// 托盘菜单「退出」项 ID
    tray_quit_id: Option<tray_icon::menu::MenuId>,
    /// 当前是否已隐藏到托盘（Arc 供托盘事件回调线程读写）
    tray_hidden: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 刚从托盘恢复窗口（托盘回调置位；update 消费后做纹理/字体/圆角全量重建）
    tray_restoring_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 托盘态清理（真正隐藏窗口 + 释放纹理/缓存）是否已完成（每次隐藏只做一次）
    tray_cleanup_done: bool,
    /// 崩溃分析结果：(服务器下标, 服务器名, 分析结论)；Some 时自动弹出小窗
    crash_report: Option<(usize, String, crashscan::CrashFinding)>,
    /// 日志页：搜索关键字
    log_query: String,
    /// 日志页：级别筛选（全部/信息/警告/错误）
    log_level: String,
    /// 日志页：来源筛选（全部 / 服务器名 / 隧道:名）
    log_src: String,
    /// 日志页：是否跟随最新
    log_follow: bool,
    /// 日志页：行高（1.0 紧凑 / 1.6 舒适）
    log_row_h: f32,
    /// 隐藏请求已受理但窗口尚未隐藏（延迟一帧隐藏：见 update 早退分支，避免隐藏窗口
    /// request_redraw 无效导致 eframe ControlFlow 滞留 Poll 空转 100% CPU）
    hide_requested: bool,
    /// 延迟隐藏命令（ViewportCommand::Visible(false)）是否已发出
    hide_sent: bool,
    /// 托盘版本号（进入/恢复托盘时递增，恢复窗口后的首次 update 消费并强制全量刷新）
    tray_version: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 上次消费的托盘版本号（update 内比对 tray_version 判断是否刚恢复窗口）
    last_tray_version: u64,
    /// egui Context 副本（供托盘回调线程 request_repaint 唤醒休眠的事件循环、供 A4 压缩线程清理缓存）
    egui_ctx: egui::Context,
    /// CJK 字体是否已触发懒加载（启动仅 ASCII，托盘态不加载，恢复/可见后异步加载一次）
    cjk_loading: bool,
    /// 托盘菜单「退出」被点击（Arc 供回调线程写入，update 轮询消费）
    tray_quit: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 托盘菜单「退出」：回调置位，update 恢复运行后消费并走退出流程
    tray_quit_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 托盘全局事件回调是否已注册（首次 update 拿到 ctx 后注册一次）
    tray_handlers_set: bool,
    /// run.bat 预览区高度（可拖动分隔条调整，所有服务器共用�?
    run_bat_h: f32,
    /// run.bat 分隔条拖动起始状态（起始高度, 起始指针Y），修复拖动漂移
    run_bat_drag_start: Option<(f32, f32)>,
    /// server.properties 预览区高度（独立于 run.bat，互不冲突）
    server_props_h: f32,
    /// server.properties 分隔条拖动起始状态
    server_props_drag_start: Option<(f32, f32)>,
    /// 内网穿透页左侧栏当前分页
    tunnel_side: TunnelSide,
    /// 内网穿透子页签切换的滑块位置（动画插值）
    tunnel_nav_anim: f32,
    /// 设置页左侧栏当前分组
    settings_side: SettingsSide,
    /// 待确认启用的测试功能 ID（弹警告确认窗）
    beta_confirm: Option<String>,
    /// 服务器删除二次确认（服务器下标）
    confirm_delete_server: Option<usize>,
    /// 隧道删除二次确认（隧道下标）
    confirm_remove_tunnel: Option<usize>,
    /// 隧道基础配置编辑窗口目标（隧道下标，None=未打开）
    tunnel_edit_idx: Option<usize>,
    /// 隧道配置弹窗编辑草稿（打开时从 cfg 拷贝一次，跨帧复用，避免每帧重建丢输入）
    tunnel_edit_draft: Option<TunnelConfig>,
    /// 文件页签排序方向：true=名称升序，false=降序（点击"名称"列头切换）
    file_sort_asc: bool,
    /// 仪表盘在线隧道双击跳转后高亮（隧道下标, 高亮截止秒数）
    tunnel_highlight: Option<(usize, f64)>,
    /// 仪表盘在线隧道单击预览的隧道下标
    tunnel_preview: Option<usize>,
    /// 隧道配置弹窗当前模式: false=简易 / true=高级
    tunnel_edit_advanced: bool,
    /// 流量累计写盘节流（秒时间戳，约 30s 一次）
    traffic_save_at: f64,
    /// 创建隧道填写方式: "form"=表单项（默认） / "toml"=直接编辑 frpc.toml
    add_tunnel_mode: String,
    // ---------- 阶段5：rathole 穿透内核 ----------
    /// rathole 创建表单：server 地址
    add_rh_server_addr: String,
    /// rathole 创建表单：server 端口
    add_rh_server_port: u16,
    /// rathole 创建表单：隧道名
    add_rh_tunnel_name: String,
    /// rathole 创建表单：本地转发地址
    add_rh_local_addr: String,
    /// rathole 创建表单：本地转发端口
    add_rh_local_port: u16,
    /// rathole 创建表单：服务端监听端口（remote_port）
    add_rh_remote_port: u16,
    /// rathole 创建表单：NOISE 加密开关
    add_rh_noise: bool,
    /// rathole 创建表单：NOISE 共享密钥（hex；留空自动生成）
    add_rh_noise_key: String,
    /// rathole 客户端是否正在下载/更新中（防重复触发）
    rathole_busy: bool,
    /// rathole 下载状态文本（UI 显示）
    rathole_dl_state: String,
    /// rathole 后台下载+启动回传通道
    rathole_tx: std::sync::mpsc::Sender<Result<(), String>>,
    rathole_rx: std::sync::mpsc::Receiver<Result<(), String>>,
    /// 正在远端转存备份中的服务器下标（防并发）
    remote_upload_busy: HashSet<usize>,
    /// 远端转存失败待重试：服务器下标 -> (zip 路径, 剩余次数)
    remote_pending: std::collections::HashMap<usize, (PathBuf, u32)>,
    /// 远端转存后台回传通道：服务器下标 -> 结果
    remote_tx: std::sync::mpsc::Sender<(usize, Result<(), String>)>,
    remote_rx: std::sync::mpsc::Receiver<(usize, Result<(), String>)>,
    /// 内网穿透管理/日志页搜索关键字
    tunnel_search: String,
    /// 流量统计诊断结果（隧道下标, 诊断文本），用于排查流量恒 0
    tunnel_diag: Option<(usize, String)>,
    /// 设置页全局搜索关键字
    settings_search: String,
    /// SQLite 日志库（阶段C）；初始化失败时保持 None 并在设置页展示错误
    logdb: Option<logdb::LogDb>,
    logdb_err: String,
    /// 玩家属性（阶段C）：目标玩家名
    player_prop_name: String,
    /// 玩家属性（阶段C）：目标游戏模式
    player_prop_gamemode: String,
    /// 玩家属性（阶段C）：是否 OP
    player_prop_op: bool,
    toast: String,
    /// 导航滑块动画进度 (0..2)
    nav_anim: f32,
    /// 服务器列表侧栏折叠状态（true=折叠为仅图标）
    servers_collapsed: bool,
    /// 服务器列表侧栏折叠动画进度 (0=折叠, 1=展开)
    servers_anim: f32,
    /// 服务器页签滑块动画进�?(0..3)
    tab_anim: f32,
    /// 备份到期扫描节流（后台轻量）
    last_backup_check: std::time::Instant,
    /// 内存风险提示状态（避免重复 toast�?
    mem_risk_active: bool,
    /// 性能采样限频
    last_perf_sample: std::time::Instant,
    /// 自动重启：已进入倒计时的服务器下�?
    auto_restart_pending: HashSet<usize>,
    /// 自动重启：倒计时到点时刻（服务器下�?-> 时刻�?
    auto_restart_scheduled: std::collections::HashMap<usize, std::time::Instant>,
    /// 自动重启：等待优雅停止完成后启动的服务器下标
    auto_restart_after_stop: HashSet<usize>,
    /// 自动重启检查节�?
    last_auto_restart_check: std::time::Instant,
    /// 开机自启模式（--autostart 参数触发）：等待 CPU 空闲后逐台启动勾选服务器
    autostart_mode: bool,
    /// 开机自启已触发的服务器下标
    autostart_launched: HashSet<usize>,
    /// CPU 空闲持续计时起点（None=当前非空闲）
    autostart_idle_since: Option<std::time::Instant>,
    /// 网络流量历史�?Tx B/s, Rx B/s)，整机网卡汇总，仅保留最�?120 条）
    net_hist: Vec<(f64, f64)>,
    /// 上一次网卡累计字节 + 采样时刻（纯 Rust GetIfTable2，无 PowerShell 子进程）
    net_last_bytes: Option<(u64, u64, std::time::Instant)>,
    /// 玩家快照异步回传通道（B1/B5：请求序号防 A/B 服数据错位）
    players_tx: std::sync::mpsc::Sender<PlayersMsg>,
    players_rx: std::sync::mpsc::Receiver<PlayersMsg>,
    /// 下一次采样节流时刻（2s 一次）
    net_next_t: std::time::Instant,
    /// 右下角通知队列
    toasts: Vec<ToastMsg>,
    next_toast_id: u64,
    // ---- Theme system (stage 6) ----
    /// Current interpolated palette (frames advance toward theme_target)
    theme_cur: theme::ThemeColors,
    /// Target palette from config (mode/preset/custom)
    theme_target: theme::ThemeColors,
    /// Loaded background image texture (None = no background image)
    bg_tex: Option<egui::TextureHandle>,
    /// Background image editor window visibility (ESC exits)
    bg_edit_open: bool,
    /// 界面字号基线：启动时系统 DPI 的 pixels_per_point（ui_font_scale 以此为基准缩放）
    base_ppp: f32,
    /// 设置页字号滑杆的待应用值：拖动不生效，点「应用」才写入 cfg
    pending_font_scale: f32,
    /// 窗口区域圆角缓存 (r_px, win_w, win_h)：仅在值变化时重设 SetWindowRgn
    last_win_rgn: (i32, i32, i32),
    /// 最近一次 theme::apply 返回的窗口圆角半径（点）：apply_bg 在应用 DWM accent 后
    /// 需要用它在同一步内重设 SetWindowRgn（accent 会重置窗口区域，导致圆角变尖锐直角）
    win_r_points: f32,
    /// 桌面捕获式窗口背景：本机环境下唯一可用的半透明/毛玻璃实现（详见 backdrop.rs）
    backdrop: backdrop::BackdropCapture,
    /// 最近一次已知的窗口屏幕矩形（物理像素 x,y,w,h），供桌面捕获使用
    win_rect_px: (i32, i32, i32, i32),
    /// 主窗口 HWND 缓存（避免每帧 EnumWindows；见 resolve_main_hwnd）
    hwnd_cache: Option<isize>,
    /// 自绘缩放状态：(方向, 起始鼠标物理坐标, 起始窗口物理矩形)。
    /// 无边框窗口不做系统 BeginResize，改为按住边缘时 GetCursorPos + SetWindowPos 自绘，
    /// 得到与普通窗口一致的自由缩放（可任意拖到任意尺寸，非仅最大化/最小化）。
    resize_drag: Option<(
        egui::viewport::ResizeDirection,
        (i32, i32),
        Option<(i32, i32, i32, i32)>,
    )>,
    /// 亚克力材质的噪点纹理（96×96 重复寻址，懒创建）
    noise_tex: Option<egui::TextureHandle>,
    /// 拖动/缩放期间冻结底图：最近一次「窗口矩形发生变化」的时间
    /// （判据是「本帧 vs 上一帧」，不是「本帧 vs 上次抓取」，详见 paint_bg 注释）
    bg_last_move_at: Option<std::time::Instant>,
    /// 拖动结束后需要立刻补抓一帧真实底图
    bg_needs_refresh: bool,
    /// 启动/脚本自动恢复背景效果时抑制提示条（避免开机弹一条像告警的提示）
    bg_suppress_toast: bool,
    /// 底图当前的自适应抓取间隔（ms）：内容静止时逐步放宽，内容变化时收紧
    bg_cap_interval_ms: f32,
    /// 上一次抓取到的画面均值（用于判断桌面内容是否在变化）
    bg_prev_mean: (u8, u8, u8),
    /// 左侧栏是否折叠为纯图标（持久化到配置）
    nav_collapsed: bool,
    /// 左侧栏宽度动画值（56 = 折叠，170 = 展开）
    nav_w_anim: f32,
    /// 背景配置落盘去抖（拖动不透明度滑杆时不要把每帧都写盘）
    bg_save_at: Option<std::time::Instant>,
    /// 上一帧的窗口屏幕矩形（用于判断是否正在移动）
    bg_prev_rect: (i32, i32, i32, i32),
    /// 窗口位置/大小记忆：最近一次保存的矩形与时间（避免每帧写配置）
    win_saved_rect: (i32, i32, i32, i32),
    win_save_at: Option<std::time::Instant>,
    /// 调试/自测截图（`XMST_SHOT=<png路径>`）：效果开启时窗口会被系统排除在截屏之外，
    /// 这个内置截图从自己的帧缓冲取图，是唯一能看清实际渲染结果的手段。
    shot_path: Option<std::path::PathBuf>,
    shot_frames: u32,
    shot_done: bool,
}

impl App {
    /// 当前界面**实际生效**的深浅（用于强调色/前景派生）。
    ///
    /// 判据必须取自「当前调色板的正文色」而不是配置里的 `theme_mode`：
    /// 材质模式下 `theme::auto_contrast` 会按材质明暗整体翻转深浅（明亮桌面 → 浅色界面），
    /// 此时配置里可能还是 dark/custom，若这里读配置就会出现
    /// 「界面已经变浅、但导航文字仍是给深色底设计的浅灰」→ 看不清、且“部分字体不跟随变色”。
    /// 约定：正文色偏暗 = 浅色界面（auto_contrast 浅色分支把 text 设为近黑）。
    fn theme_is_light(&self) -> bool {
        let t = self.theme_cur.text;
        let luma = 0.299 * t.r() as f32 + 0.587 * t.g() as f32 + 0.114 * t.b() as f32;
        luma < 128.0
    }

    /// 亮色模式前景适配：为深色背景设计的浅灰/浅彩文字在亮色背景上几乎不可见，
    /// 统一压暗使其达到 WCAG AA 正文 4.5:1；深色模式原样返回。
    fn fg(&self, c: egui::Color32) -> egui::Color32 {
        if self.theme_is_light() {
            theme::light_adapt(c)
        } else {
            c
        }
    }

    fn new(ctx: egui::Context) -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let config_path = exe_dir.join("data").join(CONFIG_FILE);
        // 便携 data 目录：单 exe 启动自动生成同级 data 目录（配�?日志等）
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut cfg = load_config(&config_path);
        let nav_collapsed_init = cfg.nav_collapsed;
        // 老配置兼容：admin_port 缺失时统一为 7400，重新编号避免端口冲突
        {
            let mut used: HashSet<u16> = HashSet::new();
            for t in cfg.tunnels.iter_mut() {
                if used.contains(&t.admin_port) || t.admin_port == 0 {
                    t.admin_port = (7400u16..7600).find(|p| !used.contains(p)).unwrap_or(7400);
                }
                used.insert(t.admin_port);
            }
        }
        let (backup_tx, backup_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let (rathole_tx, rathole_rx) = std::sync::mpsc::channel();
        let (remote_tx, remote_rx) = std::sync::mpsc::channel();
        let (frp_tx, frp_rx) = std::sync::mpsc::channel();
        let (frp_task_tx, frp_task_rx) = std::sync::mpsc::channel();
        let (players_tx, players_rx) = std::sync::mpsc::channel();
        // 正式功能（下载/玩家/插件）默认开启：启动时按持久化 feature 恢复运行态。
        // Bug4：此前 dl 硬编码 None，导致「设置已开启、重启后入口误判未开启」。
        let dl_init = features::is_enabled(&cfg.features, features::BETA_DOWNLOAD);
        let plugins_init = Self::init_plugins(&cfg);
        let theme_cur = theme::target_colors(
            &cfg.theme_mode,
            cfg.theme_mode == "custom",
            cfg.custom_accent,
            cfg.custom_bg,
            cfg.custom_highlight,
        );
        // 字号基线需在 egui_ctx: ctx 移动前读取
        let base_ppp = ctx.pixels_per_point();
        // 待应用字号初始跟随配置（cfg 随后被整体移入 self.cfg）
        let initial_font_scale = cfg.ui_font_scale;
        let mut app = Self {
            cfg,
            nav: Nav::Servers,
            server_tab: ServerTab::Overview,
            selected_server: None,
            runtimes: Vec::new(),
            tunnel_runtimes: Vec::new(),
            frp_proc: None,
            frp_log: String::new(),
            frp_traffic_last_at: None,
            frp_traffic_last_in: 0,
            frp_traffic_last_out: 0,
            frp_traffic_in_rate: 0.0,
            frp_traffic_out_rate: 0.0,
            frp_traffic_in_total: 0,
            frp_traffic_out_total: 0,
            frp_busy: false,
            backup_inflight: HashSet::new(),
            backup_tx,
            backup_rx,
            stop_tx,
            stop_rx,
            stop_inflight: HashSet::new(),
            config_path,
            exe_path: std::env::current_exe().unwrap_or_default(),
            add_tunnel_name: String::new(),
            add_tunnel_kind: "frp".to_string(),
            add_tunnel_exe: String::new(),
            add_tunnel_cfg: String::new(),
            add_tunnel_local_port: 25565,
            add_tunnel_remote_port: 25565,
            add_tunnel_proxy_type: "tcp".to_string(),
            add_tunnel_server_addr: String::new(),
            add_tunnel_server_port: 7000,
            add_tunnel_user: String::new(),
            add_tunnel_token: String::new(),
            add_java_name: String::new(),
            add_java_version: String::new(),
            add_java_path: String::new(),
            plugin_sel: None,
            create_server: None,
            force_stop: None,
            dl: if dl_init {
                Some(Box::<DlUiState>::default())
            } else {
                None
            },
            plugins: plugins_init,
            plugin_bg_style: plugins::BgStyle::Default,
            plugin_bg_opacity: 0.8,
            plugin_bg_restored: false,
            plugin_bg_owner: None,
            gl_fb_bits: None,
            confirm_restore: None,
            confirm_delete_backup: None,
            rename_server: None,
            confirm_defender_disable: false,
            confirm_close: None,
            closing_exit: false,
            ctx_close_pending: false,
            title_drag_start: None,
            tray: None,
            tray_show_id: None,
            tray_quit_id: None,
            tray_hidden: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tray_restoring_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tray_cleanup_done: false,
            crash_report: None,
            log_query: String::new(),
            log_level: "全部".to_string(),
            log_src: "全部".to_string(),
            log_follow: true,
            log_row_h: 1.0,
            hide_requested: false,
            hide_sent: false,
            tray_version: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_tray_version: 0,
            egui_ctx: ctx,
            cjk_loading: false,
            theme_cur,
            theme_target: theme_cur,
            bg_tex: None,
            plugin_cfg_new_key: String::new(),
            bg_edit_open: false,
            base_ppp,
            pending_font_scale: initial_font_scale,
            last_win_rgn: (0, 0, 0),
            win_r_points: 0.0,
            backdrop: backdrop::BackdropCapture::default(),
            win_rect_px: (0, 0, 0, 0),
            hwnd_cache: None,
            resize_drag: None,
            noise_tex: None,
            bg_last_move_at: None,
            bg_needs_refresh: false,
            bg_suppress_toast: false,
            bg_cap_interval_ms: 150.0,
            bg_prev_mean: (0, 0, 0),
            nav_collapsed: nav_collapsed_init,
            nav_w_anim: if nav_collapsed_init { 56.0 } else { 170.0 },
            bg_save_at: None,
            bg_prev_rect: (0, 0, 0, 0),
            win_saved_rect: (0, 0, 0, 0),
            win_save_at: None,
            shot_path: std::env::var("XMST_SHOT").ok().map(std::path::PathBuf::from),
            shot_frames: 0,
            shot_done: false,
            tray_quit: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tray_quit_requested: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tray_handlers_set: false,
            run_bat_h: 240.0,
            run_bat_drag_start: None,
            server_props_h: 160.0,
            server_props_drag_start: None,
            tunnel_side: TunnelSide::Dashboard,
            tunnel_nav_anim: 0.0,
            settings_side: SettingsSide::General,
            beta_confirm: None,
            confirm_delete_server: None,
            confirm_remove_tunnel: None,
            tunnel_edit_idx: None,
            tunnel_edit_draft: None,
            tunnel_edit_advanced: false,
            traffic_save_at: 0.0,
            add_tunnel_mode: "form".to_string(),
            add_rh_server_addr: String::new(),
            add_rh_server_port: 2333,
            add_rh_tunnel_name: "xmst".to_string(),
            add_rh_local_addr: "127.0.0.1".to_string(),
            add_rh_local_port: 25565,
            add_rh_remote_port: 25565,
            add_rh_noise: false,
            add_rh_noise_key: String::new(),
            rathole_busy: false,
            rathole_dl_state: String::new(),
            rathole_tx,
            rathole_rx,
            remote_upload_busy: HashSet::new(),
            remote_pending: std::collections::HashMap::new(),
            remote_tx,
            remote_rx,
            tunnel_search: String::new(),
            tunnel_diag: None,
            settings_search: String::new(),
            player_prop_name: String::new(),
            player_prop_gamemode: "survival".to_string(),
            player_prop_op: false,
            logdb: None,
            logdb_err: String::new(),
            // 托盘依赖窗口 Context 与平�?API，初始化�?new() 尾部统一完成
            //（见下方 setup_tray_and_menu 注释�?
            toast: String::new(),
            nav_anim: 0.0,
            servers_collapsed: false,
            servers_anim: 1.0,
            tab_anim: 0.0,
            last_backup_check: std::time::Instant::now(),
            mem_risk_active: false,
            last_perf_sample: std::time::Instant::now(),
            auto_restart_pending: HashSet::new(),
            auto_restart_scheduled: std::collections::HashMap::new(),
            auto_restart_after_stop: HashSet::new(),
            last_auto_restart_check: std::time::Instant::now(),
            autostart_mode: std::env::args().any(|a| a == "--autostart"),
            autostart_launched: HashSet::new(),
            autostart_idle_since: None,
            net_hist: Vec::new(),
            net_last_bytes: None,
            players_tx: players_tx,
            players_rx: players_rx,
            net_next_t: std::time::Instant::now(),
            toasts: Vec::new(),
            next_toast_id: 0,
            frp_tx,
            frp_rx,
            frp_task_tx,
            frp_task_rx,
            frp_ver_busy: false,
            frp_dl_busy: false,
            frp_update_pending: None,
            frp_local_ver: String::new(),
            settings_sections: Vec::new(),
            file_sort_asc: true,
            tunnel_highlight: None,
            tunnel_preview: None,
            test_hook_t: std::time::Instant::now(),
        };
        app.runtimes = (0..app.cfg.servers.len()).map(|_| ServerRuntime::default()).collect();
        app.tunnel_runtimes = (0..app.cfg.tunnels.len()).map(|_| TunnelRuntime::default()).collect();
        // 阶段C：初始化 SQLite 日志库（失败不阻塞主流程，设置页展示错误）
        match app.open_logdb() {
            Ok(db) => app.logdb = Some(db),
            Err(e) => app.logdb_err = e,
        }
        // 初始化系统托盘（失败不阻塞主流程，仅无托盘功能）
        // 托盘事件用全局回调线程（set_event_handler），不在 UI 线程轮询，
        // 因此把 hidden/quit 状态做成 Arc 由回调写、update 读。
        let (tray, show_id, quit_id) = setup_tray_and_menu();
        app.tray = tray;
        app.tray_show_id = show_id;
        app.tray_quit_id = quit_id;
        app
    }

    /// 阶段C：打开 SQLite 日志库（data/logs/xmst_logs.db，行数上限 50000 自动轮转）。
    fn open_logdb(&self) -> Result<logdb::LogDb, String> {
        let logs_dir = self.data_dir().join("logs");
        let _ = std::fs::create_dir_all(&logs_dir);
        logdb::LogDb::open(&logs_dir.join("xmst_logs.db"), logdb::DEFAULT_MAX_ROWS)
    }

    /// 阶段C：frpc 热重载（reload -c 配置文件），不重启进程；仅 frp 隧道支持（rathole 无等价能力）。
    /// 热重载进程不托管 Job（瞬间退出，避免 KILL_ON_JOB_CLOSE 竞争），detached 运行。
    fn frp_reload(&mut self, idx: usize) {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let t = match self.cfg.tunnels.get(idx) {
            Some(t) => t.clone(),
            None => return,
        };
        if t.kind != "frp" {
            self.set_toast("仅 frp 隧道支持热重载".to_string());
            return;
        }
        let frp_dir = self.data_dir().join("frp");
        let exe = if t.exe.trim().is_empty() {
            if !frp_dir.join("frpc.exe").exists() {
                self.set_toast("未找到 frpc.exe，请先在 Frp 页下载或导入".to_string());
                return;
            }
            frp_dir.join("frpc.exe")
        } else {
            PathBuf::from(t.exe.trim())
        };
        let cfg_path = frp_dir.join(format!("tunnel_{idx}.toml"));
        if !cfg_path.exists() {
            self.set_toast("找不到该隧道的 frpc.toml（可能未由 XMST 启动）".to_string());
            return;
        }
        let mut cmd = Command::new(&exe);
        cmd.args(["reload", "-c", cfg_path.to_str().unwrap_or("")])
            .current_dir(&frp_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000); // CREATE_NO_WINDOW
        match cmd.spawn() {
            Ok(_) => {
                self.set_toast(format!("已请求 {} 热重载配置", t.name));
                if let Some(rt) = self.tunnel_runtimes.get_mut(idx) {
                    rt.log_buf.push_str(&format!("[XMST] 已请求 frpc 热重载（reload -c {}）\n", cfg_path.display()));
                }
            }
            Err(e) => self.set_toast(format!("热重载失败: {e}")),
        }
    }

    fn save_config(&mut self) {
        let json = serde_json::to_string_pretty(&self.cfg).unwrap_or_default();
        if let Some(parent) = self.config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.config_path, json);
    }

    fn set_toast(&mut self, msg: String) {
        self.toast = format!("[{}] {}", Local::now().format("%H:%M:%S"), msg);
    }

    // ---------- 服务器操�?----------
    fn add_server_dir(&mut self, dir: PathBuf) {
        // 去重：同一目录已添加过则不再重复添加（Windows 路径不区分大小写�?
        let norm = |p: &std::path::Path| -> String {
            p.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_lowercase()
        };
        let target = norm(&dir);
        if let Some(pos) = self
            .cfg
            .servers
            .iter()
            .position(|s| norm(std::path::Path::new(&s.dir)) == target)
        {
            self.selected_server = Some(pos);
            self.save_config();
            self.set_toast("该目录已在列表中，已为你选中".to_string());
            return;
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "服务器".to_string());
        let mut sc = ServerConfig::default();
        sc.name = name;
        sc.dir = dir;
        // 新服务器不立即备份：last_backup 初始化为当前时间，间隔满后才首次备份
        sc.backup.last_backup = Some(Local::now().to_rfc3339());
        self.cfg.servers.push(sc);
        self.runtimes.push(ServerRuntime::default());
        self.selected_server = Some(self.cfg.servers.len() - 1);
        self.save_config();
        self.set_toast("已添加服务器".to_string());
    }

    fn remove_server(&mut self, idx: usize) {
        if idx >= self.cfg.servers.len() {
            return;
        }
        // 先停进程
        if let Some(rt) = self.runtimes.get(idx) {
            if let Some(p) = &rt.proc {
                let _ = process::kill(p);
            }
        }
        self.cfg.servers.remove(idx);
        self.runtimes.remove(idx);
        if self.selected_server == Some(idx) {
            self.selected_server = None;
        } else if let Some(s) = self.selected_server {
            if s > idx {
                self.selected_server = Some(s - 1);
            }
        }
        self.save_config();
        self.set_toast("已移除服务器（配置已删除，文件未动）".to_string());
    }

    fn start_server(&mut self, idx: usize) {
        if idx >= self.cfg.servers.len() || idx >= self.runtimes.len() {
            return;
        }
        let sc = self.cfg.servers[idx].clone();
        let rt = &mut self.runtimes[idx];
        if rt.proc.is_some() {
            return;
        }
        let dir = sc.dir.clone();
        if !dir.exists() {
            rt.last_msg = "目录不存在".to_string();
            return;
        }
        // 策略：优先运�?run.bat（经典方式），其次用 launch_cmd 模板
        let run_bat = dir.join("run.bat");
        let spawn_res = if run_bat.exists() {
            process::spawn_hidden("cmd", &["/c", "run.bat"], &dir, true)
        } else {
            // �?run.bat 时按解析链取 Java�?
            // 服务器指定列表项 -> 服务器自定义路径 -> run.bat 提取 -> 按版本自动匹�?-> 全局兜底 -> PATH
            let cfg = self.cfg.clone();
            let java = resolve_java_for_server(&cfg, &sc);
            let jvm = sc
                .jvm_args
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| self.cfg.default_jvm_args.clone());
            let cmd_tpl = sc.launch_cmd.clone();
            let core = detect_core_jar(&dir);
            // 公式化启动：{core} 占位符或 "-jar server.jar" 自动替换为实际核�?jar
            let cmd = cmd_tpl
                .replace("{java}", &java)
                .replace("{jvm}", &jvm)
                .replace("{core}", &core)
                .replace("-jar server.jar", &format!("-jar {core}"));
            let mut parts = cmd.split_whitespace();
            let prog = parts.next().unwrap_or("java").to_string();
            let args: Vec<&str> = parts.collect();
            process::spawn_hidden(&prog, &args, &dir, true)
        };
        match spawn_res {
            Ok(mp) => {
                rt.proc = Some(mp);
                rt.log_buf.clear();
                rt.startup_notified = false;
                rt.startup_warned = false;
                // ★ 复位插件一次性门控：以前只在 ServerRuntime::default() 里初始化 false，
                // 而 runtimes 每个服务器只建一次，导致 server_started 每个服务器每次程序运行
                // 最多只发一次（第二次开服插件再也不触发，毛玻璃"只生效过一次"）。
                rt.plugin_start_emitted = false;
                rt.started_at = Some(std::time::Instant::now());
                // 重建日志文件尾随器：优先�?logs/latest.log 增量读日志（绕开 stdout 缓冲卡死�?
                // 关键：seek_end 跳过文件已有内容，只读本次启动新增日志，避免旧日志回�?
                rt.log_tail = process::LogFileTail::new(&dir);
                if let Some(t) = &mut rt.log_tail {
                    t.seek_end();
                }
                rt.last_msg = "正在启动…（等待服务端就绪，日志出现 Done 后提示完成）".to_string();
                // 自动重启计时起点：每次成功启动都记录，interval 模式从此刻起�?
                if let Some(sc) = self.cfg.servers.get_mut(idx) {
                    if sc.auto_restart.enabled {
                        sc.auto_restart.last_restart = Some(Local::now().to_rfc3339());
                    }
                    sc.start_count = sc.start_count.saturating_add(1);
                    self.save_config();
                }
            }
            Err(e) => {
                rt.last_msg = format!("启动失败: {e}");
            }
        }
    }

    fn stop_server(&mut self, idx: usize) {
        // 手动停止会取消未执行的自动重�?
        self.auto_restart_after_stop.remove(&idx);
        self.auto_restart_pending.remove(&idx);
        self.auto_restart_scheduled.remove(&idx);
        // 手动停止取消待执行的崩溃重启（视为用户有意停机）
        if let Some(rt) = self.runtimes.get_mut(idx) {
            rt.crash_restart_at = None;
        }
        // 优雅停止可能等待服务器保存世界（最�?0秒），必须放后台线程，避�?UI 无响�?
        if self.stop_inflight.contains(&idx) {
            return;
        }
        if let Some(rt) = self.runtimes.get_mut(idx) {
            if let Some(p) = rt.proc.take() {
                rt.stopping = true;
                rt.last_msg = "正在停止…（等待服务器保存退出，最多 60 秒）".to_string();
                let tx = self.stop_tx.clone();
                self.stop_inflight.insert(idx);
                std::thread::spawn(move || {
                    let ok = process::stop_gracefully(&p, 30);
                    let _ = tx.send((idx, ok));
                });
            }
        }
    }

    fn kill_server(&mut self, idx: usize) {
        // 强杀同样取消未执行的自动重启
        self.auto_restart_after_stop.remove(&idx);
        self.auto_restart_pending.remove(&idx);
        self.auto_restart_scheduled.remove(&idx);
        if let Some(rt) = self.runtimes.get_mut(idx) {
            rt.crash_restart_at = None;
        }
        let srv_name = self
            .cfg
            .servers
            .get(idx)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let had_proc = self
            .runtimes
            .get(idx)
            .map(|rt| rt.proc.is_some())
            .unwrap_or(false);
        if let Some(rt) = self.runtimes.get_mut(idx) {
            if let Some(p) = &rt.proc {
                process::kill(p);
                rt.stopping = false;
                rt.last_msg = "已强制结束（含进程树）".to_string();
                rt.proc = None;
            }
        }
        if had_proc {
            if let Some(pm) = self.plugins.as_mut() {
                pm.emit("server_stopped", vec![Dynamic::from(srv_name), Dynamic::from("killed")]);
            }
        }
    }

    /// B3 强停二次确认：请求强杀（生成一次性 token，30s 过期）
    fn request_force_stop(&mut self, idx: usize) {
        let pid = self
            .runtimes
            .get(idx)
            .and_then(|rt| rt.proc.as_ref())
            .and_then(crate::process::pid)
            .unwrap_or(0);
        if pid == 0 {
            self.notify("XMST", "该服务器未运行，无法强制结束");
            return;
        }
        self.force_stop = Some(ForceStopReq {
            idx,
            pid,
            token: fastrand_instant_token(),
            created_at: std::time::Instant::now(),
        });
    }

    /// B3 强停二次确认：校验一次性 token（过期自动失效）
    fn force_stop_with_token(&mut self, token: u64) -> bool {
        let Some(req) = &self.force_stop else { return false };
        if req.token != token {
            return false;
        }
        if req.created_at.elapsed().as_secs() >= FORCE_STOP_TOKEN_TTL {
            // token 过期自动失效：清请求并拒绝执行
            self.force_stop = None;
            return false;
        }
        let idx = req.idx;
        self.force_stop = None;
        self.kill_server(idx);
        true
    }

    /// B3 强停确认弹窗（Window）：显示 pid + 影响，带一次性 token 执行
    fn ui_force_stop_confirm(&mut self, ctx: &egui::Context) {
        let Some(req) = &self.force_stop else { return };
        let expired = req.created_at.elapsed().as_secs() >= FORCE_STOP_TOKEN_TTL;
        let mut modal_open = true;
        let mut do_kill = false;
        let mut do_cancel = false;
        egui::Window::new("强制结束二次确认")
                .resizable(true)
            .id(egui::Id::new("force_stop_confirm_window"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut modal_open)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                let name = self.cfg.servers.get(req.idx).map(|s| s.name.clone()).unwrap_or_default();
                ui.heading("⚠ 确认强制结束服务器");
                ui.add_space(6.0);
                ui.label(format!("服务器：{name}"));
                ui.label(format!("主进程 PID：{}", req.pid));
                ui.label("影响：将强制结束该服务器及其全部子进程（进程树），");
                ui.label("未保存的世界进度可能丢失，建议先使用优雅停止。");
                if expired {
                    ui.add_space(8.0);
                    ui.colored_label(self.fg(egui::Color32::from_rgb(220, 90, 90)), "确认已过期（30 秒），请重新发起强制结束。");
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!expired, egui::Button::new("✅ 确认强制结束")).clicked() {
                        do_kill = true;
                    }
                    if ui.button("取消").clicked() {
                        do_cancel = true;
                    }
                });
            });
        if do_kill {
            let token = self.force_stop.as_ref().map(|r| r.token).unwrap_or(0);
            self.force_stop_with_token(token);
        } else if do_cancel || !modal_open {
            self.force_stop = None;
        }
    }

    /// 处理优雅停止后台线程回传结果
    fn tick_stop(&mut self) {
        while let Ok((idx, ok)) = self.stop_rx.try_recv() {
            self.stop_inflight.remove(&idx);
            if let Some(rt) = self.runtimes.get_mut(idx) {
                rt.stopping = false;
                rt.last_msg = if ok {
                    "已优雅停止".to_string()
                } else {
                    "停止等待超时".to_string()
                };
            }
            let name = self.cfg.servers.get(idx).map(|s| s.name.clone()).unwrap_or_default();
            // 插件事件：server_stopped（优雅停止回传）
            if let Some(pm) = self.plugins.as_mut() {
                pm.emit(
                    "server_stopped",
                    vec![
                        Dynamic::from(name.clone()),
                        Dynamic::from(if ok { "graceful" } else { "timeout" }),
                    ],
                );
            }
            self.notify(
                "XMST - 服务器已停止",
                &format!("{name} 已{}", if ok { "优雅停止" } else { "超时强制结束" }),
            );
        }
        // 确认退出模式：所有服务器均已停止且无停止中任务，放行退出（由关闭检查发�?Close�?
        if self.closing_exit {
            let any_active = self
                .runtimes
                .iter()
                .enumerate()
                .any(|(i, rt)| rt.proc.is_some() || self.stop_inflight.contains(&i));
            if !any_active {
                self.closing_exit = false;
                self.ctx_close_pending = true;
            }
        }
    }

    /// 注册托盘全局事件回调（只在首次 update 调用一次，需持有有效 ctx）：
    /// 回调在托盘消息线程内同步执行，不依赖 egui 事件循环，窗口隐藏后依然可靠。
    /// 左键单击托盘图标 / 菜单「显示主窗口」→ 恢复窗口；菜单「退出」→ 恢复窗口并置退出标记。
    /// 托盘显示/退出改为直接操作 Win32 HWND（ShowWindow / PostMessageW）：
    /// 窗口隐藏后 eframe/winit 事件循环休眠，ViewportCommand 不会及时被消费，
    /// 这是此前多次「托盘后打不开/关不掉」的根因。Win32 调用不依赖事件循环，稳定生效。
    fn setup_tray_handlers(&mut self, _ctx: &egui::Context) {
        // ★ 双击 exe 唤醒已有实例：命名事件 + 阻塞等待线程（零 CPU）。
        // 第二个实例在 single_instance_check 里 SetEvent，这里被唤醒后恢复窗口。
        {
            let hidden = self.tray_hidden.clone();
            let restoring = self.tray_restoring_flag.clone();
            let ctx = self.egui_ctx.clone();
            unsafe {
                use winapi::um::synchapi::{CreateEventW, WaitForSingleObject};
                use winapi::um::winbase::INFINITE;
                let ev: Vec<u16> = "Local\\XMST_ShowMainWindow"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                // 自动重置事件；bInitialState = FALSE
                let h = CreateEventW(std::ptr::null_mut(), 0, 0, ev.as_ptr());
                if !h.is_null() {
                    let handle = h as isize;
                    std::thread::spawn(move || loop {
                        let r = WaitForSingleObject(handle as _, INFINITE);
                        if r != 0 {
                            break;
                        }
                        hidden.store(false, std::sync::atomic::Ordering::Relaxed);
                        restoring.store(true, std::sync::atomic::Ordering::Relaxed);
                        show_main_window();
                        ctx.request_repaint();
                    });
                }
            }
        }
        // 托盘图标左键单击：恢复窗口（右键由系统弹出菜单，无需处理）
        {
            let hidden = self.tray_hidden.clone();
            let restoring = self.tray_restoring_flag.clone();
            let version = self.tray_version.clone();
            let ctx = self.egui_ctx.clone();
            let _ = tray_icon::TrayIconEvent::set_event_handler(Some(move |ev| {
                if let tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                } = ev
                {
                    hidden.store(false, std::sync::atomic::Ordering::Relaxed);
                    version.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    restoring.store(true, std::sync::atomic::Ordering::Relaxed);
                    show_main_window();
                    // 唤醒休眠的事件循环：请求立即重绘（否则 WaitUntil 长眠不响应）
                    ctx.request_repaint();
                }
            }));
        }
        // 菜单事件：显示主窗口 / 退出
        {
            let hidden = self.tray_hidden.clone();
            let restoring = self.tray_restoring_flag.clone();
            let version = self.tray_version.clone();
            let ctx = self.egui_ctx.clone();
            let quit_requested = self.tray_quit_requested.clone();
            let show_id = self.tray_show_id.clone();
            let quit_id = self.tray_quit_id.clone();
            use tray_icon::menu::MenuEvent;
            let _ = MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                if Some(&ev.id) == show_id.as_ref() {
                    hidden.store(false, std::sync::atomic::Ordering::Relaxed);
                    version.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    restoring.store(true, std::sync::atomic::Ordering::Relaxed);
                    show_main_window();
                    ctx.request_repaint();
                } else if Some(&ev.id) == quit_id.as_ref() {
                    hidden.store(false, std::sync::atomic::Ordering::Relaxed);
                    version.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    show_main_window();
                    ctx.request_repaint();
                    // Bug3：不再投递 WM_CLOSE（会被 close_behavior=tray 分支再次拦截成隐藏，
                    // 导致「托盘退出变黑屏、进程挂死」）；改为置位退出标记，
                    // 由 update 消费后走 request_exit：无服务器直接退出，有服务器弹确认。
                    quit_requested.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }));
        }
    }

    /// 网络流量采样（纯 Rust 读 GetIfTable2 累计字节，2s 节流求速率；无子进程）
    fn tick_net(&mut self) {
        let now = std::time::Instant::now();
        if now.duration_since(self.net_next_t).as_millis() < 2000 {
            return;
        }
        self.net_next_t = now;
        let (rx, tx) = net_iface_bytes();
        if let Some((prx, ptx, t)) = self.net_last_bytes.replace((rx, tx, now)) {
            let dt = t.elapsed().as_secs_f64();
            if dt > 0.0 {
                let r = rx.saturating_sub(prx) as f64 / dt;
                let s = tx.saturating_sub(ptx) as f64 / dt;
                self.net_hist.push((r, s));
                if self.net_hist.len() > 120 {
                    self.net_hist.remove(0);
                }
            }
        }
    }

    /// 发起退出流程：无运行中服务器直接关闭；有服务器则弹确认�?
    fn request_exit(&mut self, ctx: &egui::Context) {
        let running: Vec<usize> = self
            .runtimes
            .iter()
            .enumerate()
            .filter(|(_, rt)| rt.proc.is_some() && process::is_running(rt.proc.as_ref().unwrap()))
            .map(|(i, _)| i)
            .collect();
        if running.is_empty() {
            self.ctx_close_pending = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.confirm_close.is_none() {
            self.confirm_close = Some(running.len());
        }
    }

    /// 处理关闭请求：返�?true 表示放行关闭（退出程序），false 表示已拦�?
    fn handle_close_request(&mut self, ctx: &egui::Context) -> bool {
        // 已确认退出且全部服务器已停止：放�?
        if self.ctx_close_pending {
            return true;
        }
        // 已确认退出但仍有服务器在停止中：继续等待
        if self.closing_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return false;
        }
        // 询问框已弹出：等待用户选择
        if self.confirm_close.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return false;
        }
        // 关闭行为 = 最小化到托盘：立即隐藏窗口（任务栏消失），托盘图标可恢复；服务器保持运行。
        // Bug3：此前依赖「下一帧早退分支才发 Visible(false)」，关闭帧后若无重绘来源则
        // 窗口永不隐藏（残留黑底/任务栏不退）；改为关闭帧立即隐藏 + 标记兜底幂等。
        if self.cfg.close_behavior == "tray" {
            self.tray_hidden.store(true, std::sync::atomic::Ordering::Relaxed);
            self.tray_version
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // 注意：这一帧不释放纹理 —— 仍要正常合成一次；释放统一放到窗口确实隐藏之后
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hide_requested = true;
            self.hide_sent = false;
            // A4 工作集压缩：进入托盘态 7s 后一次性修剪（勿周期调用——周期修剪会引发抖动）。
            // 托盘态 update 早退（0 重绘），egui 缓存/字形不再需要；压缩线程与主线程解耦，
            // 恢复窗口时 tray_hidden 已复位则跳过，避免把正在恢复的进程工作集打回冷态。
            let hidden = self.tray_hidden.clone();
            let cctx = self.egui_ctx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                if !hidden.load(std::sync::atomic::Ordering::Relaxed) {
                    return; // 7s 内已恢复窗口：跳过压缩，保持流畅
                }
                // egui 缓存与字体图集已无用：清空后再压工作集，回收效果更好
                cctx.memory_mut(|mem| {
                    mem.caches = Default::default();
                });
                // 托盘态释放 CJK/emoji 字体数据（约 20-30MB 常驻 RAM）：
                // 恢复窗口时 update 恢复分支会把 cjk_loading 置回 false 触发重新懒加载
                cctx.set_fonts(egui::FontDefinitions::default());
                unsafe {
                    use winapi::um::processthreadsapi::GetCurrentProcess;
                    use winapi::um::psapi::EmptyWorkingSet;
                    // EmptyWorkingSet 即 SetProcessWorkingSetSize(h, -1, -1)：强制系统把物理页换出
                    EmptyWorkingSet(GetCurrentProcess());
                }
            });
            return false;
        }
        // 收集运行中的服务�?
        let running: Vec<usize> = self
            .runtimes
            .iter()
            .enumerate()
            .filter(|(_, rt)| rt.proc.is_some() && process::is_running(rt.proc.as_ref().unwrap()))
            .map(|(i, _)| i)
            .collect();
        if running.is_empty() {
            // 无服务器运行：按配置最小化或退�?
            if self.cfg.close_behavior == "minimize" {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                return false;
            }
            return true;
        }
        // 有服务器运行：按配置处理
        if self.cfg.close_behavior == "minimize" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            return false;
        }
        // 配置为彻底关闭：先询问，防止误触
        self.confirm_close = Some(running.len());
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        false
    }

    fn tick_backup(&mut self) {
        // 阶段5：远端转存失败重试（受 BETA_REMOTE_BACKUP 开关控制，关闭时零轮询）
        if features::is_enabled(&self.cfg.features, features::BETA_REMOTE_BACKUP) {
            let pending: Vec<(usize, PathBuf)> = self
                .remote_pending
                .iter()
                .map(|(i, (p, _))| (*i, p.clone()))
                .collect();
            for (i, path) in pending {
                if !self.remote_upload_busy.contains(&i) {
                    self.try_remote_upload(i, path);
                }
            }
        }

        // 处理后台线程回传的结�?
        while let Ok((i, res)) = self.backup_rx.try_recv() {
            self.backup_inflight.remove(&i);
            match res {
                Ok(r) => {
                    if let Some(sc) = self.cfg.servers.get_mut(i) {
                        sc.backup.last_backup = Some(Local::now().to_rfc3339());
                    }
                    if r.changed {
                        let kind = r.kind.label();
                        let name = r
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        // 与备份列表口径一致：显示压缩后 zip 文件大小；若原始字节差异明显则附带说明
                        let zip_size = std::fs::metadata(&r.path)
                            .map(|m| m.len())
                            .unwrap_or(r.bytes);
                        let mut msg = format!("{kind}备份完成: {name} ({})", fmt_size(zip_size));
                        if zip_size != r.bytes && r.bytes > 0 {
                            msg.push_str(&format!("，原始数据 {}", fmt_size(r.bytes)));
                        }
                        if r.skipped > 0 {
                            msg.push_str(&format!(
                                "，{} 个文件被占用已跳过（服务器运行时常见，关服后下次备份自动补上）",
                                r.skipped
                            ));
                        }
                        self.set_toast(msg);
                        // 插件事件：backup_done（成功且有关键数据才触发）
                        if let Some(pm) = self.plugins.as_mut() {
                            let srv_name = self
                                .cfg
                                .servers
                                .get(i)
                                .map(|s| s.name.clone())
                                .unwrap_or_default();
                            pm.emit(
                                "backup_done",
                                vec![
                                    Dynamic::from(srv_name),
                                    Dynamic::from(kind),
                                    Dynamic::from(name),
                                ],
                            );
                        }
                        // 阶段5：远端备份转存（配置了远端目标且开关开启时）
                        if features::is_enabled(&self.cfg.features, features::BETA_REMOTE_BACKUP) {
                            self.try_remote_upload(i, r.path.clone());
                        }
                    } else {
                        self.set_toast("备份检查完成：无变化，跳过本次备份".to_string());
                    }
                    self.save_config();
                }
                Err(e) => {
                    self.set_toast(format!("备份失败: {e}"));
                }
            }
        }

        // 测试功能「自动备份」禁用时：不发起新的自动备份（进行中的备份仍正常收尾回传）
        if !features::is_enabled(&self.cfg.features, features::BETA_BACKUP) {
            return;
        }

        // 检查是否到期，到期则后台执�?
        // Throttle the due-scan to once per second so background idle cost stays tiny.
        if self.last_backup_check.elapsed().as_secs() < 1 {
            return;
        }
        self.last_backup_check = std::time::Instant::now();
        let now = Local::now();
        let mut to_backup: Vec<usize> = Vec::new();
        let mut risk_toast: Option<String> = None;
        for (i, sc) in self.cfg.servers.iter().enumerate() {
            if !sc.backup.enabled || self.backup_inflight.contains(&i) {
                continue;
            }
            if sc.dir.as_os_str().is_empty() || !sc.dir.exists() {
                continue;
            }
            // 仅在服务器运行时备份：关服后数据静止，备份无意义且徒增磁�?IO
            let running = self
                .runtimes
                .get(i)
                .map(|rt| rt.proc.is_some())
                .unwrap_or(false);
            if !running {
                continue;
            }
            // Emergency mode: when system memory load crosses the threshold,
            // shorten the interval to mem_interval_min to survive a possible crash.
            let mem_risk = perf::system_mem_load_pct() >= sc.backup.mem_threshold_percent;
            if mem_risk && !self.mem_risk_active {
                self.mem_risk_active = true;
                risk_toast = Some(format!(
                    "内存占用达到 {}%，已临时切换为每 {} 分钟紧急备份（可在备份页调整阈值）",
                    sc.backup.mem_threshold_percent, sc.backup.mem_interval_min
                ));
            } else if !mem_risk && self.mem_risk_active {
                self.mem_risk_active = false;
            }
            let interval_min = if mem_risk {
                sc.backup.mem_interval_min
            } else {
                sc.backup.interval_min
            };
            let due = match &sc.backup.last_backup {
                Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                    .map(|t| t.with_timezone(&chrono::Local))
                    .ok()
                    .map(|t| t + chrono::Duration::minutes(interval_min as i64))
                    .map(|t| now >= t)
                    .unwrap_or(true),
                None => true,
            };
            if due {
                to_backup.push(i);
            }
        }
        if let Some(msg) = risk_toast {
            self.set_toast(msg);
        }
        for i in to_backup {
            self.spawn_backup(i);
        }
    }

    /// 自动重启调度：interval（距上次启动�?N 分钟）或 daily（每天固定时刻）触发�?
    /// 先优雅停止，等保存退出后自动再启动；手动停止/强杀会取消待执行的重启�?
    fn tick_auto_restart(&mut self) {
        if self.last_auto_restart_check.elapsed().as_secs() < 1 {
            return;
        }
        self.last_auto_restart_check = std::time::Instant::now();
        let now = Local::now();
        let today = now.format("%Y-%m-%d").to_string();

        // 1) 判断哪些服务器到点，进入倒计�?
        let mut due_toasts: Vec<String> = Vec::new();
        for (i, sc) in self.cfg.servers.iter().enumerate() {
            let ac = &sc.auto_restart;
            if !ac.enabled {
                continue;
            }
            if self.auto_restart_pending.contains(&i)
                || self.auto_restart_scheduled.contains_key(&i)
                || self.auto_restart_after_stop.contains(&i)
            {
                continue;
            }
            let last = ac
                .last_restart
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&chrono::Local));
            let due = if ac.mode == "daily" {
                let t = chrono::NaiveTime::parse_from_str(&ac.daily_time, "%H:%M").unwrap_or(
                    chrono::NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
                );
                let day_done = last
                    .map(|t| t.format("%Y-%m-%d").to_string() == today)
                    .unwrap_or(false);
                !day_done && now.time() >= t
            } else {
                let interval = std::time::Duration::from_secs(ac.interval_min.max(1) as u64 * 60);
                let elapsed = last
                    .map(|t| now.signed_duration_since(t).to_std().unwrap_or_default())
                    .unwrap_or(interval);
                elapsed >= interval
            };
            if !due {
                continue;
            }
            // 只有正在运行的服务器才需要重�?
            let running = self
                .runtimes
                .get(i)
                .map(|rt| rt.proc.is_some())
                .unwrap_or(false);
            if !running {
                continue;
            }
            self.auto_restart_pending.insert(i);
            self.auto_restart_scheduled
                .insert(i, std::time::Instant::now() + std::time::Duration::from_secs(ac.warn_secs.max(1) as u64));
            due_toasts.push(format!(
                "「{}」将在 {} 秒后自动重启",
                sc.name, ac.warn_secs
            ));
        }
        for t in due_toasts {
            self.set_toast(t);
        }

        // 2) 倒计时到�?-> 优雅停止并登记重�?
        let mut due_now: Vec<usize> = Vec::new();
        for (i, t) in &self.auto_restart_scheduled {
            if std::time::Instant::now() >= *t {
                due_now.push(*i);
            }
        }
        for i in due_now {
            self.auto_restart_scheduled.remove(&i);
            self.auto_restart_pending.remove(&i);
            if self.runtimes.get(i).map(|rt| rt.proc.is_some()).unwrap_or(false) {
                self.auto_restart_after_stop.insert(i);
                self.stop_server(i);
            }
        }

        // 3) 优雅停止完成 -> 重新启动
        let mut to_start: Vec<usize> = Vec::new();
        for i in self.auto_restart_after_stop.clone() {
            let idle = self
                .runtimes
                .get(i)
                .map(|rt| rt.proc.is_none() && !rt.stopping)
                .unwrap_or(false);
            if idle && !self.stop_inflight.contains(&i) {
                to_start.push(i);
            }
        }
        for i in to_start {
            self.auto_restart_after_stop.remove(&i);
            if let Some(sc) = self.cfg.servers.get_mut(i) {
                sc.auto_restart.last_restart = Some(Local::now().to_rfc3339());
            }
            self.save_config();
            self.start_server(i);
            self.set_toast(format!("「{}」自动重启完成", self.cfg.servers[i].name));
        }
    }

    /// 崩溃重启调度：到点且空闲（进程已退出、非手动停止中）时重新拉起；
    /// 手动停止/强杀已通过置空 crash_restart_at 取消待执行重启�?
    fn tick_crash_restart(&mut self) {
        let mut due: Vec<usize> = Vec::new();
        for i in 0..self.runtimes.len() {
            if let Some(t) = self.runtimes[i].crash_restart_at {
                if std::time::Instant::now() >= t {
                    due.push(i);
                }
            }
        }
        for i in due {
            self.runtimes[i].crash_restart_at = None;
            let idle = self
                .runtimes
                .get(i)
                .map(|rt| rt.proc.is_none() && !rt.stopping)
                .unwrap_or(false);
            if idle && !self.stop_inflight.contains(&i) {
                self.runtimes[i].last_msg = format!(
                    "自动重启中（第 {} 次）...",
                    self.runtimes[i].crash_count
                );
                self.start_server(i);
            }
        }
    }

    /// 在后台线程执行备份（低优先级 + 限速），完成后通过 channel 回传
    fn spawn_backup(&mut self, idx: usize) {
        if self.backup_inflight.contains(&idx) {
            return;
        }
        let Some(sc) = self.cfg.servers.get(idx) else { return };
        if sc.dir.as_os_str().is_empty() || !sc.dir.exists() {
            return;
        }
        self.backup_inflight.insert(idx);
        let dir = sc.dir.clone();
        let folders = sc.backup.folders.clone();
        let throttle = sc.backup.throttle_mbps;
        let tx = self.backup_tx.clone();
        std::thread::spawn(move || {
            backup::set_thread_low_priority();
            let r = backup::create_backup(&dir, &folders, throttle);
            let _ = tx.send((idx, r));
        });
    }

    fn do_restore(&mut self) {
        if let Some((idx, zip, _name)) = self.confirm_restore.take() {
            let folders = self
                .cfg
                .servers
                .get(idx)
                .map(|s| s.backup.folders.clone())
                .unwrap_or_default();
            let dir = self
                .cfg
                .servers
                .get(idx)
                .map(|s| s.dir.clone())
                .unwrap_or_default();
            // 先停止服务器
            if let Some(rt) = self.runtimes.get(idx) {
                if let Some(p) = &rt.proc {
                    let _ = process::stop_gracefully(p, 30);
                }
            }
            if let Some(rt) = self.runtimes.get_mut(idx) {
                rt.proc = None;
            }
            match backup::restore_backup(&dir, &zip, &folders) {
                Ok(n) => {
                    self.set_toast(format!("回退完成，恢复 {n} 个文件（旧内容在 .mcsrv_trash/）"));
                }
                Err(e) => {
                    self.set_toast(format!("回退失败: {e}"));
                }
            }
        }
    }

    fn do_rename(&mut self) {
        if let Some((idx, name)) = self.rename_server.take() {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            if let Some(sc) = self.cfg.servers.get_mut(idx) {
                sc.name = name.clone();
            }
            self.save_config();
            self.set_toast(format!("已重命名为「{name}」"));
        }
    }

    fn do_delete_backup(&mut self) {
        if let Some((idx, zip, name)) = self.confirm_delete_backup.take() {
            let dir = self
                .cfg
                .servers
                .get(idx)
                .map(|s| s.dir.clone())
                .unwrap_or_default();
            match backup::delete_backup(&dir, &zip) {
                Ok(()) => {
                    self.set_toast(format!("已删除备份 {name}"));
                }
                Err(e) => {
                    self.set_toast(format!("删除备份失败: {e}"));
                }
            }
        }
    }

    // ---------- 隧道操作 ----------
    fn add_tunnel(&mut self) {
        // 校验：备注名与配置不允许空白
        let name = self.add_tunnel_name.trim().to_string();
        if name.is_empty() {
            self.set_toast("请先填写备注名再添加隧道".to_string());
            return;
        }
        let kind = self.add_tunnel_kind.clone();
        if kind == "frp" {
            if self.add_tunnel_mode == "form" {
                if self.add_tunnel_server_addr.trim().is_empty() {
                    self.set_toast("请填写 frps 服务器地址（serverAddr）".to_string());
                    return;
                }
            } else if self.add_tunnel_cfg.trim().is_empty() {
                self.set_toast("请填写或导入 frpc.toml 配置".to_string());
                return;
            }
        } else if kind == "rathole" {
            if self.add_rh_server_addr.trim().is_empty() {
                self.set_toast("请填写 rathole 服务器地址（server_addr）".to_string());
                return;
            }
            if self.add_rh_local_port == 0 || self.add_rh_remote_port == 0 {
                self.set_toast("请填写有效的本地/远程端口".to_string());
                return;
            }
        }
        let mut t = TunnelConfig::default();
        t.name = name.clone();
        t.remark = t.name.clone();
        t.kind = kind.clone();
        if kind == "frp" {
            t.exe = self.add_tunnel_exe.trim().to_string();
            t.cfg = self.add_tunnel_cfg.trim().to_string();
            if self.add_tunnel_mode == "form" {
                // 简易模式：根据填写的数值生成 frpc.toml
                let addr = self.add_tunnel_server_addr.trim().to_string();
                let user = self.add_tunnel_user.trim().to_string();
                let token = self.add_tunnel_token.trim().to_string();
                let mut toml = format!(
                    "serverAddr = \"{addr}\"\nserverPort = {}\n",
                    self.add_tunnel_server_port
                );
                if !user.is_empty() {
                    toml.push_str(&format!("user = \"{user}\"\n"));
                }
                if !token.is_empty() {
                    toml.push_str("auth.method = \"token\"\n");
                    toml.push_str(&format!("auth.token = \"{token}\"\n"));
                }
                toml.push_str(&format!(
                    "\n[[proxies]]\nname = \"xmst_t{}\"\ntype = \"{}\"\nlocalIP = \"127.0.0.1\"\nlocalPort = {}\nremotePort = {}\n",
                    self.cfg.tunnels.len() + 1,
                    self.add_tunnel_proxy_type,
                    self.add_tunnel_local_port,
                    self.add_tunnel_remote_port
                ));
                t.cfg = toml;
            }
            t.frp_local_port = self.add_tunnel_local_port;
            t.frp_remote_port = self.add_tunnel_remote_port;
            t.frp_proxy_type = self.add_tunnel_proxy_type.clone();
            // 分配唯一 admin API 端口（7400 起，避开已占用）
            {
                let used: HashSet<u16> = self.cfg.tunnels.iter().map(|x| x.admin_port).collect();
                t.admin_port = (7400u16..7600).find(|p| !used.contains(p)).unwrap_or(7400);
            }
        } else if kind == "rathole" {
            t.rh_server_addr = self.add_rh_server_addr.trim().to_string();
            t.rh_server_port = self.add_rh_server_port;
            t.rh_tunnel_name = if self.add_rh_tunnel_name.trim().is_empty() {
                "xmst".to_string()
            } else {
                self.add_rh_tunnel_name.trim().to_string()
            };
            t.rh_local_addr = if self.add_rh_local_addr.trim().is_empty() {
                "127.0.0.1".to_string()
            } else {
                self.add_rh_local_addr.trim().to_string()
            };
            t.rh_local_port = self.add_rh_local_port;
            t.rh_remote_port = self.add_rh_remote_port;
            t.rh_noise = self.add_rh_noise;
            t.rh_noise_key = self.add_rh_noise_key.trim().to_string();
        }
        self.cfg.tunnels.push(t);
        self.tunnel_runtimes.push(TunnelRuntime::default());
        self.save_config();
        self.notify("XMST - 已添加隧道", &format!("{name} 已创建，可在「隧道管理」中启动"));
        self.add_tunnel_name.clear();
        self.add_tunnel_exe.clear();
        self.add_tunnel_cfg.clear();
    }

    fn remove_tunnel(&mut self, idx: usize) {
        if idx >= self.cfg.tunnels.len() {
            return;
        }
        if let Some(rt) = self.tunnel_runtimes.get(idx) {
            if let Some(p) = &rt.proc {
                let _ = process::kill(p);
            }
        }
        self.cfg.tunnels.remove(idx);
        self.tunnel_runtimes.remove(idx);
        self.save_config();
        self.set_toast("已移除隧道".to_string());
    }

    fn start_tunnel(&mut self, idx: usize) {
        if idx >= self.cfg.tunnels.len() || idx >= self.tunnel_runtimes.len() {
            return;
        }
        let t = self.cfg.tunnels[idx].clone();
        let frp_dir = self.data_dir().join("frp");
        // 阶段5：rathole 分支所需 self 调用预计算（避免与下方 rt 借用冲突）
        let rh_dir = self.data_dir().join("rathole");
        let rh_toml = if t.kind == "rathole" {
            Some(self.build_rathole_client_toml(&t, idx))
        } else {
            None
        };
        let rt = &mut self.tunnel_runtimes[idx];
        if rt.proc.is_some() {
            return;
        }
        if t.kind == "frp" {
            // Frp：每隧道独立 frpc.toml（完�?TOML 文本由用户填写），独立进程启�?
            if t.cfg.trim().is_empty() {
                rt.log_buf.push_str("错误: 未填写完整的 frpc.toml 内容，请在该隧道的「配置」里粘贴完整 TOML。\n");
                self.notify("XMST - 隧道启动失败", &format!("{} 未填写完整的 frpc.toml 配置", t.name));
                return;
            }
            // frpc.exe：优先使用隧道填写的 exe；留空则用共�?data/frp/frpc.exe
            let exe = if t.exe.trim().is_empty() {
                if !frp_dir.join("frpc.exe").exists() {
                    rt.log_buf.push_str("错误: 未找到 frpc.exe，请先在 Frp 页下载或导入。\n");
                    self.notify("XMST - 隧道启动失败", &format!("{} 未找到 frpc.exe，请先在仪表盘下载或导入", t.name));
                    return;
                }
                frp_dir.join("frpc.exe")
            } else {
                PathBuf::from(t.exe.trim())
            };
            let cwd = frp_dir.clone();
            // 每个隧道一个独立配置文件，文件名带序号避免重名覆盖
            let cfg_name = format!("tunnel_{idx}.toml");
            let cfg_path = frp_dir.join(&cfg_name);
            let _ = std::fs::create_dir_all(&frp_dir);
            // 流量统计：frpc.toml 未配置 admin API 时自动注入 webServer（frpc 0.52+）
            let mut final_cfg = t.cfg.clone();
            if parse_frp_admin_port(&final_cfg).is_none()
                && !final_cfg.contains("webServer.addr")
                && !final_cfg.contains("adminAddr")
            {
                final_cfg = format!(
                    "webServer.addr = \"127.0.0.1\"\nwebServer.port = {}\n\n{final_cfg}",
                    t.admin_port
                );
            }
            let write_cfg = if parse_frp_admin_port(&final_cfg).is_some() {
                final_cfg
            } else {
                t.cfg.clone()
            };
            if let Err(e) = std::fs::write(&cfg_path, write_cfg.as_bytes()) {
                rt.log_buf.push_str(&format!("写入 frpc.toml 失败: {e}\n"));
                self.notify("XMST - 隧道启动失败", &format!("{} 写入 frpc.toml 失败: {e}", t.name));
                return;
            }
            let r = process::spawn_hidden(
                exe.to_str().unwrap_or("frpc"),
                &["-c", cfg_path.to_str().unwrap_or(&cfg_name)],
                &cwd,
                false,
            );
            match r {
                Ok(mp) => {
                    rt.proc = Some(mp);
                    rt.log_buf.clear();
                    rt.stopping = false;
                    let name = self.cfg.tunnels[idx].name.clone();
                    self.notify("XMST - Frp 穿透已启动", &format!("{name} 已启动（独立 frpc 进程）"));
                }
                Err(e) => {
                    rt.log_buf.push_str(&format!("启动失败: {e}\n"));
                    self.notify("XMST - 隧道启动失败", &format!("{} 启动失败: {e}", t.name));
                }
            }
        } else if t.kind == "rathole" {
            // Rathole：确认官方客户端 exe（data/rathole/rathole.exe），缺失时自动下载
            let exe = rh_dir.join("rathole.exe");
            if !exe.exists() {
                rt.log_buf
                    .push_str("错误: 未找到 rathole.exe，正在自动下载官方客户端…\n");
                self.notify("XMST - 隧道启动失败", &format!("{} 未找到 rathole.exe，正在自动下载…", t.name));
                self.download_rathole();
                return;
            }
            // 生成 rathole-client.toml（每隧道独立文件，文件名带序号避免重名）
            let toml = rh_toml.unwrap_or_default();
            let _ = std::fs::create_dir_all(&rh_dir);
            let cfg_path = rh_dir.join(format!("rathole_{idx}.toml"));
            if let Err(e) = std::fs::write(&cfg_path, toml.as_bytes()) {
                rt.log_buf.push_str(&format!("写入 rathole-client.toml 失败: {e}\n"));
                self.notify("XMST - 隧道启动失败", &format!("{} 写入 rathole-client.toml 失败: {e}", t.name));
                return;
            }
            let cwd = rh_dir.clone();
            let r = process::spawn_hidden(
                exe.to_str().unwrap_or("rathole"),
                &["--client", cfg_path.to_str().unwrap_or("rathole-client.toml")],
                &cwd,
                false,
            );
            match r {
                Ok(mp) => {
                    rt.proc = Some(mp);
                    rt.log_buf.clear();
                    rt.stopping = false;
                    let name = self.cfg.tunnels[idx].name.clone();
                    self.notify("XMST - Rathole 穿透已启动", &format!("{name} 已启动（rathole 客户端进程）"));
                }
                Err(e) => {
                    rt.log_buf.push_str(&format!("启动失败: {e}\n"));
                    self.notify("XMST - 隧道启动失败", &format!("{} 启动失败: {e}", t.name));
                }
            }
        } else {
            rt.log_buf
                .push_str("错误: 该隧道类型已不再支持（NPS 功能已移除），请删除后重建为 Frp 或 Rathole 隧道。\n");
        }
    }

    fn stop_tunnel(&mut self, idx: usize) {
        if let Some(rt) = self.tunnel_runtimes.get_mut(idx) {
            if let Some(p) = &rt.proc {
                rt.stopping = true;
                process::kill(p);
                rt.proc = None;
                let name = self
                    .cfg
                    .tunnels
                    .get(idx)
                    .map(|t| {
                        if t.remark.trim().is_empty() {
                            t.name.clone()
                        } else {
                            t.remark.clone()
                        }
                    })
                    .unwrap_or_default();
                self.notify("XMST - 穿透已停止", &format!("{name} 已停止"));
            }
        }
    }

    // ---------- Rathole 穿透（阶段5）：客户端下载 / 配置生成 / 回传消费 ----------
    /// Generate rathole client TOML: [client] remote server + one [[client.services]]
    /// section per tunnel (local forward + remote listen port). NOISE is optional:
    /// when enabled, emit [client.transport.noise] with an auto-generated hex key.
    fn build_rathole_client_toml(&self, t: &TunnelConfig, idx: usize) -> String {
        let server_addr = if t.rh_server_addr.trim().is_empty() {
            "127.0.0.1".to_string()
        } else {
            t.rh_server_addr.trim().to_string()
        };
        let tunnel_name = if t.rh_tunnel_name.trim().is_empty() {
            format!("xmst_t{idx}")
        } else {
            t.rh_tunnel_name.trim().to_string()
        };
        let local_addr = if t.rh_local_addr.trim().is_empty() {
            "127.0.0.1".to_string()
        } else {
            t.rh_local_addr.trim().to_string()
        };
        let mut out = String::new();
        out.push_str(&format!(
            "[client]\nremote_addr = \"{server_addr}:{}\"\n\n",
            t.rh_server_port
        ));
        out.push_str(&format!("[[client.services]]\nname = \"{tunnel_name}\"\n"));
        out.push_str(&format!("local_addr = \"{local_addr}:{}\"\n", t.rh_local_port));
        out.push_str(&format!("remote_port = {}\n", t.rh_remote_port));
        if t.rh_noise {
            let key = if t.rh_noise_key.trim().is_empty() {
                gen_rh_key_hex()
            } else {
                t.rh_noise_key.trim().to_string()
            };
            out.push_str("\n[client.transport.noise]\n");
            out.push_str("pattern = \"Noise_NK_25519_ChaChaPoly_BLAKE2s\"\n");
            out.push_str(&format!("local_private_key = \"{key}\"\n"));
            // Server side must use the same pattern and pair the client public key.
        }
        out
    }

    /// Download the official rathole client (Windows x86_64) into data/rathole/rathole.exe.
    /// PowerShell Invoke-WebRequest with ordered mirror URLs; unzip the release zip in place.
    /// Runs in a background thread; result is delivered via rathole_rx.
    fn download_rathole(&mut self) {
        if self.rathole_busy {
            return;
        }
        self.rathole_busy = true;
        self.rathole_dl_state = "正在下载 rathole…".to_string();
        let rh_dir = self.data_dir().join("rathole");
        let _ = std::fs::create_dir_all(&rh_dir);
        let urls: Vec<String> = [
            "https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-pc-windows-msvc.zip",
            "https://mirror.ghproxy.com/https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-pc-windows-msvc.zip",
            "https://ghfast.top/https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-pc-windows-msvc.zip",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let tx = self.rathole_tx.clone();
        std::thread::spawn(move || {
            let res = (|| -> Result<(), String> {
                let mut last_err = String::new();
                for url in &urls {
                    let zip_path = rh_dir.join("rathole_dl.zip");
                    let ps = format!(
                        "$ErrorActionPreference='Stop'; \
                         [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; \
                         Invoke-WebRequest -Uri '{url}' -OutFile '{zip}' -UseBasicParsing; \
                         Expand-Archive -Path '{zip}' -DestinationPath '{dir}' -Force",
                        url = url,
                        zip = zip_path.to_string_lossy(),
                        dir = rh_dir.to_string_lossy()
                    );
                    match std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", &ps])
                        .creation_flags(0x08000000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                        .stdin(std::process::Stdio::null())
                        .output()
                    {
                        Ok(o) if o.status.success() => {
                            if let Some(exe) = find_exe_recursive(&rh_dir) {
                                let final_exe = rh_dir.join("rathole.exe");
                                if exe != final_exe {
                                    let _ = std::fs::copy(&exe, &final_exe);
                                }
                                let _ = std::fs::remove_file(&zip_path);
                                return Ok(());
                            }
                            last_err = format!("下载完成但未找到 rathole.exe（{url}）");
                        }
                        Ok(o) => {
                            last_err = format!(
                                "下载失败（{url}）: {}",
                                String::from_utf8_lossy(&o.stderr).trim()
                            );
                        }
                        Err(e) => {
                            last_err = format!("下载失败（{url}）: {e}");
                        }
                    }
                }
                Err(last_err)
            })();
            let _ = tx.send(res);
        });
    }

    /// Consume rathole download result; called from update (no-op when not busy).
    fn poll_rathole_dl(&mut self) {
        if !self.rathole_busy {
            return;
        }
        while let Ok(res) = self.rathole_rx.try_recv() {
            match res {
                Ok(()) => {
                    self.rathole_busy = false;
                    self.rathole_dl_state = "rathole 已就绪".to_string();
                    self.notify("XMST - rathole 下载完成", "官方客户端已就绪，可重新启动隧道");
                }
                Err(e) => {
                    self.rathole_busy = false;
                    self.rathole_dl_state = format!("下载失败: {e}");
                    self.notify("XMST - rathole 下载失败", &e);
                }
            }
        }
    }

    // ---------- 阶段5：BackupScheduler 远程存储（本地/UNC/WebDAV）----------
    /// 备份完成后转存远端：target 为空不动作；后台线程执行复制/PUT，结果回传 remote_rx。
    fn try_remote_upload(&mut self, i: usize, zip_path: PathBuf) {
        let Some(sc) = self.cfg.servers.get(i) else {
            return;
        };
        let target = sc.backup.remote_target.trim().to_string();
        if target.is_empty() || self.remote_upload_busy.contains(&i) {
            return;
        }
        if !zip_path.exists() {
            return;
        }
        let user = sc.backup.remote_user.clone();
        let pass = sc.backup.remote_password.clone();
        let tx = self.remote_tx.clone();
        self.remote_upload_busy.insert(i);
        std::thread::spawn(move || {
            let res = Self::upload_backup_worker(&zip_path, &target, &user, &pass);
            let _ = tx.send((i, res));
        });
    }

    /// 远端转存后台任务：本地/UNC 走 fs 复制，http(s) 走 WebDAV PUT。
    fn upload_backup_worker(zip_path: &Path, target: &str, user: &str, pass: &str) -> Result<(), String> {
        let lower = target.trim().to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            // WebDAV PUT：目录型 URL 末尾补文件名，文件型 URL 直接 PUT
            let url = if target.ends_with('/') {
                let fname = zip_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "backup.zip".to_string());
                format!("{target}{fname}")
            } else {
                target.to_string()
            };
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .map_err(|e| format!("HTTP 客户端创建失败: {e}"))?;
            let mut req = client.put(&url).body(std::fs::read(zip_path).map_err(|e| format!("读取备份失败: {e}"))?);
            if !user.is_empty() {
                req = req.basic_auth(user, Some(pass));
            }
            req.send()
                .map_err(|e| format!("WebDAV 上传失败: {e}"))?
                .error_for_status()
                .map_err(|e| format!("WebDAV 上传返回错误: {e}"))?;
            Ok(())
        } else {
            // 本地目录 / UNC 网络共享：复制 zip 到目标目录（自动建目录）
            let target_dir = std::path::Path::new(target);
            std::fs::create_dir_all(target_dir).map_err(|e| format!("创建远端目录失败: {e}"))?;
            let fname = zip_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "backup.zip".to_string());
            let dest = target_dir.join(&fname);
            std::fs::copy(zip_path, &dest)
                .map_err(|e| format!("复制到远端失败: {e}"))?;
            Ok(())
        }
    }

    /// Consume remote upload results and manage retries; called from update.
    fn poll_remote_upload(&mut self) {
        while let Ok((srv, res)) = self.remote_rx.try_recv() {
            self.remote_upload_busy.remove(&srv);
            match res {
                Ok(()) => {
                    self.remote_pending.remove(&srv);
                    self.set_toast("远端备份转存完成".to_string());
                }
                Err(e) => {
                    // 失败：登记重试（最多 remote_retry 次），后续 tick_backup 到期重试
                    let remaining = self
                        .remote_pending
                        .get(&srv)
                        .map(|(_, r)| *r)
                        .unwrap_or_else(|| {
                            self.cfg
                                .servers
                                .get(srv)
                                .map(|sc| sc.backup.remote_retry.max(1))
                                .unwrap_or(2)
                        });
                    if remaining > 1 {
                        let path = self
                            .remote_pending
                            .get(&srv)
                            .map(|(p, _)| p.clone())
                            .unwrap_or_default();
                        self.remote_pending
                            .insert(srv, (path, remaining - 1));
                        self.set_toast(format!("远端备份转存失败（将自动重试 {left} 次）: {e}", left = remaining - 1));
                    } else {
                        self.remote_pending.remove(&srv);
                        self.set_toast(format!("远端备份转存失败（已放弃）: {e}"));
                    }
                }
            }
        }
    }

    // ---------- Frp 集中管理（自动下载 frpc + 多隧道合并单 frpc.toml）----------
    /// 数据目录（与配置文件同级）：exe 同目�?data
    fn data_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    /// 后台线程：生�?frpc.toml + 启动单进程（frpc.exe 需已就绪），结果回�?frp_rx
    fn start_frp_all(&mut self) {
        if self.frp_busy || self.frp_proc.is_some() {
            return;
        }
        let frp_dir = self.data_dir().join("frp");
        if !frp_dir.join("frpc.exe").exists() {
            self.frp_log.push_str("未找到 frpc.exe，请先下载或导入后再启动。\n");
            return;
        }
        let server = self.cfg.frp_server.clone();
        let token = self.cfg.frp_token.clone();
        let tunnels: Vec<(String, String, u16, u16)> = self
            .cfg
            .tunnels
            .iter()
            .filter(|t| t.kind == "frp" && t.cfg.trim().is_empty())
            .map(|t| {
                (
                    t.name.clone(),
                    t.frp_proxy_type.clone(),
                    t.frp_local_port,
                    t.frp_remote_port,
                )
            })
            .collect();
        let tx = self.frp_tx.clone();
        self.frp_busy = true;
        self.frp_log.clear();
        self.frp_log.push_str("正在生成 frpc.toml 并启动…\n");
        std::thread::spawn(move || {
            let res = (|| -> Result<process::ManagedProcess, String> {
                let _ = std::fs::create_dir_all(&frp_dir);
                let toml = write_frpc_toml_file(&frp_dir, &server, &token, &tunnels)?;
                process::spawn_hidden(
                    frp_dir.join("frpc.exe").to_str().unwrap_or("frpc"),
                    &["-c", toml.to_str().unwrap_or("frpc.toml")],
                    &frp_dir,
                    false,
                )
                .map_err(|e| e.to_string())
            })();
            let _ = tx.send(res);
        });
    }

    /// 后台线程：仅下载 frpc.exe（官�?镜像），成功后回传版本号
    fn download_frpc_only(&mut self) {
        if self.frp_dl_busy {
            return;
        }
        let frp_dir = self.data_dir().join("frp");
        let tx = self.frp_task_tx.clone();
        self.frp_dl_busy = true;
        self.frp_log.push_str("正在下载 frpc.exe…\n");
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&frp_dir);
            let log_tx = tx.clone();
            let res = download_frpc(&frp_dir, move |s| {
                let _ = log_tx.send(FrpTaskMsg::Progress(s.to_string()));
            })
            .map(|_| frp_local_version(&frp_dir).unwrap_or_else(|| "未知".to_string()));
            let _ = tx.send(FrpTaskMsg::Download(res));
        });
    }

    /// 后台线程：检�?frpc 是否有新版本（GitHub latest release），回传 (本地版本, 远程版本)
    fn check_frp_update(&mut self) {
        if self.frp_ver_busy {
            return;
        }
        let frp_dir = self.data_dir().join("frp");
        let tx = self.frp_task_tx.clone();
        self.frp_ver_busy = true;
        self.frp_log.push_str("正在检查 frpc 更新…\n");
        std::thread::spawn(move || {
            let res = (|| -> Result<(String, String), String> {
                let local = frp_local_version(&frp_dir).ok_or("未找到 frpc.exe，无法检查版本")?;
                let script = "[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; (Invoke-RestMethod -Uri 'https://api.github.com/repos/fatedier/frp/releases/latest' -UseBasicParsing -Headers @{'User-Agent'='XMST'}).tag_name";
                let out = std::process::Command::new("powershell")
                    .creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                    .args(["-NoProfile", "-NonInteractive", "-Command", script])
                    .output()
                    .map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(format!(
                        "获取最新版本失败: {}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                }
                let remote = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if remote.is_empty() {
                    return Err("获取最新版本失败：接口无返回".to_string());
                }
                Ok((local, remote))
            })();
            let _ = tx.send(FrpTaskMsg::CheckUpdate(res));
        });
    }

    /// 后台线程：更�?frpc.exe 到最新（覆盖旧文件），成功后回传新版本号
    fn apply_frp_update(&mut self) {
        if self.frp_dl_busy {
            return;
        }
        let frp_dir = self.data_dir().join("frp");
        let tx = self.frp_task_tx.clone();
        self.frp_dl_busy = true;
        self.frp_log.push_str("正在更新 frpc.exe…\n");
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&frp_dir);
            let log_tx = tx.clone();
            let res = download_frpc(&frp_dir, move |s| {
                let _ = log_tx.send(FrpTaskMsg::Progress(s.to_string()));
            })
            .map(|_| frp_local_version(&frp_dir).unwrap_or_else(|| "未知".to_string()));
            let _ = tx.send(FrpTaskMsg::Download(res));
        });
    }

    fn stop_frp_all(&mut self) {
        if let Some(p) = &self.frp_proc {
            process::kill(p);
        }
        self.frp_proc = None;
        self.frp_log.push_str("已停止 frpc\n");
    }

    /// update 循环：接�?frp 后台回传 / 检�?frpc 异常退�?
    /// 隧道进程异常退出检测：frpc 进程消失（崩溃 / 连接失败退出）时清理状态并通知
    fn tick_tunnels(&mut self) {
        let mut exited: Vec<(usize, String)> = Vec::new();
        for (i, rt) in self.tunnel_runtimes.iter().enumerate() {
            if let Some(p) = &rt.proc {
                if !rt.stopping && !process::is_running(p) {
                    let name = self
                        .cfg
                        .tunnels
                        .get(i)
                        .map(|t| {
                            if t.remark.trim().is_empty() {
                                t.name.clone()
                            } else {
                                t.remark.clone()
                            }
                        })
                        .unwrap_or_default();
                    exited.push((i, name));
                }
            }
        }
        for (i, name) in exited {
            if let Some(rt) = self.tunnel_runtimes.get_mut(i) {
                rt.proc = None;
                rt.log_buf.push_str("提示: 穿透进程已退出（可能连接失败或服务端拒绝），请检查上方日志。\n");
            }
            self.notify(
                "XMST - 穿透已退出",
                &format!("{name} 的穿透进程已停止，请查看隧道日志确认原因"),
            );
        }

        // 测试功能「内网穿透流量显示」禁用时：跳过流量统计轮询（隧道运行/异常退出检测不受影响）
        if !features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC) {
            return;
        }

        // 流量统计轮询（frpc 进程级连接统计，约 2s 一次；TCP 表读取不阻塞 UI）
        let now = now_secs_f64();
        let mut jobs: Vec<(usize, u32, u16)> = Vec::new();
        for (i, rt) in self.tunnel_runtimes.iter_mut().enumerate() {
            if rt.proc.is_none() || rt.stopping {
                continue;
            }
            if let Some(t) = self.cfg.tunnels.get(i) {
                if t.kind == "frp" && rt.traffic_last_at.map_or(true, |at| now - at >= 2.0) {
                    // frpc 与 frps 的主连接固定连向 serverPort，所有隧道流量都经该连接传输；
                    // 按 frpc 进程 PID + serverPort 过滤，避免 frpc→本地服务的短命回环连接采不到
                    if let (Some(pid), Some(port)) =
                        (rt.proc.as_ref().and_then(process::pid), parse_frp_server_port(&t.cfg))
                    {
                        jobs.push((i, pid, port));
                    } else {
                        // PID 或 serverPort 解析失败（frpc.toml 缺失/进程信息不可用）：跳过本次，避免空转
                        rt.traffic_last_at = Some(now);
                    }
                }
            }
        }
        for (i, pid, port) in jobs {
            match query_frpc_traffic(pid, port) {
                Ok((tin, tout)) => {
                    let rt = &mut self.tunnel_runtimes[i];
                    match rt.traffic_last_at {
                        None => {
                            // 首次采样：只记录基准，避免把 frpc 启动以来的历史累计当成瞬时速率
                            rt.traffic_last_in = tin;
                            rt.traffic_last_out = tout;
                            rt.traffic_last_at = Some(now);
                        }
                        Some(last_at) => {
                            let delta_in = if tin >= rt.traffic_last_in {
                                tin - rt.traffic_last_in
                            } else {
                                tin
                            };
                            let delta_out = if tout >= rt.traffic_last_out {
                                tout - rt.traffic_last_out
                            } else {
                                tout
                            };
                            let dt = (now - last_at).max(0.5);
                            rt.traffic_last_in = tin;
                            rt.traffic_last_out = tout;
                            rt.traffic_last_at = Some(now);
                            rt.traffic_hist_in.push_back((delta_in as f64 / dt) as f32);
                            rt.traffic_hist_out.push_back((delta_out as f64 / dt) as f32);
                            while rt.traffic_hist_in.len() > 30 {
                                rt.traffic_hist_in.pop_front();
                            }
                            while rt.traffic_hist_out.len() > 30 {
                                rt.traffic_hist_out.pop_front();
                            }
                            // 累计持续累加不清空，随配置持久化
                            if let Some(t) = self.cfg.tunnels.get_mut(i) {
                                t.traffic_in_total = t.traffic_in_total.saturating_add(delta_in);
                                t.traffic_out_total = t.traffic_out_total.saturating_add(delta_out);
                            }
                        }
                    }
                }
                Err(_) => {
                    // admin API 未就绪（frpc 过旧 / 端口被占）：2s 后再试
                    let rt = &mut self.tunnel_runtimes[i];
                    rt.traffic_last_at = Some(now);
                }
            }
        }
        // 流量累计节流写盘（约 30s 一次，避免高频 IO）
        if now - self.traffic_save_at >= 30.0 {
            self.traffic_save_at = now;
            self.save_config();
        }
    }

    fn tick_frp(&mut self) {
        while let Ok(res) = self.frp_rx.try_recv() {
            self.frp_busy = false;
            match res {
                Ok(mp) => {
                    self.frp_proc = Some(mp);
                    self.frp_log.push_str("frpc 已启动（单进程，frpc.toml 集中管理所有隧道）。\n");
                }
                Err(e) => {
                    self.frp_log.push_str(&format!("frpc 启动失败: {e}\n"));
                }
            }
        }
        while let Ok(msg) = self.frp_task_rx.try_recv() {
            match msg {
                FrpTaskMsg::CheckUpdate(res) => {
                    self.frp_ver_busy = false;
                    match res {
                        Ok((local, remote)) => {
                            if local.trim() == remote.trim() {
                                self.frp_log.push_str(&format!("frpc 已是最新版本（{local}）。\n"));
                                self.set_toast(format!("frpc 已是最新版本（{local}）"));
                            } else {
                                self.frp_update_pending = Some((local, remote));
                            }
                        }
                        Err(e) => {
                            self.frp_log.push_str(&format!("检查更新失败: {e}\n"));
                            self.set_toast(format!("检查 frpc 更新失败: {e}"));
                        }
                    }
                }
                FrpTaskMsg::Download(res) => {
                    self.frp_dl_busy = false;
                    match res {
                        Ok(ver) => {
                            self.frp_local_ver = ver.clone();
                            self.frp_log.push_str(&format!("frpc 下载完成，版本 {ver}。\n"));
                            self.set_toast(format!("frpc 已就绪（v{ver}）"));
                        }
                        Err(e) => {
                            self.frp_log.push_str(&format!("frpc 下载失败: {e}\n"));
                            self.set_toast(format!("frpc 下载失败: {e}"));
                        }
                    }
                }
                FrpTaskMsg::Progress(s) => {
                    self.frp_log
                        .push_str(&format!("[{}] {s}\n", Local::now().format("%H:%M:%S")));
                }
            }
        }
        if let Some(p) = &self.frp_proc {
            // 集中管理流量统计：按 frpc PID + serverPort（frpc.toml 位于 data_dir/frp）过滤主连接
            // 测试功能「内网穿透流量显示」禁用时跳过统计（frpc 运行/日志/退出检测不受影响）
            if features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC) {
                let now2 = now_secs_f64();
                if self
                    .frp_traffic_last_at
                    .map_or(true, |at| now2 - at >= 2.0)
                {
                    if let Some(pid) = process::pid(p) {
                        let port = std::fs::read_to_string(self.data_dir().join("frp").join("frpc.toml"))
                            .ok()
                            .and_then(|t| parse_frp_server_port(&t));
                        if let Some(port) = port {
                            if let Ok((tin, tout)) = query_frpc_traffic(pid, port) {
                                match self.frp_traffic_last_at {
                                    None => {
                                        self.frp_traffic_last_in = tin;
                                        self.frp_traffic_last_out = tout;
                                    }
                                    Some(last_at) => {
                                        let din = if tin >= self.frp_traffic_last_in {
                                            tin - self.frp_traffic_last_in
                                        } else {
                                            tin
                                        };
                                        let dout = if tout >= self.frp_traffic_last_out {
                                            tout - self.frp_traffic_last_out
                                        } else {
                                            tout
                                        };
                                        let dt = (now2 - last_at).max(0.5);
                                        self.frp_traffic_in_rate = (din as f64 / dt) as f32;
                                        self.frp_traffic_out_rate = (dout as f64 / dt) as f32;
                                        self.frp_traffic_in_total =
                                            self.frp_traffic_in_total.saturating_add(din);
                                        self.frp_traffic_out_total =
                                            self.frp_traffic_out_total.saturating_add(dout);
                                        self.frp_traffic_last_in = tin;
                                        self.frp_traffic_last_out = tout;
                                    }
                                }
                            }
                        }
                    }
                    self.frp_traffic_last_at = Some(now2);
                }
            }

            self.frp_log.push_str(&process::drain_to_string(p, 200));
            if !process::is_running(p) {
                let code = process::exit_code(p)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".to_string());
                self.frp_log
                    .push_str(&format!("frpc 进程已退出（code {code}）。\n"));
                self.notify("XMST - 内网穿透异常退出", &format!("frpc 进程已退出（code {code}）"));
                self.frp_proc = None;
            }
        }
    }

    // ---------- 文件浏览 ----------
    /// 清除所有服务器文件浏览的客户端模组排查标记（切走页面/切换服务器时调用，状态不持久化）
    fn clear_client_mod_marks(&mut self) {
        for rt in &mut self.runtimes {
            rt.file_client_mods = None;
            rt.file_client_mods_confirm = false;
        }
    }

    /// 清除所有服务器特殊功能的 Spark 检测/文件列表缓存（切服务器/进入页面时调用，状态不持久化）。
    /// 注意：不清除 spark_prof / spark_prof_refresh_at——分析会话跨页面保留，切页回来进度条不丢失。
    fn clear_special_marks(&mut self) {
        for rt in &mut self.runtimes {
            rt.spark_detect = None;
            rt.spark_files.clear();
            rt.spark_scanned = false;
            rt.spark_analysis = None;
            rt.spark_parsing = false;
        }
    }

    /// 静态解析当前文件浏览目录下所有 .jar 模组元数据，识别客户端模组。
    /// 参考 MSL IsClientSideMod 原理：仅读 jar 内 fabric.mod.json environment / mods.toml side，
    /// 不加载模组；解析失败的文件静默忽略，视为非客户端模组。
    fn analyze_client_mods(&mut self, idx: usize) {
        if idx >= self.runtimes.len() {
            return;
        }
        let target = self.file_tab_target(idx);
        let mut clients: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&target) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.to_lowercase().ends_with(".jar") && e.path().is_file() {
                    if Self::jar_is_client_mod(&e.path()) {
                        clients.push(name);
                    }
                }
            }
        }
        clients.sort();
        self.runtimes[idx].file_client_mods = Some(clients);
    }

    /// 静态解析单个 mod jar 是否客户端模组（启发式，参考 MSL IsClientSideMod 原理，不加载模组）：
    /// - fabric.mod.json 的 environment == "client"；
    /// - META-INF/mods.toml / META-INF/neoforge.mods.toml 的 [[mods]] 块：modId=minecraft 优先，否则取首个块；side == "CLIENT"。
    /// 解析失败一律视为非客户端模组。
    fn jar_is_client_mod(path: &std::path::Path) -> bool {
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        let Ok(mut z) = zip::ZipArchive::new(file) else {
            return false;
        };
        // Fabric：fabric.mod.json environment
        if let Ok(mut f) = z.by_name("fabric.mod.json") {
            let mut buf = Vec::new();
            if std::io::Read::read_to_end(&mut f, &mut buf).is_ok() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) {
                    if v["environment"].as_str() == Some("client") {
                        return true;
                    }
                }
            }
        }
        // Forge / NeoForge：META-INF/mods.toml / neoforge.mods.toml
        for meta in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
            if let Ok(mut f) = z.by_name(meta) {
                let mut buf = Vec::new();
                if std::io::Read::read_to_end(&mut f, &mut buf).is_ok() {
                    if let Ok(v) = toml::from_str::<toml::Value>(&String::from_utf8_lossy(&buf)) {
                        let mods = v.get("mods").and_then(|m| m.as_array());
                        let first_side = mods
                            .and_then(|arr| arr.first())
                            .and_then(|m| m.get("side"))
                            .and_then(|s| s.as_str());
                        let mc_side = mods
                            .and_then(|arr| {
                                arr.iter()
                                    .find(|m| m.get("modId").and_then(|x| x.as_str()) == Some("minecraft"))
                            })
                            .and_then(|m| m.get("side"))
                            .and_then(|s| s.as_str());
                        if mc_side.or(first_side) == Some("CLIENT") {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// 页签根目录（datapack 位于 world/datapacks�?
    fn file_tab_base(&self, idx: usize) -> PathBuf {
        let dir = self.cfg.servers[idx].dir.clone();
        match self.runtimes[idx].file_tab.as_str() {
            "mods" => dir.join("mods"),
            "config" => dir.join("config"),
            "logs" => dir.join("logs"),
            "crash-reports" => dir.join("crash-reports"),
            "datapack" => dir.join("world").join("datapacks"),
            _ => dir.clone(),
        }
    }

    /// 当前浏览目标：页签根 + 子目录栈；切换页签时自动清空子目录栈
    fn file_tab_target(&mut self, idx: usize) -> PathBuf {
        let tab = self.runtimes[idx].file_tab.clone();
        if self.runtimes[idx].file_tab_sub != tab {
            self.runtimes[idx].file_tab_sub = tab.clone();
            self.runtimes[idx].file_sub.clear();
        }
        let mut target = self.file_tab_base(idx);
        let sub = self.runtimes[idx].file_sub.clone();
        for s in &sub {
            target = target.join(s);
        }
        target
    }

    fn refresh_file_list(&mut self, idx: usize) {
        if idx >= self.cfg.servers.len() || idx >= self.runtimes.len() {
            return;
        }
        let target = self.file_tab_target(idx);
        let mut list: Vec<(String, u64, bool)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&target) {
            for e in rd.flatten() {
                let is_dir = e
                    .file_type()
                    .map(|ft| ft.is_dir())
                    .or_else(|_| e.metadata().map(|m| m.is_dir()))
                    .unwrap_or(false);
                let size = if is_dir {
                    // 文件夹显示真实总大小（递归统计），便于排序与展示
                    dir_stats(&e.path()).0
                } else {
                    e.metadata().map(|m| m.len()).unwrap_or(0)
                };
                let name = e.file_name().to_string_lossy().to_string();
                list.push((name, size, is_dir));
            }
        }
        let is_logs_tab = self.runtimes[idx].file_tab == "logs";
        list.sort_by(|a, b| {
            // 目录优先
            if a.2 != b.2 {
                return b.2.cmp(&a.2);
            }
            // logs 页签下 latest.log 置顶，其余老日志（压缩包）下沉
            if is_logs_tab {
                let a_latest = !a.2 && a.0.eq_ignore_ascii_case("latest.log");
                let b_latest = !b.2 && b.0.eq_ignore_ascii_case("latest.log");
                if a_latest != b_latest {
                    return b_latest.cmp(&a_latest);
                }
            }
            // 名称按列头方向排序（正序/倒序）
            if self.file_sort_asc {
                a.0.cmp(&b.0)
            } else {
                b.0.cmp(&a.0)
            }
        });
        self.runtimes[idx].file_list = list;
    }

    fn open_file_edit(&mut self, idx: usize, name: &str) {
        let target = self.file_tab_target(idx);
        let p = target.join(name);
        let content = std::fs::read_to_string(&p).unwrap_or_else(|e| format!("读取失败: {e}"));
        self.runtimes[idx].file_content_edit = Some((p.to_string_lossy().to_string(), content));
    }

    fn save_file_edit(&mut self, idx: usize) {
        if let Some((path, content)) = self.runtimes[idx].file_content_edit.take() {
            match std::fs::write(&path, content) {
                Ok(_) => {
                    self.set_toast("文件已保存".to_string());
                }
                Err(e) => {
                    self.set_toast(format!("保存失败: {e}"));
                }
            }
            self.refresh_file_list(idx);
        }
    }

    /// 用系统默认关联程序打开文件（如 .jar → 7zip / 默认解压器）。
    ///
    /// ★ 改为 `ShellExecuteW(open)`：这是 Windows 官方推荐的"用默认程序打开"方式，
    /// 不再 spawn `explorer.exe`。此前用 `explorer.exe <文件>` 在部分机器/文件类型上
    /// 会弹「explorer.exe 应用程序无法正常启动(0xc0000142)」且打不开
    /// （GUI 子系统进程没有有效标准句柄，子进程初始化失败）。`ShellExecuteW` 不创建
    /// 子进程，彻底绕开这一类问题。
    fn open_file_system(&self, path: &std::path::Path) {
        shell_open(path);
    }

    /// 打开文件所在目录并选中该文件（explorer 的 /select, 无 ShellExecute 等价物；
    /// 传空标准句柄避免 0xc0000142，失败则退化为"打开父目录"）。
    fn open_file_location(&self, path: &std::path::Path) {
        use std::process::Stdio;
        let p = path.to_string_lossy().to_string();
        let ok = std::process::Command::new("explorer.exe")
            .arg("/select,")
            .arg(&p)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok();
        if !ok {
            let parent = path.parent().unwrap_or(path);
            shell_open(parent);
        }
    }

    /// 打开文件夹（纯目录）
    fn open_folder(&self, path: &std::path::Path) {
        shell_open(path);
    }

    /// 删除文件/目录到系统回收站（阶段16④：经 PowerShell Microsoft.VisualBasic.FileIO，
    /// RecycleOption::SendToRecycleBin，可还原；禁止永久删除）
    fn delete_to_recycle_bin(&self, path: &std::path::Path) -> Result<(), String> {
        let p_str = path.to_string_lossy().to_string().replace('\'', "''");
        let api = if path.is_dir() {
            "DeleteDirectory"
        } else {
            "DeleteFile"
        };
        let script = format!(
            "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::{api}('{p_str}','OnlyErrorDialogs','SendToRecycleBin')"
        );
        match std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
        {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    }



    /// mods 内 .jar 检查更新：后台算 SHA1 -> Modrinth version_files 指纹匹配 -> 对比项目最新版本
    fn mod_update_check(&mut self, idx: usize, name: &str, is_disabled: bool) {
        if idx >= self.runtimes.len() {
            return;
        }
        let target = self.file_tab_target(idx);
        let p = target.join(name);
        if !p.exists() {
            return;
        }
        {
            let rt = &mut self.runtimes[idx];
            rt.mod_update_busy = true;
            rt.mod_update_msg = "正在检查更新…".to_string();
            rt.mod_update_target = name.to_string();
            rt.mod_update_target_disabled = is_disabled;
        }
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
        self.runtimes[idx].mod_update_shared = Some(shared.clone());
        let _ = idx;
        std::thread::spawn(move || {
            let res = (|| -> Result<String, String> {
                let bytes = std::fs::read(&p).map_err(|e| format!("读取文件失败: {e}"))?;
                let sha1 = sha1_hex(&bytes);
                let (pid, vid, ver, _date, _furl, _fname) =
                    crate::modrinth::version_by_sha1(&sha1)?;
                let versions = crate::modrinth::project_versions(&pid)?;
                let newest = versions.first().cloned().unwrap_or_default();
                if newest.id == vid {
                    return Ok(format!("latest:{ver}"));
                }
                let nver = newest.version_number;
                let (url, fname) = newest
                    .files
                    .iter()
                    .find(|f| f.filename.ends_with(".jar"))
                    .map(|f| (f.url.clone(), f.filename.clone()))
                    .ok_or("最新版本无 jar 文件")?;
                Ok(format!("new:{pid}:{nver}:{url}:{fname}:{ver}"))
            })();
            *shared.lock().unwrap() = Some(res);
        });
    }

    /// 执行待更新：后台下载新 jar 并原子替换旧文件（.jar.disabled 保持禁用态）
    fn mod_update_apply(&mut self, idx: usize) {
        let (url, new_fname, old_name, is_disabled, new_ver) = {
            let rt = &mut self.runtimes[idx];
            let Some(p) = rt.mod_update_pending.take() else {
                return;
            };
            rt.mod_update_busy = true;
            rt.mod_update_msg = "正在下载更新…".to_string();
            p
        };
        let target = self.file_tab_target(idx);
        let old_p = target.join(&old_name);
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
        self.runtimes[idx].mod_update_shared = Some(shared.clone());
        std::thread::spawn(move || {
            let res = (|| -> Result<String, String> {
                let client = download::new_client();
                let resp = client
                    .get(&url)
                    .send()
                    .map_err(|e| format!("下载请求失败: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("HTTP {}", resp.status()));
                }
                let bytes = resp
                    .bytes()
                    .map_err(|e| format!("读取响应失败: {e}"))?;
                let tmp = target.join(format!(".xmst_update_{}.tmp", std::process::id()));
                std::fs::write(&tmp, &bytes).map_err(|e| format!("写入临时文件失败: {e}"))?;
                let final_name = if is_disabled {
                    format!("{new_fname}.disabled")
                } else {
                    new_fname.clone()
                };
                let new_p = target.join(&final_name);
                if new_p.exists() {
                    let _ = std::fs::remove_file(&new_p);
                }
                if old_p.exists() {
                    let _ = std::fs::remove_file(&old_p);
                }
                std::fs::rename(&tmp, &new_p).map_err(|e| format!("替换文件失败: {e}"))?;
                Ok(format!("done:{new_ver}:{final_name}"))
            })();
            *shared.lock().unwrap() = Some(res);
        });
    }

    /// 消费 mods 更新回传（每帧）
    fn tick_mod_update(&mut self) {
        for idx in 0..self.runtimes.len() {
            let mut done: Option<Result<String, String>> = None;
            {
                let rt = &mut self.runtimes[idx];
                if let Some(shared) = rt.mod_update_shared.take() {
                    done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
                    if done.is_none() {
                        rt.mod_update_shared = Some(shared);
                    }
                }
            }
            let Some(res) = done else { continue };
            let rt = &mut self.runtimes[idx];
            rt.mod_update_busy = false;
            match res {
                Ok(msg) => {
                    if let Some(rest) = msg.strip_prefix("latest:") {
                        rt.mod_update_msg = format!("已是最新版本 {rest}");
                    } else if let Some(rest) = msg.strip_prefix("new:") {
                        // new:project_id:new_ver:url:fname:local_ver
                        let parts: Vec<&str> = rest.splitn(6, ':').collect();
                        if parts.len() == 6 {
                            rt.mod_update_pending = Some((
                                parts[3].to_string(),
                                parts[4].to_string(),
                                rt.mod_update_target.clone(),
                                rt.mod_update_target_disabled,
                                parts[5].to_string(),
                            ));
                            rt.mod_update_msg = format!(
                                "发现新版本 {}（当前 {}），点击「更新」替换",
                                parts[2], parts[5]
                            );
                        } else {
                            rt.mod_update_msg = rest.to_string();
                        }
                    } else if let Some(rest) = msg.strip_prefix("done:") {
                        let parts: Vec<&str> = rest.splitn(3, ':').collect();
                        if parts.len() == 3 {
                            rt.mod_update_msg = format!("已更新为 {}（{}）", parts[1], parts[2]);
                            self.set_toast(format!("模组已更新: {}", parts[2]));
                            self.refresh_file_list(idx);
                        } else {
                            rt.mod_update_msg = rest.to_string();
                        }
                    } else {
                        rt.mod_update_msg = msg;
                    }
                }
                Err(e) => {
                    rt.mod_update_msg = format!("更新失败: {e}");
                }
            }
            self.egui_ctx.request_repaint();
        }
    }
}

/// SHA-1（RFC 3174）十六进制摘要；XMST 不引入额外依赖，供 Modrinth 指纹匹配用
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let msg_len = data.len();
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((msg_len as u64) << 3).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Windows 文件名合法性校验：非法字符、保留名、结尾点/空格
/// Generate a pseudo-random 32-byte hex key for rathole NOISE (xorshift64 seeded by time + pid).
/// Deterministic enough for an optional convenience default; users may paste their own key.
fn gen_rh_key_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut x = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    x ^= std::process::id() as u64;
    let mut out = String::with_capacity(64);
    for _ in 0..32 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push_str(&format!("{:02x}", (x & 0xFF) as u8));
    }
    out
}

/// Recursively find rathole.exe under dir (release zip may nest in a subfolder).
fn find_exe_recursive(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let p = entry.path();
        if p.is_dir() {
            if let Some(f) = find_exe_recursive(&p) {
                return Some(f);
            }
        } else if p
            .file_name()
            .map(|n| n.to_string_lossy().eq_ignore_ascii_case("rathole.exe"))
            .unwrap_or(false)
        {
            return Some(p);
        }
    }
    None
}

fn validate_windows_filename(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("文件名不能为空".to_string());
    }
    if n == "." || n == ".." {
        return Err("不允许使用 . 或 .. 作为文件名".to_string());
    }
    for ch in n.chars() {
        if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            return Err(format!("文件名包含非法字符: {ch}"));
        }
    }
    if n.ends_with('.') || n.ends_with(' ') {
        return Err("文件名不能以点或空格结尾".to_string());
    }
    // 保留设备名（含扩展名形式，如 CON.txt）
    let stem = n.split('.').next().unwrap_or("").to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|r| *r == stem)
        || (stem.starts_with("COM") && stem.len() == 4 && stem[3..].chars().all(|c| c.is_ascii_digit()))
        || (stem.starts_with("LPT") && stem.len() == 4 && stem[3..].chars().all(|c| c.is_ascii_digit()));
    if reserved {
        return Err(format!("{stem} 是 Windows 保留设备名，不能作为文件名"));
    }
    Ok(())
}

/// 将服务器名清理为安全的目录名（非法字符替换为 _）
fn sanitize_server_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.trim().chars() {
        if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let out = out.trim_end_matches(['.', ' ']).to_string();
    if out.is_empty() {
        "server".to_string()
    } else {
        out
    }
}

/// 整合包内启动命令相对化：去除前导 / 与 ./
fn w_launch_rel(entry: &str) -> String {
    entry
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_string()
}

/// 生成一次性 token：系统纳秒时间戳 + 进程号 + 静态计数器
fn fastrand_instant_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    nanos
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((std::process::id() as u64) << 32)
        .wrapping_add(seq)
}

fn load_config(path: &Path) -> GlobalConfig {
    // 旧版配置兼容迁移：新配置（data\xmst_config.json）不存在时，按优先级读取旧位置并自动迁移
    let mut cfg = if !path.exists() {
        if let Some(data_dir) = path.parent() {
            if let Some(exe_dir) = data_dir.parent() {
                for old_name in [CONFIG_FILE, OLD_CONFIG_FILE] {
                    let old = exe_dir.join(old_name);
                    if old.exists() {
                        if let Ok(s) = std::fs::read_to_string(&old) {
                            let cfg: GlobalConfig = serde_json::from_str(&s).unwrap_or_default();
                            if let Ok(json) = serde_json::to_string_pretty(&cfg) {
                                let _ = std::fs::create_dir_all(data_dir);
                                let _ = std::fs::write(path, json);
                            }
                            return cfg;
                        }
                    }
                }
            }
        }
        GlobalConfig::default()
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => GlobalConfig::default(),
        }
    };
    // 旧配置兼容：跟随系统/自动化配色已移除，历史 theme_mode="auto" 一律迁移为夜间。
    if cfg.theme_mode == "auto" {
        cfg.theme_mode = "dark".to_string();
    }
    // 旧配置兼容：自定义配色升级为独立第三模式（theme_mode="custom"），
    // 历史 custom_colors=true（覆盖开关）迁移为新模式，避免旧自定义配色丢失。
    if cfg.custom_colors && cfg.theme_mode != "custom" {
        cfg.theme_mode = "custom".to_string();
    }
    // Bug5：启动时修复历史脏配置（GBK 字节被按 UTF-8 读出的乱码隧道名/备注），并回写磁盘
    if config::repair_cfg_mojibake(&mut cfg) {
        if let Ok(json) = serde_json::to_string_pretty(&cfg) {
            if let Some(data_dir) = path.parent() {
                let _ = std::fs::create_dir_all(data_dir);
            }
            let _ = std::fs::write(path, json);
        }
    }
    // 内容衬底脏值自愈：旧版本滑杆曾把 bg_content_scrim 钳制成 0（第四轮复位 0.25 后又复发），
    // 0 意味着内容面板完全无衬底、桌面细节直穿 UI（「半透明不够清晰」反馈的根因之一）。
    // <0.35 一律视为被误钳制的脏值，抬到新默认 0.5 并回写磁盘；用户显式调高的值保留。
    if cfg.bg_content_scrim < 0.35 {
        cfg.bg_content_scrim = 0.5;
        if let Ok(json) = serde_json::to_string_pretty(&cfg) {
            if let Some(data_dir) = path.parent() {
                let _ = std::fs::create_dir_all(data_dir);
            }
            let _ = std::fs::write(path, json);
        }
    }
    cfg
}

// ---------- Windows 辅助：右下角通知与开机自�?----------

impl App {
    /// Feedback #7: frameless window — explicit edge/corner resize handles.
    /// A 6px hit margin around the window issues `ViewportCommand::BeginResize` on press
    /// so the whole tool can be resized by dragging its borders/corners. The top edge is
    /// intentionally excluded (the custom title bar owns it for moving the window).
    /// 最大化窗口不再直接 return：拖边缘时先还原再进入自绘缩放（行为对齐普通窗口——
    /// 普通窗口最大化后拖边框会先还原再缩放；无边框若直接禁用则边缘拖动完全无反应）。
    fn handle_edge_resize(&mut self, ctx: &egui::Context) {
        let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
        let maximized = window_is_maximized_now(hwnd, self.win_rect_px);
        let rect = ctx.screen_rect();
        // 无边框窗口的缩放热区：太窄会「感觉没法缩放」。
        // 左右下用 9pt，上边用 5pt（上边还要留给标题栏拖动）。
        let r = 9.0f32;
        let rt = 5.0f32;
        let pressed = ctx.input(|i| i.pointer.primary_pressed());
        let Some(p) = ctx.pointer_hover_pos() else { return };
        let on_right = rect.right() - p.x < r;
        let on_left = p.x - rect.left() < r;
        let on_top = p.y - rect.top() < rt;
        let on_bottom = rect.bottom() - p.y < r;
        use egui::viewport::ResizeDirection as D;
        let dir = if on_right && on_bottom {
            Some(D::SouthEast)
        } else if on_left && on_bottom {
            Some(D::SouthWest)
        } else if on_right && on_top {
            Some(D::NorthEast)
        } else if on_left && on_top {
            Some(D::NorthWest)
        } else if on_right {
            Some(D::East)
        } else if on_left {
            Some(D::West)
        } else if on_top {
            Some(D::North)
        } else if on_bottom {
            Some(D::South)
        } else {
            None
        };
        if let Some(dir) = dir {
            let icon = match dir {
                D::East | D::West => egui::CursorIcon::ResizeHorizontal,
                D::South | D::North => egui::CursorIcon::ResizeVertical,
                D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
                _ => egui::CursorIcon::ResizeNwSe,
            };
            ctx.set_cursor_icon(icon);
            // 按下：普通状态交给**系统**缩放（BeginResize）—— 与普通窗口完全一致，
            // 命中测试/最小尺寸/拖拽跟手都由系统负责，最不容易"感觉缩放没反应"。
            // 仅最大化时走自绘路径：先还原、再以还原后的矩形为基准逐帧 SetWindowPos，
            // 因为系统 BeginResize 不会自动从最大化还原（那需要拖标题栏）。
            if pressed {
                use winapi::shared::windef::POINT;
                use winapi::um::winuser::{GetCursorPos, SetCapture};
                if maximized {
                    let mut pt: POINT = unsafe { std::mem::zeroed() };
                    if unsafe { GetCursorPos(&mut pt) } != 0 {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                        self.resize_drag = Some((dir, (pt.x, pt.y), None));
                        unsafe {
                            SetCapture(hwnd);
                        }
                    }
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                }
            }
        }
    }

    /// 窗口是否处于最大化（状态位 + 矩形比对工作区，见 `window_is_maximized_now`）。
    fn window_is_maximized(&mut self) -> bool {
        if self.egui_ctx.input(|i| i.viewport().maximized == Some(true)) {
            return true;
        }
        let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
        window_is_maximized_now(hwnd, self.win_rect_px)
    }

    /// 侧栏导航图标：**全部用矢量图元绘制**，不再用 emoji。
    ///
    /// 为什么不再用 emoji：📊 / 🗂 / ⬇️ / 🧩 / 🔗 / ⚙️ 来自不同字体（彩色 emoji 字体 vs
    /// 符号字体），字形宽度、基线、视觉重量都不同；即使按 CENTER_CENTER 居中，
    /// 实际墨迹位置依然参差 —— 用户连续两轮反馈"图标错位"就是这个原因。
    /// 矢量图标统一画在 18×18 方框内居中，天生像素级对齐。
    fn nav_icon(&self, painter: &egui::Painter, center: egui::Pos2, kind: Nav, color: Color32) {
        let c = center;
        let h = 9.0f32; // 18×18 方框的半边长
        let st = egui::Stroke::new(1.5, color);
        match kind {
            Nav::Logs => {
                // 日志：三横线文档 + 右下角一点（区别于"设置"的推子）
                for k in 0..3 {
                    let y = c.y - 5.0 + k as f32 * 5.0;
                    painter.line_segment(
                        [egui::pos2(c.x - h + 2.0, y), egui::pos2(c.x + h - 4.0, y)],
                        egui::Stroke::new(1.4, color),
                    );
                }
                painter.circle_filled(egui::pos2(c.x + h - 3.0, c.y + 5.0), 2.0, color);
            }
            Nav::Dashboard => {
                // 柱状图：三根高低不同的柱子
                let bw = 3.0;
                for (i, hh) in [0.45f32, 0.9, 0.62].iter().enumerate() {
                    let x = c.x - h + 2.0 + i as f32 * (bw + 2.0);
                    let y0 = c.y + h - 1.0;
                    let y1 = y0 - 18.0 * hh;
                    painter.rect_filled(
                        egui::Rect::from_min_max(egui::pos2(x, y1), egui::pos2(x + bw, y0)),
                        1.0,
                        color,
                    );
                }
            }
            Nav::Servers => {
                // 机架：两个叠放的圆角矩形，各带一个指示点
                for k in 0..2 {
                    let y = c.y - h + 2.0 + k as f32 * (h + 1.0);
                    let rect = egui::Rect::from_min_max(
                        egui::pos2(c.x - h + 1.0, y),
                        egui::pos2(c.x + h - 1.0, y + h - 2.0),
                    );
                    painter.rect_stroke(rect, 1.0, st);
                    painter.circle_filled(egui::pos2(rect.left() + 3.0, rect.center().y), 1.2, color);
                }
            }
            Nav::Download => {
                // 下载：向下箭头 + 底部托盘
                painter.line_segment([egui::pos2(c.x, c.y - h + 2.0), egui::pos2(c.x, c.y + 2.0)], st);
                painter.line_segment([egui::pos2(c.x - 5.0, c.y - 3.0), egui::pos2(c.x, c.y + 2.5)], st);
                painter.line_segment([egui::pos2(c.x + 5.0, c.y - 3.0), egui::pos2(c.x, c.y + 2.5)], st);
                painter.line_segment(
                    [egui::pos2(c.x - h + 2.0, c.y + h - 3.0), egui::pos2(c.x + h - 2.0, c.y + h - 3.0)],
                    st,
                );
            }
            Nav::Plugins => {
                // 插件：插头（圆角矩形 + 两个触点 + 引线）
                let rect = egui::Rect::from_min_max(
                    egui::pos2(c.x - 5.0, c.y - 2.0),
                    egui::pos2(c.x + 5.0, c.y + h - 1.0),
                );
                painter.rect_stroke(rect, 2.0, st);
                painter.line_segment([egui::pos2(c.x - 3.0, c.y - h + 2.0), egui::pos2(c.x - 3.0, c.y - 2.0)], st);
                painter.line_segment([egui::pos2(c.x + 3.0, c.y - h + 2.0), egui::pos2(c.x + 3.0, c.y - 2.0)], st);
                painter.line_segment([egui::pos2(c.x, c.y + h - 1.0), egui::pos2(c.x, c.y + h + 1.0)], st);
            }
            Nav::Tunnel => {
                // 内网穿透：链条（两个斜向交叠的圆角矩形）
                let a = egui::Rect::from_min_max(
                    egui::pos2(c.x - h + 1.0, c.y - 4.0),
                    egui::pos2(c.x + 2.0, c.y + 4.0),
                );
                let b = egui::Rect::from_min_max(
                    egui::pos2(c.x - 2.0, c.y - 4.0),
                    egui::pos2(c.x + h - 1.0, c.y + 4.0),
                );
                painter.rect_stroke(a, 4.0, st);
                painter.rect_stroke(b, 4.0, st);
            }
            Nav::Settings => {
                // 设置：三条带滑块旋钮的推子（最易识别）
                for (i, off) in [-5.0f32, 0.0, 5.0].iter().enumerate() {
                    let y = c.y + off;
                    painter.line_segment(
                        [egui::pos2(c.x - h + 1.0, y), egui::pos2(c.x + h - 1.0, y)],
                        egui::Stroke::new(1.2, color),
                    );
                    let kx = match i {
                        0 => c.x - 2.0,
                        1 => c.x + 3.0,
                        _ => c.x - 4.0,
                    };
                    painter.circle_filled(egui::pos2(kx, y), 2.0, color);
                }
            }
        }
    }

    /// 每帧轮询自绘缩放：左键按住期间按鼠标位移 SetWindowPos 调整窗口
    fn poll_resize_drag(&mut self, ctx: &egui::Context) {
        let Some((dir, start_cur, start_rect_opt)) = self.resize_drag else {
            return;
        };
        let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
        if hwnd.is_null() {
            self.resize_drag = None;
            return;
        }
        use winapi::shared::windef::POINT;
        use winapi::um::winuser::{
            GetAsyncKeyState, GetCursorPos, ReleaseCapture, SetWindowPos, SWP_NOACTIVATE,
            SWP_NOZORDER, VK_LBUTTON,
        };
        // 左键已松开 → 结束缩放（GetAsyncKeyState 返回 i16，先升 i32 再按位与 0x8000）
        let held = (unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as i32) & 0x8000 != 0;
        if !held {
            self.resize_drag = None;
            unsafe {
                ReleaseCapture();
            }
            return;
        }
        // 最大化窗口按下时 start_rect 为 None：等还原命令生效（窗口矩形退出最大化）
        // 后再取还原后的矩形作缩放基准，避免拿最大化屏幕矩形当基准导致尺寸跳变。
        let start_rect = match start_rect_opt {
            Some(r) => r,
            None => {
                if window_is_maximized_now(hwnd, self.win_rect_px) {
                    return; // 还原尚未生效，跳过本帧
                }
                let r = self.win_rect_px;
                self.resize_drag = Some((dir, start_cur, Some(r)));
                r
            }
        };
        let mut pt: POINT = unsafe { std::mem::zeroed() };
        if unsafe { GetCursorPos(&mut pt) } == 0 {
            return;
        }
        // 最小尺寸：逻辑 960x600，按当前 DPI 换算成物理像素
        let ppp = ctx.pixels_per_point().max(0.1);
        let min_w = (960.0 * ppp) as i32;
        let min_h = (600.0 * ppp) as i32;
        let (sx, sy, sw, sh) = start_rect;
        let dx = pt.x - start_cur.0;
        let dy = pt.y - start_cur.1;
        use egui::viewport::ResizeDirection as D;
        let (mut x, mut y, mut w, mut h) = (sx, sy, sw, sh);
        match dir {
            D::East => w = sw + dx,
            D::South => h = sh + dy,
            D::SouthEast => {
                w = sw + dx;
                h = sh + dy;
            }
            D::West => {
                w = sw - dx;
                x = sx + dx;
            }
            D::North => {
                h = sh - dy;
                y = sy + dy;
            }
            D::SouthWest => {
                w = sw - dx;
                x = sx + dx;
                h = sh + dy;
            }
            D::NorthEast => {
                w = sw + dx;
                h = sh - dy;
                y = sy + dy;
            }
            D::NorthWest => {
                w = sw - dx;
                x = sx + dx;
                h = sh - dy;
                y = sy + dy;
            }
            _ => {}
        }
        if w < min_w {
            if x != sx {
                x = sx + sw - min_w;
            }
            w = min_w;
        }
        if h < min_h {
            if y != sy {
                y = sy + sh - min_h;
            }
            h = min_h;
        }
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// 右下角自绘通知（替代系统气泡）：按配置的弹出方式与滞留时间渲染
    fn push_toast(&mut self, title: &str, body: &str) {
        self.next_toast_id += 1;
        self.toasts.push(ToastMsg {
            id: self.next_toast_id,
            title: title.to_string(),
            body: body.to_string(),
            born: std::time::Instant::now(),
        });
        // 最多同时保�?4 条，超出丢弃最旧的
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// 桌面级自绘弹窗（窗口关闭/最小化时也可见）：无边框深色卡片，右下角，6 秒自动关闭。
    /// 非 Windows 通知中心气泡，而是自绘 WinForms 悬浮卡片，保证样式与工具内浮层一致。
    fn sys_toast(&self, title: &str, body: &str) {
        if !self.cfg.sys_notify {
            return;
        }
        let t = title.replace('\'', "''");
        let b = body.replace('\'', "''");
        // 强调条颜色与工具内一致：异常/失败红、已关闭灰、默认蓝
        let (br, bg, bb) = if title.contains("异常") || title.contains("失败") {
            (255, 80, 80)
        } else if title.contains("已关闭") {
            (160, 160, 160)
        } else {
            (0, 191, 255)
        };
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; \
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition 'using System; using System.Runtime.InteropServices; using System.Windows.Forms; public static class DpiHelper {{ [DllImport(\"user32.dll\")] public static extern bool SetProcessDPIAware(); }} public class ToastForm : Form {{ protected override CreateParams CreateParams {{ get {{ CreateParams cp = base.CreateParams; cp.ClassStyle |= 0x00020000; return cp; }} }} }}'; \
[DpiHelper]::SetProcessDPIAware() | Out-Null; \
[System.Windows.Forms.Application]::EnableVisualStyles(); \
$f = New-Object ToastForm; \
$f.FormBorderStyle = 'None'; \
$f.TopMost = $true; \
$f.ShowInTaskbar = $false; \
$f.AutoScaleMode = 'None'; \
$f.StartPosition = 'Manual'; \
$f.BackColor = [System.Drawing.Color]::FromArgb(32,34,42); \
$f.Width = 320; \
$f.Height = 62; \
$f.SetStyle([System.Windows.Forms.ControlStyles]::OptimizedDoubleBuffer -bor [System.Windows.Forms.ControlStyles]::AllPaintingInWmPaint, $true); \
$wa = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea; \
$f.Location = New-Object System.Drawing.Point(($wa.Right - $f.Width - 16), ($wa.Bottom - $f.Height - 16)); \
$path = New-Object System.Drawing.Drawing2D.GraphicsPath; \
$d = 10; \
$path.AddArc(0, 0, $d, $d, 180, 90); \
$path.AddArc(($f.Width - $d), 0, $d, $d, 270, 90); \
$path.AddArc(($f.Width - $d), ($f.Height - $d), $d, $d, 0, 90); \
$path.AddArc(0, ($f.Height - $d), $d, $d, 90, 90); \
$path.CloseFigure(); \
$f.Region = New-Object System.Drawing.Region($path); \
$bar = New-Object System.Windows.Forms.Panel; \
$bar.BackColor = [System.Drawing.Color]::FromArgb({br},{bg},{bb}); \
$bar.Location = New-Object System.Drawing.Point(0,8); \
$bar.Width = 4; \
$bar.Height = 46; \
$tl = New-Object System.Windows.Forms.Label; \
$tl.Text = '{t}'; \
$tl.Font = New-Object System.Drawing.Font('Microsoft YaHei UI', 10.5, [System.Drawing.FontStyle]::Bold); \
$tl.ForeColor = [System.Drawing.Color]::White; \
$tl.Location = New-Object System.Drawing.Point(14,8); \
$tl.AutoSize = $true; \
$bl = New-Object System.Windows.Forms.Label; \
$bl.Text = '{b}'; \
$bl.Font = New-Object System.Drawing.Font('Microsoft YaHei UI', 10); \
$bl.ForeColor = [System.Drawing.Color]::FromArgb(200,205,215); \
$bl.Location = New-Object System.Drawing.Point(14,30); \
$bl.AutoSize = $true; \
$bl.MaximumSize = New-Object System.Drawing.Size(290,30); \
$f.Controls.Add($bar); \
$f.Controls.Add($tl); \
$f.Controls.Add($bl); \
$timer = New-Object System.Windows.Forms.Timer; \
$timer.Interval = 6000; \
$timer.Add_Tick({{ $f.Close() }}); \
$timer.Start(); \
[System.Windows.Forms.Application]::Run($f)"
        );
        let encoded = utf16le_b64(&script);
        let _ = std::process::Command::new("powershell")
            .creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded])
            .spawn();
    }

    /// 自绘 + 系统级同时通知（工具内与系统右下角都可见）
    fn notify(&mut self, title: &str, body: &str) {
        self.sys_toast(title, body);
        self.push_toast(title, body);
    }

    /// 渲染右下角通知（每�?UI 绘制之后调用�?
    fn draw_toasts(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let style = self.cfg.toast_style.clone();
        let hold = self.cfg.toast_duration_secs.max(0.5);
        let now = std::time::Instant::now();
        let screen = ctx.screen_rect();
        let margin = 12.0_f32;
        let card_w = 320.0_f32.min(screen.width() - 2.0 * margin).max(200.0);
        let card_h = 62.0_f32;
        let gap = 8.0_f32;
        let anim = if self.cfg.ui_animations { 0.25_f32 } else { 0.001_f32 }; // 进出动画时长（秒）；关闭动效时近似瞬时
        let total = hold + 2.0 * anim;
        // 逐条计算位置与透明度，渲染后移除过期项
        let mut render: Vec<(u64, String, String, egui::Pos2, f32)> = Vec::new();
        let mut alive: Vec<ToastMsg> = Vec::new();
        for (i, t) in self.toasts.iter().enumerate() {
            let el = now.duration_since(t.born).as_secs_f32();
            if el > total {
                continue; // 已过期，移除
            }
            alive.push(ToastMsg { id: t.id, title: t.title.clone(), body: t.body.clone(), born: t.born });
            let alpha = if el < anim {
                el / anim
            } else if el > hold + anim {
                (total - el) / anim
            } else {
                1.0
            };
            let y = screen.bottom() - margin - (i as f32) * (card_h + gap) - card_h;
            let x = match style.as_str() {
                "fade" => screen.right() - margin - card_w,
                _ => {
                    // slide：从右侧滑入 / 滑出
                    let k = if el < anim {
                        el / anim
                    } else if el > hold + anim {
                        (total - el) / anim
                    } else {
                        1.0
                    };
                    screen.right() - k * (card_w + margin)
                }
            };
            render.push((t.id, t.title.clone(), t.body.clone(), egui::pos2(x, y), alpha));
        }
        self.toasts = alive;
        for (id, title, body, pos, alpha) in render {
            egui::Area::new(egui::Id::new(("toast", id)))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style())
                        .fill(egui::Color32::from_rgba_unmultiplied(32, 34, 42, (alpha * 240.0) as u8))
                        .rounding(egui::Rounding::same(10.0))
                        .shadow(egui::epaint::Shadow {
                            offset: egui::vec2(0.0, 4.0),
                            blur: 18.0,
                            spread: 0.0,
                            color: egui::Color32::from_black_alpha(90),
                        });
                    frame.show(ui, |ui| {
                        ui.set_width(card_w);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            // 左侧强调条（按通知类型着色）
                            let (resp, painter) = ui.allocate_painter(egui::vec2(4.0, card_h - 16.0), egui::Sense::hover());
                            let mut fill = egui::Color32::from_rgba_unmultiplied(0, 191, 255, (alpha * 255.0) as u8);
                            if title.contains("异常") || title.contains("失败") {
                                fill = egui::Color32::from_rgba_unmultiplied(255, 80, 80, (alpha * 255.0) as u8);
                            } else if title.contains("已关闭") {
                                fill = egui::Color32::from_rgba_unmultiplied(160, 160, 160, (alpha * 255.0) as u8);
                            }
                            painter.rect_filled(resp.rect, 2.0, fill);
                            ui.vertical(|ui| {
                                ui.set_width(card_w - 24.0);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&title)
                                            .strong()
                                            .color(egui::Color32::from_rgba_unmultiplied(255, 255, 255, (alpha * 255.0) as u8)),
                                    ),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&body)
                                            .color(egui::Color32::from_rgba_unmultiplied(200, 205, 215, (alpha * 255.0) as u8))
                                            .size(13.0),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    });
                });
        }
    }
}

const AUTOSTART_RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const AUTOSTART_VALUE_NAME: &str = "XMST";

/// �?Frp 隧道默认预填�?frpc.toml 示例（引导用户按自己 frps 服务器信息修改）
const DEFAULT_FRPC_TOML: &str = "\
# frpc.toml —— 请改成你自己的 frps 服务器信息
serverAddr = \"你的服务器IP\"
serverPort = 7000
# user / auth.token 需�与 frps 端配置一致（frps 未启用认证可删除 user 与 auth 段）
user = \"你的用户名\"
auth.method = \"token\"
auth.token = \"你的token\"

[[proxies]]
name = \"mc\"
type = \"tcp\"
localIP = \"127.0.0.1\"
localPort = 25565
remotePort = 25565";

/// 读取开机自启注册表项当前状�?
fn autostart_enabled() -> bool {
    #[cfg(windows)]
    unsafe {
        use winapi::um::winnt::KEY_READ;
        use winapi::um::winreg::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
        let key: Vec<u16> = AUTOSTART_RUN_KEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name: Vec<u16> = AUTOSTART_VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut buf = [0u16; 4096];
        let mut len = (buf.len() * 2) as u32;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut len,
        );
        return rc == 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 写入/删除开机自启注册表项；exe 路径后追�?--autostart 参数
fn set_autostart(exe_path: &str, enable: bool) -> bool {
    #[cfg(windows)]
    unsafe {
        use winapi::um::winnt::{KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ};
        use winapi::um::winreg::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY_CURRENT_USER,
        };
        let key: Vec<u16> = AUTOSTART_RUN_KEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name: Vec<u16> = AUTOSTART_VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut hkey = std::ptr::null_mut();
        let mut disp = 0u32;
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null_mut(),
            &mut hkey,
            &mut disp,
        );
        if rc != 0 || hkey.is_null() {
            return false;
        }
        let ok = if enable {
            let val = format!("{exe_path} --autostart");
            let wide: Vec<u16> = val.encode_utf16().chain(std::iter::once(0)).collect();
            RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                REG_SZ,
                wide.as_ptr() as *const u8,
                (wide.len() * 2) as u32,
            ) == 0
        } else {
            RegDeleteValueW(hkey, name.as_ptr()) == 0
        };
        RegCloseKey(hkey);
        return ok;
    }
    #[cfg(not(windows))]
    {
        let _ = (exe_path, enable);
        false
    }
}

/// 在资源管理器中定位并选中文件（打开所在目录）
fn explorer_select(path: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{path}"))
            .creation_flags(0x08000000)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

fn fmt_size(b: u64) -> String {
    if b >= 1073741824 {
        format!("{:.2} GB", b as f64 / 1073741824.0)
    } else if b >= 1048576 {
        format!("{:.2} MB", b as f64 / 1048576.0)
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}

/// 递归统计目录总字节数与文件数（含子目录），失败时返回 (0, 0)
fn dir_stats(path: &std::path::Path) -> (u64, usize) {
    let mut total = 0u64;
    let mut files = 0usize;
    for e in walkdir::WalkDir::new(path).follow_links(false) {
        match e {
            Ok(entry) => {
                if entry.file_type().is_file() {
                    if let Ok(m) = entry.metadata() {
                        total = total.saturating_add(m.len());
                    }
                    files += 1;
                }
            }
            Err(_) => continue,
        }
    }
    (total, files)
}

/// 异步加载 CJK 字体（启动仅 ASCII，不阻塞；托盘态不加载；恢复/可见后懒加载一次）。
/// 返回构造好的 FontDefinitions，由调用方 set_fonts + request_repaint（egui::Context 线程安全）。
fn load_cjk_fonts() -> Option<egui::FontDefinitions> {
    // 内存优先：msyh.ttc（微软雅黑，约 20MB 常驻）对低内存目标不友好；
    // simhei.ttf（黑体）字形覆盖足且仅约 10MB，先试它，失败再退回雅黑/宋体
    let candidates = [
        "C:\\Windows\\Fonts\\simhei.ttf", // SimHei（最小可用 CJK 字体）
        "C:\\Windows\\Fonts\\msyh.ttc",   // Microsoft YaHei
        "C:\\Windows\\Fonts\\simsun.ttc", // SimSun
    ];
    let mut bytes = None;
    for p in candidates {
        if let Ok(b) = std::fs::read(p) {
            bytes = Some(b);
            break;
        }
    }
    let Some(bytes) = bytes else { return None };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
    // 追加 Windows 系统 emoji 字体兜底：egui 内置 NotoEmoji/emoji-icon-font
    // 覆盖不全时（📁/⚙️/🔗 等）会渲染成方格，Segoe UI Emoji 覆盖面最广
    if let Ok(eb) = std::fs::read("C:\\Windows\\Fonts\\seguiemj.ttf") {
        fonts
            .font_data
            .insert("emoji".to_owned(), egui::FontData::from_owned(eb).into());
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("emoji".to_owned());
            list.push("cjk".to_owned());
        }
    }
    Some(fonts)
}

/// Single-instance guard: create a named mutex once per session.
/// Returns false when another XMST instance is already running.
fn single_instance_check() -> bool {
    unsafe {
        use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
        use winapi::um::errhandlingapi::GetLastError;
        use winapi::um::synchapi::CreateMutexW;
        let name: Vec<u16> = "Local\\XMST_SingleInstance"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mutex = CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr());
        if mutex.is_null() {
            return false;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // 已有实例（多数情况是上一个实例正停留在托盘里）。
            //
            // 旧行为：弹模态 MessageBox「已在运行，请先关闭已有实例」—— 因为关闭窗口 =
            // 最小化到托盘、进程仍在，所以**每次双击 exe 都会弹一次**，用户把它当成了
            // "打开文件警告"。现在改为：**通知已有实例把窗口弹到前台**，然后静默退出。
            //
            // 用命名事件做进程间唤醒（比 FindWindow+ShowWindow 可靠得多）：
            // 目标实例里有一个线程阻塞在 WaitForSingleObject 上，收到即恢复窗口；
            // 直接 Win32 ShowWindow 会绕过程序内部状态（winit 仍以为窗口是隐藏的），
            // 那正是"恢复后黑屏"的成因之一。
            use winapi::um::handleapi::CloseHandle;
            use winapi::um::synchapi::{OpenEventW, SetEvent};
            use winapi::um::winnt::EVENT_MODIFY_STATE;
            let ev: Vec<u16> = "Local\\XMST_ShowMainWindow"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let h = OpenEventW(EVENT_MODIFY_STATE, 0, ev.as_ptr());
            if !h.is_null() {
                SetEvent(h);
                CloseHandle(h);
            }
            return false;
        }
        true
    }
}

/// 读取服务器目录里 .bat 的 `set "MAX_RESTARTS=n"`（用于发现 bat 自带的重启循环）。
fn read_bat_max_restarts(dir: &Path) -> Option<(String, i32)> {
    let rd = std::fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x.eq_ignore_ascii_case("bat")).unwrap_or(false) {
            let Ok(s) = std::fs::read_to_string(&p) else { continue };
            for line in s.lines() {
                let l = line.trim();
                if l.to_ascii_uppercase().contains("MAX_RESTARTS") {
                    // set "MAX_RESTARTS=5" / set MAX_RESTARTS=5
                    if let Some(v) = l.split('=').nth(1) {
                        let n = v.trim().trim_matches('"').trim().parse::<i32>().ok()?;
                        return Some((
                            p.file_name().unwrap_or_default().to_string_lossy().to_string(),
                            n,
                        ));
                    }
                }
            }
        }
    }
    None
}

/// 把 .bat 里的 `MAX_RESTARTS` 改成指定值（保持其它内容不变）。返回被修改的文件名。
fn write_bat_max_restarts(dir: &Path, value: i32) -> Result<String, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().map(|x| x.eq_ignore_ascii_case("bat")).unwrap_or(false) {
            continue;
        }
        let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        if !s.to_ascii_uppercase().contains("MAX_RESTARTS") {
            continue;
        }
        let mut out = String::with_capacity(s.len());
        for line in s.lines() {
            if line.to_ascii_uppercase().contains("MAX_RESTARTS") && line.contains('=') {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                // 保留原有的 set 写法风格
                if line.contains('"') {
                    out.push_str(&format!("{indent}set \"MAX_RESTARTS={value}\""));
                } else {
                    out.push_str(&format!("{indent}set MAX_RESTARTS={value}"));
                }
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        std::fs::write(&p, out).map_err(|e| e.to_string())?;
        return Ok(p.file_name().unwrap_or_default().to_string_lossy().to_string());
    }
    Err("未找到含 MAX_RESTARTS 的 .bat".to_string())
}

/// 用系统默认关联打开文件/目录（ShellExecuteW，不创建子进程）。
fn shell_open(path: &Path) {
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::winuser::SW_SHOWNORMAL;
    let op: Vec<u16> = "open\0".encode_utf16().collect();
    let file: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}
/// 在资源管理器中定位到某个文件/目录（文件则选中它）。
///
/// 注意：**不要**用 `raw_arg` 传带引号的整串，也不要给 explorer.exe 加 CREATE_NO_WINDOW ——
/// 实测会弹出「explorer.exe 应用程序无法正常启动(0xc0000142)」错误框，而且资源管理器不打开。
/// 这里用最标准的写法：`explorer /select,<path>` 两个参数分开传。
fn reveal_in_explorer(p: &Path) {
    let mut cmd = std::process::Command::new("explorer.exe");
    if p.is_dir() {
        cmd.arg(p);
    } else {
        cmd.arg("/select,").arg(p);
    }
    let _ = cmd.spawn();
}

/// 目录体积（人类可读，如 "1.2 GB"）；不存在返回 "—"。
fn dir_size_mb(p: &Path) -> String {
    if !p.exists() {
        return "—".to_string();
    }
    let mut total: u64 = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else {
                total += md.len();
            }
        }
    }
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if total as f64 >= GB {
        format!("{:.2} GB", total as f64 / GB)
    } else {
        format!("{:.0} MB", total as f64 / MB)
    }
}

/// 整个虚拟桌面尺寸（物理像素，含所有显示器）。用于 F1 的尺寸/位置越界校验。
fn primary_screen_size() -> Option<(f32, f32)> {
    #[cfg(windows)]
    unsafe {
        use winapi::um::winuser::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN};
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w > 0 && h > 0 {
            return Some((w as f32, h as f32));
        }
    }
    None
}

/// 主屏逻辑尺寸（物理像素 ÷ DPI scale）。
///
/// 窗口恢复的尺寸上限/位置校验**必须**用它，而不是 `primary_screen_size()`：
/// 虚拟屏 = 多显示器物理像素总和，且未除 DPI scale——直接当逻辑上限用，会在
/// 副屏/低 DPI 机器上把记忆的窗口尺寸原样还原（巨大窗口、拖不到缩放边缘的根因）。
#[cfg(windows)]
fn primary_screen_logical() -> Option<(f32, f32)> {
    unsafe {
        use winapi::um::winuser::{GetDpiForSystem, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        if w <= 0 || h <= 0 {
            return None;
        }
        // GetDpiForSystem 自 Win10 1607 起可用；失败按 96 DPI（scale=1.0）兜底。
        let dpi = GetDpiForSystem();
        let scale = (dpi as f32 / 96.0).max(1.0);
        Some((w as f32 / scale, h as f32 / scale))
    }
}

#[cfg(not(windows))]
fn primary_screen_logical() -> Option<(f32, f32)> {
    None
}

/// 窗口是否最大化：状态位之外，再用「矩形 vs 显示器工作区」兜底判断。
///
/// 兜底是必需的：Aero Snap（拖到屏幕顶端）/双击标题栏/系统热键最大化时，
/// winit 的状态位可能不反映，而按矩形比对不会漏 —— 上一版正是漏了，
/// 把「最大化后的整屏尺寸」写进配置，导致下次启动开出巨大窗口。
#[cfg(windows)]
fn window_is_maximized_now(
    hwnd: winapi::shared::windef::HWND,
    rect_px: (i32, i32, i32, i32),
) -> bool {
    use winapi::um::winuser::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    if hwnd.is_null() {
        return false;
    }
    let (_, _, w, h) = rect_px;
    if w <= 0 || h <= 0 {
        return false;
    }
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if mon.is_null() {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) == 0 {
            return false;
        }
        let work_w = mi.rcWork.right - mi.rcWork.left;
        let work_h = mi.rcWork.bottom - mi.rcWork.top;
        w * 100 >= work_w * 97 && h * 100 >= work_h * 97
    }
}

#[cfg(not(windows))]
fn window_is_maximized_now(_hwnd: isize, _rect_px: (i32, i32, i32, i32)) -> bool {
    false
}

fn main() -> eframe::Result {
    // 诊断入口：XMST_CRASHSCAN=<服务器目录> → 只跑崩溃分析并把结论打到控制台后退出。
    // 用于验证分析规则（无需启动 GUI，也不依赖真实崩溃现场）。
    if let Ok(dir) = std::env::var("XMST_CRASHSCAN") {
        match crashscan::analyze(Path::new(&dir)) {
            Some(f) => {
                println!("[崩溃分析] 来源={} 摘要={}", f.source, f.summary);
                for c in &f.causes {
                    println!("- 结论：{}", c.title);
                    if !c.suspects.is_empty() {
                        println!("  涉及：{}", c.suspects.join("、"));
                    }
                    println!("  建议：{}", c.advice);
                    if let Some(p) = &c.path {
                        println!(
                            "  跳转：{p}{}",
                            if c.path_broken { "（内容不是合法 JSON）" } else { "" }
                        );
                    }
                    for e in &c.evidence {
                        println!("  证据：{}", e);
                    }
                }
            }
            None => println!("[崩溃分析] 未发现可识别的崩溃原因（dir={dir}）"),
        }
        std::process::exit(0);
    }
    // 崩溃日志钩子：panic 时把信息+堆栈写入 data\crash.log 并弹窗提示（便于定位崩溃�?
    {
        let crash_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("data").join("crash.log")))
            .unwrap_or_else(|| PathBuf::from("crash.log"));
        std::panic::set_hook(Box::new(move |info| {
            let msg = info.to_string();
            let bt = std::backtrace::Backtrace::force_capture();
            let text = format!(
                "[{}] PANIC: {}\n\nBacktrace:\n{}\n{}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                msg,
                bt,
                "=".repeat(70)
            );
            if let Some(parent) = crash_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // D7：崩溃日志轮转——超过 1MB 就滚成 crash.log.1（保留上一次），
            // 避免反复崩溃/长 backtrace 把文件撑到几十 MB
            if let Ok(meta) = std::fs::metadata(&crash_path) {
                if meta.len() > 1024 * 1024 {
                    let _ = std::fs::rename(&crash_path, crash_path.with_extension("log.1"));
                }
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&crash_path)
            {
                use std::io::Write;
                let _ = f.write_all(text.as_bytes());
            }
            unsafe {
                use winapi::um::winuser::{MessageBoxW, MB_ICONERROR, MB_OK};
                let m: Vec<u16> = format!(
                    "XMST 发生异常即将退出，崩溃日志已写入\n{}",
                    crash_path.display()
                )
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
                let t: Vec<u16> = "XMST 崩溃".encode_utf16().chain(std::iter::once(0)).collect();
                MessageBoxW(
                    std::ptr::null_mut(),
                    m.as_ptr(),
                    t.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }));
    }
    if !single_instance_check() {
        std::process::exit(0);
    }
    // F1：窗口位置/大小记忆（读取启动前即可用的配置；data 目录与 App::new 同源）
    let startup_cfg = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("data").join(config::CONFIG_FILE)))
        .map(|p| load_config(&p));
    let (win_pos, win_size) = startup_cfg
        .as_ref()
        .map(|c| (c.window_pos, c.window_size))
        .unwrap_or(([0.0, 0.0], [0.0, 0.0]));
    // 主屏逻辑尺寸（物理 ÷ DPI）：同时用于「尺寸上限」「最小尺寸」与「位置越界」校验。
    // 注意不要用 primary_screen_size()（虚拟屏物理像素）——多显示器/低 DPI 下会把
    // 记忆尺寸原样还原，导致巨大窗口且拖不到缩放边缘（本问题修复的核心）。
    let screen = primary_screen_logical().unwrap_or((1920.0, 1080.0));
    let want_size = if win_size[0] >= 400.0 && win_size[1] >= 300.0 {
        [
            win_size[0].clamp(400.0, screen.0),
            win_size[1].clamp(300.0, screen.1),
        ]
    } else {
        [1180.0_f32.min(screen.0), 760.0_f32.min(screen.1)]
    };
    let mut vp = egui::ViewportBuilder::default()
        .with_inner_size(want_size)
        .with_min_inner_size([960.0_f32.min(screen.0), 600.0_f32.min(screen.1)]);
    // 位置：0,0 视为未保存；还原前做越界校验（窗口**整体**须在屏内 ±64 容差），
    // 避免记忆到已拔掉的显示器导致窗口出现在屏幕外、且边缘拖不到无法缩放。
    if win_pos[0] != 0.0 || win_pos[1] != 0.0 {
        if win_pos[0] >= -64.0
            && win_pos[1] >= -64.0
            && win_pos[0] + want_size[0] <= screen.0 + 64.0
            && win_pos[1] + want_size[1] <= screen.1 + 64.0
        {
            vp = vp.with_position([win_pos[0], win_pos[1]]);
        }
    }
    let options = eframe::NativeOptions {
        viewport: vp
            .with_title("XMST - 修暝的服务器工具")
            .with_icon(make_app_icon())
            .with_resizable(true)
            // 去除系统标题栏：最小化/最大化/关闭按钮由工具内自绘标题栏提�?
            .with_decorations(false)
            // 透明窗口底座：egui surface 带 alpha，供 DWM 亚克力/磨砂透出；default 由 clear_color 不透明兜底
            .with_transparent(true),
        // P0 阶段 0：显式关闭 MSAA / 深度缓冲，垂直同步开启（托盘瘦身 + 低占用）
        multisampling: 0,
        depth_buffer: 0,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "xmst",
        options,
        Box::new(|cc| {
            // Stage 6：主题由 App::tick_theme 每帧动态应用（初始色板来自已加载配置，默认夜间），
            // 不再在此硬编码暗色 visuals
            // P0 阶段 0 A3：CJK 字体懒加载——启动仅 ASCII，不在此处同步加载，
            // 由 App::update 可见态异步加载（托盘态不加载，恢复窗口后补加载）
            Ok(Box::new(App::new(cc.egui_ctx.clone())))
        }),
    )
}

use egui::{Color32, RichText, TextEdit};

/// 读取整机网卡累计收发字节（纯 Rust GetIfTable，替代 powershell Get-Counter，
/// 修复 Bug2：运行时不再闪现 PowerShell/conhost 进程）。失败返回 (0, 0)。
fn net_iface_bytes() -> (u64, u64) {
    use winapi::shared::ifmib::{MIB_IFTABLE, MIB_IFROW};
    use winapi::um::iphlpapi::GetIfTable;
    unsafe {
        let mut size: u32 = 0;
        // 第一次调用获取所需缓冲区大小（期望 ERROR_INSUFFICIENT_BUFFER）
        GetIfTable(std::ptr::null_mut(), &mut size, 0);
        if size == 0 {
            return (0, 0);
        }
        let mut buf = vec![0u8; size as usize];
        let table = buf.as_mut_ptr() as *mut MIB_IFTABLE;
        if GetIfTable(table, &mut size, 0) != 0 {
            return (0, 0);
        }
        let mut rx: u64 = 0;
        let mut tx: u64 = 0;
        let count = (*table).dwNumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).table.as_ptr(), count);
        for row in rows {
            // 跳过软件回环(24)与隧道(131)等非物理接口，避免速率抖动
            if row.dwType == 24 || row.dwType == 131 {
                continue;
            }
            rx = rx.saturating_add(row.dwInOctets as u64);
            tx = tx.saturating_add(row.dwOutOctets as u64);
        }
        (rx, tx)
    }
}

/// 生成 egui 官方风格图标 RGBA（橙色圆角底 + 白色 e，与 exe/托盘图标一致）
fn make_theme_icon_rgba(S: usize) -> Vec<u8> {
    match S {
        64 => include_bytes!("../assets/icon_64.rgba").to_vec(),
        _ => include_bytes!("../assets/icon_32.rgba").to_vec(),
    }
}

/// 生成窗口图标（64x64，egui 默认风格）
fn make_app_icon() -> egui::IconData {
    const S: usize = 64;
    egui::IconData {
        rgba: make_theme_icon_rgba(S),
        width: S as u32,
        height: S as u32,
    }
}

/// 生成托盘图标（32x32，egui 默认风格）
fn make_tray_icon() -> tray_icon::Icon {
    const S: usize = 32;
    tray_icon::Icon::from_rgba(make_theme_icon_rgba(S), S as u32, S as u32).expect("tray icon")
}

/// 创建系统托盘图标 + 菜单；返�?(TrayIcon, show_id, quit_id)
/// 事件处理：注册全局回调（回调线程），把状态写进 Arc<AtomicBool>，
/// 窗口显隐/退出由 update 轮询消费。避免「托盘事件在 UI 线程 try_recv 轮询、
/// 窗口隐藏后事件循环休眠导致事件积压不响应」的根因。
fn setup_tray_and_menu() -> (
    Option<tray_icon::TrayIcon>,
    Option<tray_icon::menu::MenuId>,
    Option<tray_icon::menu::MenuId>,
) {
    // 事件处理采用全局回调（TrayIconEvent/MenuEvent::set_event_handler）：
    // 回调在托盘消息线程内同步执行，不依赖 egui 事件循环，窗口隐藏后依然可靠；
    // 回调需要 egui::Context，因此在首次 update 里注册（见 setup_tray_handlers）。
    // hidden/quit 状态用 Arc<AtomicBool> 由回调写、update 读。
    use tray_icon::menu::{Menu, MenuItem};
    let menu = Menu::new();
    let show_item = MenuItem::new("显示主窗口", true, None);
    let quit_item = MenuItem::new("退出 XMST", true, None);
    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();
    if menu.append(&show_item).is_err() || menu.append(&quit_item).is_err() {
        return (None, None, None);
    }
    let icon = make_tray_icon();
    match tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("修暝的服务器工具 (XMST)")
        .with_icon(icon)
        .build()
    {
        Ok(tray) => (Some(tray), Some(show_id), Some(quit_id)),
        Err(_) => (None, None, None),
    }
}

/// 自绘标题栏按钮：返回 Response；绘制内容由 draw 闭包给出（painter + rect）�?
/// hover_fill 为悬�?按下时的背景色；图标线条统一 1.6~1.8px 居中绘制�?
/// 按进程 ID 查找本进程的主窗口，按窗口标题识别（含 "XMST"）。
/// 不能只看可见性：托盘图标的消息窗口（tray_icon_app 类）同属本进程且不可见，
/// 隐藏主窗口后若按"不可见优先"会误选它，导致 ShowWindow / PostMessageW 全部无效。
fn main_hwnd() -> winapi::shared::windef::HWND {
    use winapi::shared::minwindef::{BOOL, LPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId};
    unsafe extern "system" fn find_main(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
            if n > 0 {
                let t = String::from_utf16_lossy(&buf[..n as usize]);
                if t.contains("XMST") {
                    let slot = lparam as *mut HWND;
                    *slot = hwnd;
                    return 0; // 找到即停止
                }
            }
        }
        1
    }
    let mut found: HWND = std::ptr::null_mut();
    unsafe {
        EnumWindows(Some(find_main), &mut found as *mut HWND as LPARAM);
    }
    found
}

/// 写 32bpp BMP（自写文件头，避免为诊断截图链入 PNG 编码器让 exe 增大约 3.5MB）。
fn write_bmp32(
    path: &std::path::Path,
    w: usize,
    h: usize,
    rgba: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    let data_size = w * 4 * h;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"BM")?;
    f.write_all(&((54 + data_size) as u32).to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?; // reserved
    f.write_all(&54u32.to_le_bytes())?; // pixel data offset
    f.write_all(&40u32.to_le_bytes())?; // BITMAPINFOHEADER size
    f.write_all(&(w as i32).to_le_bytes())?;
    f.write_all(&(h as i32).to_le_bytes())?; // 正高度 = 自下而上
    f.write_all(&1u16.to_le_bytes())?; // planes
    f.write_all(&32u16.to_le_bytes())?; // bpp
    f.write_all(&0u32.to_le_bytes())?; // BI_RGB
    f.write_all(&(data_size as u32).to_le_bytes())?;
    f.write_all(&2835u32.to_le_bytes())?; // 96 DPI
    f.write_all(&2835u32.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    for y in (0..h).rev() {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if i + 2 < rgba.len() {
                f.write_all(&[rgba[i + 2], rgba[i + 1], rgba[i], 255])?;
            }
        }
    }
    Ok(())
}

/// 主窗口 HWND 的缓存版本：`main_hwnd()` 每次都要 `EnumWindows` 枚举全部顶层窗口，/// 而 `apply_window_round_region` / 背景捕获等每帧路径都会用到它（纯浪费 + 标题变动时脆弱）。
/// 这里缓存一次，仅在 `IsWindow` 失效（窗口被重建）时重新解析。
fn resolve_main_hwnd(cache: &mut Option<isize>) -> winapi::shared::windef::HWND {
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::IsWindow;
    if let Some(v) = *cache {
        let h = v as HWND;
        if !h.is_null() && unsafe { IsWindow(h) } != 0 {
            return h;
        }
    }
    let h = main_hwnd();
    *cache = if h.is_null() { None } else { Some(h as isize) };
    h
}

/// 恢复主窗口到前台（托盘「显示」/ 左键单击）。
fn show_main_window() {
    use winapi::um::winuser::{SetForegroundWindow, ShowWindow, SW_SHOW};
    unsafe {
        let hwnd = main_hwnd();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    }
}

fn titlebar_button(
    ui: &mut egui::Ui,
    _id: &str,
    hover_fill: Color32,
    draw: impl Fn(&egui::Painter, egui::Rect),
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    let painter = ui.painter();
    if resp.is_pointer_button_down_on() {
        painter.rect_filled(rect, 4.0, Color32::from_rgb(92, 98, 114));
    } else if resp.hovered() {
        painter.rect_filled(rect, 4.0, hover_fill);
    }
    draw(&painter, rect);
    resp
}

impl eframe::App for App {
    /// 清屏颜色：任一非默认背景模式都清为全透明，让 DWM 模糊/亚克力或半透明覆盖层之外
    /// 的像素真的透出桌面；默认模式清为不透明底色，避免透明窗口在面板未覆盖处露出桌面。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.plugin_bg_style.needs_transparency() {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            egui::Color32::from_rgb(12, 12, 12).to_normalized_gamma_f32()
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // P0 阶段0 A1：托盘态 0 重绘——隐藏时立即返回，不渲染、不注册心跳。
        // 事件循环进入休眠（延迟隐藏帧已把 ControlFlow 拉回 Wait 后长眠，无任何重绘请求）；
        // 托盘事件由全局回调（set_event_handler）经 Arc 版本号 + request_repaint 唤醒一次。
        if self.tray_hidden.load(std::sync::atomic::Ordering::Relaxed) {
            // 延迟一帧发隐藏命令兜底（通常关闭帧已立即隐藏，此处幂等）：
            // 若关闭帧 Visible(false) 已生效则无事发生；若因事件循环时序未消费则本帧补发。
            if self.hide_requested && !self.hide_sent {
                self.hide_sent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            // ★ 托盘态的清理必须放在**窗口确实已经隐藏之后**：
            // 之前是在关闭帧里就丢纹理/清缓存，那一帧的交换缓冲可能还是黑的，
            // 而窗口又没真正隐藏（winit 命令未生效）→ 桌面上留着一个黑窗口。
            // 现在：先兜底用 Win32 真正隐藏窗口，再释放纹理与缓存 —— 黑屏在结构上不可能出现。
            if !self.tray_cleanup_done {
                self.tray_cleanup_done = true;
                use winapi::um::winuser::ShowWindow;
                let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
                if !hwnd.is_null() {
                    unsafe {
                        ShowWindow(hwnd, winapi::um::winuser::SW_HIDE);
                    }
                }
                // 窗口已不可见：丢弃会随缓存清理一起失效的纹理句柄（恢复时重建）
                self.backdrop.forget_texture();
                self.noise_tex = None;
                self.bg_tex = None;
                ctx.memory_mut(|mem| {
                    mem.caches = Default::default();
                });
            }
            return;
        }
        // ★ 恢复窗口（托盘 → 显示）后的第一帧：全量重建渲染状态。
        //
        // 黑屏窗口的根因：进入托盘态后会清空 egui 缓存 + 重置字体 + EmptyWorkingSet（为了把
        // 工作集压到 ~2MB），这会让**背景材质纹理/壁纸纹理句柄失效**；而材质模式下内容面板
        // 的填充 alpha 是 0（靠纹理透出桌面），纹理一旦没了 → 整窗只剩 clear 色 = 黑屏。
        // 这里在恢复帧把一切都重建：字体重新懒加载、壁纸重载、材质强制重抓、
        // 窗口圆角区域重设（隐藏会丢 SetWindowRgn）、并连续请求几帧重绘保证刷出来。
        if self
            .tray_restoring_flag
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            // 关键：先让 winit 的窗口状态与系统一致。
            // 旧实现只调了 Win32 的 show_main_window()，winit 仍认为窗口是隐藏的
            // → 不渲染 = 恢复后黑屏。
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.tray_cleanup_done = false;
            self.cjk_loading = false; // 触发 CJK/emoji 字体重新加载（托盘态被释放过）
            if !self.cfg.bg_image.is_empty() {
                self.refresh_bg();
            }
            self.bg_needs_refresh = true; // 强制立刻重抓一帧材质
            self.last_win_rgn = (-1, -1, -1); // 隐藏会丢掉 SetWindowRgn 的圆角区域
            self.apply_window_round_region(ctx, self.win_r_points);
            for _ in 0..3 {
                ctx.request_repaint();
            }
        }
        // 恢复窗口后的首次 update：消费托盘版本号，强制全量同步
        // （隐藏期间日志/玩家/流量均未拉取，本帧 tick 已全量补拉，再请求一帧确保 UI 全刷新）
        // Feedback #7: frameless window needs explicit edge resize handles. The top edge
        // is left to the custom title-bar drag (move); all other edges + corners enter a
        // custom SetWindowPos resize loop when pressed within the 9px hit margin
        // (system BeginResize replaced by custom loop for frameless consistency).
        self.handle_edge_resize(ctx);
        // 无边框窗口自绘缩放：边缘按住拖动逐帧 SetWindowPos（与系统窗口一致的自由缩放）
        self.poll_resize_drag(ctx);
        // F12：把当前窗口画面存成 BMP（背景效果开启时窗口会被系统排除在截屏之外，
        // 外部截屏/录屏都拍不到本窗口，因此提供内置截图作为唯一可靠手段）。
        if !self.shot_done
            && self.shot_path.is_none()
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::F12))
        {
            let name = format!(
                "screenshot_{}.bmp",
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            );
            self.shot_path = Some(self.data_dir().join(name));
            self.shot_frames = 0;
        }
        // 内置自测截图（XMST_SHOT=<路径> 或 F12）：等材质稳定后拍一帧并存盘。
        if let Some(path) = self.shot_path.clone() {
            if !self.shot_done {
                self.shot_frames += 1;
                if self.shot_frames == 45 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }                let img = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = img {
                    let (w, h) = (img.size[0], img.size[1]);
                    let mut bytes = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        bytes.extend_from_slice(&[p.r(), p.g(), p.b(), p.a()]);
                    }
                    // 写 BMP（自己写头，避免为诊断功能链入 PNG 编码器让 exe 大一截）
                    match write_bmp32(&path, w, h, &bytes) {
                        Ok(()) => {
                            eprintln!("XMST_SHOT saved: {} ({w}x{h})", path.display());
                            self.set_toast(format!("已保存窗口截图：{}", path.display()));
                        }
                        Err(e) => eprintln!("XMST_SHOT failed: {e}"),
                    }
                    self.shot_done = true;
                    if std::env::var("XMST_SHOT_EXIT").is_ok() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        // 诊断（只取一次）：GL 帧缓冲 (红, alpha) 位数。注意 update 阶段 GL 上下文不一定当前，
        // red<=0 视为「测不到」，不伪造数值；结果写入 data\bg_debug.log（见 App::bg_debug_log）。
        if self.gl_fb_bits.is_none() {
            use eframe::glow::HasContext as _;
            // GL_RED_BITS / GL_ALPHA_BITS 的数值（不同 glow 版本导出名不一致，写字面量最稳）
            const GL_RED_BITS: u32 = 0x0D52;
            const GL_ALPHA_BITS: u32 = 0x0D55;
            if let Some(gl) = frame.gl() {
                let red = unsafe { gl.get_parameter_i32(GL_RED_BITS) };
                if red > 0 {
                    let alpha = unsafe { gl.get_parameter_i32(GL_ALPHA_BITS) };
                    self.gl_fb_bits = Some((red, alpha));
                }
            }
        }
        let tray_ver = self.tray_version.load(std::sync::atomic::Ordering::Relaxed);
        if self.last_tray_version != tray_ver {
            self.last_tray_version = tray_ver;
            // 复位延迟隐藏状态（本次隐藏命令已发出，恢复后清空以免残留）
            self.hide_requested = false;
            self.hide_sent = false;
            // 托盘态已释放 CJK 字体：恢复窗口后重新懒加载（否则中文渲染回退为方框）
            self.cjk_loading = false;
            ctx.request_repaint();
        }
        // A3 CJK 字体懒加载：启动仅 ASCII（main 不再同步加载）；
        // 托盘态早退不触发；可见/恢复后异步加载一次，完成后 set_fonts + repaint 生效。
        if !self.cjk_loading {
            self.cjk_loading = true;
            let cctx = self.egui_ctx.clone();
            std::thread::spawn(move || {
                if let Some(fonts) = load_cjk_fonts() {
                    cctx.set_fonts(fonts);
                }
                cctx.request_repaint();
            });
        }
        // 动效总开关：关闭时同时关�?egui 内置控件动画（折叠、悬浮、滚动条等）
        let target_anim = if self.cfg.ui_animations { 0.15 } else { 0.0 };
        ctx.style_mut(|s| s.animation_time = target_anim);
        // Stage 6 主题系统：目标色板 → 帧率无关插值 → 应用（深浅/预设/自定义/圆角/背景透明度）
        self.tick_theme(ctx);
        // 背景底图 / 背景材质：必须在**面板之前**绘制。egui 的 CentralPanel 内容就画在
        // background 层，而同一层内按插入顺序绘制；帧末追加底图会盖住中央内容
        // （本轮「材质盖住整个 UI、按钮全不可见」的根因，也是历史「壁纸盖住按钮」的同源问题）。
        self.paint_bg(ctx);
        // 窗口隐藏到托盘后 egui 事件循环会休眠，托盘点击/菜单事件由全局回调（set_event_handler）处理；
        // 首次 update 注册回调（此时才持有有效 ctx），注册后不依赖事件循环也能响应托盘
        if !self.tray_handlers_set {
            self.tray_handlers_set = true;
            if self.tray.is_some() {
                self.setup_tray_handlers(ctx);
            }
        }
        // 托盘事件已由 set_event_handler 全局回调 + Win32 直操作处理（显示/退出均不依赖事件循环）；
        // 隐藏期不需要 30ms 高频兜底唤醒，改为 update 末尾 1000ms 低频唤醒维持后台任务心跳（CPU 近零）
        // 托盘菜单「退出」：回调置位后窗口已恢复可见，此处消费并走退出流程
        if self.tray_quit_requested.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.request_exit(ctx);
        }
        // 网络流量采样（后台线程，2s 节流�?
        self.tick_net();
        // 关闭请求拦截（点 ×、Alt+F4、任务栏关闭均走这里）：
        // 按配置决定最小化 / 直接退�?/ 询问后静默关服退�?
        // B7 拖入文件夹自动识别服务端（1.3.7）
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            for p in dropped {
                let is_zip = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase() == "zip")
                    .unwrap_or(false);
                if is_zip && self.nav == Nav::Plugins {
                    self.import_plugin_zip(&p);
                    continue;
                }
                if p.is_dir() {
                    match detect_server_source(&p) {
                        Ok(cores) if !cores.is_empty() => {
                            self.add_server_dir(p);
                        }
                        Ok(_) => {
                            self.set_toast(format!("未在「{}」中检测到服务端核心", p.display()));
                        }
                        Err(e) => {
                            self.set_toast(format!("识别失败: {e}"));
                        }
                    }
                } else {
                    self.set_toast(format!(
                        "已接收「{}」，请拖入服务端所在文件夹以自动识别",
                        p.display()
                    ));
                }
            }
            ctx.request_repaint();
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.handle_close_request(ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        // 已确认退出且所有服务器停止完毕：主动发起关闭（触发 close_requested 后放行）
        if self.ctx_close_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // [TEST-HOOK] 自动选中第一个服务器，用于复现点击崩溃（验证后删除）
        if std::env::var_os("XMST_AUTO_SELECT").is_some()
            && self.selected_server.is_none()
            && !self.cfg.servers.is_empty()
            && self.nav == Nav::Servers
        {
            if self.test_hook_t.elapsed().as_secs_f32() > 1.5 {
                self.selected_server = Some(0);
            }
        }
        // 开机自启模式：等待系统 CPU 空闲（错峰）后逐台启动勾选了"开机自�?的服务器
        if self.autostart_mode {
            let need = self
                .cfg
                .servers
                .iter()
                .enumerate()
                .any(|(i, s)| s.autostart_enabled && !self.autostart_launched.contains(&i));
            if need {
                let cpu = perf::system_cpu_usage_pct();
                let idle_now = cpu < 30.0;
                let steady = match self.autostart_idle_since {
                    Some(t) => idle_now && t.elapsed().as_secs() >= 5,
                    None => {
                        if idle_now {
                            self.autostart_idle_since = Some(std::time::Instant::now());
                        }
                        false
                    }
                };
                if steady {
                    for i in 0..self.cfg.servers.len() {
                        if self.cfg.servers[i].autostart_enabled
                            && !self.autostart_launched.contains(&i)
                        {
                            let free = self
                                .runtimes
                                .get(i)
                                .map(|r| r.proc.is_none() && !r.stopping)
                                .unwrap_or(false);
                            if free {
                                self.start_server(i);
                                self.autostart_launched.insert(i);
                            }
                        }
                    }
                }
            }
        }
        // 每帧拉取日志 + 异常退出检�?
        let mut notify_queue: Vec<(String, String)> = Vec::new();
        // 插件事件收集（本帧待分发；BETA_PLUGINS 关闭时保持空 Vec，零开销）
        let mut plugin_evts: Vec<PluginEvt> = Vec::new();
        let plugins_active = self
            .plugins
            .as_ref()
            .map(|p| !p.plugins_dir.to_string_lossy().contains("placeholder"))
            .unwrap_or(false);
        for (idx, rt) in self.runtimes.iter_mut().enumerate() {
            let max = self.cfg.max_log_lines;
            let mut buf = std::mem::take(&mut rt.log_buf); // 移出缓冲，避免每帧大字符串 clone 造成内存峰值
            let old_len = buf.len();
            let mut added = 0usize;
            // 日志文件尾随优先：MC 服务器日志文件实时写盘，绕开 stdout 管道�?Java 缓冲的卡死问�?
            let mut tail_active = false;
            if let Some(tail) = &mut rt.log_tail {
                if let Some(n) = tail.tail_logs(&mut buf, max) {
                    added = n;
                    tail_active = tail.seen_data();
                }
            }
            if let Some(p) = &rt.proc {
                if tail_active {
                    // 文件通道已生效：丢弃 stdout 管道数据（内容重复且格式不一致）
                    process::drain_logs_discard(p);
                } else {
                    // 文件通道尚未生效（启动初�?�?MC 服务器）：stdout 管道兜底
                    added += process::drain_logs(p, &mut buf, max);
                }
                // MC 服务器启动完成标志（"Done (x.xxxs)! For help, type ..."）→ 更新就绪状态 + 通知（防重复）
                // D5：判定放宽到「Done (」+「s)」同时出现，兼容不同版本/包装端的小差异。
                if !rt.startup_notified && buf.contains("Done (") && buf.contains("s)") {
                    rt.startup_notified = true;
                    rt.last_msg = "✅ 启动完成，服务端已就绪".to_string();
                    // 成功运行：重置崩溃重启计数与熔断窗口
                    rt.crash_count = 0;
                    rt.crash_first_at = None;
                    rt.crash_restart_at = None;
                    let name = self.cfg.servers.get(idx).map(|s| s.name.clone()).unwrap_or_default();
                    notify_queue.push(("XMST - 服务器启动完成".to_string(), format!("{name} 已就绪，可以连接")));
                }
                // 就绪超时提示：长时间未出现 Done（非标准 MC 服务端或启动异常），给出可见状态
                if !rt.startup_notified
                    && !rt.startup_warned
                    && rt.started_at.map(|t| t.elapsed().as_secs() > 180).unwrap_or(false)
                {
                    rt.startup_warned = true;
                    rt.last_msg = "已运行，但未检测到服务端就绪标志（可能不是标准 MC 服务端），请查看日志确认".to_string();
                    // D5 兜底：非标准/代理端/被改过本地化的服务端可能永远不输出 "Done ("
                    // → 插件事件永远不触发。超时后仍按「已启动」补发一次 server_started。
                    if plugins_active && !rt.plugin_start_emitted {
                        rt.plugin_start_emitted = true;
                        let name = self.cfg.servers.get(idx).map(|s| s.name.clone()).unwrap_or_default();
                        plugin_evts.push(PluginEvt::ServerStarted(name));
                    }
                }
                // 非用户主动停止时进程消失：视为异常退出（崩溃/强杀/断电�?
                if !rt.stopping && !process::is_running(p) {
                    let code = process::exit_code(p)
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let name = self.cfg.servers.get(idx).map(|s| s.name.clone()).unwrap_or_default();
                    notify_queue.push(("XMST - 服务器异常退出".to_string(), format!("{name} 进程已退出（code {code}），未经过正常停止")));
                    // ★ 崩溃根因分析：从日志/crash-report 里提取可操作的原因（缺少前置、Java 版本、
                    // 内存、端口、Mixin 冲突…），随后弹出小窗提示，而不是只丢一句"异常退出"。
                    let sdir = self.cfg.servers.get(idx).map(|s| s.dir.clone());
                    if let Some(sdir) = sdir {
                        if let Some(finding) = crashscan::analyze(&sdir) {
                            let first = finding
                                .causes
                                .first()
                                .map(|c| c.title.clone())
                                .unwrap_or_else(|| "疑似启动失败".to_string());
                            notify_queue.push((
                                "XMST - 崩溃原因分析".to_string(),
                                format!("{name}：{first}（详见「崩溃分析」窗口）"),
                            ));
                            self.crash_report = Some((idx, name.clone(), finding));
                        }
                    }
                    rt.proc = None;
                    // 插件事件：异常退出视为 server_stopped("crashed")
                    if plugins_active {
                        plugin_evts.push(PluginEvt::ServerStopped(name.clone(), "crashed".to_string()));
                    }
                    // 崩溃自动重启：窗口内计数 -> 计划重启 / 熔断
                    let cr = self
                        .cfg
                        .servers
                        .get(idx)
                        .map(|s| s.crash_restart.clone())
                        .unwrap_or_default();
                    if cr.enabled {
                        let now = std::time::Instant::now();
                        let win_secs = cr.circuit_minutes.saturating_mul(60);
                        let in_window = rt
                            .crash_first_at
                            .map(|t| now.duration_since(t).as_secs() <= win_secs)
                            .unwrap_or(false);
                        if !in_window {
                            rt.crash_first_at = Some(now);
                            rt.crash_count = 0;
                        }
                        rt.crash_count += 1;
                        let n = rt.crash_count;
                        if n <= cr.max_restarts {
                            let wait = std::time::Duration::from_secs(cr.wait_secs);
                            rt.crash_restart_at = Some(now + wait);
                            rt.last_msg = format!(
                                "⚠️ 服务器进程已退出（exit code {code}），将在 {} 秒后自动重启（第 {n}/{} 次）",
                                cr.wait_secs, cr.max_restarts
                            );
                        } else {
                            rt.last_msg = format!(
                                "⚠️ 服务器进程已退出（exit code {code}）。{} 分钟内连续崩溃达到上限（{n} 次），已熔断停止自动重启；可手动启动或等待窗口重置",
                                cr.circuit_minutes
                            );
                        }
                    } else {
                        rt.last_msg = format!(
                            "⚠️ 服务器进程已退出（exit code {code}），未经过正常停止。如遇数据异常，可在「自动功能」页回滚")
                        ;
                    }
                }
            }

            // 插件事件：服务器启动完成 / 新日志行 / 玩家加入离开（BETA_PLUGINS 启用且目录已就绪才收集）
            if plugins_active {
                let srv_name = self
                    .cfg
                    .servers
                    .get(idx)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                if rt.startup_notified && !rt.plugin_start_emitted {
                    rt.plugin_start_emitted = true;
                    plugin_evts.push(PluginEvt::ServerStarted(srv_name.clone()));
                }
                if buf.len() > old_len {
                    let new_part = &buf[old_len..];
                    for line in new_part.split('\n') {
                        let line = line.trim_end();
                        if line.is_empty() {
                            continue;
                        }
                        if let Some(p) = line.find(" joined the game").and_then(|pos| Self::extract_player_name(line, pos)) {
                            plugin_evts.push(PluginEvt::PlayerJoined(srv_name.clone(), p));
                        } else if let Some(p) = line.find(" left the game").and_then(|pos| Self::extract_player_name(line, pos)) {
                            plugin_evts.push(PluginEvt::PlayerLeft(srv_name.clone(), p));
                        }
                        plugin_evts.push(PluginEvt::LogLine(srv_name.clone(), line.to_string()));
                    }
                }
            }
            // 无条件写回：mem::take 已移出缓冲，即使本帧无新增也必须保留旧内容，
            // 否则无新日志的帧会把已显示的日志清空（表现为日志闪一下消失）
            // 阶段C：日志落库 SQLite（仅新增行；行数超限由 logdb 内部轮转）
            if added > 0 {
                if let Some(db) = self.logdb.as_mut() {
                    let lines: Vec<String> = buf
                        .lines()
                        .rev()
                        .take(added)
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    let srv = self
                        .cfg
                        .servers
                        .get(idx)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    db.insert_batch(&srv, &lines);
                }
            }
            rt.log_buf = buf;
            if added > 0 {
                rt.log_pending += added;
            }
        }
        for (idx, rt) in self.tunnel_runtimes.iter_mut().enumerate() {
            if let Some(p) = &rt.proc {
                let added_t = process::drain_logs(p, &mut rt.log_buf, 2000);
                // 阶段C：隧道日志落库 SQLite（来源标记为「隧道:名称」）
                if added_t > 0 {
                    if let Some(db) = self.logdb.as_mut() {
                        let lines: Vec<String> = rt
                            .log_buf
                            .lines()
                            .rev()
                            .take(added_t)
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                        let tname = self
                            .cfg
                            .tunnels
                            .get(idx)
                            .map(|t| t.name.clone())
                            .unwrap_or_default();
                        db.insert_batch(&format!("隧道:{tname}"), &lines);
                    }
                }
                rt.log_pending += added_t;
                // 非手动停止时进程消失：视为穿透异常退�?
                if !rt.stopping && !process::is_running(p) {
                    let code = process::exit_code(p)
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let name = self.cfg.tunnels.get(idx).map(|t| t.name.clone()).unwrap_or_default();
                    notify_queue.push(("XMST - 内网穿透异常退出".to_string(), format!("{name} 进程已退出（code {code}）")));
                    rt.proc = None;
                }
            }
        }
        // 统一投递本轮收集的通知（循环结束后避免借用冲突）桌面弹窗 + 工具内通知同步
        for (title, body) in notify_queue {
            self.notify(&title, &body);
        }

        // Frp 集中管理：后台回�?+ 异常退出检�?
        self.tick_tunnels();
        self.tick_frp();
        // Rathole download result (stage 5; no-op when idle)
        self.poll_rathole_dl();
        // Remote backup upload result + retry (stage 5; no-op when idle)
        self.poll_remote_upload();

        // 自动备份检查（简化：每帧检查，间隔�?last_backup 控制�?
        self.tick_backup();
        // 自动重启调度
        self.tick_auto_restart();
        // 崩溃重启调度
        self.tick_crash_restart();
        // 优雅停止后台线程回传
        self.tick_stop();
        // 网络下载模块回传（下载完成唤醒一次 repaint；托盘态不轮询）
        self.tick_download();
        // mods 内 .jar 更新回传（Modrinth 指纹检测）
        self.tick_mod_update();
        // 创建服务器下载回传（B7）
        self.tick_create_server();
        // 插件系统：事件分发 + 消息消费（BETA_PLUGINS 关闭时零开销）
        self.emit_plugin_events(plugin_evts);
        self.tick_plugins(ctx);
        // 玩家管理轮询（B1/B5：前台 5s 轮询 + 操作后延迟刷新；BETA_PLAYERS 关闭时零开销）
        self.tick_players();
        // Spark 性能分析推进（问题2：到期自动发送 stop --save-to-file；停止后延迟刷新输出文件）
        self.tick_spark_prof();
        // 玩家快照异步回传消费（B5：请求序号不一致则丢弃过期响应；托盘态不消费）
        {
            let msgs: Vec<PlayersMsg> = {
                let mut v = Vec::new();
                while let Ok(m) = self.players_rx.try_recv() {
                    v.push(m);
                }
                v
            };
            let mut applied = false;
            for (i, seq, snap) in msgs {
                if i >= self.runtimes.len() {
                    continue;
                }
                if seq != self.runtimes[i].players_seq {
                    continue; // 过期响应，直接丢弃（防 A/B 服数据错位）
                }
                self.runtimes[i].players_snapshot = Some(snap);
                self.runtimes[i].players_snapshot_seq = seq;
                self.runtimes[i].players_state = PlayersState::Ready;
                applied = true;
            }
            if applied {
                self.egui_ctx.request_repaint();
            }
        }

        // （P0 阶段0 A1）隐藏态已在此函数最开头 early return：不渲染、不注册心跳，
        // 本块旧版"1000ms 低频心跳"随早退成为死代码，已删除；事件循环靠隐藏帧注册的
        // 超长 repaint 请求休眠，托盘恢复由全局回调 request_repaint 唤醒。

        // 顶部栏（无背景标题：XMST + 小版本号；右侧为工具内窗口按钮：最小化/最大化/关闭）
        // ★ 颜色必须跟随调色板：此前 Default 分支写死了近黑色 (22,25,32)，
        // 于是**日间模式下顶栏依旧全黑**（用户截图反馈）。现在一律取当前调色板的 panel，
        // 材质模式下取 window_fill（= 材质的近不透明版），保证与内容区同色系。
        egui::TopBottomPanel::top("top")
            .frame(
                egui::Frame::none()
                    .fill(match self.plugin_bg_style {
                        plugins::BgStyle::Default => self.theme_cur.panel,
                        _ => ctx.style().visuals.window_fill,
                    })
                    .inner_margin(egui::Margin::symmetric(12.0, 5.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // 标题栏拖拽区：仅覆盖左侧文字区域（排除右侧按钮区域 124px），
                    // 用系统拖拽（StartDrag）避免手�?OuterPosition 造成窗口乱窜
                    let bar_rect = egui::Rect::from_min_max(
                        ui.max_rect().min,
                        egui::pos2(ui.max_rect().right() - 124.0, ui.max_rect().bottom()),
                    );
                    let bar_resp =
                        ui.interact(bar_rect, ui.id().with("title_drag"), egui::Sense::drag());
                    if bar_resp.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    // 双击标题栏 = 最大化/还原（与普通 Windows 窗口一致；用户反馈"窗口无法缩放"
                    // 时往往正是窗口处于最大化状态，需要一条明显的退路）。
                    if bar_resp.double_clicked() {
                        let is_max = self.window_is_maximized();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                    }
                    // 左上角品牌区：**全部用 painter 绘制**（不用 Label/富文本）。
                    // 原因（用户反馈）：Label 是可选中文本，在标题栏左上角会吃掉鼠标按下事件
                    // —— 想拖左上角缩放时变成"框选文字"，缩放失效。绘制出来的图形没有任何
                    // 交互命中，事件全都归标题栏拖拽/边缘缩放。
                    {
                        let logo_h = 20.0f32;
                        let logo_w = 20.0f32;
                        let avail = ui.available_rect_before_wrap();
                        let logo_rect = egui::Rect::from_min_size(
                            egui::pos2(avail.left() + 2.0, avail.center().y - logo_h * 0.5),
                            egui::vec2(logo_w, logo_h),
                        );
                        let accent = self.theme_cur.accent;
                        // 圆角徽标 + 字母 X
                        ui.painter().rect_filled(
                            logo_rect,
                            egui::Rounding::same(5.0),
                            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 210),
                        );
                        ui.painter().text(
                            logo_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "X",
                            egui::FontId::proportional(13.0),
                            Color32::from_rgb(250, 250, 253),
                        );
                        // 品牌名 + 版本号（同一行绘制，不是可复制文本）
                        ui.painter().text(
                            egui::pos2(logo_rect.right() + 7.0, logo_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            "XMST",
                            egui::FontId::proportional(15.0),
                            self.theme_cur.text,
                        );
                        ui.painter().text(
                            egui::pos2(logo_rect.right() + 7.0 + 44.0, logo_rect.center().y + 1.0),
                            egui::Align2::LEFT_CENTER,
                            "v0.1alpha",
                            egui::FontId::proportional(10.0),
                            self.theme_cur.weak,
                        );
                        // 占位：把后续内容推到品牌区右侧（不产生任何可交互控件）
                        ui.add_space(logo_w + 7.0 + 44.0 + 46.0);
                    }
                    // 中区：状态区（toast 优先；否则显示当前服务器/就绪），预留右侧按钮区 132px
                    let mid_w = ui.available_width() - 132.0;
                    if mid_w > 40.0 {
                        ui.allocate_ui(egui::vec2(mid_w, ui.available_height()), |ui| {
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add_space(4.0);
                                    if !self.toast.is_empty() {
                                        // truncate：toast 过长时截断加省略号，避免溢出覆盖右侧窗口按钮
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&self.toast)
                                                    .size(12.0)
                                                    .color(Color32::from_rgb(255, 200, 80)),
                                            )
                                            .truncate(),
                                        );
                                    } else {
                                        let cur = self
                                            .selected_server
                                            .and_then(|idx| self.cfg.servers.get(idx))
                                            .map(|s| s.name.clone())
                                            .unwrap_or_else(|| "就绪".to_string());
                                        ui.label(
                                            RichText::new("●")
                                                .size(10.0)
                                                .color(Color32::from_rgb(150, 160, 175)),
                                        );
                                        ui.add_space(5.0);
                                        // truncate：服务器名过长时同样截断，防止溢出覆盖右侧窗口按钮
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(cur)
                                                    .size(12.0)
                                                    .color(Color32::from_rgb(170, 170, 178)),
                                            )
                                            .truncate(),
                                        );
                                    }
                                },
                            );
                        });
                    }
                    // 右侧：窗口按钮（最小化 / 最大化 / 关闭�?
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 关闭（发送关闭请求，走关闭行为拦截逻辑�?
                        let close_btn = titlebar_button(
                            ui,
                            "title_close",
                            Color32::from_rgba_unmultiplied(200, 60, 60, 170),
                            |p, rect| {
                                let m = rect.center();
                                p.line_segment(
                                    [
                                        egui::pos2(m.x - 5.0, m.y - 5.0),
                                        egui::pos2(m.x + 5.0, m.y + 5.0),
                                    ],
                                    egui::Stroke::new(1.8, self.theme_cur.text),
                                );
                                p.line_segment(
                                    [
                                        egui::pos2(m.x + 5.0, m.y - 5.0),
                                        egui::pos2(m.x - 5.0, m.y + 5.0),
                                    ],
                                    egui::Stroke::new(1.8, self.theme_cur.text),
                                );
                            },
                        );
                        if close_btn.clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        // 最大化 / 还原
                        let is_max = ctx.input(|i| i.viewport().maximized == Some(true));
                        let max_btn = titlebar_button(
                            ui,
                            "title_max",
                            Color32::from_rgba_unmultiplied(self.theme_target.accent.r(), self.theme_target.accent.g(), self.theme_target.accent.b(), 60),
                            |p, rect| {
                                let r = egui::Rect::from_min_max(
                                    egui::pos2(rect.left() + 8.0, rect.top() + 8.0),
                                    egui::pos2(rect.right() - 8.0, rect.bottom() - 8.0),
                                );
                                p.rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(1.6, self.theme_cur.text),
                                );
                            },
                        );
                        if max_btn.clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                        }
                        // 最小化
                        let min_btn = titlebar_button(
                            ui,
                            "title_min",
                            Color32::from_rgba_unmultiplied(self.theme_target.accent.r(), self.theme_target.accent.g(), self.theme_target.accent.b(), 60),
                            |p, rect| {
                                p.line_segment(
                                    [
                                        egui::pos2(rect.left() + 8.0, rect.center().y),
                                        egui::pos2(rect.right() - 8.0, rect.center().y),
                                    ],
                                    egui::Stroke::new(1.6, self.theme_cur.text),
                                );
                            },
                        );
                        if min_btn.clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        // 主题切换（日/夜/自定义三态轮转）。
                        // 图标重画：太阳=实心圆+八条短光线（更细更匀），月亮=真·新月（外圆内切偏移圆的多边形），
                        // 不再用"亮圆 + 深色圆覆盖"（在浅色顶栏上会露出一个突兀的深色圆斑）。
                        // 切换动效：日/夜两枚图标按 animate_bool 交叉淡入淡出，并带一点旋转与缩放。
                        let is_light_theme = self.cfg.theme_mode == "light";
                        let is_custom_theme = self.cfg.theme_mode == "custom";
                        let dark_t = ctx.animate_bool_with_time(
                            ui.id().with("theme_icon_dark"),
                            !is_light_theme && !is_custom_theme,
                            if self.cfg.ui_animations { 0.25 } else { 0.0 },
                        );
                        let theme_btn = titlebar_button(
                            ui,
                            "title_theme",
                            Color32::from_rgba_unmultiplied(self.theme_target.accent.r(), self.theme_target.accent.g(), self.theme_target.accent.b(), 60),
                            |p, rect| {
                                let m = rect.center();
                                let ic = self.theme_cur.text;
                                if is_custom_theme {
                                    // 调色板：三个彩色圆点
                                    p.circle_filled(m + egui::vec2(-5.0, 2.6), 2.6, Color32::from_rgb(28, 150, 130));
                                    p.circle_filled(m, 2.6, Color32::from_rgb(235, 120, 60));
                                    p.circle_filled(m + egui::vec2(5.0, 2.6), 2.6, Color32::from_rgb(90, 140, 235));
                                } else {
                                    // 交叉淡入淡出：dark_t=0 → 太阳；dark_t=1 → 月亮
                                    let sun_a = (1.0 - dark_t).clamp(0.0, 1.0);
                                    let moon_a = dark_t.clamp(0.0, 1.0);
                                    if sun_a > 0.01 {
                                        let col = ic.gamma_multiply(sun_a);
                                        let scale = 0.82 + 0.18 * sun_a;
                                        // 参考 DeepSeek 的线性图标风格：**描边**圆 + 8 条细光线（不填充）
                                        p.circle_stroke(
                                            m,
                                            3.3 * scale,
                                            egui::Stroke::new(1.4, col),
                                        );
                                        for k in 0..8 {
                                            let ang = k as f32 * std::f32::consts::TAU / 8.0
                                                + (1.0 - sun_a) * 0.6;
                                            let d = egui::Vec2::angled(ang) * (6.2 * scale);
                                            p.line_segment(
                                                [m + d, m + d * 1.30],
                                                egui::Stroke::new(1.4, col),
                                            );
                                        }
                                    }
                                    if moon_a > 0.01 {
                                        let col = ic.gamma_multiply(moon_a);
                                        // 新月：外圆 + 内切偏移圆构成的多边形，**只描边不填充**
                                        // （DeepSeek 那枚就是细线月牙；实心月牙在小尺寸下显得很怪）
                                        let r = 5.2f32;
                                        let dx = 2.9f32;
                                        let dy = -1.1f32;
                                        let mut pts: Vec<egui::Pos2> = Vec::with_capacity(48);
                                        for i in 0..=24 {
                                            let a = -std::f32::consts::FRAC_PI_2
                                                + std::f32::consts::PI * (i as f32 / 24.0);
                                            pts.push(m + egui::vec2(a.cos() * r, a.sin() * r));
                                        }
                                        for i in 0..=24 {
                                            let a = std::f32::consts::FRAC_PI_2
                                                - std::f32::consts::PI * (i as f32 / 24.0);
                                            pts.push(
                                                m + egui::vec2(
                                                    dx + a.cos() * r * 0.88,
                                                    dy + a.sin() * r * 0.88,
                                                ),
                                            );
                                        }
                                        p.add(egui::Shape::closed_line(
                                            pts,
                                            egui::Stroke::new(1.4, col),
                                        ));
                                    }
                                }
                            },
                        );
                        if theme_btn.clicked() {
                            self.cfg.theme_mode = match self.cfg.theme_mode.as_str() {
                                "light" => "dark".to_string(),
                                "dark" => "custom".to_string(),
                                _ => "light".to_string(),
                            };
                            self.save_config();
                        }
                    });
                });
                // 顶部栏底�?1px 分隔线，与内容区区分
                let top_rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(top_rect.left(), top_rect.bottom()),
                        egui::pos2(top_rect.right(), top_rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, self.theme_cur.stroke),
                );
            });

        // 左侧导航（带平滑滑块动画，可在设置中关闭；支持折叠成纯图标）
        // 折叠/展开宽度做**动画插值**（此前是硬跳变，用户反馈"没有平滑效果"）。
        let target_w = if self.nav_collapsed { 56.0 } else { 170.0 };
        if self.cfg.ui_animations {
            let sp = (0.25 * self.cfg.anim_speed.clamp(0.1, 2.0)).clamp(0.06, 0.6);
            self.nav_w_anim += (target_w - self.nav_w_anim) * sp;
            if (self.nav_w_anim - target_w).abs() < 0.4 {
                self.nav_w_anim = target_w;
            } else {
                ctx.request_repaint();
            }
        } else {
            self.nav_w_anim = target_w;
        }
        let nav_w = self.nav_w_anim;
        // t: 1.0 = 完全展开，0.0 = 完全折叠（文字/标题按它淡出）
        let t = ((nav_w - 56.0) / (170.0 - 56.0)).clamp(0.0, 1.0);
        egui::SidePanel::left("nav")
            .resizable(false)
            .default_width(nav_w)
            .exact_width(nav_w)
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(ctx.style().visuals.window_fill))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                // Stage 6：SeaLantern 风格导航分组（管理 / 系统），条件项并入对应分组。
                // Bug7：顶部栏已绘制「XMST + v0.1alpha」，此处不再重复绘制标题。
                ui.separator();
                let mut nav_items: Vec<(&str, &str, Nav)> = vec![];
                // 分组一：管理
                nav_items.push(("📊", "仪表盘", Nav::Dashboard));
                nav_items.push(("🗂", "服务器", Nav::Servers));
                if features::is_enabled(&self.cfg.features, features::BETA_DOWNLOAD) {
                    nav_items.push(("⬇️", "下载", Nav::Download));
                }
                if features::is_enabled(&self.cfg.features, features::BETA_PLUGINS) {
                    nav_items.push(("🧩", "插件", Nav::Plugins));
                }
                // 分组二：系统（内网穿透/设置 永远可用）
                let sys_group_start = nav_items.len();
                // 工具自身日志（独立页：日志库浏览 + 相关设置，从设置里搬出来）
                nav_items.push(("📜", "日志", Nav::Logs));
                nav_items.push(("🔗", "内网穿透", Nav::Tunnel));
                nav_items.push(("⚙️", "设置", Nav::Settings));
                let group_headers: std::collections::HashMap<usize, &str> =
                    [(0usize, "管理"), (sys_group_start, "系统")]
                        .into_iter()
                        .filter(|(i, _)| *i < nav_items.len())
                        .collect();
                let target_idx = nav_items
                    .iter()
                    .position(|(_, _, v)| *v == self.nav)
                    .unwrap_or(0) as f32;
                if self.cfg.ui_animations {
                    self.nav_anim += (target_idx - self.nav_anim) * 0.22;
                    if (self.nav_anim - target_idx).abs() < 0.02 {
                        self.nav_anim = target_idx;
                    }
                } else {
                    self.nav_anim = target_idx;
                }
                let item_h = 30.0f32;
                // 先渲染按钮拿到实际矩形，再按 nav_anim 在按钮间插值绘制滑块（严格对齐，不偏移�?
                // 图标与文字分离绘制：图标占固定 30px 区域，文字起点固定，
                // 避免不同 emoji 字形宽度差异导致文字右偏/参差
                let mut nav_rects = vec![egui::Rect::NOTHING; nav_items.len()];
                let mut clicked_nav: Option<Nav> = None;
                for (i, (_emoji, text, val)) in nav_items.iter().enumerate() {
                    // Stage 6：分组小标题（不参与按钮与滑块）；折叠时随 t 淡出
                    if let Some(h) = group_headers.get(&i) {
                        if t > 0.25 {
                            ui.add_space(6.0 * t);
                            ui.label(
                                RichText::new(*h)
                                    .small()
                                    .weak()
                                    .color(self.theme_cur.weak.gamma_multiply(t)),
                            );
                            ui.add_space(2.0);
                        }
                    }
                    let active = *val == self.nav;
                    // 颜色一律取自**当前调色板**（已含材质自动对比度的结果），
                    // 不再用写死的浅灰/浅青 —— 否则亮色（含明亮材质导致的浅色界面）
                    // 下会出现「仪表盘等文字不跟随变色、且对比度不足」。
                    let color = if active {
                        let a = self.theme_cur.accent;
                        self.fg(Color32::from_rgb(a.r(), a.g(), a.b()))
                    } else {
                        self.theme_cur.weak
                    };
                    // 透明整行按钮：负责点击与滑块定位
                    let btn = egui::Button::new("")
                        .frame(false)
                        .min_size(egui::vec2(ui.available_width(), item_h));
                    let resp = ui.add(btn);
                    if !active {
                        // 悬停底色做平滑过渡（此前是"一碰就亮/一走就灭"的硬切换）
                        let hover_t = ui.ctx().animate_bool_with_time(
                            resp.id.with("nav_hover"),
                            resp.hovered(),
                            if self.cfg.ui_animations { 0.12 } else { 0.0 },
                        );
                        if hover_t > 0.01 {
                            ui.painter().rect_filled(
                                resp.rect,
                                6.0,
                                Color32::from_rgba_unmultiplied(
                                    self.theme_target.accent.r(),
                                    self.theme_target.accent.g(),
                                    self.theme_target.accent.b(),
                                    (18.0 * hover_t) as u8,
                                ),
                            );
                        }
                    }
                    nav_rects[i] = resp.rect;
                    if resp.clicked() {
                        clicked_nav = Some(*val);
                    }
                    // 图标固定画在 30px 宽**单元格的中心**（矢量图元，见 nav_icon）。
                    // 折叠/展开过程中图标中心在「面板中心 ↔ 单元格中心」之间插值。
                    let expanded_cx = resp.rect.min.x + 23.0;
                    let cell_cx = expanded_cx * t + resp.rect.center().x * (1.0 - t);
                    self.nav_icon(
                        ui.painter(),
                        egui::pos2(cell_cx, resp.rect.center().y),
                        *val,
                        color,
                    );
                    if t > 0.2 {
                        ui.painter().text(
                            egui::pos2(resp.rect.min.x + 44.0, resp.rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            *text,
                            egui::FontId::proportional(14.0),
                            color.gamma_multiply(t),
                        );
                    }
                    ui.add_space(2.0);
                }
                if let Some(v) = clicked_nav {
                    self.nav = v;
                }
                // 滑块沿实际按钮位置插值（选中高亮背景：强调色半透明，保留并叠加竖条）
                let i0 = self.nav_anim.floor() as usize;
                let i1 = (self.nav_anim.ceil() as usize).min(nav_items.len() - 1);
                let t = self.nav_anim - i0 as f32;
                let r0 = nav_rects[i0];
                let r1 = nav_rects[i1];
                let slider_rect = egui::Rect::from_min_max(
                    egui::pos2(r0.min.x + 3.0, r0.min.y + 3.0 + (r1.min.y - r0.min.y) * t),
                    egui::pos2(r0.max.x - 3.0, r0.max.y - 3.0 + (r1.max.y - r0.max.y) * t),
                );
                ui.painter()
                    .rect_filled(slider_rect, 6.0, Color32::from_rgba_unmultiplied(
                        self.theme_target.accent.r(),
                        self.theme_target.accent.g(),
                        self.theme_target.accent.b(),
                        30,
                    ));
                // 选中态：左侧 3px 圆角竖条（当前强调色），沿 nav_anim 插值；
                // 随折叠动画淡出（完全折叠时不画：贴边竖条会让居中图标产生"偏左"错觉）
                if t > 0.02 {
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(r0.min.x + 1.0, r0.min.y + 3.0 + (r1.min.y - r0.min.y) * t),
                        egui::pos2(r0.min.x + 4.0, r0.max.y - 3.0 + (r1.max.y - r0.max.y) * t),
                    );
                    ui.painter().rect_filled(
                        bar_rect,
                        1.5,
                        self.theme_target.accent.gamma_multiply(t),
                    );
                }
                ui.separator();
                // 折叠/展开按钮固定在左下角（用 <> 箭头表示），折叠后侧栏只剩图标。
                // 采用 bottom_up 布局把它压在面板底部，不随导航项数量浮动。
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(6.0);
                    let arrow = if self.nav_collapsed { "▶" } else { "◀" };
                    let hint = if self.nav_collapsed {
                        "展开侧栏"
                    } else {
                        "折叠侧栏（只显示图标）"
                    };
                    let btn = egui::Button::new(
                        RichText::new(arrow)
                            .size(14.0)
                            .color(self.theme_cur.weak),
                    )
                    .frame(false);
                    if ui.add(btn).on_hover_text(hint).clicked() {
                        self.nav_collapsed = !self.nav_collapsed;
                        self.cfg.nav_collapsed = self.nav_collapsed;
                        self.save_config();
                    }
                    ui.separator();
                    if !self.nav_collapsed {
                        if ui.button("💾 保存配置").clicked() {
                            self.save_config();
                            self.set_toast("配置已保存".to_string());
                        }
                    }
                });
            });

        match self.nav {
            Nav::Dashboard => self.ui_dashboard(ctx),
            Nav::Servers => self.ui_servers(ctx),
            Nav::Tunnel => self.ui_tunnel(ctx),
            Nav::Logs => self.ui_logs_page(ctx),
            Nav::Settings => self.ui_settings(ctx),
            Nav::Download => self.ui_download(ctx),
            Nav::Plugins => self.ui_plugins(ctx),
        }

        // B3 强停二次确认弹窗（全局，不依赖当前页签）
        if features::is_enabled(&self.cfg.features, features::FEATURE_FORCE_STOP_CONFIRM) {
            self.ui_force_stop_confirm(ctx);
        }

        // 回退确认弹窗
        if self.confirm_restore.is_some() {
            let mut do_it = false;
            egui::Window::new("确认回退")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    if let Some((_, _, name)) = &self.confirm_restore {
                        ui.label(format!("将从备份 [{name}] 恢复 world/config"));
                        ui.label(RichText::new("服务器将被停止，当前 world/config 会移动到 .mcsrv_trash/）").color(Color32::YELLOW));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("确认回退").clicked() {
                                do_it = true;
                            }
                            if ui.button("取消").clicked() {
                                self.confirm_restore = None;
                            }
                        });
                    }
                });
            if do_it {
                self.do_restore();
            }
        }

        // 删除备份确认弹窗
        if self.confirm_delete_backup.is_some() {
            let mut do_it = false;
            egui::Window::new("确认删除备份")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    if let Some((_, _, name)) = &self.confirm_delete_backup {
                        ui.label(format!("确定删除备份 [{name}] 吗？"));
                        ui.label(RichText::new("该操作不可撤销。若删除的是全量备份，下次备份会自动补做全量）").color(Color32::YELLOW));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("确认删除").clicked() {
                                do_it = true;
                            }
                            if ui.button("取消").clicked() {
                                self.confirm_delete_backup = None;
                            }
                        });
                    }
                });
            if do_it {
                self.do_delete_backup();
            }
        }

        // 关闭确认弹窗（点 × 且配置为彻底关闭、且有服务器在运行时�?
        if self.confirm_close.is_some() {
            let n = self.confirm_close.unwrap_or(0);
            egui::Window::new("确认关闭")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("当前有 {n} 台服务器正在运行"));
                    ui.label("选择“是”将先静默关闭所有服务器，全部停止后再退出工具");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("是，关闭服务器并退出").clicked() {
                            let running: Vec<usize> = self
                                .runtimes
                                .iter()
                                .enumerate()
                                .filter(|(_, rt)| rt.proc.is_some() && process::is_running(rt.proc.as_ref().unwrap()))
                                .map(|(i, _)| i)
                                .collect();
                            self.confirm_close = None;
                            self.closing_exit = true;
                            for i in running {
                                self.stop_server(i);
                            }
                            // 先最小化，等待后台静默关服完成后自动退�?
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        if ui.button("否，仅最小化").clicked() {
                            self.confirm_close = None;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        if ui.button("取消").clicked() {
                            self.confirm_close = None;
                        }
                    });
                });
        }

        // 重命名服务器弹窗
        if self.rename_server.is_some() {
            let mut do_it = false;
            let mut cancel = false;
            egui::Window::new("重命名服务器")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    // 直接编辑持久字段，避免每帧从旧名重建导致输入丢失
                    let Some((_, name)) = self.rename_server.as_mut() else { return };
                    ui.label("服务器显示名称：");
                    ui.text_edit_singleline(name);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("保存").clicked() {
                            do_it = true;
                        }
                        if ui.button("取消").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_it {
                self.do_rename();
            }
            if cancel {
                self.rename_server = None;
            }
        }

        // 创建服务器弹窗（B7）
        self.ui_create_server(ctx);

        // frpc 更新确认弹窗（检查更新发现新版本时）
        if let Some((local, remote)) = self.frp_update_pending.clone() {
            let mut do_it = false;
            let mut cancel = false;
            egui::Window::new("发现 frpc 新版本")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "本地版本 {local} → 最新版本 {remote}"
                    ));
                    ui.label(
                        RichText::new("将下载最新版并覆盖现有 frpc.exe")
                            .color(Color32::YELLOW),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("立即更新").clicked() {
                            do_it = true;
                        }
                        if ui.button("取消").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_it {
                self.frp_update_pending = None;
                self.apply_frp_update();
            }
            if cancel {
                self.frp_update_pending = None;
            }
        }

        // 性能采样：Servers(状态/概览) 采选中服务器；Dashboard 采全部在线服务器；2 Hz 限频
        if self.last_perf_sample.elapsed().as_millis() >= 500 {
            self.last_perf_sample = std::time::Instant::now();
            let need_perf_sel = matches!(self.nav, Nav::Servers)
                && (matches!(self.server_tab, ServerTab::Status)
                    || matches!(self.server_tab, ServerTab::Overview));
            let on_dash = matches!(self.nav, Nav::Dashboard);
            let idxs: Vec<usize> = if on_dash {
                (0..self.cfg.servers.len())
                    .filter(|&i| {
                        self.runtimes
                            .get(i)
                            .and_then(|rt| rt.proc.as_ref())
                            .map(|p| process::is_running(p))
                            .unwrap_or(false)
                    })
                    .collect()
            } else {
                self.selected_server.into_iter().collect()
            };
            for idx in idxs {
                if let Some(rt) = self.runtimes.get_mut(idx) {
                    if need_perf_sel || on_dash {
                        if let Some(p) = &rt.proc {
                            if let Some(pid) = process::pid(p) {
                                rt.perf.sample(pid);
                            } else {
                                rt.perf.reset();
                            }
                        } else {
                            rt.perf.reset();
                        }
                    } else {
                        rt.perf.reset();
                    }
                }
            }
        }

        // 服务器目录占用大小（后台线程每 5 秒刷新，避免遍历大 world 卡 UI）
        // Servers 刷选中服务器；Dashboard 刷全部在线服务器
        let dir_idxs: Vec<usize> = if self.nav == Nav::Dashboard {
            (0..self.cfg.servers.len())
                .filter(|&i| {
                    self.runtimes
                        .get(i)
                        .and_then(|rt| rt.proc.as_ref())
                        .map(|p| process::is_running(p))
                        .unwrap_or(false)
                })
                .collect()
        } else {
            self.selected_server.into_iter().collect()
        };
        for idx in dir_idxs {
            let stale = self
                .runtimes
                .get(idx)
                .map(|rt| {
                    rt.dir_size_at
                        .map(|t| t.elapsed().as_secs() >= 5)
                        .unwrap_or(true)
                })
                .unwrap_or(false);
            if stale {
                if let Some(rt) = self.runtimes.get_mut(idx) {
                    rt.dir_size_at = Some(std::time::Instant::now());
                }
                if let Some(target) = self.cfg.servers.get(idx).map(|s| s.dir.clone()) {
                    let arc = self.runtimes[idx].dir_size.clone();
                    std::thread::spawn(move || {
                        let bytes = dir_size_bytes(&target);
                        if let Ok(mut g) = arc.lock() {
                            *g = bytes;
                        }
                    });
                }
            }
        }

        // 动态重绘：动画收敛期间 60 FPS，空闲时降到 5 FPS（后台最小消耗）
        let target_nav = [Nav::Dashboard, Nav::Servers, Nav::Tunnel, Nav::Settings]
            .iter()
            .position(|v| *v == self.nav)
            .unwrap_or(0) as f32;
        let target_tab = [
            ServerTab::Overview,
            ServerTab::Scripts,
            ServerTab::Files,
            ServerTab::Backup,
            ServerTab::Perf,
            ServerTab::Status,
        ]
        .iter()
        .position(|v| *v == self.server_tab)
        .unwrap_or(0) as f32;
        let animating = self.cfg.ui_animations
            && ((self.nav_anim - target_nav).abs() > 0.02
                || (self.tab_anim - target_tab).abs() > 0.02);
        // 刷新率：动画 16ms；托盘态不注册任何心跳（A1 零重绘：update 早退 + 延迟隐藏帧把
        // ControlFlow 拉回 Wait 后事件循环长眠、CPU≈0，托盘恢复由回调 request_repaint 立即唤醒；
        // 注意不能在托盘态注册超长心跳——该请求到期时窗口已隐藏，request_redraw 无效会使
        // ControlFlow 滞留 Poll 满转）；其余 200ms
        if !self.tray_hidden.load(std::sync::atomic::Ordering::Relaxed) {
            let period_ms = if animating { 16 } else { 200 };
            ctx.request_repaint_after(std::time::Duration::from_millis(period_ms));
        }
        // Stage 6：背景编辑弹窗（ESC 退出）
        // 注意：背景底图/材质必须在**画任何面板之前**绘制（见 update 开头 tick_theme 之后的
        // paint_bg 调用）——egui 的 CentralPanel 内容本身就画在 background 层，若在帧末再往
        // 该层追加底图，底图会盖在中央内容之上（历史上「壁纸盖住按钮」同源）。
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.bg_edit_open = false;
        }
        self.ui_bg_editor(ctx);
        // 崩溃分析小窗（服务端异常退出后自动出现）
        self.ui_crash_report(ctx);
    }
}

impl App {
    fn ui_dashboard(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // 第二批：全页面 max-width 1100 左对齐（字号小时内容贴合左侧）
            let _avail = ui.available_rect_before_wrap();
            let _w = _avail.width().min(1100.0);
            let _centered = egui::Rect::from_min_size(
                egui::pos2(_avail.left(), _avail.top()),
                egui::vec2(_w, _avail.height()),
            );
            let mut _inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(_centered)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .id_salt("page_center_1100"),
            );
            _inner.set_width(_w);
            let ui = &mut _inner;
            ui.add_space(8.0);
            ui.label(RichText::new("📊 仪表盘").size(20.0).strong());
            ui.separator();
            let total_start: u64 = self.cfg.servers.iter().map(|s| s.start_count).sum();
            let total_dl: u64 = self.cfg.servers.iter().map(|s| s.download_count).sum();
            let first_dl: Option<String> = self
                .cfg
                .servers
                .iter()
                .filter_map(|s| s.first_download_at.clone())
                .min();
            // 在线服务器列表（含玩家/资源/存储）
            let mut online: Vec<(String, usize, Option<(f32, f32)>, u64)> = vec![];
            for (i, s) in self.cfg.servers.iter().enumerate() {
                let running = self
                    .runtimes
                    .get(i)
                    .and_then(|rt| rt.proc.as_ref())
                    .map(|p| process::is_running(p))
                    .unwrap_or(false);
                if running {
                    if let Some(rt) = self.runtimes.get(i) {
                        let perf = rt.perf.last();
                        let ds = rt.dir_size.lock().map(|g| *g).unwrap_or(0);
                        online.push((s.name.clone(), rt.players_online.len(), perf, ds));
                    }
                }
            }
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(120, 160, 255, 14))
                .stroke(egui::Stroke::new(
                    1.0,
                    Color32::from_rgba_unmultiplied(120, 160, 255, 60),
                ))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.label(RichText::new("累计统计").strong());
                    ui.separator();
                    egui::Grid::new("dash_global")
                        .num_columns(4)
                        .spacing([24.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("🚀 启动服务器次数");
                            ui.label(total_start.to_string());
                            ui.label("⬇️ 下载次数");
                            ui.label(total_dl.to_string());
                            ui.end_row();
                            ui.label("🕒 首次下载");
                            ui.label(
                                first_dl.unwrap_or_else(|| "从未下载".to_string()),
                            );
                            ui.label("🟢 在线服务器数");
                            ui.label(online.len().to_string());
                            ui.end_row();
                        });
                });
            ui.add_space(10.0);
            ui.label(RichText::new("在线服务器运行状况").size(15.0).strong());
            ui.separator();
            if online.is_empty() {
                ui.add_space(8.0);
                ui.centered_and_justified(|ui| {
                    ui.label("当前没有在线服务器");
                });
            } else {
                egui::Grid::new("dash_online")
                    .num_columns(5)
                    .spacing([28.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("服务器").strong());
                        ui.label(RichText::new("在线玩家").strong());
                        ui.label(RichText::new("CPU · 内存").strong());
                        ui.label(RichText::new("存储占用").strong());
                        ui.end_row();
                        for (name, players, perf, ds) in online {
                            ui.label(name);
                            ui.label(players.to_string());
                            match perf {
                                Some((cpu, mem)) => {
                                    ui.label(format!("{cpu:.0}% · {mem:.0} MB"));
                                }
                                None => {
                                    ui.label("—");
                                }
                            }
                            ui.label(fmt_bytes(ds));
                            ui.end_row();
                        }
                    });
            }
        });
    }

    fn ui_servers(&mut self, ctx: &egui::Context) {
        // 侧栏折叠动画：0=折叠(仅图标)，1=展开
        let target = if self.servers_collapsed { 0.0 } else { 1.0 };
        if self.cfg.ui_animations {
            self.servers_anim += (target - self.servers_anim) * 0.18;
            if (self.servers_anim - target).abs() < 0.02 {
                self.servers_anim = target;
            }
        } else {
            self.servers_anim = target;
        }
        let t = self.servers_anim;
        let expanded = t > 0.6;
        let icon_only = !expanded;
        let anim_w = 44.0 + (240.0 - 44.0) * t;

        // 收藏的服务器置顶（未收藏保持原有顺序）
        let mut order: Vec<usize> = (0..self.cfg.servers.len()).collect();
        order.sort_by_key(|&i| !self.cfg.servers[i].favorited);

        let mut to_remove: Option<usize> = None;
        let mut to_rename: Option<(usize, String)> = None;
        let mut to_backup_now: Option<usize> = None;
        let mut toggle_fav: Option<usize> = None;
        let mut switch_server: Option<usize> = None;
        let mut want_toggle = false;

        let mut panel = egui::SidePanel::left("server_list").frame(
            egui::Frame::side_top_panel(&ctx.style()).fill(ctx.style().visuals.window_fill),
        );
        panel = if expanded {
            // 展开完成：恢复可拖拽调宽
            panel
                .resizable(true)
                .default_width(240.0)
                .width_range(44.0..=480.0)
        } else {
            // 折叠 / 动画过渡：强制当前动画宽度
            panel.exact_width(anim_w)
        };
        panel.show(ctx, |ui| {
            if !icon_only {
                // ================= 展开态：标题 + 列表 =================
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new("服务器列表").strong());
                        ui.separator();
                        for &i in &order {
                            let sc = &self.cfg.servers[i];
                            let name = if sc.name.is_empty() {
                                "未命名"
                            } else {
                                &sc.name
                            };
                            let running = self
                                .runtimes
                                .get(i)
                                .and_then(|r| r.proc.as_ref())
                                .map(|p| process::is_running(p))
                                .unwrap_or(false);
                            let label = if running {
                                format!("📁 {name}")
                            } else {
                                name.to_string()
                            };
                            ui.horizontal(|ui| {
                                let star = if sc.favorited { "★" } else { "☆" };
                                if ui
                                    .add(egui::Button::new(star).frame(false).small())
                                    .on_hover_text("收藏 / 取消收藏（收藏的服务器置顶）")
                                    .clicked()
                                {
                                    toggle_fav = Some(i);
                                }
                                let resp = ui.selectable_label(self.selected_server == Some(i), &label);
                                if resp.clicked() {
                                    switch_server = Some(i);
                                }
                                resp.context_menu(|ui| {
                                    if ui.button("✏️ 重命名").clicked() {
                                        to_rename = Some((i, sc.name.clone()));
                                        ui.close_menu();
                                    }
                                    if features::is_enabled(&self.cfg.features, features::BETA_BACKUP)
                                        && ui.button("📦 立即备份").clicked()
                                    {
                                        to_backup_now = Some(i);
                                        ui.close_menu();
                                    }
                                    if ui.button("🗑 移除服务器（配置删除，文件不动）").clicked() {
                                        to_remove = Some(i);
                                        ui.close_menu();
                                    }
                                });
                            });
                        }
                        ui.separator();
                        ui.vertical(|ui| {
                            if ui.button("➕ 创建新服务器").clicked() {
                                self.create_server = Some(CreateServerState::new());
                            }
                            if ui.button("➕ 添加服务器目录").clicked() {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.add_server_dir(dir);
                                }
                            }
                        });
                    });
                ui.separator();
                if ui.button("◀ 收起侧栏").on_hover_text("折叠为仅图标").clicked() {
                    want_toggle = true;
                }
            } else {
                // ================= 折叠态：仅图标 =================
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for &i in &order {
                            let sc = &self.cfg.servers[i];
                            let name = if sc.name.is_empty() {
                                "未命名".to_string()
                            } else {
                                sc.name.clone()
                            };
                            let running = self
                                .runtimes
                                .get(i)
                                .and_then(|r| r.proc.as_ref())
                                .map(|p| process::is_running(p))
                                .unwrap_or(false);
                            let ch = name.chars().next().unwrap_or('?');
                            // 折叠态行：整行透明按钮承载点击/右键，星标 + 首字符图标 + 状态点
                            // 按固定坐标绘制，不随星标字形宽度偏移（☆ 与 ★ 宽度不同会造成图标列左右错位）
                            let (row_rect, row_resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 26.0),
                                egui::Sense::click(),
                            );
                            let cy = row_rect.center().y;
                            let icon_x = row_rect.center().x;
                            let star_x = row_rect.min.x + 6.0;
                            let star = if sc.favorited { "★" } else { "☆" };
                            let star_color = if sc.favorited {
                                self.fg(Color32::from_rgb(230, 180, 60))
                            } else {
                                self.theme_cur.weak
                            };
                            ui.painter().text(
                                egui::pos2(star_x, cy),
                                egui::Align2::CENTER_CENTER,
                                star,
                                egui::FontId::proportional(12.0),
                                star_color,
                            );
                            ui.painter().text(
                                egui::pos2(icon_x, cy),
                                egui::Align2::CENTER_CENTER,
                                ch.to_string(),
                                egui::FontId::proportional(15.0),
                                self.fg(Color32::from_rgb(235, 235, 240)),
                            );
                            // 运行状态点：图标右上角
                            let dot = if running {
                                self.fg(Color32::from_rgb(90, 200, 110))
                            } else {
                                Color32::from_gray(90)
                            };
                            ui.painter().circle_filled(
                                egui::pos2(icon_x + 9.0, row_rect.top() + 3.0),
                                3.0,
                                dot,
                            );
                            if row_resp.hovered() {
                                ui.painter().rect_filled(
                                    row_rect,
                                    6.0,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 10),
                                );
                            }
                            if row_resp.clicked() {
                                // 按点击横坐标区分：左 14px 为星标区（收藏切换），其余为选中服务器
                                let px = ui.ctx().pointer_interact_pos().map(|p| p.x);
                                if let Some(px) = px {
                                    if px < row_rect.min.x + 14.0 {
                                        toggle_fav = Some(i);
                                    } else {
                                        switch_server = Some(i);
                                    }
                                }
                            }
                            let row_resp = row_resp.on_hover_text(name.clone());
                            row_resp.context_menu(|ui| {
                                    if ui.button("✏️ 重命名").clicked() {
                                        to_rename = Some((i, sc.name.clone()));
                                        ui.close_menu();
                                    }
                                    if features::is_enabled(&self.cfg.features, features::BETA_BACKUP)
                                        && ui.button("📦 立即备份").clicked()
                                    {
                                        to_backup_now = Some(i);
                                        ui.close_menu();
                                    }
                                    if ui.button("🗑 移除服务器（配置删除，文件不动）").clicked() {
                                        to_remove = Some(i);
                                        ui.close_menu();
                                    }
                                });
                        }
                        ui.separator();
                        ui.horizontal_centered(|ui| {
                            if ui
                                .add(egui::Button::new("＋").frame(false).small())
                                .on_hover_text("创建新服务器")
                                .clicked()
                            {
                                self.create_server = Some(CreateServerState::new());
                            }
                            if ui
                                .add(egui::Button::new("📂").frame(false).small())
                                .on_hover_text("添加服务器目录")
                                .clicked()
                            {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.add_server_dir(dir);
                                }
                            }
                        });
                    });
                ui.separator();
                ui.vertical_centered(|ui| {
                    if ui.button("▶").on_hover_text("展开侧栏").clicked() {
                        want_toggle = true;
                    }
                });
            }
        });

        if want_toggle {
            self.servers_collapsed = !self.servers_collapsed;
        }
        if let Some(i) = switch_server {
            self.selected_server = Some(i);
            // 切换服务器：文件浏览页临时状态（客户端模组排查标记）一并恢复
            self.clear_client_mod_marks();
            // 切换服务器：特殊功能页 Spark 检测/文件列表缓存一并清除
            self.clear_special_marks();
        }
        if let Some(i) = toggle_fav {
            if let Some(sc) = self.cfg.servers.get_mut(i) {
                sc.favorited = !sc.favorited;
                self.save_config();
            }
        }
        if let Some(i) = to_remove {
            // 二次确认：防止手误删除服务器配置
            self.confirm_delete_server = Some(i);
        }
        if let Some((i, name)) = to_rename {
            self.rename_server = Some((i, name));
        }
        if let Some(i) = to_backup_now {
            if self.cfg.servers.get(i).is_some() {
                let nm = self.cfg.servers[i].name.clone();
                self.spawn_backup(i);
                self.set_toast(format!("已对「{nm}」发起立即备份"));
            }
        }

        // 服务器删除二次确认弹窗（仅移除配置，服务器文件/世界数据不动）
        if let Some(i) = self.confirm_delete_server {
            let name = self
                .cfg
                .servers
                .get(i)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let mut close = false;
            let mut do_delete = false;
            egui::Window::new("删除服务器")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("确定要删除服务器「{name}」吗？"));
                    ui.label(RichText::new("仅删除工具内的服务器配置，服务器文件与存档不会动。").weak());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                        if ui.button(RichText::new("确认删除配置").color(self.fg(Color32::from_rgb(230, 120, 120)))).clicked() {
                            do_delete = true;
                            close = true;
                        }
                    });
                });
            if close {
                self.confirm_delete_server = None;
            }
            if do_delete {
                self.remove_server(i);
                if self.selected_server == Some(i) {
                    self.selected_server = None;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // 第二批：max-width 1100 左对齐（服务器详情页；空态/early-return 在容器内）
            let _avail = ui.available_rect_before_wrap();
            let _w = _avail.width().min(1100.0);
            let _centered = egui::Rect::from_min_size(
                egui::pos2(_avail.left(), _avail.top()),
                egui::vec2(_w, _avail.height()),
            );
            let mut _inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(_centered)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .id_salt("page_center_1100"),
            );
            _inner.set_width(_w);
            let ui = &mut _inner;
            let Some(idx) = self.selected_server else {
                ui.centered_and_justified(|ui| {
                    ui.label("在左侧添加或选择一个服务器");
                });
                return;
            };
            if idx >= self.cfg.servers.len() {
                return;
            }
            self.ui_server_detail(ui, idx);
        });

        // 右下角自绘通知（浮层，最后绘制保证在最上层�?
        self.draw_toasts(ctx);
    }

    fn ui_server_detail(&mut self, ui: &mut egui::Ui, idx: usize) {
        // 顶部页签（带平滑滑块动画，可在设置中关闭）
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.spacing_mut().item_spacing.x = 4.0; // 页签间距收紧，选项整体靠左
        // 玩家管理页签仅在 BETA_PLAYERS 启用时出现（渐进式开关：关闭时不占用 UI 与调度）
        let mut tabs: Vec<(&str, ServerTab)> = vec![
            ("概览", ServerTab::Overview),
            ("服务器设置", ServerTab::Scripts),
            ("文件浏览", ServerTab::Files),
            ("自动功能", ServerTab::Backup),
            ("服务器状态", ServerTab::Status),
        ];
        if features::is_enabled(&self.cfg.features, features::BETA_PLAYERS) {
            tabs.insert(4, ("玩家管理", ServerTab::Players));
        }
        // 特殊功能页在总开关 BETA_SPECIAL 启用时出现（2026-10-02 起属测试功能，默认禁用；可在设置-测试中的功能开关）
        if features::is_enabled(&self.cfg.features, features::BETA_SPECIAL) {
            tabs.push(("特殊功能", ServerTab::Special));
        }
        let n_tabs = tabs.len() as f32;
        let target_idx = tabs
            .iter()
            .position(|(_, v)| *v == self.server_tab)
            .unwrap_or(0) as f32;
        if self.cfg.ui_animations {
            self.tab_anim += (target_idx - self.tab_anim) * 0.22;
            if (self.tab_anim - target_idx).abs() < 0.02 {
                self.tab_anim = target_idx;
            }
        } else {
            self.tab_anim = target_idx;
        }
        let item_h = 26.0f32;
        // 页签总宽 = n*tab_w + (n-1)*间距(item_spacing 4 + add_space 2) + 左侧边距(8) + 右侧留白(6)
        let tab_w = (ui.available_width() - 8.0 - 6.0 - (n_tabs - 1.0) * 6.0) / n_tabs;
        // 先渲染按钮拿到实际矩形，再按 tab_anim 横向插值绘制滑块（与按钮严格对齐）
        let mut tab_rects: Vec<egui::Rect> = Vec::with_capacity(tabs.len());
        let mut clicked_tab: Option<ServerTab> = None;
        ui.horizontal(|ui| {
            for (i, (label, val)) in tabs.iter().enumerate() {
                let active = *val == self.server_tab;
                let btn = egui::Button::new(
                    RichText::new(*label)
                        .size(14.0)
                        .color(if active {
                            self.fg(Color32::from_rgb(120, 230, 160))
                        } else {
                            self.fg(Color32::from_rgb(205, 205, 205))
                        }),
                )
                .frame(false)
                .min_size(egui::vec2(tab_w, item_h));
                let resp = ui.add(btn);
                tab_rects.push(resp.rect);
                if resp.clicked() {
                    clicked_tab = Some(*val);
                }
                ui.add_space(2.0);
            }
        });
        if let Some(v) = clicked_tab {
            // 切回概览时自动贴到日志底部（用户需求：切回来就应在最底下，而非加按钮）
            if v == ServerTab::Overview {
                if let Some(rt) = self.runtimes.get_mut(idx) {
                    rt.force_scroll_bottom = true;
                }
            }
            // 进入文件浏览页：清除客户端模组排查标记（临时 UI 状态，切界面回来恢复正常）
            if v == ServerTab::Files {
                self.clear_client_mod_marks();
            }
            // 进入特殊功能页：清除 Spark 检测/文件列表缓存（进入即重新扫描）
            if v == ServerTab::Special {
                self.clear_special_marks();
            }
            self.server_tab = v;
        }
        // 滑块沿实际按钮位置横向插�?
        let i0 = self.tab_anim.floor() as usize;
        let i1 = (self.tab_anim.ceil() as usize).min(tabs.len() - 1);
        let t = self.tab_anim - i0 as f32;
        let r0 = tab_rects[i0];
        let r1 = tab_rects[i1];
        let slider_rect = egui::Rect::from_min_max(
            egui::pos2(r0.min.x + 2.0 + (r1.min.x - r0.min.x) * t, r0.min.y + 2.0),
            egui::pos2(r0.max.x - 2.0 + (r1.max.x - r0.max.x) * t, r0.max.y - 2.0),
        );
        ui.painter()
            .rect_filled(slider_rect, 5.0, Color32::from_rgba_unmultiplied(80, 200, 120, 36));
        ui.separator();

        match self.server_tab {
            ServerTab::Overview => self.ui_overview(ui, idx),
            ServerTab::Scripts => self.ui_scripts(ui, idx),
            ServerTab::Files => self.ui_files(ui, idx),
            ServerTab::Backup => self.ui_backup(ui, idx),
            ServerTab::Perf => self.ui_perf(ui, idx),
            ServerTab::Status => self.ui_status(ui, idx),
            ServerTab::Players => self.ui_players(ui, idx),
            ServerTab::Special => self.ui_special(ui, idx),
        }
    }

    /// 独立性能页：服务器运行中显示实时采样图表，未运行提示无数�?
    fn ui_perf(&mut self, ui: &mut egui::Ui, idx: usize) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
        let running = self
            .runtimes
            .get(idx)
            .and_then(|r| r.proc.as_ref())
            .map(|p| process::is_running(p))
            .unwrap_or(false);
        if !running {
            ui.add_space(24.0);
            ui.centered_and_justified(|ui| {
                ui.label("服务器未运行，无性能数据");
            });
            return;
        }
        show_perf_bar(ui, &self.runtimes[idx], true);
        ui.add_space(4.0);
        ui.label(
            RichText::new("CPU / 内存采样")
                .size(12.0)
                .color(self.fg(Color32::from_rgb(160, 160, 160))),
        );
            });
    }


    /// 从服务器日志解析当前在线玩家（无需 RCON）：跟踪 " joined the game" / " left the game" 事件。
    fn parse_online_players(log: &str) -> Vec<String> {
        let mut players: Vec<String> = Vec::new();
        for line in log.lines() {
            let line = line.trim();
            if let Some(pos) = line.find(" joined the game") {
                if let Some(name) = Self::extract_player_name(line, pos) {
                    if !players.iter().any(|p| p == &name) {
                        players.push(name);
                    }
                }
            } else if let Some(pos) = line.find(" left the game") {
                if let Some(name) = Self::extract_player_name(line, pos) {
                    players.retain(|p| p != &name);
                }
            }
        }
        players
    }

    /// 从日志行中 " joined/left the game" 事件位置向前提取玩家名（去掉可能的后缀 [/IP:port] 等）。
    fn extract_player_name(line: &str, event_pos: usize) -> Option<String> {
        let before = &line[..event_pos];
        let sep = before
            .rfind("]: ")
            .map(|i| i + 3)
            .or_else(|| before.rfind(": ").map(|i| i + 2))?;
        let mut name = before[sep..].trim().to_string();
        if let Some(b) = name.find('[') {
            name.truncate(b);
        }
        let name = name.trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }

    /// 状态页：玩家列表（日志解析）+ 性能 + 网络
    fn ui_status(&mut self, ui: &mut egui::Ui, idx: usize) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
        let running = self
            .runtimes
            .get(idx)
            .and_then(|r| r.proc.as_ref())
            .map(|p| process::is_running(p))
            .unwrap_or(false);

        // ---------- 性能（合并自原独�?性能"页：无需再区分页面） ----------
        ui.separator();
        ui.label(RichText::new("性能").strong());
        if !running {
            ui.label(RichText::new("服务器未运行，无性能数据").weak());
        } else {
            ui.label(
                RichText::new("CPU / 内存采样")
                    .size(12.0)
                    .color(self.fg(Color32::from_rgb(160, 160, 160))),
            );
            ui.add_space(2.0);
            show_perf_bar(ui, &self.runtimes[idx], true);
        }

        // ---------- TPS/MSPT（Spark 方案，开发中） ----------
        ui.separator();
        ui.label(RichText::new("服务端性能").strong());
        ui.label(RichText::new("TPS / MSPT 与卡顿源分析：待接入 Spark 方案（当前版本未实现）").weak());

        // ---------- 崩溃报告分析（离线可用，参考 PCL 的日志/报告模式匹配思路） ----------
        // 测试功能「崩溃报告分析」禁用时整块隐藏（分析结果缓存保留，重新启用后可见）
        if features::is_enabled(&self.cfg.features, features::BETA_CRASH_ANALYSIS) {
            ui.separator();
            ui.label(RichText::new("崩溃报告分析").strong());
            ui.label(RichText::new("列出可分析的崩溃来源（latest.log / crash-reports / hs_err_pid），可逐个或一键全量分析").weak().small());
            ui.horizontal(|ui| {
                if ui.button("🔍 分析全部").clicked() {
                    let sc = self.cfg.servers[idx].clone();
                    self.runtimes[idx].crash_analysis = Some(analyze_crash_reports(&sc.dir));
                }
                if ui.button("🔄 刷新列表").clicked() {
                    let sc = self.cfg.servers[idx].clone();
                    self.runtimes[idx].crash_list = collect_crash_sources(&sc.dir);
                    self.runtimes[idx].crash_analysis = None;
                }
                if self.runtimes[idx].crash_analysis.is_some() {
                    if ui.button("清除结果").clicked() {
                        self.runtimes[idx].crash_analysis = None;
                    }
                }
            });
            // 崩溃来源列表（首次进入自动构建一次）
            if self.runtimes[idx].crash_list.is_empty() {
                let sc = self.cfg.servers[idx].clone();
                self.runtimes[idx].crash_list = collect_crash_sources(&sc.dir);
            }
            let sources = self.runtimes[idx].crash_list.clone();
            if !sources.is_empty() {
                ui.label(RichText::new("可选分析对象：").strong());
                egui::ScrollArea::vertical()
                    .id_salt(("crash_sources_scroll", idx))
                    .max_height(140.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (kind, rel) in &sources {
                            ui.horizontal(|ui| {
                                let icon = match kind {
                                    0 => "📋",
                                    1 => "💥",
                                    _ => "🧨",
                                };
                                let sel = self.runtimes[idx].crash_sel.as_ref()
                                    .map(|(k, r)| k == kind && r == rel)
                                    .unwrap_or(false);
                                if ui.selectable_label(sel, format!("{icon} {rel}")).clicked() {
                                    self.runtimes[idx].crash_sel = Some((*kind, rel.clone()));
                                }
                                if ui.button("分析").clicked() {
                                    let sc = self.cfg.servers[idx].clone();
                                    let res = analyze_crash_single(&sc.dir, *kind, rel);
                                    self.runtimes[idx].crash_analysis = Some(res);
                                    self.runtimes[idx].crash_sel = Some((*kind, rel.clone()));
                                }
                            });
                        }
                    });
            }
            if let Some(text) = &self.runtimes[idx].crash_analysis {
                let text = text.clone();
                egui::ScrollArea::vertical()
                    .id_salt(("crash_analysis_scroll", idx))
                    .max_height(280.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        render_analysis_text(ui, &text);
                    });
            } else {
                ui.label(
                    RichText::new("点击来源右侧「分析」可单独分析该文件，或「分析全部」一键诊断；服务器未运行也可分析")
                        .weak(),
                );
            }
        }


        if !running {
            ui.add_space(12.0);
            ui.centered_and_justified(|ui| {
                ui.label("服务器未运行，无法获取状态");
            });
            return;
        }

            });
    }

    /// 特殊功能页：Spark 性能分析（问题2 重构：总折叠 + 子折叠，左侧分析控制 + 右侧输出/预览）
    fn ui_special(&mut self, ui: &mut egui::Ui, idx: usize) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 总折叠：整页可折叠，标题「Spark 性能分析」
                egui::CollapsingHeader::new(RichText::new("Spark 性能分析").strong().size(16.0))
                    .id_salt(("spark_total", idx))
                    .default_open(true)
                    .show(ui, |ui| {
                        self.ui_special_body(ui, idx);
                    });
            });
    }

    /// Spark 性能分析页主体（子折叠 + 左右分栏：左侧分析控制，右侧输出文件/分析预览）
    fn ui_special_body(&mut self, ui: &mut egui::Ui, idx: usize) {
        ui.separator();

        // 子开关未启用：提示未启用并隐藏 Spark 操作区（正式功能：设置-服务器-Spark 分析可关闭）
        if !features::is_enabled(&self.cfg.features, features::FEATURE_SPARK) {
            ui.label(
                RichText::new("Spark 分析功能未启用。")
                    .color(self.fg(Color32::from_rgb(240, 168, 82))),
            );
            ui.label("启用后：检测服务器 mods 目录下是否安装 spark（文件名含 spark 的 jar），向服务器控制台发送 /spark profiler 命令进行性能采样，并列出 spark --save-to-file 生成的输出文件（.sparkprofile / .sparkhealth）。");
            ui.label("点击列表中的输出文件即可解析分析：.sparkprofile（调用树热点、模组性能占用、卡顿热点 TOP）与 .sparkhealth（TPS/MSPT/CPU/内存/GC 概览 + 波动曲线）。");
            if ui.button("启用 Spark 分析").clicked() {
                self.set_feature(features::FEATURE_SPARK, true);
                self.set_toast("已启用「Spark 分析」".to_string());
            }
            return;
        }

        // Spark 可用性检测（进入页面前已清除缓存，此处惰性自动扫描）
        if self.runtimes.get(idx).map_or(true, |r| r.spark_detect.is_none()) {
            self.detect_spark(idx);
        }
        let Some((available, jars)) = self.runtimes[idx].spark_detect.clone() else {
            return;
        };

        // 子折叠：安装检测
        egui::CollapsingHeader::new(RichText::new("安装检测").strong())
            .id_salt(("spark_detect", idx))
            .default_open(false)
            .show(ui, |ui| {
                if !available {
                    ui.label(RichText::new("未检测到 Spark").strong());
                    ui.label("服务器 mods 目录下未找到文件名包含 “spark” 的 jar（不区分大小写）。");
                    ui.label("请将 spark 模组放入服务器 mods 目录并重启服务器，或确认已安装后再进入本页。");
                    return;
                }
                ui.label(
                    RichText::new(format!("✓ 已检测到 Spark（{} 个匹配 jar）", jars.len()))
                        .color(self.fg(Color32::from_rgb(92, 183, 120))),
                );
                for j in &jars {
                    ui.label(RichText::new(format!("  · {j}")).weak().small());
                }
            });

        // 未检测到 Spark：下方控制区不可用（仅提示）
        if !available {
            ui.add_space(8.0);
            ui.label(
                RichText::new("安装 spark 模组并重启服务器后，本页将提供性能采样控制与输出文件预览。")
                    .weak(),
            );
            return;
        }
        ui.add_space(6.0);

        // 左右分栏：左侧分析控制（固定宽），右侧输出文件 + 分析预览
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(300.0, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.ui_spark_control(ui, idx);
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                self.ui_spark_preview(ui, idx);
            });
        });
    }

    /// 左侧：Spark 性能分析控制（开始/时长/进度/强停）
    fn ui_spark_control(&mut self, ui: &mut egui::Ui, idx: usize) {
        egui::CollapsingHeader::new(RichText::new("性能分析").strong())
            .id_salt(("spark_control", idx))
            .default_open(true)
            .show(ui, |ui| {
                let running = self.runtimes[idx].spark_prof.is_some();
                if running {
                    let (total_secs, start, done) = {
                        let p = self.runtimes[idx].spark_prof.as_ref().unwrap();
                        (p.total_secs, p.start, p.sent_stop)
                    };
                    let elapsed = std::time::Instant::now().duration_since(start).as_secs();
                    let frac = (elapsed as f32 / total_secs.max(1) as f32).min(1.0);
                    let remain = total_secs.saturating_sub(elapsed);
                    if done {
                        ui.label(
                            RichText::new("✓ 采样完成，文件已保存并刷新列表")
                                .color(self.fg(Color32::from_rgb(92, 183, 120))),
                        );
                    } else {
                        ui.add(egui::ProgressBar::new(frac).text(format!(
                            "{}/{} 秒（剩余 {} 秒）",
                            elapsed, total_secs, remain
                        )));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("强制停止").clicked() {
                            let ok = self.spark_send_cmd(idx, "spark profiler stop");
                            self.runtimes[idx].spark_prof = None;
                            self.runtimes[idx].spark_prof_refresh_at = None;
                            self.notify(
                                "XMST - Spark 分析",
                                &if ok {
                                    "已强制停止 Spark 采样（未保存文件）".to_string()
                                } else {
                                    "强制停止失败：服务器未运行或 stdin 未启用".to_string()
                                },
                            );
                        }
                        if done && ui.button("关闭").clicked() {
                            self.runtimes[idx].spark_prof = None;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label("采样时长");
                        ui.add(
                            egui::DragValue::new(&mut self.runtimes[idx].spark_prof_secs)
                                .range(5..=3600)
                                .suffix(" 秒"),
                        );
                    });
                    ui.add_space(4.0);
                    let server_running = self.runtimes[idx].proc.is_some();
                    if ui
                        .add_enabled(
                            server_running,
                            egui::Button::new(RichText::new("开始分析").strong()),
                        )
                        .on_hover_text("向服务器控制台发送 /spark profiler start（到期自动发送 stop --save-to-file 落盘）")
                        .clicked()
                    {
                        let secs = self.runtimes[idx].spark_prof_secs.max(1);
                        // 不用 --timeout：spark 的 --timeout 到期只自动停止并上传 Web URL，不会生成本地文件；
                        // 由 tick_spark_prof 到期发送 stop --save-to-file 才能落盘 .sparkprofile 并出现在输出列表。
                        let ok = self.spark_send_cmd(idx, "spark profiler start");
                        if ok {
                            self.runtimes[idx].spark_prof = Some(SparkProfiling {
                                total_secs: secs,
                                start: std::time::Instant::now(),
                                sent_stop: false,
                            });
                            self.notify(
                                "XMST - Spark 分析",
                                &format!("采样已开始（{secs} 秒），到期自动保存文件"),
                            );
                        } else {
                            self.set_toast("无法开始采样：服务器未运行或 stdin 未启用".to_string());
                        }
                    }
                    if !server_running {
                        ui.label(
                            RichText::new("服务器未运行，无法发送采样命令。")
                                .weak()
                                .small(),
                        );
                    }
                }
                ui.add_space(4.0);
            });
    }

    /// 右侧：输出文件列表（子折叠）+ 分析结果预览（子折叠）
    fn ui_spark_preview(&mut self, ui: &mut egui::Ui, idx: usize) {
        egui::CollapsingHeader::new(RichText::new("输出文件").strong())
            .id_salt(("spark_files", idx))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("刷新列表").clicked() {
                        self.runtimes[idx].spark_scanned = false;
                        self.scan_spark_files(idx);
                    }
                });

                // 空列表时自动扫描一次（空目录也置位 spark_scanned，避免每帧重扫）
                if self.runtimes[idx].spark_files.is_empty() && !self.runtimes[idx].spark_scanned {
                    self.scan_spark_files(idx);
                }
                let files = self.runtimes[idx].spark_files.clone();
                if !files.is_empty() {
                    egui::ScrollArea::vertical()
                        .id_salt(("spark_files_scroll", idx))
                        .max_height(260.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            egui::Grid::new(("spark_files_grid", idx))
                                .striped(true)
                                .min_col_width(60.0)
                                .show(ui, |ui| {
                                    ui.label(RichText::new("目录").strong());
                                    ui.label(RichText::new("文件名").strong());
                                    ui.label(RichText::new("大小").strong());
                                    ui.label(RichText::new("修改时间").strong());
                                    ui.end_row();
                                    for (dir, name, size, mtime) in &files {
                                        ui.label(RichText::new(dir).weak().small());
                                        let full = format!("{}\\{}", dir, name);
                                        // Resolve relative scan dir to server absolute dir (fs::read resolves against process CWD otherwise -> os error 3)
                                        let full_abs = self
                                            .cfg
                                            .servers
                                            .get(idx)
                                            .map(|sc| sc.dir.join(&full).display().to_string())
                                            .unwrap_or_else(|| full.clone());
                                        let selected = self.runtimes[idx]
                                            .spark_analysis
                                            .as_ref()
                                            .map_or(false, |a| a.path == full_abs);
                                        if ui
                                            .selectable_label(selected, RichText::new(name).small())
                                            .clicked()
                                        {
                                            self.parse_spark_file(idx, full_abs.clone());
                                        }
                                        ui.label(RichText::new(fmt_size(*size)).small());
                                        let ts = chrono::DateTime::from_timestamp(*mtime, 0)
                                            .map(|t| {
                                                t.with_timezone(&chrono::Local)
                                                    .format("%Y-%m-%d %H:%M:%S")
                                                    .to_string()
                                            })
                                            .unwrap_or_else(|| "-".to_string());
                                        ui.label(RichText::new(ts).small());
                                        ui.end_row();
                                    }
                                });
                        });
                }
            });

        // 分析结果（文件列表点击后展示；页签化布局 + 模组定位/详情）
        if let Some(st) = self.runtimes[idx].spark_analysis.clone() {
            egui::CollapsingHeader::new(RichText::new("分析结果预览").strong())
                .id_salt(("spark_preview", idx))
                .default_open(true)
                .show(ui, |ui| {
                    // 惰性扫描 mods jar 定位索引（分析页「卡顿源→模组 jar」跳转依赖）
                    if self.runtimes[idx].mod_jar_index.is_none() {
                        self.scan_mod_jar_index(idx);
                    }
                    let jar_index = self.runtimes[idx].mod_jar_index.clone().unwrap_or_default();
                    let mut view_tab = self.runtimes[idx].spark_view_tab;
                    let mut mod_detail = self.runtimes[idx].spark_mod_detail.clone();
                    let mut locate = |jar: String| self.locate_mod_jar(idx, jar);
                    spark_analysis::ui_analysis(
                        ui,
                        &st,
                        &mut view_tab,
                        &mut mod_detail,
                        &jar_index,
                        &mut locate,
                    );
                    drop(locate);
                    self.runtimes[idx].spark_view_tab = view_tab;
                    self.runtimes[idx].spark_mod_detail = mod_detail;
                });
        }
    }

    /// 向服务器控制台 stdin 发送命令（Spark 分析用）
    fn spark_send_cmd(&mut self, idx: usize, cmd: &str) -> bool {
        self.runtimes
            .get(idx)
            .and_then(|rt| rt.proc.as_ref())
            .map(|p| process::write_stdin(p, cmd).is_ok())
            .unwrap_or(false)
    }

    /// Spark 性能分析推进：到期自动发送 stop --save-to-file（start 不带 --timeout，确保 stop 时采样仍在运行、文件可落盘）；停止后延迟刷新输出文件列表
    fn tick_spark_prof(&mut self) {
        if !features::is_enabled(&self.cfg.features, features::FEATURE_SPARK) {
            return;
        }
        let now = std::time::Instant::now();
        for i in 0..self.runtimes.len() {
            // 分析进行中：到期自动停止并保存
            let due_stop = self.runtimes[i].spark_prof.as_ref().map_or(false, |p| {
                !p.sent_stop && now.duration_since(p.start).as_secs() >= p.total_secs
            });
            if due_stop {
                let ok = self.spark_send_cmd(i, "spark profiler stop --save-to-file");
                if let Some(p) = self.runtimes[i].spark_prof.as_mut() {
                    p.sent_stop = true;
                }
                self.runtimes[i].spark_prof_refresh_at =
                    Some(now + std::time::Duration::from_millis(2500));
                self.notify(
                    "XMST - Spark 分析",
                    &if ok {
                        "采样到期，已发送 stop --save-to-file，正在刷新输出文件…".to_string()
                    } else {
                        "采样到期，但无法向服务器控制台发送停止命令（服务器未运行？）".to_string()
                    },
                );
            }
            // 停止后延迟刷新输出文件列表（发送保存命令 2.5s 后扫描）
            if let Some(t) = self.runtimes[i].spark_prof_refresh_at {
                if now >= t {
                    self.runtimes[i].spark_prof_refresh_at = None;
                    self.runtimes[i].spark_scanned = false;
                    self.scan_spark_files(i);
                }
            }
        }
    }

    /// 扫描 服务器目录\mods\*.jar，文件名（不区分大小写）含 "spark" 即视为已安装 spark
    fn detect_spark(&mut self, idx: usize) {
        if idx >= self.runtimes.len() {
            return;
        }
        let mut jars: Vec<String> = Vec::new();
        if let Some(sc) = self.cfg.servers.get(idx) {
            let mods_dir = sc.dir.join("mods");
            if let Ok(rd) = std::fs::read_dir(&mods_dir) {
                for e in rd.flatten() {
                    if !e.path().is_file() {
                        continue;
                    }
                    let name = e.file_name().to_string_lossy().to_string();
                    let low = name.to_lowercase();
                    if low.ends_with(".jar") && low.contains("spark") {
                        jars.push(name);
                    }
                }
            }
        }
        jars.sort();
        let available = !jars.is_empty();
        self.runtimes[idx].spark_detect = Some((available, jars));
    }

    /// 扫描 mods 目录下所有 jar，提取元数据别名（fabric.mod.json id/name、mods.toml modId/displayName），缓存到 mod_jar_index
    fn scan_mod_jar_index(&mut self, idx: usize) {
        if idx >= self.runtimes.len() {
            return;
        }
        let mut infos: Vec<spark_analysis::ModJarInfo> = Vec::new();
        if let Some(sc) = self.cfg.servers.get(idx) {
            let mods_dir = sc.dir.join("mods");
            if let Ok(rd) = std::fs::read_dir(&mods_dir) {
                for e in rd.flatten() {
                    if !e.path().is_file() {
                        continue;
                    }
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.to_lowercase().ends_with(".jar") {
                        continue;
                    }
                    infos.push(Self::mod_jar_info(&e.path(), name));
                }
            }
        }
        infos.sort_by(|a, b| a.jar.cmp(&b.jar));
        self.runtimes[idx].mod_jar_index = Some(infos);
    }

    /// 解析单个 jar 的模组元数据别名（fabric.mod.json 的 id/name、META-INF/mods.toml 与 neoforge.mods.toml 的 modId/displayName）
    fn mod_jar_info(path: &std::path::Path, jar: String) -> spark_analysis::ModJarInfo {
        let mut aliases: Vec<String> = Vec::new();
        let Ok(file) = std::fs::File::open(path) else {
            return spark_analysis::ModJarInfo { jar, aliases };
        };
        let Ok(mut z) = zip::ZipArchive::new(file) else {
            return spark_analysis::ModJarInfo { jar, aliases };
        };
        if let Ok(mut f) = z.by_name("fabric.mod.json") {
            let mut buf = Vec::new();
            if std::io::Read::read_to_end(&mut f, &mut buf).is_ok() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) {
                    if let Some(id) = v["id"].as_str() {
                        aliases.push(id.to_string());
                    }
                    if let Some(n) = v["name"].as_str() {
                        aliases.push(n.to_string());
                    }
                }
            }
        }
        for meta in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
            if let Ok(mut f) = z.by_name(meta) {
                let mut buf = Vec::new();
                if std::io::Read::read_to_end(&mut f, &mut buf).is_ok() {
                    if let Ok(v) = toml::from_str::<toml::Value>(&String::from_utf8_lossy(&buf)) {
                        if let Some(mods) = v.get("mods").and_then(|m| m.as_array()) {
                            for m in mods {
                                if let Some(id) = m.get("modId").and_then(|x| x.as_str()) {
                                    aliases.push(id.to_string());
                                }
                                if let Some(n) = m.get("displayName").and_then(|x| x.as_str()) {
                                    aliases.push(n.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        aliases.retain(|a| !a.is_empty());
        spark_analysis::ModJarInfo { jar, aliases }
    }

    /// 从 Spark 分析页跳转到文件浏览 mods 页签并定位目标 jar（搜索词置为该 jar 名 + 滚动定位）
    fn locate_mod_jar(&mut self, idx: usize, jar_name: String) {
        if idx >= self.runtimes.len() {
            return;
        }
        self.runtimes[idx].file_tab = "mods".to_string();
        self.runtimes[idx].file_sub.clear();
        self.runtimes[idx].file_search = jar_name.clone();
        self.runtimes[idx].file_scroll_to = Some(jar_name.clone());
        self.runtimes[idx].file_client_mods = None;
        self.runtimes[idx].file_client_mods_confirm = false;
        self.refresh_file_list(idx);
        self.server_tab = ServerTab::Files;
        self.set_toast(format!("已定位模组：{jar_name}"));
    }

    /// 扫描 spark 输出目录（模组端 config/spark、插件端 plugins/spark）下 .sparkprofile / .sparkhealth 文件
    fn scan_spark_files(&mut self, idx: usize) {
        if idx >= self.runtimes.len() {
            return;
        }
        let mut files: Vec<(String, String, u64, i64)> = Vec::new();
        if let Some(sc) = self.cfg.servers.get(idx) {
            for sub in ["config\\spark", "plugins\\spark"] {
                let dir = sc.dir.join(sub);
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let Ok(md) = e.metadata() else { continue };
                        if !md.is_file() {
                            continue;
                        }
                        let name = e.file_name().to_string_lossy().to_string();
                        let low = name.to_lowercase();
                        if low.ends_with(".sparkprofile") || low.ends_with(".sparkhealth") {
                            let mtime = md
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            files.push((sub.to_string(), name, md.len(), mtime));
                        }
                    }
                }
            }
        }
        files.sort_by(|a, b| b.3.cmp(&a.3));
        self.runtimes[idx].spark_files = files;
        self.runtimes[idx].spark_scanned = true;
    }

    /// 解析 spark 输出文件并缓存分析状态（gzip 解压 + protobuf 解码；失败记录错误信息供 UI 展示）
    fn parse_spark_file(&mut self, idx: usize, path: String) {
        if idx >= self.runtimes.len() || self.runtimes[idx].spark_parsing {
            return;
        }
        self.runtimes[idx].spark_parsing = true;
        let outcome = spark_analysis::parse_spark_file(&path);
        let state = spark_analysis::SparkAnalysisState {
            path,
            summary: outcome.clone().ok(),
            parse_error: outcome.err(),
        };
        self.runtimes[idx].spark_analysis = Some(state);
        self.runtimes[idx].spark_parsing = false;
    }

    fn ui_overview(&mut self, ui: &mut egui::Ui, idx: usize) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
        let sc = self.cfg.servers[idx].clone();
        let running = self
            .runtimes
            .get(idx)
            .and_then(|r| r.proc.as_ref())
            .map(|p| process::is_running(p))
            .unwrap_or(false);
        let stopping = self.runtimes.get(idx).map(|r| r.stopping).unwrap_or(false);
        let busy = running || stopping;
        let last_msg = self.runtimes.get(idx).map(|r| r.last_msg.clone()).unwrap_or_default();

        ui.add_space(6.0);
        // ===== 头部：名称 + 目录/文件夹快捷入口 =====
        ui.horizontal(|ui| {
            ui.label(RichText::new(&sc.name).size(18.0).strong());
            ui.add_space(6.0);
            if ui.button("📂 服务器文件夹").clicked() {
                self.open_folder(&sc.dir);
            }
            if ui.button("📂 存档").clicked() {
                self.open_folder(&sc.dir.join("world"));
            }
            if ui.button("📂 模组").clicked() {
                self.open_folder(&sc.dir.join("mods"));
            }
            if ui.button("📂 配置").clicked() {
                self.open_folder(&sc.dir.join("config"));
            }
            if ui.button("📂 日志").clicked() {
                self.open_folder(&sc.dir.join("logs"));
            }
        });
        ui.label(RichText::new(sc.dir.display().to_string()).weak().small());
        // ===== 卡片流：状态 / 平台 / 目录 =====
        // 原实现是把这些信息一行行平铺（信息密度低、重点不突出），改为卡片流。
        let pinfo = serverinfo::detect(&sc.dir);
        let card_fill = self.theme_cur.widget_bg;
        let card_stroke = egui::Stroke::new(1.0, self.theme_cur.stroke);
        let make_card = |ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::none()
                .fill(card_fill)
                .stroke(card_stroke)
                .rounding(8.0)
                .inner_margin(egui::Margin::symmetric(12.0, 9.0))
                .show(ui, |ui| {
                    ui.set_width(230.0);
                    ui.label(RichText::new(title).weak().small());
                    ui.add_space(2.0);
                    add(ui);
                });
        };
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            // 卡片 1：运行状态
            let (label, col) = if stopping {
                ("正在停止…".to_string(), Color32::from_rgb(240, 200, 120))
            } else if running {
                ("运行中".to_string(), Color32::from_rgb(80, 200, 120))
            } else {
                ("已停止".to_string(), Color32::from_rgb(160, 160, 166))
            };
            make_card(ui, "状态", &mut |ui| {
                ui.label(RichText::new(label.clone()).size(16.0).strong().color(col));
                if !last_msg.is_empty() {
                    ui.label(RichText::new(last_msg.clone()).small().weak());
                }
                if !sc.dir.exists() {
                    ui.label(RichText::new("⚠ 目录不存在").small().color(Color32::RED));
                }
            });
            // 卡片 2：服务端平台
            make_card(ui, "🧩 服务端平台", &mut |ui| {
                let color = if pinfo.kind.is_modded() {
                    self.fg(Color32::from_rgb(120, 200, 255))
                } else if pinfo.kind == serverinfo::PlatformKind::Vanilla {
                    Color32::from_rgb(240, 176, 96)
                } else {
                    self.fg(Color32::from_rgb(160, 220, 180))
                };
                let r = ui.label(RichText::new(pinfo.summary()).strong().color(color));
                if !pinfo.evidence.is_empty() {
                    r.on_hover_text(pinfo.evidence.join("\n"));
                }
                if let Some(lv) = &pinfo.loader_version {
                    ui.label(RichText::new(format!("加载器 {lv}")).small().weak());
                }
                ui.label(
                    RichText::new(format!("模组 {} · 插件 {}", pinfo.mod_count, pinfo.plugin_count))
                        .small()
                        .weak(),
                );
            });
            // 卡片 3：目录与体积
            make_card(ui, "📁 目录", &mut |ui| {
                let world = dir_size_mb(&sc.dir.join("world"));
                let mods = dir_size_mb(&sc.dir.join("mods"));
                ui.label(format!("world {world}"));
                ui.label(format!("mods {mods}"));
                let eula = sc.dir.join("eula.txt").exists();
                ui.label(
                    RichText::new(if eula { "eula.txt 已生成" } else { "eula.txt 缺失" })
                        .small()
                        .weak(),
                );
            });
        });
        ui.separator();

        ui.horizontal(|ui| {
            let start_btn = ui.add_enabled(!busy, egui::Button::new(RichText::new("▶ 启动服务器").size(15.0)));
            if start_btn.clicked() {
                // 手动启动：重置崩溃重启状态（含熔断）
                if let Some(rt) = self.runtimes.get_mut(idx) {
                    rt.crash_count = 0;
                    rt.crash_first_at = None;
                    rt.crash_restart_at = None;
                }
                self.start_server(idx);
            }
            let stop_btn = ui.add_enabled(running, egui::Button::new(RichText::new("⏹ 停止 (stop)").size(15.0)));
            if stop_btn.clicked() {
                self.stop_server(idx);
            }
            let kill_btn = ui.add_enabled(running, egui::Button::new(RichText::new("⏹ 强制结束").size(15.0)));
            if kill_btn.clicked() {
                if features::is_enabled(&self.cfg.features, features::FEATURE_FORCE_STOP_CONFIRM) {
                    // B3 强停二次确认：先弹确认（pid + 影响），确认后带一次性 token 强杀
                    self.request_force_stop(idx);
                } else {
                    self.kill_server(idx);
                }
            }
        });
        if !last_msg.is_empty() {
            let msg_color = if last_msg.starts_with('✅') {
                self.fg(Color32::from_rgb(255, 180, 80))
            } else {
                self.fg(Color32::from_rgb(120, 180, 255))
            };
            ui.label(RichText::new(&last_msg).color(msg_color));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("服务器输出日志").strong());
            // 清空日志：seek_end 跳过文件已有内容，避免重启后旧日志回灌（修复"假清�?重启不清�?�?
            if ui.button("清空日志").clicked() {
                if let Some(rt) = self.runtimes.get_mut(idx) {
                    rt.log_buf.clear();
                    rt.log_pending = 0;
                    if let Some(t) = &mut rt.log_tail {
                        t.seek_end();
                    }
                }
            }
        });
        let (changed, force_bottom) = {
            let rt = &mut self.runtimes[idx];
            // 缓冲被裁剪后长度可能恒定，必须按“新增行数”判断是否贴�?
            let changed = rt.log_pending > 0;
            rt.log_pending = 0;
            let fb = rt.force_scroll_bottom;
            rt.force_scroll_bottom = false;
            (changed, fb)
        };
        // 借用而非 clone：show_colored_log 只读日志缓冲，避免可见态每帧大字符串克隆造成内存峰值
        let log_resp = show_colored_log(
            ui,
            &self.runtimes[idx].log_buf,
            changed,
            force_bottom,
            Some(ui.available_height() - 44.0),
        );
        // 点击日志区域：聚焦命令输入框（用户需求：点击日志即可输入，而非独立搜索框）
        if log_resp.clicked() {
            if let Some(rt) = self.runtimes.get_mut(idx) {
                rt.cmd_focus = true;
            }
        }
        // 命令输入框（日志下方控制台风格）：Enter 发送，�?�?回看历史
        ui.horizontal(|ui| {
            // 先取聚焦标记并复位（避免�?cmd_input 的可变借用冲突�?
            let want_focus = self.runtimes.get_mut(idx).map(|r| {
                let f = r.cmd_focus;
                r.cmd_focus = false;
                f
            }).unwrap_or(false);
            let mut send_cmd = String::new();
            let mut hist_move: Option<i32> = None; // -1=停止 1=启动
            let input_opt = self.runtimes.get_mut(idx).map(|r| &mut r.cmd_input);
            if let Some(input) = input_opt {
                let input_id = egui::Id::new(("cmd_input", idx));
                if want_focus {
                    ui.memory_mut(|m| m.request_focus(input_id));
                }
                let resp = ui.add(
                    TextEdit::singleline(input)
                        .id(input_id)
                        .hint_text("输入 stop / say hello 等命令（Enter 发送，↑↓ 回看历史）")
                        .desired_width(f32::INFINITY),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    send_cmd = input.clone();
                }
                if resp.has_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        hist_move = Some(-1);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                        hist_move = Some(1);
                    }
                }
            }
            // 上下键历史导�?
            if let Some(dir) = hist_move {
                if let Some(rt) = self.runtimes.get_mut(idx) {
                    if !rt.cmd_history.is_empty() {
                        let pos = match rt.cmd_hist_pos {
                            Some(p) => p,
                            None => rt.cmd_history.len(), // 光标在末尾（当前输入中
                        };
                        let new_pos = if dir < 0 {
                            pos.saturating_sub(1)
                        } else {
                            (pos + 1).min(rt.cmd_history.len())
                        };
                        if new_pos == rt.cmd_history.len() {
                            rt.cmd_hist_pos = None;
                        } else {
                            rt.cmd_input = rt.cmd_history[new_pos].clone();
                            rt.cmd_hist_pos = Some(new_pos);
                        }
                    }
                }
            }
            if ui.button("发送命令").clicked() {
                send_cmd = self.runtimes.get(idx).map(|r| r.cmd_input.clone()).unwrap_or_default();
            }
            if !send_cmd.is_empty() {
                let cmd = send_cmd.trim().to_string();
                if !cmd.is_empty() {
                    let sent = self
                        .runtimes
                        .get(idx)
                        .and_then(|rt| rt.proc.as_ref())
                        .map(|p| process::write_stdin(p, &cmd).is_ok())
                        .unwrap_or(false);
                    if sent {
                        if let Some(rt) = self.runtimes.get_mut(idx) {
                            rt.cmd_input.clear();
                            rt.cmd_hist_pos = None;
                            // 历史去重后追加（最多保�?50 条）
                            if rt.cmd_history.last().map(|s| s.as_str()) != Some(cmd.as_str()) {
                                rt.cmd_history.push(cmd);
                                if rt.cmd_history.len() > 50 {
                                    rt.cmd_history.remove(0);
                                }
                            }
                        }
                    } else {
                        self.set_toast("发送失败：服务器未运行，stdin 未启用".to_string());
                    }
                }
            }
        });
            });
    }

    fn ui_scripts(&mut self, ui: &mut egui::Ui, idx: usize) {
        let sc = self.cfg.servers[idx].clone();
        let dir = sc.dir.clone();
        let run_bat = dir.join("run.bat");
        let jvm_args = dir.join("user_jvm_args.txt");

        // 整页滚动：内容超出一屏时保证下方保存按钮可见可点
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
        ui.add_space(6.0);

        // 分区一：白名单/黑名单管理（标题由 ui_whitelist_blacklist 内部绘制，置于页面顶部）
        // 整合去重（B1）：BETA_PLAYERS 启用时白名单/封禁已并入「玩家」页签，此处不再重复展示；
        // 仅当玩家管理开关关闭时保留原三折叠区作为降级入口（行为不变，仅写 JSON 文件）。
        if !features::is_enabled(&self.cfg.features, features::BETA_PLAYERS) {
            self.ui_whitelist_blacklist(ui, idx);
        }
        ui.separator();

        // 分区二：服务器设置（server.properties 核心配置）
        ui.label(RichText::new("服务器设置").strong());
        ui.separator();

        // 初始化编辑缓�?
        if self.runtimes[idx].run_bat_edit.is_none() {
            self.runtimes[idx].run_bat_edit = Some(
                std::fs::read_to_string(&run_bat).unwrap_or_else(|_| {
                    "@echo off\njava -Xmx4G -Xms2G -jar server.jar nogui\npause\n".to_string()
                }),
            );
        }
        if self.runtimes[idx].jvm_args_edit.is_none() {
            self.runtimes[idx].jvm_args_edit = Some(
                std::fs::read_to_string(&jvm_args).unwrap_or_else(|_| {
                    "-Xmx4G -Xms2G\n".to_string()
                }),
            );
        }

        // run.bat（折叠：内容长时不再占满整屏，点开才展开编辑�?

        // server.properties（折叠：汉化类型化编�?+ 高级文本�?
        // 默认展开：此前入口藏在折叠里用户找不到，改为默认展开方便直接编辑
        egui::CollapsingHeader::new("server.properties（点击展开编辑）")
            .default_open(true)
            .show(ui, |ui| {
                let sp_path = dir.join("server.properties");
                if self.runtimes[idx].server_props_map.is_none() {
                    let text = std::fs::read_to_string(&sp_path).unwrap_or_default();
                    self.runtimes[idx].server_props_map = Some(parse_properties(&text));
                    self.runtimes[idx].server_props_text = Some(text);
                }
                if !sp_path.exists() {
                    ui.label(RichText::new("（服务器目录下暂存 server.properties，保存后自动创建").weak());
                }
                let mut adv = self.runtimes[idx].server_props_advanced;
                ui.horizontal(|ui| {
                    if ui.selectable_label(!adv, "类型化编辑").clicked() {
                        let text = self.runtimes[idx].server_props_text.clone().unwrap_or_default();
                        self.runtimes[idx].server_props_map = Some(parse_properties(&text));
                        adv = false;
                    }
                    if ui.selectable_label(adv, "高级文本").clicked() {
                        let map = self.runtimes[idx].server_props_map.clone().unwrap_or_default();
                        self.runtimes[idx].server_props_text = Some(format_properties(&map));
                        adv = true;
                    }
                });
                self.runtimes[idx].server_props_advanced = adv;
                let mut save_props = false;
                // 编辑区（高度由下方分隔条拖动调整，展开互不冲突，允许拉倒最大）
                egui::ScrollArea::vertical()
                    .id_salt("server_props_edit")
                    .max_height(self.server_props_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if adv {
                            let mut text = self.runtimes[idx].server_props_text.clone().unwrap_or_default();
                            ui.add(
                                TextEdit::multiline(&mut text)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(12),
                            );
                            self.runtimes[idx].server_props_text = Some(text);
                        } else {
                            let map = self.runtimes[idx].server_props_map.clone().unwrap_or_default();
                            let new_map = show_server_props_typed(ui, &map);
                            if new_map != map {
                                self.runtimes[idx].server_props_map = Some(new_map);
                            }
                        }
                    });
                draggable_divider(ui, &mut self.server_props_h, &mut self.server_props_drag_start);
                if ui.button("💾 保存 server.properties").clicked() {
                    let map = self.runtimes[idx].server_props_map.clone().unwrap_or_default();
                    let text = if adv {
                        self.runtimes[idx].server_props_text.clone().unwrap_or_default()
                    } else {
                        format_properties(&map)
                    };
                    self.runtimes[idx].server_props_text = Some(text.clone());
                    self.runtimes[idx].server_props_map = Some(if adv { parse_properties(&text) } else { map });
                    save_props = true;
                }
                if save_props {
                    let out = self.runtimes[idx].server_props_text.clone().unwrap_or_default();
                    let _ = std::fs::create_dir_all(&dir);
                    match std::fs::write(&sp_path, out) {
                        Ok(_) => self.set_toast("server.properties 已保存".to_string()),
                        Err(e) => self.set_toast(format!("保存失败: {e}")),
                    }
                }
            });
        ui.separator();
        egui::CollapsingHeader::new("run.bat（点击展开编辑）")
            .default_open(false)
            .show(ui, |ui| {
                let mut bat_content = self.runtimes[idx].run_bat_edit.clone().unwrap_or_default();
                // 预览区（高度由下方分隔条拖动调整，允许拉倒最大显示全部脚本）
                egui::ScrollArea::vertical()
                    .id_salt("run_bat_preview")
                    .max_height(self.run_bat_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut bat_content)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(10),
                        );
                    });
                draggable_divider(ui, &mut self.run_bat_h, &mut self.run_bat_drag_start);
                if ui.button("💾 保存 run.bat").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    match std::fs::write(&run_bat, &bat_content) {
                        Ok(_) => self.set_toast("run.bat 已保存".to_string()),
                        Err(e) => self.set_toast(format!("保存失败: {e}")),
                    }
                }
                self.runtimes[idx].run_bat_edit = Some(bat_content);
            });
            });
        ui.separator();

        // 分区三：启动脚本（run.bat / user_jvm_args / 自定义启动命令）
        ui.label(RichText::new("启动脚本").strong());
        ui.label("run.bat 存在时启动优先执行；未使用 run.bat 时按下方自定义启动命令或工具默认设置启动");
        ui.separator();

        // user_jvm_args.txt（折叠）
        egui::CollapsingHeader::new("user_jvm_args.txt（点击展开编辑）")
            .default_open(true)
            .show(ui, |ui| {
                let mut jvm_content = self.runtimes[idx].jvm_args_edit.clone().unwrap_or_default();
                ui.add(
                    TextEdit::multiline(&mut jvm_content)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
                if ui.button("💾 保存 user_jvm_args.txt").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    match std::fs::write(&jvm_args, &jvm_content) {
                        Ok(_) => self.set_toast("user_jvm_args.txt 已保存".to_string()),
                        Err(e) => self.set_toast(format!("保存失败: {e}")),
                    }
                }
                self.runtimes[idx].jvm_args_edit = Some(jvm_content);
            });
        ui.separator();

        // launch_cmd 模板（run.bat 不存在时使用�?
        ui.label(RichText::new("自定义启动命令（未使用 run.bat 时使用）").strong());
        ui.label("占位符: {java} Java路径, {jvm} JVM参数");
        let mut launch_cmd = self.cfg.servers[idx].launch_cmd.clone();
        ui.add(
            TextEdit::singleline(&mut launch_cmd)
                .desired_width(f32::INFINITY),
        );
        if launch_cmd != self.cfg.servers[idx].launch_cmd {
            self.cfg.servers[idx].launch_cmd = launch_cmd;
            self.save_config();
        }
        ui.separator();


        // 开机自启（配合全局"开机自动启�?XMST"使用�?-autostart 模式下生效）
        ui.label(RichText::new("开机自启").strong());
        ui.label("需先在设置页勾选「开机自动启动 XMST」；勾选后本机开机登录时等待 CPU 空闲再自动启动本服务器（错峰）");
        let mut ae = self.cfg.servers[idx].autostart_enabled;
        if ui.checkbox(&mut ae, "开机自动启动本服务器").changed() {
            self.cfg.servers[idx].autostart_enabled = ae;
            self.save_config();
        }
        let mut aci = self.cfg.servers[idx].autostart_cpu_idle;
        if ui.checkbox(&mut aci, "等待系统 CPU 空闲后再启动（错峰）").changed() {
            self.cfg.servers[idx].autostart_cpu_idle = aci;
            self.save_config();
        }
        ui.separator();

        // Java 设置：服务器�?MC 版本 + 指定全局列表�?
        ui.label(RichText::new("Java 设置").strong());
        ui.label("未使用 run.bat 时优先执行 run.bat；无 run.bat 时按下方解析链自动选择 Java");
        ui.horizontal(|ui| {
            ui.label("MC 版本");
            let mut ver = self.cfg.servers[idx].mc_version.clone().unwrap_or_default();
            let resp = ui.add(
                TextEdit::singleline(&mut ver)
                    .hint_text("如 1.21.1")
                    .desired_width(120.0),
            );
            if resp.changed() {
                let v = ver.trim().to_string();
                self.cfg.servers[idx].mc_version = if v.is_empty() { None } else { Some(v) };
                self.save_config();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Java 来源");
            let homes = self.cfg.java_homes.clone();
            let sel = self.cfg.servers[idx].java_home_id.clone();
            let sel_text = match &sel {
                Some(id) => homes
                    .iter()
                    .find(|h| &h.name == id)
                    .map(|h| format!("{} ({})", h.name, h.path))
                    .unwrap_or_else(|| format!("{id} (已删除)")),
                None => "自动（按版本匹配 / run.bat / 全局）".to_string(),
            };
            egui::ComboBox::from_label("")
                .selected_text(sel_text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.cfg.servers[idx].java_home_id,
                        None,
                        "自动（按版本匹配 / run.bat / 全局）",
                    );
                    for h in &homes {
                        ui.selectable_value(
                            &mut self.cfg.servers[idx].java_home_id,
                            Some(h.name.clone()),
                            format!("{} ({})", h.name, h.path),
                        );
                    }
                });
            // 打开所�?Java 所在目录（便于核对位置�?
            if ui.button("📂 打开所在目录").clicked() {
                let p = sel
                    .as_ref()
                    .and_then(|id| homes.iter().find(|h| &h.name == id))
                    .map(|h| h.path.clone())
                    .unwrap_or_else(|| self.cfg.java_path.clone());
                if !p.trim().is_empty() {
                    explorer_select(&p);
                }
            }
        });
        // 解析结果预览（只读）
        let cfg = self.cfg.clone();
        let resolved = resolve_java_for_server(&cfg, &sc);
        ui.label(RichText::new(format!("当前解析: {resolved}")).weak());
        if run_bat.exists() {
            ui.label(RichText::new("（run.bat 存在，启动时优先执行 run.bat，Java 以脚本内为准").weak());
        }
    }

    /// 白名单 / 黑名单管理（whitelist.json / banned-players.json / banned-ips.json）
    /// 预览显示：名称/IP、UUID、添加人/封禁者（json source 优先，日志补充，缺省 Server）、时间、理由
    fn ui_whitelist_blacklist(&mut self, ui: &mut egui::Ui, idx: usize) {
        let sc = self.cfg.servers[idx].clone();
        let dir = sc.dir;
        if !dir.exists() {
            return;
        }
        let wl_path = dir.join("whitelist.json");
        let bp_path = dir.join("banned-players.json");
        let bi_path = dir.join("banned-ips.json");

        ui.label(RichText::new("白名单 / 黑名单管理").size(15.0).strong());
        ui.label(
            RichText::new("直接读写服务器目录下的 whitelist.json / banned-players.json / banned-ips.json；由本工具添加的条目操作人记为 Server")
                .weak()
                .small(),
        );

        // 白名单
        egui::CollapsingHeader::new(format!("白名单（{}）", load_json_list(&wl_path).len()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("添加玩家");
                    let resp = ui.add(
                        TextEdit::singleline(&mut self.runtimes[idx].wl_add_name)
                            .hint_text("玩家名称")
                            .desired_width(140.0),
                    );
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.add_whitelist_entry(idx, &wl_path);
                    }
                    if ui.button("➕ 添加").clicked() {
                        self.add_whitelist_entry(idx, &wl_path);
                    }
                });
                let list = load_json_list(&wl_path);
                if list.is_empty() {
                    ui.label(RichText::new("（暂无白名单条目）").weak());
                } else {
                    egui::Grid::new(("wl_grid", idx))
                        .striped(true)
                        .num_columns(5)
                        .show(ui, |ui| {
                            ui.label(RichText::new("玩家").strong());
                            ui.label(RichText::new("UUID").strong());
                            ui.label(RichText::new("添加人").strong());
                            ui.label(RichText::new("添加时间").strong());
                            ui.label(RichText::new("操作").strong());
                            ui.end_row();
                            for v in &list {
                                let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                                let uuid = v.get("uuid").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                ui.label(&name);
                                ui.label(RichText::new(&uuid).monospace().small());
                                ui.label("Server");
                                ui.label("—");
                                if ui.button("移除").clicked() {
                                    let mut nl = load_json_list(&wl_path);
                                    nl.retain(|e| {
                                        e.get("name").and_then(|x| x.as_str()) != Some(name.as_str())
                                            && e.get("uuid").and_then(|x| x.as_str()) != Some(uuid.as_str())
                                    });
                                    if save_json_list(&wl_path, &nl).is_ok() {
                                        self.set_toast(format!("已从白名单移除 {name}"));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                }
            });

        // 封禁玩家
        egui::CollapsingHeader::new(format!("封禁玩家（{}）", load_json_list(&bp_path).len()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("封禁玩家");
                    ui.add(
                        TextEdit::singleline(&mut self.runtimes[idx].ban_add_name)
                            .hint_text("玩家名称")
                            .desired_width(120.0),
                    );
                    ui.label("理由");
                    ui.add(
                        TextEdit::singleline(&mut self.runtimes[idx].ban_add_reason)
                            .hint_text("封禁理由（可空）")
                            .desired_width(180.0),
                    );
                    if ui.button("🔨 封禁").clicked() {
                        self.add_ban_entry(idx, &bp_path);
                    }
                });
                let list = load_json_list(&bp_path);
                if list.is_empty() {
                    ui.label(RichText::new("（暂无封禁玩家）").weak());
                } else {
                    egui::Grid::new(("bp_grid", idx))
                        .striped(true)
                        .num_columns(6)
                        .show(ui, |ui| {
                            ui.label(RichText::new("玩家").strong());
                            ui.label(RichText::new("UUID").strong());
                            ui.label(RichText::new("封禁者").strong());
                            ui.label(RichText::new("封禁时间").strong());
                            ui.label(RichText::new("理由").strong());
                            ui.label(RichText::new("操作").strong());
                            ui.end_row();
                            for v in &list {
                                let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                                let uuid = v.get("uuid").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                let source = v
                                    .get("source")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("Server")
                                    .to_string();
                                let created = v
                                    .get("created")
                                    .and_then(|x| x.as_i64())
                                    .map(fmt_ms_time)
                                    .unwrap_or_else(|| "—".to_string());
                                let reason = v
                                    .get("reason")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                ui.label(&name);
                                ui.label(RichText::new(&uuid).monospace().small());
                                ui.label(&source);
                                ui.label(&created);
                                ui.label(if reason.is_empty() { "—" } else { &reason });
                                if ui.button("解除").clicked() {
                                    let mut nl = load_json_list(&bp_path);
                                    nl.retain(|e| {
                                        e.get("name").and_then(|x| x.as_str()) != Some(name.as_str())
                                            && e.get("uuid").and_then(|x| x.as_str()) != Some(uuid.as_str())
                                    });
                                    if save_json_list(&bp_path, &nl).is_ok() {
                                        self.set_toast(format!("已解除玩家 {name} 的封禁"));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                }
            });

        // 封禁 IP
        egui::CollapsingHeader::new(format!("封禁 IP（{}）", load_json_list(&bi_path).len()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("封禁 IP");
                    ui.add(
                        TextEdit::singleline(&mut self.runtimes[idx].banip_add_ip)
                            .hint_text("IP 地址")
                            .desired_width(140.0),
                    );
                    ui.label("理由");
                    ui.add(
                        TextEdit::singleline(&mut self.runtimes[idx].banip_add_reason)
                            .hint_text("封禁理由（可空）")
                            .desired_width(180.0),
                    );
                    if ui.button("🔨 封禁").clicked() {
                        self.add_banip_entry(idx, &bi_path);
                    }
                });
                let list = load_json_list(&bi_path);
                if list.is_empty() {
                    ui.label(RichText::new("（暂无封禁 IP）").weak());
                } else {
                    egui::Grid::new(("bi_grid", idx))
                        .striped(true)
                        .num_columns(5)
                        .show(ui, |ui| {
                            ui.label(RichText::new("IP").strong());
                            ui.label(RichText::new("封禁者").strong());
                            ui.label(RichText::new("封禁时间").strong());
                            ui.label(RichText::new("理由").strong());
                            ui.label(RichText::new("操作").strong());
                            ui.end_row();
                            for v in &list {
                                let ip = v.get("ip").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                                let source = v
                                    .get("source")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("Server")
                                    .to_string();
                                let created = v
                                    .get("created")
                                    .and_then(|x| x.as_i64())
                                    .map(fmt_ms_time)
                                    .unwrap_or_else(|| "—".to_string());
                                let reason = v
                                    .get("reason")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                ui.label(&ip);
                                ui.label(&source);
                                ui.label(&created);
                                ui.label(if reason.is_empty() { "—" } else { &reason });
                                if ui.button("解除").clicked() {
                                    let mut nl = load_json_list(&bi_path);
                                    nl.retain(|e| e.get("ip").and_then(|x| x.as_str()) != Some(ip.as_str()));
                                    if save_json_list(&bi_path, &nl).is_ok() {
                                        self.set_toast(format!("已解除 IP {ip} 的封禁"));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                }
            });
    }

    /// 添加白名单条目（双路径：运行中优先 /whitelist add；否则写 whitelist.json，操作人 Server）
    fn add_whitelist_entry(&mut self, idx: usize, path: &std::path::Path) {
        let name = self.runtimes[idx].wl_add_name.trim().to_string();
        if name.is_empty() {
            self.set_toast("请输入玩家名称".to_string());
            return;
        }
        // 双路径：服务端运行中优先走命令
        if self.players_send_cmd(idx, &format!("whitelist add {name}")) {
            self.set_toast(format!("已发送 /whitelist add {name}，等待服务端处理"));
            self.runtimes[idx].wl_add_name.clear();
            return;
        }
        let mut list = load_json_list(path);
        if list.iter().any(|e| e.get("name").and_then(|x| x.as_str()) == Some(name.as_str())) {
            self.set_toast(format!("{name} 已在白名单中"));
            return;
        }
        let entry = serde_json::json!({
            "uuid": offline_uuid(&name),
            "name": name.clone(),
        });
        list.push(entry);
        match save_json_list(path, &list) {
            Ok(_) => {
                self.set_toast(format!("已将 {name} 加入白名单（操作人: Server）"));
                self.runtimes[idx].wl_add_name.clear();
                self.players_schedule_refresh(idx);
            }
            Err(e) => self.set_toast(format!("写白名单失败: {e}")),
        }
    }

    /// 添加封禁玩家条目（双路径：运行中优先 /ban；否则写 banned-players.json，操作人 Server）
    fn add_ban_entry(&mut self, idx: usize, path: &std::path::Path) {
        let name = self.runtimes[idx].ban_add_name.trim().to_string();
        if name.is_empty() {
            self.set_toast("请输入玩家名称".to_string());
            return;
        }
        let reason = self.runtimes[idx].ban_add_reason.trim().to_string();
        // 双路径：服务端运行中优先走命令
        if self.players_send_cmd(idx, &format!("ban {name} {reason}")) {
            self.set_toast(format!("已发送 /ban {name}，等待服务端处理"));
            self.runtimes[idx].ban_add_name.clear();
            self.runtimes[idx].ban_add_reason.clear();
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut list = load_json_list(path);
        if list.iter().any(|e| e.get("name").and_then(|x| x.as_str()) == Some(name.as_str())) {
            self.set_toast(format!("{name} 已在封禁列表中"));
            return;
        }
        list.push(serde_json::json!({
            "uuid": offline_uuid(&name),
            "name": name.clone(),
            "created": now_ms,
            "source": "Server",
            "expires": "forever",
            "reason": reason,
        }));
        match save_json_list(path, &list) {
            Ok(_) => {
                self.set_toast(format!("已封禁 {name}（操作人: Server）"));
                self.runtimes[idx].ban_add_name.clear();
                self.runtimes[idx].ban_add_reason.clear();
                self.players_schedule_refresh(idx);
            }
            Err(e) => self.set_toast(format!("写封禁列表失败: {e}")),
        }
    }

    /// 添加封禁 IP 条目（双路径：运行中优先 /ban-ip；否则写 banned-ips.json，操作人 Server）
    fn add_banip_entry(&mut self, idx: usize, path: &std::path::Path) {
        let ip = self.runtimes[idx].banip_add_ip.trim().to_string();
        if ip.is_empty() {
            self.set_toast("请输入 IP 地址".to_string());
            return;
        }
        let reason = self.runtimes[idx].banip_add_reason.trim().to_string();
        // 双路径：服务端运行中优先走命令
        if self.players_send_cmd(idx, &format!("ban-ip {ip} {reason}")) {
            self.set_toast(format!("已发送 /ban-ip {ip}，等待服务端处理"));
            self.runtimes[idx].banip_add_ip.clear();
            self.runtimes[idx].banip_add_reason.clear();
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut list = load_json_list(path);
        if list.iter().any(|e| e.get("ip").and_then(|x| x.as_str()) == Some(ip.as_str())) {
            self.set_toast(format!("IP {ip} 已在封禁列表中"));
            return;
        }
        list.push(serde_json::json!({
            "ip": ip.clone(),
            "created": now_ms,
            "source": "Server",
            "expires": "forever",
            "reason": reason,
        }));
        match save_json_list(path, &list) {
            Ok(_) => {
                self.set_toast(format!("已封禁 IP {ip}（操作人: Server）"));
                self.runtimes[idx].banip_add_ip.clear();
                self.runtimes[idx].banip_add_reason.clear();
                self.players_schedule_refresh(idx);
            }
            Err(e) => self.set_toast(format!("写封禁列表失败: {e}")),
        }
    }

    /// 添加 OP 条目（B1：未运行时写 ops.json；运行中优先 /op 命令）
    fn add_op_entry(&mut self, idx: usize, path: &std::path::Path) {
        let name = self.runtimes[idx].op_add_name.trim().to_string();
        if name.is_empty() {
            self.set_toast("请输入玩家名称".to_string());
            return;
        }
        // 双路径：服务端运行中优先走命令
        if self.players_send_cmd(idx, &format!("op {name}")) {
            self.set_toast(format!("已发送 /op {name}，等待服务端处理"));
            self.runtimes[idx].op_add_name.clear();
            return;
        }
        let mut list = load_json_list(path);
        if list.iter().any(|e| e.get("name").and_then(|x| x.as_str()) == Some(name.as_str())) {
            self.set_toast(format!("{name} 已是 OP"));
            return;
        }
        list.push(serde_json::json!({
            "uuid": offline_uuid(&name),
            "name": name.clone(),
            "level": 4,
            "bypassesPlayerLimit": false,
        }));
        match save_json_list(path, &list) {
            Ok(_) => {
                self.set_toast(format!("已将 {name} 设为 OP（操作人: Server）"));
                self.runtimes[idx].op_add_name.clear();
                self.players_schedule_refresh(idx);
            }
            Err(e) => self.set_toast(format!("写 ops.json 失败: {e}")),
        }
    }

    /// 玩家操作双路径：服务端进程运行中优先走 stdin 命令通道（/whitelist /ban /pardon 等），
    /// 发送成功返回 true；未运行返回 false，由调用方回退写 JSON 文件。
    /// 玩家属性（阶段C）：gamemode / OP 权限，经服务器控制台 stdin 发送命令；服务端未运行时提示不生效。
    fn ui_players_props(&mut self, ui: &mut egui::Ui, idx: usize) {
        ui.horizontal(|ui| {
            ui.label("玩家名:");
            ui.add(
                TextEdit::singleline(&mut self.player_prop_name)
                    .desired_width(160.0)
                    .hint_text("如 Steve"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("游戏模式:");
            for (label, mode) in [
                ("生存", "survival"),
                ("创造", "creative"),
                ("冒险", "adventure"),
                ("旁观", "spectator"),
            ] {
                ui.radio_value(&mut self.player_prop_gamemode, mode.to_string(), label);
            }
        });
        ui.checkbox(&mut self.player_prop_op, "设为 OP（勾选执行 op，取消执行 deop）");
        let name = self.player_prop_name.trim().to_string();
        let apply = ui.add_enabled(!name.is_empty(), egui::Button::new("▶ 应用属性"));
        if apply.clicked() && !name.is_empty() {
            let mut ok = self.players_send_cmd(idx, &format!("gamemode {} {}", self.player_prop_gamemode, name));
            if ok {
                let op_cmd = if self.player_prop_op { "op" } else { "deop" };
                ok = self.players_send_cmd(idx, &format!("{} {}", op_cmd, name));
            }
            if ok {
                self.set_toast(format!(
                    "已应用属性: {name} ({}){}",
                    self.player_prop_gamemode,
                    if self.player_prop_op { ", OP" } else { "" }
                ));
            } else {
                self.set_toast("服务器未运行或命令发送失败，属性未生效".to_string());
            }
        }
        ui.label(
            RichText::new("说明：gamemode / op 命令经服务器控制台 stdin 发送，需服务端运行中才生效")
                .weak()
                .small(),
        );
    }

    fn players_send_cmd(&mut self, idx: usize, cmd: &str) -> bool {
        let sent = self
            .runtimes
            .get(idx)
            .and_then(|rt| rt.proc.as_ref())
            .map(|p| process::write_stdin(p, cmd).is_ok())
            .unwrap_or(false);
        if sent {
            self.players_schedule_refresh(idx);
        }
        sent
    }

    /// 操作成功后 1-2s 延迟刷新（避免写入/命令尚未生效时刷新出旧数据）
    fn players_schedule_refresh(&mut self, idx: usize) {
        if let Some(rt) = self.runtimes.get_mut(idx) {
            rt.players_refresh_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500));
        }
    }

    /// 玩家管理前台轮询（B1/B5）：5s 一次快照刷新 + 操作后延迟刷新；
    /// BETA_PLAYERS 关闭时零开销（入口早退）；托盘态由 update 早退整体停止。
    fn tick_players(&mut self) {
        if !features::is_enabled(&self.cfg.features, features::BETA_PLAYERS) {
            return;
        }
        let now = std::time::Instant::now();
        let five_s = std::time::Duration::from_secs(5);
        for i in 0..self.runtimes.len() {
            let due = match self.runtimes[i].players_last_tick {
                Some(t) => now.duration_since(t) >= five_s,
                None => true,
            };
            if due {
                self.runtimes[i].players_last_tick = Some(now);
                self.refresh_players_snapshot(i);
                continue;
            }
            // 操作成功后的延迟刷新窗口（1-2s）：仅当无更新的快照请求在途时执行
            if let Some(t) = self.runtimes[i].players_refresh_at {
                if now >= t {
                    self.runtimes[i].players_refresh_at = None;
                    if self.runtimes[i].players_seq == self.runtimes[i].players_snapshot_seq {
                        self.refresh_players_snapshot(i);
                    }
                }
            }
        }
    }

    /// 刷新玩家快照（B1/B5）：在线玩家同步解析内存日志；白名单/封禁/OP 异步读文件，
    /// 经 players_tx 回传并按请求序号丢弃过期响应（防 A/B 服数据错位）。
    fn refresh_players_snapshot(&mut self, idx: usize) {
        let rt = &mut self.runtimes[idx];
        rt.players_state = PlayersState::Refreshing;
        rt.players_seq += 1;
        let seq = rt.players_seq;
        // 在线玩家：同步解析（日志在内存，取最新 join/leave 状态）
        rt.players_online = {
            let log = rt.log_buf.clone();
            Self::parse_online_players(&log)
        };
        let wl_path = self.cfg.servers[idx].dir.join("whitelist.json");
        let bp_path = self.cfg.servers[idx].dir.join("banned-players.json");
        let bi_path = self.cfg.servers[idx].dir.join("banned-ips.json");
        let op_path = self.cfg.servers[idx].dir.join("ops.json");
        let tx = self.players_tx.clone();
        let _ = std::thread::spawn(move || {
            let snap = PlayersSnapshot {
                wl: load_json_list(&wl_path),
                bp: load_json_list(&bp_path),
                bi: load_json_list(&bi_path),
                ops: load_json_list(&op_path),
            };
            let _ = tx.send((idx, seq, snap));
        });
    }

    /// 玩家管理页（B1 四 Tab：在线 / 白名单 / 封禁 / OP；操作双路径；B5 序号防竞态）
    fn ui_players(&mut self, ui: &mut egui::Ui, idx: usize) {
        let dir = self.cfg.servers[idx].dir.clone();
        ui.add_space(6.0);
        ui.label(RichText::new("玩家管理").size(15.0).strong());
        ui.label(RichText::new("在线=日志解析；白名单/封禁/OP=读取服务器 JSON；操作优先命令，服务端未运行时写文件并提示")
            .weak().small());
        if !dir.exists() {
            ui.label(RichText::new("服务器目录不存在，无法读取玩家数据").weak());
            return;
        }
        // 首次进入立即刷新一次
        if self.runtimes[idx].players_snapshot.is_none() && self.runtimes[idx].players_seq == 0 {
            self.refresh_players_snapshot(idx);
        }
        // 子页签
        ui.horizontal(|ui| {
            let tabs = [
                ("🌐 在线", PlayerTab::Online),
                ("📜 白名单", PlayerTab::Whitelist),
                ("🔨 封禁", PlayerTab::Banned),
                ("⭐ OP", PlayerTab::Ops),
                ("🧬 属性", PlayerTab::Props),
            ];
            for (label, tab) in tabs {
                let sel = self.runtimes[idx].players_tab == tab;
                if ui.selectable_label(sel, label).clicked() {
                    self.runtimes[idx].players_tab = tab;
                }
            }
            ui.separator();
            if ui.button("🔄 刷新").clicked() {
                self.refresh_players_snapshot(idx);
            }
        });
        let st = match self.runtimes[idx].players_state {
            PlayersState::Ready => "数据就绪 · 前台每 5 秒自动刷新".to_string(),
            PlayersState::Refreshing => "正在刷新…".to_string(),
            PlayersState::Unknown => "状态未知（等待首次刷新）".to_string(),
        };
        ui.label(RichText::new(st).weak().small());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("players_scroll", idx))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match self.runtimes[idx].players_tab {
                    PlayerTab::Online => self.ui_players_online(ui, idx),
                    PlayerTab::Whitelist => self.ui_players_wl(ui, idx),
                    PlayerTab::Banned => self.ui_players_banned(ui, idx),
                    PlayerTab::Ops => self.ui_players_ops(ui, idx),
                    // 玩家属性功能暂时禁用（用户要求）：入口保留但只显示说明，
                    // 不再渲染任何可操作控件（原实现 ui_players_props 仍保留在代码里备用）。
                    PlayerTab::Props => self.ui_players_props_disabled(ui, idx),
                }
            });
    }

    /// 玩家属性（阶段C）——**暂时禁用**（用户要求）。
    ///
    /// 保留标签页入口与说明，但不渲染任何控件，避免误导用户以为可用。
    /// 恢复时把下面的提示替换回原实现即可（原实现见 git 历史 / 备份分支）。
    fn ui_players_props_disabled(&mut self, ui: &mut egui::Ui, idx: usize) {
        let _ = idx;
        ui.add_space(8.0);
        egui::Frame::none()
            .fill(self.theme_cur.widget_bg)
            .stroke(egui::Stroke::new(1.0, self.theme_cur.stroke))
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("玩家属性功能暂时禁用")
                        .strong()
                        .color(self.theme_cur.text),
                );
                ui.label(
                    RichText::new(
                        "该功能通过服务器控制台下发 gamemode / op 指令，目前存在稳定性问题，已临时下线。\n\
                         可以改用「在线玩家」页查看列表，或在「OP」页管理 OP 权限。",
                    )
                    .weak()
                    .small(),
                );
            });
    }

    /// 玩家页：在线玩家（日志解析，服务端运行中才有数据）
    fn ui_players_online(&mut self, ui: &mut egui::Ui, idx: usize) {
        let running = self.runtimes[idx].proc.is_some();
        if !running {
            ui.label(RichText::new("服务器未运行，无在线玩家数据（日志解析需服务端运行）").weak());
            return;
        }
        let players = self.runtimes[idx].players_online.clone();
        ui.label(RichText::new(format!("共 {} 人在线", players.len())).strong());
        if players.is_empty() {
            ui.label("（暂无玩家在线，或日志中尚未出现 join/leave 记录）");
        } else {
            ui.horizontal_wrapped(|ui| {
                for p in &players {
                    ui.label(RichText::new(format!("[{p}]")).color(self.fg(Color32::from_rgb(140, 220, 140))));
                }
            });
        }
    }

    /// 玩家页：白名单（whitelist.json）
    fn ui_players_wl(&mut self, ui: &mut egui::Ui, idx: usize) {
        let wl_path = self.cfg.servers[idx].dir.join("whitelist.json");
        ui.horizontal(|ui| {
            ui.label("添加玩家");
            let resp = ui.add(
                TextEdit::singleline(&mut self.runtimes[idx].wl_add_name)
                    .hint_text("玩家名称")
                    .desired_width(140.0),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.add_whitelist_entry(idx, &wl_path);
            }
            if ui.button("➕ 添加").clicked() {
                self.add_whitelist_entry(idx, &wl_path);
            }
        });
        let list = self.runtimes[idx].players_snapshot.as_ref().map(|s| s.wl.clone()).unwrap_or_default();
        if list.is_empty() {
            ui.label(RichText::new("（暂无白名单条目）").weak());
        } else {
            egui::Grid::new(("players_wl_grid", idx)).striped(true).num_columns(5).show(ui, |ui| {
                ui.label(RichText::new("玩家").strong());
                ui.label(RichText::new("UUID").strong());
                ui.label(RichText::new("添加人").strong());
                ui.label(RichText::new("添加时间").strong());
                ui.label(RichText::new("操作").strong());
                ui.end_row();
                for v in &list {
                    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                    let uuid = v.get("uuid").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    ui.label(&name);
                    ui.label(RichText::new(&uuid).monospace().small());
                    ui.label("Server");
                    ui.label("—");
                    if ui.button("移除").clicked() {
                        let cmd = format!("whitelist remove {name}");
                        if self.players_send_cmd(idx, &cmd) {
                            self.set_toast(format!("已发送 /{cmd}，等待服务端处理"));
                        } else {
                            let mut nl = load_json_list(&wl_path);
                            nl.retain(|e| {
                                e.get("name").and_then(|x| x.as_str()) != Some(name.as_str())
                                    && e.get("uuid").and_then(|x| x.as_str()) != Some(uuid.as_str())
                            });
                            if save_json_list(&wl_path, &nl).is_ok() {
                                self.set_toast(format!("已从白名单移除 {name}（操作人: Server）"));
                                self.players_schedule_refresh(idx);
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        }
    }

    /// 玩家页：封禁（banned-players.json 玩家 + banned-ips.json IP）
    fn ui_players_banned(&mut self, ui: &mut egui::Ui, idx: usize) {
        let bp_path = self.cfg.servers[idx].dir.join("banned-players.json");
        let bi_path = self.cfg.servers[idx].dir.join("banned-ips.json");
        // 封禁玩家
        egui::CollapsingHeader::new("封禁玩家").default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("封禁玩家");
                ui.add(
                    TextEdit::singleline(&mut self.runtimes[idx].ban_add_name)
                        .hint_text("玩家名称")
                        .desired_width(120.0),
                );
                ui.label("理由");
                ui.add(
                    TextEdit::singleline(&mut self.runtimes[idx].ban_add_reason)
                        .hint_text("封禁理由（可空）")
                        .desired_width(180.0),
                );
                if ui.button("🔨 封禁").clicked() {
                    self.add_ban_entry(idx, &bp_path);
                }
            });
            let list = self.runtimes[idx].players_snapshot.as_ref().map(|s| s.bp.clone()).unwrap_or_default();
            if list.is_empty() {
                ui.label(RichText::new("（暂无封禁玩家）").weak());
            } else {
                egui::Grid::new(("players_bp_grid", idx)).striped(true).num_columns(6).show(ui, |ui| {
                    ui.label(RichText::new("玩家").strong());
                    ui.label(RichText::new("UUID").strong());
                    ui.label(RichText::new("封禁者").strong());
                    ui.label(RichText::new("封禁时间").strong());
                    ui.label(RichText::new("理由").strong());
                    ui.label(RichText::new("操作").strong());
                    ui.end_row();
                    for v in &list {
                        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                        let uuid = v.get("uuid").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let source = v.get("source").and_then(|x| x.as_str()).unwrap_or("Server").to_string();
                        let created = v.get("created").and_then(|x| x.as_i64()).map(fmt_ms_time).unwrap_or_else(|| "—".to_string());
                        let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        ui.label(&name);
                        ui.label(RichText::new(&uuid).monospace().small());
                        ui.label(&source);
                        ui.label(&created);
                        ui.label(if reason.is_empty() { "—" } else { &reason });
                        if ui.button("解除").clicked() {
                            let cmd = format!("pardon {name}");
                            if self.players_send_cmd(idx, &cmd) {
                                self.set_toast(format!("已发送 /{cmd}，等待服务端处理"));
                            } else {
                                let mut nl = load_json_list(&bp_path);
                                nl.retain(|e| {
                                    e.get("name").and_then(|x| x.as_str()) != Some(name.as_str())
                                        && e.get("uuid").and_then(|x| x.as_str()) != Some(uuid.as_str())
                                });
                                if save_json_list(&bp_path, &nl).is_ok() {
                                    self.set_toast(format!("已解除玩家 {name} 的封禁（操作人: Server）"));
                                    self.players_schedule_refresh(idx);
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            }
        });
        // 封禁 IP
        egui::CollapsingHeader::new("封禁 IP").default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("封禁 IP");
                ui.add(
                    TextEdit::singleline(&mut self.runtimes[idx].banip_add_ip)
                        .hint_text("IP 地址")
                        .desired_width(140.0),
                );
                ui.label("理由");
                ui.add(
                    TextEdit::singleline(&mut self.runtimes[idx].banip_add_reason)
                        .hint_text("封禁理由（可空）")
                        .desired_width(180.0),
                );
                if ui.button("🔨 封禁").clicked() {
                    self.add_banip_entry(idx, &bi_path);
                }
            });
            let list = self.runtimes[idx].players_snapshot.as_ref().map(|s| s.bi.clone()).unwrap_or_default();
            if list.is_empty() {
                ui.label(RichText::new("（暂无封禁 IP）").weak());
            } else {
                egui::Grid::new(("players_bi_grid", idx)).striped(true).num_columns(5).show(ui, |ui| {
                    ui.label(RichText::new("IP").strong());
                    ui.label(RichText::new("封禁者").strong());
                    ui.label(RichText::new("封禁时间").strong());
                    ui.label(RichText::new("理由").strong());
                    ui.label(RichText::new("操作").strong());
                    ui.end_row();
                    for v in &list {
                        let ip = v.get("ip").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                        let source = v.get("source").and_then(|x| x.as_str()).unwrap_or("Server").to_string();
                        let created = v.get("created").and_then(|x| x.as_i64()).map(fmt_ms_time).unwrap_or_else(|| "—".to_string());
                        let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        ui.label(&ip);
                        ui.label(&source);
                        ui.label(&created);
                        ui.label(if reason.is_empty() { "—" } else { &reason });
                        if ui.button("解除").clicked() {
                            let cmd = format!("pardon-ip {ip}");
                            if self.players_send_cmd(idx, &cmd) {
                                self.set_toast(format!("已发送 /{cmd}，等待服务端处理"));
                            } else {
                                let mut nl = load_json_list(&bi_path);
                                nl.retain(|e| e.get("ip").and_then(|x| x.as_str()) != Some(ip.as_str()));
                                if save_json_list(&bi_path, &nl).is_ok() {
                                    self.set_toast(format!("已解除 IP {ip} 的封禁（操作人: Server）"));
                                    self.players_schedule_refresh(idx);
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            }
        });
    }

    /// 玩家页：OP（ops.json）
    fn ui_players_ops(&mut self, ui: &mut egui::Ui, idx: usize) {
        let op_path = self.cfg.servers[idx].dir.join("ops.json");
        ui.horizontal(|ui| {
            ui.label("添加 OP");
            let resp = ui.add(
                TextEdit::singleline(&mut self.runtimes[idx].op_add_name)
                    .hint_text("玩家名称")
                    .desired_width(140.0),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.add_op_entry(idx, &op_path);
            }
            if ui.button("⭐ 设为 OP").clicked() {
                self.add_op_entry(idx, &op_path);
            }
        });
        let list = self.runtimes[idx].players_snapshot.as_ref().map(|s| s.ops.clone()).unwrap_or_default();
        if list.is_empty() {
            ui.label(RichText::new("（暂无 OP 条目）").weak());
        } else {
            egui::Grid::new(("players_ops_grid", idx)).striped(true).num_columns(5).show(ui, |ui| {
                ui.label(RichText::new("玩家").strong());
                ui.label(RichText::new("UUID").strong());
                ui.label(RichText::new("等级").strong());
                ui.label(RichText::new("绕过人数限制").strong());
                ui.label(RichText::new("操作").strong());
                ui.end_row();
                for v in &list {
                    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
                    let uuid = v.get("uuid").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let level = v.get("level").and_then(|x| x.as_i64()).unwrap_or(4);
                    let bpl = v.get("bypassesPlayerLimit").and_then(|x| x.as_bool()).unwrap_or(false);
                    ui.label(&name);
                    ui.label(RichText::new(&uuid).monospace().small());
                    ui.label(format!("{level}"));
                    ui.label(if bpl { "是" } else { "否" });
                    if ui.button("取消 OP").clicked() {
                        let cmd = format!("deop {name}");
                        if self.players_send_cmd(idx, &cmd) {
                            self.set_toast(format!("已发送 /{cmd}，等待服务端处理"));
                        } else {
                            let mut nl = load_json_list(&op_path);
                            nl.retain(|e| e.get("name").and_then(|x| x.as_str()) != Some(name.as_str()));
                            if save_json_list(&op_path, &nl).is_ok() {
                                self.set_toast(format!("已取消 {name} 的 OP（操作人: Server）"));
                                self.players_schedule_refresh(idx);
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        }
    }

    fn ui_files(&mut self, ui: &mut egui::Ui, idx: usize) {
        let tabs = ["mods", "config", "logs", "crash-reports", "datapack"];
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for t in tabs {
                let selected = self.runtimes[idx].file_tab == t;
                if ui.selectable_label(selected, t).clicked() {
                    self.runtimes[idx].file_tab = t.to_string();
                    // 切换页签即恢复：客户端模组排查标记不跨页签保留（状态不持久化）
                    self.runtimes[idx].file_client_mods = None;
                    self.runtimes[idx].file_client_mods_confirm = false;
                    self.refresh_file_list(idx);
                }
            }
            if self.runtimes[idx].file_tab == "mods" {
                // 功能优化：进入模组页即识别该服务端平台 ——
                // 原版/纯插件端不支持模组，直接把「下载模组」按钮置灰并说明原因；
                // 模组端则把识别到的加载器/版本记下来，点下载时自动套用。
                let pinfo = serverinfo::detect(&self.cfg.servers[idx].dir);
                let can_mod = pinfo.kind.is_modded();
                let btn = ui.add_enabled(
                    can_mod,
                    egui::Button::new("⬇️ 下载模组").min_size(egui::vec2(96.0, 24.0)),
                );
                let btn = if can_mod {
                    btn.on_hover_text(format!(
                        "将按 {} 自动选择加载器与 MC 版本",
                        pinfo.summary()
                    ))
                } else {
                    btn.on_disabled_hover_text(format!(
                        "当前服务端识别为「{}」，不支持模组（原版/纯插件端）。\n如需模组请改装 Fabric/Forge/NeoForge 服务端。",
                        pinfo.kind.label()
                    ))
                };
                if btn.clicked() {
                    if let Some(dl) = self.dl.as_mut() {
                        dl.mod_project_type = "mod".to_string();
                        dl.mod_target_use_server = true;
                        dl.mod_target_dir = self.cfg.servers[idx].dir.display().to_string();
                        if let Some(loader) = pinfo.kind.modrinth_loader() {
                            dl.mod_loader = loader.to_string();
                        }
                        if let Some(v) = &pinfo.mc_version {
                            dl.mod_mc_version = v.clone();
                        }
                        self.nav = Nav::Download;
                    } else {
                        self.set_toast("下载功能未启用".to_string());
                    }
                }
                if ui
                    .button("🔍 排查客户端模组")
                    .on_hover_text("静态解析 mods 下 .jar 的 fabric.mod.json / mods.toml，标出仅客户端模组（启发式，仅供参考）")
                    .clicked()
                {
                    self.runtimes[idx].file_client_mods_confirm = true;
                }
            }
            ui.separator();
            if ui.button("🔄 刷新").clicked() {
                self.refresh_file_list(idx);
            }
            if ui.button("📂 打开当前目录").clicked() {
                let p = self.file_tab_target(idx);
                self.open_folder(&p);
            }
        });
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                TextEdit::singleline(&mut self.runtimes[idx].file_search)
                    .frame(false)
                    .hint_text("Search...")
                    .desired_width(260.0),
            );
            if !self.runtimes[idx].file_search.is_empty() && ui.button("清除").clicked() {
                self.runtimes[idx].file_search.clear();
            }
        });
        ui.separator();

        // 若首次进入，刷新列表
        if self.runtimes[idx].file_list.is_empty() {
            self.refresh_file_list(idx);
        }

        // 子目录导航：面包屑 + 上级按钮
        ui.horizontal(|ui| {
            if !self.runtimes[idx].file_sub.is_empty() {
                if ui.button("⬆ 上级").clicked() {
                    self.runtimes[idx].file_sub.pop();
                    self.refresh_file_list(idx);
                }
            }
            let sub = self.runtimes[idx].file_sub.clone();
            if sub.is_empty() {
                ui.label(RichText::new("根目录").weak());
            } else {
                ui.label(sub.join(" / "));
            }
        });

        let avail_w = ui.available_size().x;
        let avail_h = ui.available_height();
        let left_w = (avail_w * 0.58).clamp(380.0, 900.0);
        ui.horizontal_top(|ui| {
            // 左：文件列表（高度取当前可用高度，否则内层 ScrollArea 视口=内容高度、永不滚动）
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, avail_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("file_list_scroll", idx))
                        .auto_shrink([false, false])
                        .max_height(avail_h)
                        .show(ui, |ui| {
                            let list = self.runtimes[idx].file_list.clone();
                            if list.is_empty() {
                                ui.label("目录为空或不存在（datapack 页签指向 world\\datapacks");
                            }
                            let q = self.runtimes[idx].file_search.trim().to_lowercase();
                            let mut shown: Vec<(String, u64, bool)> = list
                                .iter()
                                .filter(|(n, _, _)| q.is_empty() || n.to_lowercase().contains(&q))
                                .cloned()
                                .collect();
                            // 收藏的文件置顶；其后客户端模组置顶（优先级低于收藏）；再按名称列头方向排序
                            let favs = self.runtimes[idx].file_favs.clone();
                            let client_mods = self.runtimes[idx]
                                .file_client_mods
                                .clone()
                                .unwrap_or_default();
                            shown.sort_by(|a, b| {
                                let af = favs.contains(&a.0);
                                let bf = favs.contains(&b.0);
                                if af != bf {
                                    return bf.cmp(&af);
                                }
                                // 客户端模组置顶（收藏组内/非收藏组内均排前，优先级低于收藏）
                                let ac = client_mods.contains(&a.0);
                                let bc = client_mods.contains(&b.0);
                                if ac != bc {
                                    return bc.cmp(&ac);
                                }
                                // 文件夹始终置顶（无论名称正序/倒序）
                                if a.2 != b.2 {
                                    return b.2.cmp(&a.2);
                                }
                                if self.file_sort_asc {
                                    a.0.cmp(&b.0)
                                } else {
                                    b.0.cmp(&a.0)
                                }
                            });
                            egui::Grid::new(("file_grid", idx))
                                .striped(false)
                                .num_columns(4)
                                .show(ui, |ui| {
                                    ui.label("");
                                    let head = ui
                                        .button(
                                            RichText::new(if self.file_sort_asc { "名称 ▲" } else { "名称 ▼" }).strong(),
                                        )
                                        .on_hover_text("点击切换名称排序方向");
                                    if head.clicked() {
                                        self.file_sort_asc = !self.file_sort_asc;
                                        self.refresh_file_list(idx);
                                    }
                                    ui.label(RichText::new("大小").strong());
                                    ui.label(RichText::new("操作").strong());
                                    ui.end_row();
                                    for (name, size, is_dir) in &shown {
                                        let fav = favs.contains(name);
                                        if *is_dir {
                                            if ui.button(if fav { "★" } else { "☆" }).on_hover_text("收藏/取消收藏文件夹").clicked() {
                                                if fav {
                                                    self.runtimes[idx].file_favs.retain(|x| x != name);
                                                } else {
                                                    self.runtimes[idx].file_favs.push(name.clone());
                                                }
                                                self.set_toast(if fav { "已取消收藏" } else { "已收藏" }.to_string());
                                            }
                                        } else if ui.button(if fav { "★" } else { "☆" }).on_hover_text("收藏/取消收藏").clicked() {
                                            if fav {
                                                self.runtimes[idx].file_favs.retain(|x| x != name);
                                            } else {
                                                self.runtimes[idx].file_favs.push(name.clone());
                                            }
                                            self.set_toast(if fav { "已取消收藏" } else { "已收藏" }.to_string());
                                        }
                                        let display = if *is_dir {
                                            format!("📁 {name}")
                                        } else {
                                            name.clone()
                                        };
                                        // 客户端模组标橙显示（排查结果存在时）
                                        let display_text = if !*is_dir && client_mods.contains(name) {
                                            RichText::new(display).color(self.fg(Color32::from_rgb(255, 170, 60)))
                                        } else {
                                            RichText::new(display)
                                        };
                                        if *is_dir {
                                            let sel = self
                                                .runtimes[idx]
                                                .file_preview
                                                .as_ref()
                                                .map(|(n, _, _, _)| n == name)
                                                .unwrap_or(false);
                                            let resp = ui
                                                .selectable_label(sel, display_text)
                                                .on_hover_text("单击预览目录信息，双击进入目录，右键更多操作");
                                            // 滚动定位：来自 Spark 分析页「📍 定位」跳转
                                            if self.runtimes[idx].file_scroll_to.as_deref() == Some(name.as_str()) {
                                                resp.scroll_to_me(Some(egui::Align::Center));
                                                self.runtimes[idx].file_scroll_to = None;
                                            }
                                            if resp.clicked() {
                                                let target = self.file_tab_target(idx);
                                                let (sz, fc) = dir_stats(&target.join(name));
                                                self.runtimes[idx].file_preview = Some((name.clone(), sz, true, fc));
                                            }
                                            if resp.double_clicked() {
                                                self.runtimes[idx].file_sub.push(name.clone());
                                                self.runtimes[idx].file_preview = None; // 进入目录后无选中项，清空预览
                                                self.refresh_file_list(idx);
                                            }
                                            resp.context_menu(|ui| {
                                                if ui.button("📂 进入目录").clicked() {
                                                    self.runtimes[idx].file_sub.push(name.clone());
                                                    self.runtimes[idx].file_preview = None;
                                                    self.refresh_file_list(idx);
                                                    ui.close_menu();
                                                }
                                                if ui.button("✏ 重命名").clicked() {
                                                    self.runtimes[idx].file_rename = Some(name.clone());
                                                    self.runtimes[idx].file_rename_draft = name.clone();
                                                    ui.close_menu();
                                                }
                                                if ui.button("📂 打开所在目录").clicked() {
                                                    let target = self.file_tab_target(idx);
                                                    self.open_file_location(&target.join(name));
                                                    ui.close_menu();
                                                }
                                            });
                                        } else {
                                            let sel = self
                                                .runtimes[idx]
                                                .file_preview
                                                .as_ref()
                                                .map(|(n, _, _, _)| n == name)
                                                .unwrap_or(false);
                                            let is_client = client_mods.contains(name);
                                            let resp = ui
                                                .selectable_label(sel, display_text)
                                                .on_hover_text(if is_client {
                                                    "疑似客户端模组（已标橙置顶，仅供参考）。单击预览，双击用系统默认程序打开，右键更多操作"
                                                } else {
                                                    "单击预览，双击用系统默认程序打开，右键更多操作"
                                                });
                                            // 滚动定位：来自 Spark 分析页「📍 定位」跳转
                                            if self.runtimes[idx].file_scroll_to.as_deref() == Some(name.as_str()) {
                                                resp.scroll_to_me(Some(egui::Align::Center));
                                                self.runtimes[idx].file_scroll_to = None;
                                            }
                                            if resp.clicked() {
                                                self.runtimes[idx].file_preview = Some((name.clone(), *size, false, 0));
                                            }
                                            if resp.double_clicked() {
                                                let target = self.file_tab_target(idx);
                                                self.open_file_system(&target.join(name));
                                            }
                                            resp.context_menu(|ui| {
                                                let target = self.file_tab_target(idx);
                                                let p = target.join(name);
                                                if ui.button("📂 打开（系统默认程序）").clicked() {
                                                    self.open_file_system(&p);
                                                    ui.close_menu();
                                                }
                                                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                                                let editable = matches!(
                                                    ext.as_str(),
                                                    "txt" | "json" | "jsonc" | "json5" | "yml" | "yaml" | "toml"
                                                        | "properties" | "mcmeta" | "lang" | "cfg" | "conf" | "log"
                                                        | "xml" | "ini" | "snbt" | "mcfunction" | "md" | "csv"
                                                        | "bat" | "cmd" | "sh" | "ps1" | "svg"
                                                );
                                                if editable && ui.button("✏ 编辑").clicked() {
                                                    self.open_file_edit(idx, name);
                                                    ui.close_menu();
                                                }
                                                if ui.button("✏ 重命名").clicked() {
                                                    self.runtimes[idx].file_rename = Some(name.clone());
                                                    self.runtimes[idx].file_rename_draft = name.clone();
                                                    ui.close_menu();
                                                }
                                                if ui.button("📂 打开所在目录").clicked() {
                                                    self.open_file_location(&p);
                                                    ui.close_menu();
                                                }
                                            });
                                        }
                                        ui.label(fmt_size(*size));
                                        if *is_dir {
                                            if ui.button("📂 打开").clicked() {
                                                self.runtimes[idx].file_sub.push(name.clone());
                                                self.runtimes[idx].file_preview = None;
                                                self.refresh_file_list(idx);
                                            }
                                            if ui
                                                .button("🗑 删除")
                                                .on_hover_text("删除到回收站（需二次确认，不会永久删除）")
                                                .clicked()
                                            {
                                                self.runtimes[idx].file_delete_confirm = Some(name.clone());
                                            }
                                        } else {
                                            let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                                            let editable = matches!(
                                                ext.as_str(),
                                                "txt" | "json" | "jsonc" | "json5" | "yml" | "yaml" | "toml"
                                                    | "properties" | "mcmeta" | "lang" | "cfg" | "conf" | "log"
                                                    | "xml" | "ini" | "snbt" | "mcfunction" | "md" | "csv"
                                                    | "bat" | "cmd" | "sh" | "ps1" | "svg"
                                            );
                                            let in_mods = self.runtimes[idx].file_tab == "mods";
                                            let is_jar = in_mods && ext == "jar";
                                            let is_jar_disabled =
                                                in_mods && ext == "disabled" && name.to_lowercase().ends_with(".jar.disabled");
                                            if is_jar || is_jar_disabled {
                                                // mods 内 .jar：禁用 = 追加 .disabled；已禁用 = 启用（去掉 .disabled）
                                                let btn = if is_jar_disabled { "✅ 启用" } else { "🚫 禁用" };
                                                if ui
                                                    .button(btn)
                                                    .on_hover_text(if is_jar_disabled {
                                                        "恢复为 a.jar（模组重新生效）"
                                                    } else {
                                                        "重命名为 a.jar.disabled（模组不再加载）"
                                                    })
                                                    .clicked()
                                                {
                                                    let target = self.file_tab_target(idx);
                                                    let old_p = target.join(name);
                                                    let new_name = if is_jar_disabled {
                                                        name[..name.len() - ".disabled".len()].to_string()
                                                    } else {
                                                        format!("{name}.disabled")
                                                    };
                                                    match std::fs::rename(&old_p, target.join(&new_name)) {
                                                        Ok(_) => {
                                                            if let Some((pn, _, _, _)) = &mut self.runtimes[idx].file_preview {
                                                                if pn == name {
                                                                    *pn = new_name.clone();
                                                                }
                                                            }
                                                            if let Some(fv) = self.runtimes[idx]
                                                                .file_favs
                                                                .iter_mut()
                                                                .find(|x| **x == *name)
                                                            {
                                                                *fv = new_name.clone();
                                                            }
                                                            self.refresh_file_list(idx);
                                                            self.set_toast(if is_jar_disabled {
                                                                format!("已启用模组「{name}」")
                                                            } else {
                                                                format!("已禁用模组「{name}」")
                                                            });
                                                        }
                                                        Err(e) => {
                                                            self.set_toast(format!("操作失败: {e}"));
                                                        }
                                                    }
                                                }
                                                // PCL 式更新：Modrinth 指纹检测 -> 下载替换
                                                let is_update_target =
                                                    &self.runtimes[idx].mod_update_target == name;
                                                if is_update_target
                                                    && self.runtimes[idx].mod_update_pending.is_some()
                                                {
                                                    if ui.button("⬇ 更新").clicked() {
                                                        self.mod_update_apply(idx);
                                                    }
                                                } else if ui
                                                    .button("🔄 更新")
                                                    .on_hover_text("通过 Modrinth 指纹检查并更新到最新版本")
                                                    .clicked()
                                                {
                                                    self.mod_update_check(idx, name, is_jar_disabled);
                                                }
                                                if is_update_target && !self.runtimes[idx].mod_update_msg.is_empty() {
                                                    ui.label(
                                                        RichText::new(self.runtimes[idx].mod_update_msg.clone())
                                                            .weak()
                                                            .small(),
                                                    );
                                                }
                                            } else if editable {
                                                if ui.button("编辑").clicked() {
                                                    self.open_file_edit(idx, name);
                                                }
                                            } else if ui
                                                .button("打开")
                                                .on_hover_text("用系统默认程序打开（如 .jar → 7zip）")
                                                .clicked()
                                            {
                                                let target = self.file_tab_target(idx);
                                                let p = target.join(name);
                                                self.open_file_system(&p);
                                            }
                                            if ui
                                                .button("🗑 删除")
                                                .on_hover_text("删除到回收站（需二次确认，不会永久删除）")
                                                .clicked()
                                            {
                                                self.runtimes[idx].file_delete_confirm = Some(name.clone());
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                },
            );
            // 右：文件预览 + 扩展区
            ui.vertical(|ui| {
                ui.label(RichText::new("文件预览").strong());
                ui.separator();
                if let Some((name, size, is_dir, file_count)) = self.runtimes[idx].file_preview.clone() {
                    let target = self.file_tab_target(idx);
                    let p = target.join(&name);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("名称:").strong());
                        ui.label(&name);
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("大小:").strong());
                        ui.label(if is_dir {
                            format!("{}（{} 个文件）", fmt_size(size), file_count)
                        } else {
                            fmt_size(size)
                        });
                    });
                    if is_dir {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("包含文件:").strong());
                            ui.label(format!("{file_count} 个"));
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("路径:").strong());
                        ui.label(p.to_string_lossy().to_string());
                    });
                    ui.horizontal(|ui| {
                        if is_dir {
                            if ui.button("📂 进入目录").clicked() {
                                self.runtimes[idx].file_sub.push(name.clone());
                                self.runtimes[idx].file_preview = None; // 进入后无选中项，清空预览，避免对同一目录反复进入
                                self.refresh_file_list(idx);
                            }
                        } else {
                            if ui.button("📂 打开所在目录").on_hover_text("在资源管理器中定位该文件").clicked() {
                                self.open_file_location(&p);
                            }
                            if ui.button("✏ 重命名").on_hover_text("重命名该文件（自动校验非法格式）").clicked() {
                                self.runtimes[idx].file_rename = Some(name.clone());
                                self.runtimes[idx].file_rename_draft = name.clone();
                            }
                            if ui.button("打开").on_hover_text("用系统默认程序打开（如 .jar → 7zip）").clicked() {
                                self.open_file_system(&p);
                            }
                        }
                    });
                    ui.separator();
                    // 目录不预览内容；文本文件：显示开头内容
                    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                    let text_like = matches!(
                        ext.as_str(),
                        "txt" | "json" | "jsonc" | "json5" | "yml" | "yaml" | "toml" | "properties"
                            | "mcmeta" | "lang" | "cfg" | "conf" | "log" | "xml" | "ini" | "snbt"
                            | "mcfunction" | "md" | "csv" | "bat" | "cmd" | "sh" | "ps1" | "svg"
                            | "java" | "py" | "js" | "ts" | "gradle" | "kts"
                    );
                    if !is_dir && text_like && size <= 512 * 1024 {
                        let content = std::fs::read_to_string(&p).unwrap_or_default();
                        let mut lines: Vec<&str> = content.lines().collect();
                        if lines.len() > 40 {
                            lines.truncate(40);
                        }
                        egui::ScrollArea::vertical()
                            .id_salt(("file_preview_scroll", idx))
                            .auto_shrink([false, false])
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for ln in lines {
                                    ui.label(RichText::new(ln).monospace().size(11.0));
                                }
                            });
                        if content.lines().count() > 40 {
                            ui.label(RichText::new("…（仅预览前 40 行，点击「编辑」查看完整内容）").weak().small());
                        }
                    } else if size > 512 * 1024 {
                        ui.label(RichText::new("（文件较大，不预览内容；可使用「编辑」查看）").weak());
                    } else {
                        ui.label(RichText::new("（二进制文件，不预览内容）").weak());
                    }
                } else {
                    ui.label(RichText::new("点击左侧文件后在右侧预览信息").weak());
                }

            });
        });

        // 文件编辑器弹窗
        if self.runtimes[idx].file_content_edit.is_some() {
            let mut save = false;
            let mut close = false;
            egui::Window::new("文件编辑")
                .resizable(true)
                .collapsible(false)
                .default_size([720.0, 480.0])
                .show(ui.ctx(), |ui| {
                    let is_light = self.theme_is_light();
                    let fg = |c: egui::Color32| if is_light { theme::light_adapt(c) } else { c };
                    let Some((path, content)) = self.runtimes[idx].file_content_edit.as_mut() else {
                        return;
                    };
                    ui.label(RichText::new(path.clone()).color(fg(Color32::from_rgb(120, 180, 255))));
                    egui::ScrollArea::vertical()
                        .id_salt(("file_edit_scroll", idx))
                        .auto_shrink([false, false])
                        .max_height(420.0)
                        .show(ui, |ui| {
                            ui.add(
                                TextEdit::multiline(content)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(24),
                            );
                        });
                    ui.horizontal(|ui| {
                        if ui.button("💾 保存").clicked() {
                            save = true;
                        }
                        if ui.button("关闭").clicked() {
                            close = true;
                        }
                    });
                    if save {
                        self.save_file_edit(idx);
                    } else if close {
                        self.runtimes[idx].file_content_edit = None;
                    }
                });
        }

        // 文件重命名弹窗：旧名存 file_rename（不变），新名编辑存 file_rename_draft（跨帧持久）
        if self.runtimes[idx].file_rename.is_some() {
            let mut close = false;
            let mut err: Option<String> = None;
            egui::Window::new("重命名文件")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    let old_name = self.runtimes[idx].file_rename.clone().unwrap_or_default();
                    ui.label(format!("当前文件名：{old_name}"));
                    ui.horizontal(|ui| {
                        ui.label("新文件名:");
                        ui.add(TextEdit::singleline(&mut self.runtimes[idx].file_rename_draft).desired_width(240.0));
                    });
                    if let Some(e) = &err {
                        ui.label(RichText::new(e).color(self.fg(Color32::from_rgb(230, 120, 120))));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("保存").clicked() {
                            // 新名从跨帧草稿字段取（TextEdit 已把输入写回 file_rename_draft）
                            let new_n = self.runtimes[idx].file_rename_draft.trim().to_string();
                            match validate_windows_filename(&new_n) {
                                Err(e) => err = Some(e),
                                Ok(_) => {
                                    let target = self.file_tab_target(idx);
                                    let old_p = target.join(&old_name);
                                    if new_n != old_name && target.join(&new_n).exists() {
                                        err = Some(format!("「{new_n}」已存在"));
                                    } else {
                                        match std::fs::rename(&old_p, target.join(&new_n)) {
                                            Ok(_) => {
                                                if let Some((pn, _, _, _)) = &mut self.runtimes[idx].file_preview {
                                                    if pn == &old_name {
                                                        *pn = new_n.clone();
                                                    }
                                                }
                                                if let Some(fv) = self.runtimes[idx]
                                                    .file_favs
                                                    .iter_mut()
                                                    .find(|x| **x == old_name)
                                                {
                                                    *fv = new_n.clone();
                                                }
                                                self.refresh_file_list(idx);
                                                self.set_toast(format!("已重命名为「{new_n}」"));
                                                close = true;
                                            }
                                            Err(e) => err = Some(format!("重命名失败: {e}")),
                                        }
                                    }
                                }
                            }
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                self.runtimes[idx].file_rename = None;
                self.runtimes[idx].file_rename_draft.clear();
            }
        }

        // 排查客户端模组二次确认（启发式分析，结果仅供参考；复用 egui Window 确认机制）
        if self.runtimes[idx].file_client_mods_confirm {
            let mut close = false;
            let mut do_scan = false;
            egui::Window::new("排查客户端模组")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label("将扫描当前 mods 目录下的所有 .jar 模组元数据");
                    ui.label("（fabric.mod.json 的 environment / mods.toml 的 side），");
                    ui.label("标出标记为「仅客户端」的模组并标橙置顶显示。");
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("⚠ 客户端模组排查为启发式分析，结果不一定正确，请结合经验自行判断。")
                            .color(self.fg(Color32::from_rgb(255, 170, 60))),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                        if ui
                            .button(RichText::new("开始排查").color(self.fg(Color32::from_rgb(120, 230, 160))))
                            .clicked()
                        {
                            do_scan = true;
                            close = true;
                        }
                    });
                });
            if close {
                self.runtimes[idx].file_client_mods_confirm = false;
            }
            if do_scan {
                self.analyze_client_mods(idx);
                let n = self.runtimes[idx]
                    .file_client_mods
                    .as_ref()
                    .map(|v| v.len())
                    .unwrap_or(0);
                self.set_toast(if n > 0 {
                    format!("排查完成：发现 {n} 个疑似客户端模组（已标橙置顶）")
                } else {
                    "排查完成：未发现标记为 client/CLIENT 的模组".to_string()
                });
            }
        }

        // 阶段16④：文件删除二次确认（确认后删到回收站，禁止永久删除）
        if let Some(del_name) = self.runtimes[idx].file_delete_confirm.clone() {
            let mut close = false;
            let mut do_delete = false;
            egui::Window::new("删除文件")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label(RichText::new("确定删除以下文件/目录？").strong());
                    ui.label(RichText::new(&del_name).color(self.fg(Color32::from_rgb(120, 180, 255))));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("删除后将移入系统回收站（可还原），不会永久删除。")
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                        if ui
                            .button(RichText::new("确认删除").color(self.fg(Color32::from_rgb(230, 110, 110))))
                            .clicked()
                        {
                            do_delete = true;
                            close = true;
                        }
                    });
                });
            if close {
                self.runtimes[idx].file_delete_confirm = None;
            }
            if do_delete {
                let target = self.file_tab_target(idx);
                let p = target.join(&del_name);
                match self.delete_to_recycle_bin(&p) {
                    Ok(()) => {
                        // 清理关联状态：收藏、预览、当前子目录栈（删除的正是当前目录时退回上级）
                        self.runtimes[idx].file_favs.retain(|x| x != &del_name);
                        if let Some((pn, _, _, _)) = &self.runtimes[idx].file_preview {
                            if pn == &del_name {
                                self.runtimes[idx].file_preview = None;
                            }
                        }
                        if let Some(top) = self.runtimes[idx].file_sub.last() {
                            if top == &del_name {
                                self.runtimes[idx].file_sub.pop();
                                self.runtimes[idx].file_preview = None;
                            }
                        }
                        self.refresh_file_list(idx);
                        self.set_toast(format!("已删除到回收站：「{del_name}」"));
                    }
                    Err(e) => self.set_toast(format!("删除失败: {e}")),
                }
            }
        }
    }

    fn ui_backup(&mut self, ui: &mut egui::Ui, idx: usize) {
        let sc = self.cfg.servers[idx].clone();
        // 整页滚动：自动功能选项较多，页面内容超出一屏时可滚动
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
        ui.add_space(6.0);
        ui.label(RichText::new("自动功能").strong());
        ui.separator();

        if features::is_enabled(&self.cfg.features, features::BETA_BACKUP) {
            ui.label(RichText::new("自动备份").strong());
            ui.separator();
    
            let mut b = sc.backup.clone();
            ui.checkbox(&mut b.enabled, "启用自动备份（仅服务器运行时触发）");
            ui.horizontal(|ui| {
                ui.label("间隔(分钟):");
                ui.add(egui::DragValue::new(&mut b.interval_min).range(1..=10080));
            });
            ui.horizontal(|ui| {
                ui.label("备份限速 (MB/s, 0=不限):");
                ui.add(egui::DragValue::new(&mut b.throttle_mbps).range(0.0..=1024.0).speed(1.0));
            });
            ui.horizontal(|ui| {
                ui.label("备份列表分组:");
                let mode_label = if b.view_mode == "month" { "按月" } else { "按日" };
                egui::ComboBox::from_id_salt("backup_view_mode")
                    .selected_text(mode_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut b.view_mode, "day".to_string(), "按日");
                        ui.selectable_value(&mut b.view_mode, "month".to_string(), "按月");
                    });
            });
            ui.horizontal(|ui| {
                ui.label("内存风险阈值 (%):");
                ui.add(egui::DragValue::new(&mut b.mem_threshold_percent).range(50..=100));
            });
            ui.horizontal(|ui| {
                ui.label("紧急备份间隔 (分钟):");
                ui.add(egui::DragValue::new(&mut b.mem_interval_min).range(1..=120));
            });
            ui.horizontal(|ui| {
                ui.label("备份内容: ");
                let folders_str = b.folders.join(", ");
                let mut fs = folders_str;
                if ui.add(TextEdit::singleline(&mut fs).desired_width(240.0)).changed() {
                    b.folders = fs.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
            });
            // 阶段5：远端备份目标（本地目录 / UNC / WebDAV URL）
            ui.add_space(4.0);
            ui.label(RichText::new("远端备份目标（可选）").strong());
            ui.label("留空 = 仅本地备份；填本地目录 / UNC 网络共享（如 D:\\backup 或 \\\\nas\\share）自动复制，或 WebDAV URL（http(s)://...）自动上传");
            ui.horizontal(|ui| {
                ui.label("目标:");
                ui.add(
                    TextEdit::singleline(&mut b.remote_target)
                        .desired_width(280.0)
                        .hint_text("D:\\backup 或 \\\\nas\\share 或 https://dav.example.com/backup/"),
                );
            });
            let is_http = b.remote_target.trim().to_lowercase().starts_with("http://")
                || b.remote_target.trim().to_lowercase().starts_with("https://");
            if is_http {
                ui.horizontal(|ui| {
                    ui.label("账号:");
                    ui.add(TextEdit::singleline(&mut b.remote_user).desired_width(160.0));
                });
                ui.horizontal(|ui| {
                    ui.label("密码:");
                    ui.add(TextEdit::singleline(&mut b.remote_password).password(true).desired_width(160.0));
                });
            }
            ui.horizontal(|ui| {
                ui.label("失败重试次数:");
                ui.add(egui::DragValue::new(&mut b.remote_retry).range(0..=5));
            });
            if self.remote_upload_busy.contains(&idx) {
                ui.label(RichText::new("📤 远端转存进行中").color(self.fg(Color32::from_rgb(255, 200, 80))));
            } else if let Some((_, left)) = self.remote_pending.get(&idx) {
                ui.label(
                    RichText::new(format!("📤 远端转存失败，待重试 {left} 次"))
                        .color(self.fg(Color32::from_rgb(255, 160, 80))),
                );
            }
            if b != sc.backup {
                self.cfg.servers[idx].backup = b;
                self.save_config();
                self.set_toast("备份设置已保存".to_string());
            }
            ui.separator();
    
            if ui.button("🔄 立即备份一次").clicked() {
                self.spawn_backup(idx);
                self.set_toast("备份已开始（后台执行，完成后提示".to_string());
            }
            if self.backup_inflight.contains(&idx) {
                ui.label(RichText::new("🔄 备份进行中").color(self.fg(Color32::from_rgb(255, 200, 80))));
            }
            if let Some(lb) = &self.cfg.servers[idx].backup.last_backup {
                ui.label(format!("上次备份: {lb}"));
            }
            ui.separator();
        }


        // ---------- 自动重启 ----------
        ui.label(RichText::new("自动重启").strong());
        ui.label("到点后使用 /stop 关服，进程退出后自动重新启动。手动停止/强杀会取消待执行的重启");
        let mut ar = sc.auto_restart.clone();
        let mut ar_changed = false;
        if ui.checkbox(&mut ar.enabled, "启用自动重启").changed() {
            ar_changed = true;
        }
        if ar.enabled {
            ui.horizontal(|ui| {
                ui.label("模式:");
                let mode_label = if ar.mode == "daily" { "每天固定时刻" } else { "按间隔" };
                egui::ComboBox::from_label("")
                    .selected_text(mode_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut ar.mode, "interval".to_string(), "按间隔");
                        ui.selectable_value(&mut ar.mode, "daily".to_string(), "每天固定时刻");
                    });
            });
            if ar.mode == "daily" {
                ui.horizontal(|ui| {
                    ui.label("每天时刻 (HH:MM):");
                    ui.add(TextEdit::singleline(&mut ar.daily_time).desired_width(70.0));
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label("间隔(分钟):");
                    ui.add(egui::DragValue::new(&mut ar.interval_min).range(1..=10080));
                });
            }
            ui.horizontal(|ui| {
                ui.label("重启前倒计时（秒）:");
                ui.add(egui::DragValue::new(&mut ar.warn_secs).range(3..=300));
            });
            if ar != sc.auto_restart {
                ar_changed = true;
            }
        }
        if ar_changed {
            self.cfg.servers[idx].auto_restart = ar;
            self.save_config();
            self.set_toast("自动重启设置已保存".to_string());
        }
        let ar_cfg = &self.cfg.servers[idx].auto_restart;
        if ar_cfg.enabled {
            if let Some(lr) = &ar_cfg.last_restart {
                ui.label(format!("上次自动重启: {lr}"));
            } else {
                ui.label("尚未自动重启过（下次启动服务器后开始计时）");
            }
            if self.auto_restart_pending.contains(&idx) {
                ui.label(RichText::new("⏳ 自动重启倒计时中").color(self.fg(Color32::from_rgb(255, 200, 80))));
            } else if self.auto_restart_after_stop.contains(&idx) {
                ui.label(RichText::new("⏳ 正在停止，准备自动重启").color(self.fg(Color32::from_rgb(255, 200, 80))));
            }
        }
        ui.separator();

        // ---------- 崩溃重启 ----------
        ui.label(RichText::new("崩溃重启").strong());
        ui.label("进程异常退出（崩溃/强杀/断电，非手动停止）时自动重新拉起。熔断窗口内连续崩溃达到上限后停止，防止故障循环刷日志");
        let mut cr = sc.crash_restart.clone();
        let mut cr_changed = false;
        if ui.checkbox(&mut cr.enabled, "启用崩溃自动重启").changed() {
            cr_changed = true;
        }
        if cr.enabled {
            ui.horizontal(|ui| {
                ui.label("窗口内最大重启次数");
                ui.add(egui::DragValue::new(&mut cr.max_restarts).range(1..=50));
            });
            ui.horizontal(|ui| {
                ui.label("崩溃后等待（秒）:");
                ui.add(egui::DragValue::new(&mut cr.wait_secs).range(1..=300));
            });
            ui.horizontal(|ui| {
                ui.label("熔断窗口(分钟):");
                ui.add(egui::DragValue::new(&mut cr.circuit_minutes).range(1..=240));
            });
            if cr != sc.crash_restart {
                cr_changed = true;
            }
        }
        if cr_changed {
            self.cfg.servers[idx].crash_restart = cr;
            self.save_config();
            self.set_toast("崩溃重启设置已保存".to_string());
        }
        // ★ run.bat 自带的 MAX_RESTARTS 重启循环与工具自重启会**叠加**（用户反馈"一直重启很多次"）。
        // 这里读出来、明确告警，并提供一键同步按钮把 bat 里的上限改成工具的上限。
        if let Some((bat, cur_max)) = read_bat_max_restarts(&sc.dir) {
            let tool_max = self.cfg.servers[idx].crash_restart.max_restarts as i32;
            egui::Frame::none()
                .fill(self.theme_cur.widget_bg)
                .stroke(egui::Stroke::new(1.0, self.theme_cur.stroke))
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("⚠ 检测到 {bat} 自带重启循环：MAX_RESTARTS={cur_max}"))
                            .color(Color32::from_rgb(240, 176, 96)),
                    );
                    ui.label(
                        RichText::new(
                            "它与「崩溃自动重启」叠加时会导致反复重启：建议只保留一处。\
                             可以把 bat 的上限同步为工具的上限（或把 bat 改成 1 等于只用工具重启）。",
                        )
                        .weak()
                        .small(),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!("把 MAX_RESTARTS 同步为 {tool_max}"))
                            .clicked()
                        {
                            match write_bat_max_restarts(&sc.dir, tool_max) {
                                Ok(f) => self.set_toast(format!("已更新 {f} 的 MAX_RESTARTS={tool_max}")),
                                Err(e) => self.set_toast(format!("写入失败：{e}")),
                            }
                        }
                        if ui.button("设为 1（等于只用工具重启）").clicked() {
                            match write_bat_max_restarts(&sc.dir, 1) {
                                Ok(f) => self.set_toast(format!("已更新 {f} 的 MAX_RESTARTS=1")),
                                Err(e) => self.set_toast(format!("写入失败：{e}")),
                            }
                        }
                    });
                });
        }
        // 崩溃重启状态展�?
        let rt = self.runtimes.get(idx);
        if let Some(rt) = rt {
            if rt.crash_count > 0 {
                ui.label(format!(
                    "本窗口已连续崩溃 {} 次（上限 {}），{}",
                    rt.crash_count,
                    self.cfg.servers[idx].crash_restart.max_restarts,
                    if rt.crash_restart_at.is_some() {
                        "等待自动重启中"
                    } else {
                        "已熔断停止自动重启"
                    }
                ));
            }
        }
        ui.separator();

        if features::is_enabled(&self.cfg.features, features::BETA_BACKUP) {
        ui.label(RichText::new("已备份列表").strong());
        let dir = self.cfg.servers[idx].dir.clone();
        let backups = backup::list_backups(&dir);
        if backups.is_empty() {
            ui.label("（暂无备份）");
        } else {
            let mut restore_target: Option<(PathBuf, String)> = None;
            let mut delete_target: Option<(PathBuf, String)> = None;
            // 按日/月分组折叠显示（mtime 格式 %Y-%m-%d %H:%M:%S，取前 10/7 位）
            let view_mode = self.cfg.servers[idx].backup.view_mode.clone();
            let group_key = |b: &backup::BackupInfo| -> String {
                if view_mode == "month" {
                    b.mtime.chars().take(7).collect()
                } else {
                    b.mtime.chars().take(10).collect()
                }
            };
            let mut groups: Vec<(String, Vec<&backup::BackupInfo>)> = Vec::new();
            for b in &backups {
                let key = group_key(b);
                match groups.iter_mut().find(|(k, _)| k == &key) {
                    Some((_, v)) => v.push(b),
                    None => groups.push((key, vec![b])),
                }
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(320.0).show(ui, |ui| {
                for (key, items) in groups {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("{key}（{} 个备份）", items.len())).strong(),
                    )
                    .default_open(items.len() <= 5)
                    .show(ui, |ui| {
                        for b in items {
                            ui.horizontal(|ui| {
                                let kind_color = match b.kind {
                                    backup::BackupKind::Full => self.fg(Color32::from_rgb(80, 200, 120)),
                                    backup::BackupKind::Incremental => self.fg(Color32::from_rgb(120, 180, 255)),
                                };
                                ui.label(RichText::new(format!("[{}]", b.kind.label())).color(kind_color));
                                ui.label(format!("{}  ({}, {})", b.name, fmt_size(b.size), b.mtime));
                                if ui.button("回退").clicked() {
                                    restore_target = Some((b.path.clone(), b.name.clone()));
                                }
                                if ui.button("删除").clicked() {
                                    delete_target = Some((b.path.clone(), b.name.clone()));
                                }
                            });
                        }
                    });
                }
            });
            if let Some((zip, name)) = restore_target {
                self.confirm_restore = Some((idx, zip, name));
            }
            if let Some((zip, name)) = delete_target {
                self.confirm_delete_backup = Some((idx, zip, name));
            }
        }
        }
            });
    }

    fn ui_tunnel(&mut self, ctx: &egui::Context) {
        // 左侧栏（与服务器列表同风格）：仪表盘 / 创建隧道 / 隧道管理 / 隧道日志 / 教程
        egui::SidePanel::left("tunnel_side")
            .resizable(true)
            .default_width(170.0)
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(ctx.style().visuals.window_fill))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new("内网穿透").strong());
                        ui.separator();
                        let items: [(&str, TunnelSide); 5] = [
                            ("📊 仪表盘", TunnelSide::Dashboard),
                            ("➕ 创建隧道", TunnelSide::Create),
                            ("📡 隧道管理", TunnelSide::Manage),
                            ("📜 隧道日志", TunnelSide::Logs),
                            ("📖 教程", TunnelSide::Tutorial),
                        ];
                        // 子页签切换动效：与主侧栏同款（滑块位置插值 + 悬停底色过渡），
                        // 此前是 selectable_label 硬切换，用户反馈"内网穿透内部切换没有平滑动效"。
                        let idx_of = |s: TunnelSide| items.iter().position(|(_, x)| *x == s).unwrap_or(0);
                        let target = idx_of(self.tunnel_side) as f32;
                        if self.cfg.ui_animations {
                            self.tunnel_nav_anim += (target - self.tunnel_nav_anim) * 0.25;
                            if (self.tunnel_nav_anim - target).abs() < 0.02 {
                                self.tunnel_nav_anim = target;
                            } else {
                                ctx.request_repaint_after(std::time::Duration::from_millis(16));
                            }
                        } else {
                            self.tunnel_nav_anim = target;
                        }
                        let row_h = 24.0f32;
                        let mut rects: Vec<egui::Rect> = Vec::with_capacity(items.len());
                        let mut clicked: Option<TunnelSide> = None;
                        for (label, side) in items {
                            let active = self.tunnel_side == side;
                            let btn = egui::Button::new("")
                                .frame(false)
                                .min_size(egui::vec2(ui.available_width(), row_h));
                            let resp = ui.add(btn);
                            let hover_t = ctx.animate_bool_with_time(
                                resp.id.with("tunnel_nav_hover"),
                                resp.hovered(),
                                if self.cfg.ui_animations { 0.12 } else { 0.0 },
                            );
                            if !active && hover_t > 0.01 {
                                let acc = self.theme_target.accent;
                                ui.painter().rect_filled(
                                    resp.rect,
                                    5.0,
                                    Color32::from_rgba_unmultiplied(
                                        acc.r(),
                                        acc.g(),
                                        acc.b(),
                                        (18.0 * hover_t) as u8,
                                    ),
                                );
                            }
                            let col = if active {
                                let a = self.theme_cur.accent;
                                self.fg(Color32::from_rgb(a.r(), a.g(), a.b()))
                            } else {
                                self.theme_cur.weak
                            };
                            ui.painter().text(
                                egui::pos2(resp.rect.min.x + 8.0, resp.rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                label,
                                egui::FontId::proportional(13.5),
                                col,
                            );
                            rects.push(resp.rect);
                            if resp.clicked() {
                                clicked = Some(side);
                            }
                            ui.add_space(2.0);
                        }
                        // 选中滑块：在相邻两项之间插值，切换时滑动过去
                        let i0 = self.tunnel_nav_anim.floor().clamp(0.0, (rects.len() - 1) as f32) as usize;
                        let i1 = (i0 + 1).min(rects.len() - 1);
                        let tt = self.tunnel_nav_anim - i0 as f32;
                        if rects.len() > 1 {
                            let a = rects[i0];
                            let b = rects[i1];
                            let y = a.min.y + (b.min.y - a.min.y) * tt;
                            let h = a.height();
                            let sl = egui::Rect::from_min_size(
                                egui::pos2(a.min.x + 2.0, y + 2.0),
                                egui::vec2(a.width() - 4.0, h - 4.0),
                            );
                            let acc = self.theme_target.accent;
                            ui.painter().rect_filled(
                                sl,
                                5.0,
                                Color32::from_rgba_unmultiplied(acc.r(), acc.g(), acc.b(), 30),
                            );
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(sl.min.x, sl.min.y + 2.0),
                                    egui::vec2(2.5, sl.height() - 4.0),
                                ),
                                1.2,
                                acc,
                            );
                        }
                        if let Some(s) = clicked {
                            self.tunnel_side = s;
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // 第二批：max-width 1100 左对齐（内网穿透内容区，ScrollArea 移入容器内）
            let _avail = ui.available_rect_before_wrap();
            let _w = _avail.width().min(1100.0);
            let _centered = egui::Rect::from_min_size(
                egui::pos2(_avail.left(), _avail.top()),
                egui::vec2(_w, _avail.height()),
            );
            let mut _inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(_centered)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .id_salt("page_center_1100"),
            );
            _inner.set_width(_w);
            let ui = &mut _inner;
            egui::ScrollArea::vertical()
                .id_salt("tunnel_content_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    match self.tunnel_side {
                        TunnelSide::Dashboard => self.ui_tunnel_dashboard(ui),
                        TunnelSide::Create => self.ui_tunnel_create(ui),
                        TunnelSide::Manage => self.ui_tunnel_manage(ui),
                        TunnelSide::Logs => self.ui_tunnel_logs(ui),
                        TunnelSide::Tutorial => self.ui_tunnel_tutorial(ui),
                    }
                });
        });

        // 隧道编辑弹窗 / 删除二次确认（置于最上层）
        self.ui_tunnel_edit_window(ctx);
        self.ui_tunnel_confirm_delete(ctx);
        self.draw_toasts(ctx);
    }

    /// 仪表盘：隧道统计 + frpc 状态
    fn ui_tunnel_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("内网穿透仪表盘").strong());
        ui.separator();
        // 隧道统计
        let total = self.cfg.tunnels.len();
        let online = self
            .cfg
            .tunnels
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                self.tunnel_runtimes
                    .get(*i)
                    .and_then(|r| r.proc.as_ref())
                    .map(|p| process::is_running(p))
                    .unwrap_or(false)
            })
            .count();
        if features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC) {
            // 永久累计总流量（写入 TunnelConfig 持久化，跨重启保留）
            let total_in: u64 = self.cfg.tunnels.iter().map(|t| t.traffic_in_total).sum();
            let total_out: u64 = self.cfg.tunnels.iter().map(|t| t.traffic_out_total).sum();
            ui.horizontal(|ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(120, 160, 255, 16))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 160, 255, 60)))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(format!("隧道总数: {total}")).strong());
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("在线隧道: {online}"))
                                .strong()
                                .color(if online > 0 {
                                    self.fg(Color32::from_rgb(120, 220, 120))
                                } else {
                                    self.fg(Color32::from_rgb(200, 200, 200))
                                }),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("累计总流量")
                                    .strong()
                                    .color(self.fg(Color32::from_rgb(255, 200, 120))),
                            );
                            ui.label(format!(
                                "↑ {}　↓ {}　合计 {}",
                                fmt_bytes(total_in),
                                fmt_bytes(total_out),
                                fmt_bytes(total_in.saturating_add(total_out))
                            ));
                        });
                    });
            });
        }
        ui.add_space(8.0);
        // frpc.exe 状态
        let frp_dir = self.data_dir().join("frp");
        if self.frp_local_ver.is_empty() {
            self.frp_local_ver = frp_local_version(&frp_dir).unwrap_or_default();
        }
        let frpc_found = frp_dir.join("frpc.exe").exists();
        ui.label(RichText::new("frpc.exe").strong());
        ui.horizontal(|ui| {
            if frpc_found {
                let ver_txt = if self.frp_local_ver.is_empty() {
                    "已找到 frpc.exe".to_string()
                } else {
                    format!("已找到 frpc.exe（v{}）", self.frp_local_ver)
                };
                ui.label(RichText::new("●").color(self.fg(Color32::from_rgb(120, 220, 120))));
                ui.label(ver_txt);
            } else {
                ui.label(RichText::new("○").color(self.fg(Color32::from_rgb(230, 120, 120))));
                ui.label("未找到 frpc.exe");
            }
            ui.add_space(8.0);
            if !frpc_found {
                if ui
                    .add_enabled(!self.frp_dl_busy, egui::Button::new("⬇ 下载 frpc.exe"))
                    .clicked()
                {
                    self.download_frpc_only();
                }
            } else if ui
                .add_enabled(!self.frp_ver_busy && !self.frp_dl_busy, egui::Button::new("检查更新"))
                .clicked()
            {
                self.check_frp_update();
            }
            if ui
                .add_enabled(!self.frp_dl_busy, egui::Button::new("导入 frpc.exe"))
                .clicked()
            {
                if let Some(f) = rfd::FileDialog::new()
                    .add_filter("frpc", &["exe"])
                    .pick_file()
                {
                    let _ = std::fs::create_dir_all(&frp_dir);
                    let dst = frp_dir.join("frpc.exe");
                    let ok = std::fs::copy(&f, &dst)
                        .map(|_| {
                            self.frp_local_ver = frp_local_version(&frp_dir).unwrap_or_default();
                            self.frp_log.push_str(&format!(
                                "已导入 frpc.exe（{} → {}）\n",
                                f.display(),
                                dst.display()
                            ));
                        })
                        .is_ok();
                    if ok {
                        self.set_toast("frpc.exe 导入成功".to_string());
                    } else {
                        self.set_toast("frpc.exe 导入失败，请检查文件是否被占用".to_string());
                    }
                }
            }
            if self.frp_ver_busy {
                ui.label(RichText::new("检查中").color(self.fg(Color32::from_rgb(120, 200, 255))));
            }
            if self.frp_dl_busy {
                ui.label(RichText::new("下载中").color(self.fg(Color32::from_rgb(120, 200, 255))));
            }
        });
        ui.label(format!("frpc: {}", frp_dir.join("frpc.exe").display()));
        if features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC) {
            // 集中管理实时流量（与独立隧道同样的统计口径：frpc 进程 × serverPort 主连接）
            if self.frp_proc.is_some() {
                let running_frp = self
                    .frp_proc
                    .as_ref()
                    .map(|p| process::is_running(p))
                    .unwrap_or(false);
                if running_frp {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("实时流量:").strong());
                        if self.frp_traffic_in_total > 0 || self.frp_traffic_out_total > 0 {
                            ui.label(format!(
                                "↑ {}/s　↓ {}/s　|　累计 ↑ {}　↓ {}",
                                fmt_bytes(self.frp_traffic_in_rate as u64),
                                fmt_bytes(self.frp_traffic_out_rate as u64),
                                fmt_bytes(self.frp_traffic_in_total),
                                fmt_bytes(self.frp_traffic_out_total)
                            ));
                        } else {
                            ui.label(RichText::new("采样中…（TCP 隧道 2 秒后出数；UDP 隧道不建 TCP 连接无法统计）").weak());
                        }
                    });
                }
            }
        }
        if !self.frp_log.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("frp_log_dash")
                .max_height(140.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    show_colored_log(ui, &self.frp_log, true, false, None);
                });
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("在线隧道（单击预览，双击跳转管理）").strong());
        let mut order: Vec<usize> = (0..self.cfg.tunnels.len()).collect();
        order.sort_by_key(|&i| !self.cfg.tunnels[i].favorited);
        let online_ids: Vec<usize> = order
            .into_iter()
            .filter(|&i| {
                self.tunnel_runtimes
                    .get(i)
                    .and_then(|r| r.proc.as_ref())
                    .map(|p| process::is_running(p))
                    .unwrap_or(false)
            })
            .collect();
        if online_ids.is_empty() {
            ui.label(RichText::new("（当前没有运行中的隧道）").weak());
        } else {
            ui.horizontal(|ui| {
                // 左：在线隧道列表（单击选中预览，双击跳转隧道管理并高亮）
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("dash_tunnel_list")
                        .max_height(300.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for i in online_ids.iter().copied() {
                                let t = &self.cfg.tunnels[i];
                                let name = if t.remark.trim().is_empty() {
                                    t.name.clone()
                                } else {
                                    t.remark.clone()
                                };
                                let ptype = if t.frp_proxy_type.is_empty() {
                                    "tcp".to_string()
                                } else {
                                    t.frp_proxy_type.clone()
                                };
                                let sel = self.tunnel_preview == Some(i);
                                let resp =
                                    ui.selectable_label(sel, format!("▶ {} [{}]", name, ptype));
                                if resp.double_clicked() {
                                    self.tunnel_side = TunnelSide::Manage;
                                    self.tunnel_highlight = Some((i, now_secs_f64() + 3.0));
                                } else if resp.clicked() {
                                    self.tunnel_preview = Some(i);
                                }
                            }
                        });
                });
                ui.separator();
                // 右：只读详情预览（与隧道管理展开内容一致，无修改按钮）
                ui.vertical(|ui| {
                    if let Some(i) = self.tunnel_preview {
                        if i < self.cfg.tunnels.len() {
                            self.tunnel_detail_preview(ui, i);
                        }
                    } else {
                        ui.label(
                            RichText::new("（单击左侧在线隧道查看详情预览）")
                                .weak()
                                .small(),
                        );
                    }
                });
            });
        }
    }

    /// 隧道只读详情预览：与隧道管理页单隧道展开内容一致，但不提供任何修改/控制按钮
    fn tunnel_detail_preview(&mut self, ui: &mut egui::Ui, idx: usize) {
        let t = self.cfg.tunnels[idx].clone();
        let running = self
            .tunnel_runtimes
            .get(idx)
            .and_then(|r| r.proc.as_ref())
            .map(|p| process::is_running(p))
            .unwrap_or(false);
        let ptype = if t.frp_proxy_type.is_empty() {
            "tcp".to_string()
        } else {
            t.frp_proxy_type.clone()
        };
        let (cfg_lp, cfg_rp) = parse_frp_ports(&t.cfg);
        let local_port = if cfg_lp != 0 { cfg_lp } else { t.frp_local_port }.to_string();
        let remote_port = if cfg_rp != 0 { cfg_rp } else { t.frp_remote_port }.to_string();
        let server_addr = parse_frp_server_addr(&t.cfg);
        let display = if t.remark.trim().is_empty() {
            t.name.clone()
        } else {
            t.remark.clone()
        };
        ui.label(RichText::new(format!("{display}（只读预览）")).strong());
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("隧道类型:").strong());
            ui.label(if ptype == "udp" { "UDP" } else { "TCP" });
        });
        let conn = format!(
            "{}:{}",
            if server_addr.is_empty() { "未设置" } else { &server_addr },
            remote_port
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("连接地址:").strong());
            if ui
                .button(conn.clone())
                .on_hover_text("点击复制连接地址")
                .clicked()
            {
                ui.output_mut(|o| o.copied_text = conn.clone());
                self.set_toast(format!("已复制连接地址: {conn}"));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("映射端口:").strong());
            ui.label(local_port);
        });
        if running {
            ui.label(
                RichText::new("🔒 隧道运行中")
                    .color(self.fg(Color32::from_rgb(230, 180, 90))),
            );
        }
        if features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC) {
            // 流量统计：迷你波动图（上行绿 / 下行蓝）+ 汇总，与管理页一致
            {
                let t_total_in = t.traffic_in_total;
                let t_total_out = t.traffic_out_total;
                let hist_in: Vec<f32> = self
                    .tunnel_runtimes
                    .get(idx)
                    .map(|r| r.traffic_hist_in.iter().copied().collect())
                    .unwrap_or_default();
                let hist_out: Vec<f32> = self
                    .tunnel_runtimes
                    .get(idx)
                    .map(|r| r.traffic_hist_out.iter().copied().collect())
                    .unwrap_or_default();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let chart_w = 150.0_f32;
                    let chart_h = 40.0_f32;
                    let (resp, painter) =
                        ui.allocate_painter(egui::vec2(chart_w, chart_h), egui::Sense::hover());
                    let rect = resp.rect;
                    painter.rect_filled(rect, 4.0, self.theme_cur.widget_bg);
                    painter.rect_stroke(
                        rect,
                        4.0,
                        egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
                    );
                    if running && hist_in.len() >= 2 {
                        let plot = rect.shrink(3.0);
                        let n = hist_in.len().max(hist_out.len()) as f32;
                        let max_v = hist_in
                            .iter()
                            .chain(hist_out.iter())
                            .copied()
                            .fold(0.0_f32, f32::max)
                            .max(1.0_f32);
                        let x = |i: usize| plot.left() + (i as f32 / (n - 1.0_f32)) * plot.width();
                        let to_y = |v: f32| plot.bottom() - (v / max_v).min(1.0_f32) * plot.height();
                        let pts_in: Vec<egui::Pos2> = hist_in
                            .iter()
                            .enumerate()
                            .map(|(k, v)| egui::pos2(x(k), to_y(*v)))
                            .collect();
                        let pts_out: Vec<egui::Pos2> = hist_out
                            .iter()
                            .enumerate()
                            .map(|(k, v)| egui::pos2(x(k), to_y(*v)))
                            .collect();
                        painter.add(egui::Shape::line(
                            pts_out,
                            egui::Stroke::new(1.6_f32, Color32::from_rgb(120, 180, 255)),
                        ));
                        painter.add(egui::Shape::line(
                            pts_in,
                            egui::Stroke::new(1.6_f32, Color32::from_rgb(80, 200, 120)),
                        ));
                    } else {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            if running { "采样中…" } else { "未运行" },
                            egui::FontId::proportional(11.0),
                            Color32::from_rgb(120, 125, 130),
                        );
                    }
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("流量统计").strong().small());
                        ui.label(format!(
                            "累计　↑ {}　↓ {}",
                            fmt_bytes(t_total_in),
                            fmt_bytes(t_total_out)
                        ));
                        if running && !hist_in.is_empty() {
                            let avg_in = hist_in.iter().sum::<f32>() / hist_in.len() as f32;
                            let avg_out = hist_out.iter().sum::<f32>() / hist_out.len() as f32;
                            ui.label(format!(
                                "平均速率　↑ {}/s　↓ {}/s",
                                fmt_bytes(avg_in as u64),
                                fmt_bytes(avg_out as u64)
                            ));
                        } else if !running {
                            ui.label(RichText::new("启动后开始统计").weak().small());
                        }
                    });
                });
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("提示：双击左侧隧道可跳转到「隧道管理」定位并编辑")
                .weak()
                .small(),
        );
    }

    /// 创建隧道：简易（表单）/ 高级（frpc.toml）两种模式，支持导入
    fn ui_tunnel_create(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("创建隧道").strong());
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("备注名:");
            ui.add(TextEdit::singleline(&mut self.add_tunnel_name).hint_text("如 我的世界联机"));
        });
        ui.horizontal(|ui| {
            ui.label("穿透内核:");
            ui.radio_value(&mut self.add_tunnel_kind, "frp".to_string(), "Frp（保留兼容）");
            // Rathole 内核为渐进式功能：开关关闭时不显示创建入口（已有 rathole 隧道不受影响）
            if features::is_enabled(&self.cfg.features, features::BETA_RATHOLE) {
                ui.radio_value(&mut self.add_tunnel_kind, "rathole".to_string(), "Rathole（纯 Rust）");
            }
        });
        ui.horizontal(|ui| {
            ui.label("填写方式:");
            ui.radio_value(&mut self.add_tunnel_mode, "form".to_string(), "简易模式");
            ui.radio_value(&mut self.add_tunnel_mode, "toml".to_string(), "高级模式");
        });
        ui.separator();
        if self.add_tunnel_kind == "rathole" {
            // Rathole 内核表单：连接 rathole server + 单隧道转发（阶段5）
            ui.label(RichText::new("rathole 服务器信息").strong());
            ui.horizontal(|ui| {
                ui.label("服务器地址 server_addr:");
                ui.add(TextEdit::singleline(&mut self.add_rh_server_addr).hint_text("如 1.2.3.4"));
            });
            ui.horizontal(|ui| {
                ui.label("服务器端口 server_port:");
                ui.add(egui::DragValue::new(&mut self.add_rh_server_port).range(1..=65535));
            });
            ui.add_space(6.0);
            ui.label(RichText::new("隧道映射").strong());
            ui.horizontal(|ui| {
                ui.label("隧道名 name（服务端须对应）:");
                ui.add(TextEdit::singleline(&mut self.add_rh_tunnel_name).hint_text("留空默认 xmst"));
            });
            ui.horizontal(|ui| {
                ui.label("本地地址 local_addr:");
                ui.add(TextEdit::singleline(&mut self.add_rh_local_addr).hint_text("留空默认 127.0.0.1"));
            });
            ui.horizontal(|ui| {
                ui.label("本地端口 local_port:");
                ui.add(egui::DragValue::new(&mut self.add_rh_local_port).range(1..=65535));
            });
            ui.horizontal(|ui| {
                ui.label("远程端口 remote_port（服务端监听）:");
                ui.add(egui::DragValue::new(&mut self.add_rh_remote_port).range(1..=65535));
            });
            ui.add_space(6.0);
            ui.label(RichText::new("NOISE 加密（可选）").strong());
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.add_rh_noise, "启用 NOISE 加密");
                if self.add_rh_noise {
                    ui.label("（服务端需配置相同 pattern 并配对密钥）");
                }
            });
            if self.add_rh_noise {
                ui.horizontal(|ui| {
                    ui.label("私钥 private_key（留空自动生成）:");
                    ui.add(
                        TextEdit::singleline(&mut self.add_rh_noise_key)
                            .password(true)
                            .hint_text("32 字节 hex"),
                    );
                });
            }
        } else if self.add_tunnel_mode == "form" {
            // 简易模式：数值填写（类似 server.properties 类型化编辑）
            ui.label(RichText::new("frps 服务器信息").strong());
            ui.horizontal(|ui| {
                ui.label("服务器地址 serverAddr:");
                ui.add(TextEdit::singleline(&mut self.add_tunnel_server_addr).hint_text("如 1.2.3.4"));
            });
            ui.horizontal(|ui| {
                ui.label("服务器端口 serverPort:");
                ui.add(egui::DragValue::new(&mut self.add_tunnel_server_port).range(1..=65535));
            });
            ui.horizontal(|ui| {
                ui.label("用户名 user（可选）:");
                ui.add(TextEdit::singleline(&mut self.add_tunnel_user).hint_text("与 frps 端一致"));
            });
            ui.horizontal(|ui| {
                ui.label("认证 token（可选）:");
                ui.add(TextEdit::singleline(&mut self.add_tunnel_token).password(true).hint_text("与 frps 端一致"));
            });
            ui.add_space(6.0);
            ui.label(RichText::new("隧道映射").strong());
            ui.horizontal(|ui| {
                ui.label("隧道类型:");
                ui.radio_value(&mut self.add_tunnel_proxy_type, "tcp".to_string(), "TCP");
                ui.radio_value(&mut self.add_tunnel_proxy_type, "udp".to_string(), "UDP");
            });
            ui.horizontal(|ui| {
                ui.label("映射端口（本机端口 localPort）:");
                ui.add(egui::DragValue::new(&mut self.add_tunnel_local_port).range(1..=65535));
            });
            ui.horizontal(|ui| {
                ui.label("服务器端口（对外 remotePort）:");
                ui.add(egui::DragValue::new(&mut self.add_tunnel_remote_port).range(1..=65535));
            });
        } else {
            // 高级模式：直接编辑 frpc.toml（留空不预填，仅显示虚影提示）
            ui.add(
                TextEdit::multiline(&mut self.add_tunnel_cfg)
                    .desired_rows(14)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("示例：\nserverAddr = \"1.2.3.4\"\nserverPort = 7000\nuser = \"...\"\nauth.method = \"token\"\nauth.token = \"...\"\n\n[[proxies]]\nname = \"mc\"\ntype = \"tcp\"\nlocalIP = \"127.0.0.1\"\nlocalPort = 25565\nremotePort = 25565"),
            );
            ui.horizontal(|ui| {
                if ui.button("📂 导入文件").clicked() {
                    if let Some(f) = rfd::FileDialog::new()
                        .add_filter("frpc.toml", &["toml"])
                        .pick_file()
                    {
                        match std::fs::read_to_string(&f) {
                            Ok(text) => {
                                self.add_tunnel_cfg = text;
                                self.set_toast("已导入 frpc.toml（已复制内容，原文件不受影响）".to_string());
                            }
                            Err(e) => self.set_toast(format!("读取失败: {e}")),
                        }
                    }
                }
            });
        }
        if self.add_tunnel_kind == "frp" {
            ui.horizontal(|ui| {
                ui.label("frpc.exe 路径（留空自动用共享 frpc.exe）");
                ui.add(TextEdit::singleline(&mut self.add_tunnel_exe).hint_text("留空 = data/frp/frpc.exe"));
                if ui.button("浏览").clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        self.add_tunnel_exe = f.to_string_lossy().to_string();
                    }
                }
            });
        }
        ui.add_space(8.0);
        if ui.button(RichText::new("➕ 添加隧道").strong()).clicked() {
            self.add_tunnel();
        }
    }

    /// 隧道管理：折叠/展开、信息展示、配置弹窗、删除二次确认、收藏
    fn ui_tunnel_manage(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        // 高亮过期自动清除（从仪表盘双击跳转后高亮约 3 秒）
        if let Some((_, until)) = self.tunnel_highlight {
            if now_secs_f64() > until {
                self.tunnel_highlight = None;
            }
        }
        ui.label(RichText::new("隧道管理").strong());
        ui.separator();
        if self.cfg.tunnels.is_empty() {
            ui.label("暂无穿透配置，请先在「创建隧道」中添加");
            return;
        }
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                TextEdit::singleline(&mut self.tunnel_search)
                    .frame(false)
                    .hint_text("Search...")
                    .desired_width(240.0),
            );
            if !self.tunnel_search.is_empty() && ui.button("清除").clicked() {
                self.tunnel_search.clear();
            }
        });
        ui.separator();
        // 收藏优先（未收藏保持原有顺序）
        let mut order: Vec<usize> = (0..self.cfg.tunnels.len()).collect();
        order.sort_by_key(|&i| !self.cfg.tunnels[i].favorited);
        let q_search = self.tunnel_search.trim().to_lowercase();
        let mut removed = false;
        for i in order {
            if removed {
                break;
            }
            if !q_search.is_empty() {
                let tq = &self.cfg.tunnels[i];
                let (lq, rq) = parse_frp_ports(&tq.cfg);
                let hay = format!("{} {} {} {}", tq.remark, tq.name, lq, rq).to_lowercase();
                if !hay.contains(&q_search) {
                    continue;
                }
            }
            let running = self
                .tunnel_runtimes
                .get(i)
                .and_then(|r| r.proc.as_ref())
                .map(|p| process::is_running(p))
                .unwrap_or(false);
            let (remark, favorited) = {
                let t = &self.cfg.tunnels[i];
                (t.remark.clone(), t.favorited)
            };
            let display = if remark.trim().is_empty() {
                self.cfg.tunnels[i].name.clone()
            } else {
                remark
            };
            let star = if favorited { "★ " } else { "" };
            let ptype_head = {
                let t = &self.cfg.tunnels[i];
                if t.kind == "rathole" {
                    "rathole".to_string()
                } else if t.frp_proxy_type.is_empty() {
                    "tcp".to_string()
                } else {
                    t.frp_proxy_type.clone()
                }
            };
            // 高亮目标隧道（从仪表盘双击跳转）：展开并着色约 3 秒
            let hl = self
                .tunnel_highlight
                .as_ref()
                .map(|(h, _)| *h == i)
                .unwrap_or(false);
            let header_title = format!(
                "{} {}[{}] {}",
                if running { "▶" } else { "⏹" },
                star,
                ptype_head,
                display
            );
            let header_txt = if hl {
                RichText::new(header_title).color(self.fg(Color32::from_rgb(255, 214, 102)))
            } else {
                RichText::new(header_title)
            };
            let mut ch = egui::CollapsingHeader::new(header_txt).id_salt(("tunnel_mng", i));
            if hl {
                ch = ch.open(Some(true));
            }
            ch.show(ui, |ui| {
                // 闭包内 clone 数据避免借用冲突，随后可安全调用 self 方法
                let t = self.cfg.tunnels[i].clone();
                let running = self
                    .tunnel_runtimes
                    .get(i)
                    .and_then(|r| r.proc.as_ref())
                    .map(|p| process::is_running(p))
                    .unwrap_or(false);
                // 隧道关键信息（从配置解析 / 字段读取）
                if t.kind == "rathole" {
                    // Rathole 内核信息（阶段5）
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("隧道类型:").strong());
                        ui.label("Rathole（纯 Rust）");
                    });
                    let srv = format!(
                        "{}:{}",
                        if t.rh_server_addr.trim().is_empty() { "未设置" } else { t.rh_server_addr.trim() },
                        t.rh_server_port
                    );
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("服务器地址:").strong());
                        if ui.button(srv.clone()).on_hover_text("点击复制").clicked() {
                            ui.output_mut(|o| o.copied_text = srv.clone());
                            self.set_toast(format!("已复制服务器地址: {srv}"));
                        }
                    });
                    let name = if t.rh_tunnel_name.trim().is_empty() {
                        "xmst_t{i}".to_string()
                    } else {
                        t.rh_tunnel_name.trim().to_string()
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("隧道名:").strong());
                        ui.label(name);
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("本地映射:").strong());
                        ui.label(format!(
                            "{}:{} → 远程 :{}",
                            if t.rh_local_addr.trim().is_empty() { "127.0.0.1" } else { t.rh_local_addr.trim() },
                            t.rh_local_port,
                            t.rh_remote_port
                        ));
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("NOISE 加密:").strong());
                        ui.label(if t.rh_noise { "开启" } else { "关闭" });
                    });
                } else {
                    let ptype = if t.frp_proxy_type.is_empty() {
                        "tcp".to_string()
                    } else {
                        t.frp_proxy_type.clone()
                    };
                    let (cfg_lp, cfg_rp) = parse_frp_ports(&t.cfg);
                    let local_port = if cfg_lp != 0 { cfg_lp } else { t.frp_local_port }.to_string();
                    let remote_port = if cfg_rp != 0 { cfg_rp } else { t.frp_remote_port }.to_string();
                    let server_addr = parse_frp_server_addr(&t.cfg);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("隧道类型:").strong());
                        ui.label(if ptype == "udp" { "UDP" } else { "TCP" });
                    });
                    let conn = format!(
                        "{}:{}",
                        if server_addr.is_empty() { "未设置" } else { &server_addr },
                        remote_port
                    );
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("连接地址:").strong());
                        if ui.button(conn.clone()).on_hover_text("点击复制连接地址").clicked() {
                            ui.output_mut(|o| o.copied_text = conn.clone());
                            self.set_toast(format!("已复制连接地址: {conn}"));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("映射端口:").strong());
                        ui.label(local_port);
                    });
                }
                if running {
                    ui.label(RichText::new("🔒 隧道运行中：修改配置后需重启隧道才能生效").color(self.fg(Color32::from_rgb(230, 180, 90))));
                }
                ui.horizontal(|ui| {
                    let start = ui.add_enabled(!running, egui::Button::new("▶ 启动"));
                    if start.clicked() {
                        self.start_tunnel(i);
                    }
                    let stop = ui.add_enabled(running, egui::Button::new("⏹ 停止"));
                    if stop.clicked() {
                        self.stop_tunnel(i);
                    }
                    if ui.button(if favorited { "★ 已收藏" } else { "☆ 收藏" }).clicked() {
                        self.cfg.tunnels[i].favorited = !self.cfg.tunnels[i].favorited;
                        self.save_config();
                    }
                    if ui.button("⚙ 配置").clicked() {
                        self.tunnel_edit_idx = Some(i);
                        self.tunnel_edit_draft = Some(self.cfg.tunnels[i].clone());
                    }
                    if running {
                        if t.kind == "frp" {
                            if ui.button("🔄 热重载").on_hover_text("运行中 frpc 热重载配置（不重启进程）").clicked() {
                                self.frp_reload(i);
                            }
                        }
                        if features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC)
                            && ui.button("🩺 诊断流量").on_hover_text("检查 frpc 进程连接与 EStats 统计是否命中，用于排查流量恒为 0").clicked()
                        {
                            let pid = self
                                .tunnel_runtimes
                                .get(i)
                                .and_then(|r| r.proc.as_ref())
                                .and_then(process::pid);
                            let port = parse_frp_server_port(&t.cfg);
                            let diag = match (pid, port) {
                                (Some(p), Some(pt)) => query_frpc_traffic_diag(p, pt),
                                (None, _) => "无法获取 frpc 进程 PID（隧道可能不是由 XMST 启动）。\n".to_string(),
                                (_, None) => "无法从 frpc.toml 解析 serverPort（请检查配置是否包含 serverPort = 端口）。\n".to_string(),
                            };
                            self.tunnel_diag = Some((i, diag));
                        }
                    }
                    if ui.button("🗑 删除").on_hover_text("按住 Shift 点击可直接删除，无需确认").clicked() {
                        if ui.input(|i| i.modifiers.shift) {
                            self.remove_tunnel(i);
                            self.set_toast("已删除隧道".to_string());
                        } else {
                            self.confirm_remove_tunnel = Some(i);
                        }
                    }
                });
                if features::is_enabled(&self.cfg.features, features::BETA_TRAFFIC) {
                    // ---- 流量统计：迷你波动图（上行绿 / 下行蓝）+ 汇总 ----
                    // 诊断结果显示（若有）
                    if let Some((di, dtxt)) = &self.tunnel_diag {
                        if *di == i {
                            let dtxt = dtxt.clone();
                            ui.add_space(4.0);
                            egui::Frame::group(ui.style())
                                .fill(self.theme_cur.widget_bg)
                                .show(ui, |ui| {
                                    ui.label(RichText::new("🩺 流量诊断").strong().small().color(Color32::from_rgb(220, 224, 232)));
                                    ui.label(RichText::new(dtxt).monospace().size(11.0).color(Color32::from_rgb(200, 205, 215)));
                                });
                        }
                    }
                    {
                        let t_total_in = t.traffic_in_total;
                        let t_total_out = t.traffic_out_total;
                        let hist_in: Vec<f32> = self
                            .tunnel_runtimes
                            .get(i)
                            .map(|r| r.traffic_hist_in.iter().copied().collect())
                            .unwrap_or_default();
                        let hist_out: Vec<f32> = self
                            .tunnel_runtimes
                            .get(i)
                            .map(|r| r.traffic_hist_out.iter().copied().collect())
                            .unwrap_or_default();
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let chart_w = 150.0_f32;
                            let chart_h = 40.0_f32;
                            let (resp, painter) = ui.allocate_painter(
                                egui::vec2(chart_w, chart_h),
                                egui::Sense::hover(),
                            );
                            let rect = resp.rect;
                            painter.rect_filled(rect, 4.0, self.theme_cur.widget_bg);
                            painter.rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
                            );
                            if running && hist_in.len() >= 2 {
                                let plot = rect.shrink(3.0);
                                let n = hist_in.len().max(hist_out.len()) as f32;
                                let max_v = hist_in
                                    .iter()
                                    .chain(hist_out.iter())
                                    .copied()
                                    .fold(0.0_f32, f32::max)
                                    .max(1.0_f32);
                                let x = |idx: usize| {
                                    plot.left() + (idx as f32 / (n - 1.0_f32)) * plot.width()
                                };
                                let to_y = |v: f32| {
                                    plot.bottom() - (v / max_v).min(1.0_f32) * plot.height()
                                };
                                let pts_in: Vec<egui::Pos2> = hist_in
                                    .iter()
                                    .enumerate()
                                    .map(|(k, v)| egui::pos2(x(k), to_y(*v)))
                                    .collect();
                                let pts_out: Vec<egui::Pos2> = hist_out
                                    .iter()
                                    .enumerate()
                                    .map(|(k, v)| egui::pos2(x(k), to_y(*v)))
                                    .collect();
                                painter.add(egui::Shape::line(
                                    pts_out,
                                    egui::Stroke::new(1.6_f32, Color32::from_rgb(120, 180, 255)),
                                ));
                                painter.add(egui::Shape::line(
                                    pts_in,
                                    egui::Stroke::new(1.6_f32, Color32::from_rgb(80, 200, 120)),
                                ));
                            } else {
                                painter.text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    if running { "采样中…" } else { "未运行" },
                                    egui::FontId::proportional(11.0),
                                    Color32::from_rgb(120, 125, 130),
                                );
                            }
                            ui.add_space(10.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new("流量统计").strong().small());
                                ui.label(format!(
                                    "累计　↑ {}　↓ {}",
                                    fmt_bytes(t_total_in),
                                    fmt_bytes(t_total_out)
                                ));
                                if running && !hist_in.is_empty() {
                                    let avg_in = hist_in.iter().sum::<f32>() / hist_in.len() as f32;
                                    let avg_out = hist_out.iter().sum::<f32>() / hist_out.len() as f32;
                                    ui.label(format!(
                                        "平均速率　↑ {}/s　↓ {}/s",
                                        fmt_bytes(avg_in as u64),
                                        fmt_bytes(avg_out as u64)
                                    ));
                                } else if !running {
                                    ui.label(RichText::new("启动后开始统计").weak().small());
                                }
                            });
                        });
                    }
                }
            });
        }
    }

    fn ui_tunnel_logs(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("隧道日志").strong());
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                TextEdit::singleline(&mut self.tunnel_search)
                    .frame(false)
                    .hint_text("Search...")
                    .desired_width(240.0),
            );
            if !self.tunnel_search.is_empty() && ui.button("清除").clicked() {
                self.tunnel_search.clear();
            }
        });
        ui.separator();
        let mut order: Vec<usize> = (0..self.cfg.tunnels.len()).collect();
        order.sort_by_key(|&i| !self.cfg.tunnels[i].favorited);
        let q_search = self.tunnel_search.trim().to_lowercase();
        let mut any = false;
        for i in order {
            if !q_search.is_empty() {
                let tq = &self.cfg.tunnels[i];
                let (lq, rq) = parse_frp_ports(&tq.cfg);
                let hay = format!("{} {} {} {}", tq.remark, tq.name, lq, rq).to_lowercase();
                if !hay.contains(&q_search) {
                    continue;
                }
            }
            let running = self
                .tunnel_runtimes
                .get(i)
                .and_then(|r| r.proc.as_ref())
                .map(|p| process::is_running(p))
                .unwrap_or(false);
            if !running {
                continue;
            }
            any = true;
            let (remark, favorited, proxy_type) = {
                let t = &self.cfg.tunnels[i];
                (t.remark.clone(), t.favorited, t.frp_proxy_type.clone())
            };
            let display = if remark.trim().is_empty() {
                self.cfg.tunnels[i].name.clone()
            } else {
                remark
            };
            let star = if favorited { "★ " } else { "" };
            let ptype = if proxy_type.is_empty() {
                "tcp".to_string()
            } else {
                proxy_type
            };
            egui::CollapsingHeader::new(format!(
                "{} {}[{}] {}",
                "▶",
                star,
                ptype,
                display
            ))
            .id_salt(("tunnel_log_item", i))
            .show(ui, |ui| {
                let (log, scroll_to_end) = if let Some(rt) = self.tunnel_runtimes.get_mut(i) {
                    let log = rt.log_buf.clone();
                    let changed = rt.log_pending > 0;
                    rt.log_pending = 0;
                    (log, changed)
                } else {
                    (String::new(), false)
                };
                let _ = scroll_to_end;
                if log.is_empty() {
                    ui.label(RichText::new("（暂无日志，若长时间无输出请检查 frpc 配置与网络连接）").weak());
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt(("tunnel_log_view", i))
                    .max_height(220.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 最多显示 20 行
                        let mut lines: Vec<&str> = log.lines().collect();
                        if lines.len() > 20 {
                            lines = lines[lines.len() - 20..].to_vec();
                        }
                        for ln in lines {
                            ui.label(RichText::new(ln).monospace().size(12.0));
                        }
                    });
            });
        }
        if !any {
            ui.label(RichText::new("（当前没有运行中的隧道）").weak());
        }
    }

    /// 教程：符合本次修改的新版说明
    fn ui_tunnel_tutorial(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("内网穿透教程（Frp）").strong());
        ui.separator();
        egui::Frame::none()
            .fill(Color32::from_rgba_unmultiplied(120, 160, 255, 14))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 160, 255, 60)))
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("本工具使用 Frp 实现内网穿透。每条隧道是独立配置（完整 frpc.toml）和独立进程").strong());
                ui.add_space(3.0);
                ui.label(RichText::new("【Frp 使用步骤】").strong().color(self.fg(Color32::from_rgb(120, 200, 255))));
                ui.label(RichText::new("1️⃣ 准备 frpc.exe").strong().color(self.fg(Color32::from_rgb(255, 220, 130))));
                ui.label("在「仪表盘」点「⬇ 下载 frpc.exe」自动获取（观察日志进度），也可「导入 frpc.exe」使用自备文件；「检查更新」可升级");
                ui.label(RichText::new("2️⃣ 创建隧道（两种方式）").strong().color(self.fg(Color32::from_rgb(255, 220, 130))));
                ui.label("在「创建隧道」填写备注名。简易模式直接填服务器地址/端口/token/映射端口即可；高级模式可直接编辑 frpc.toml 配置文件，或点「导入文件」导入你自己的 frpc.toml（工具会复制一份到配置目录，原文件不受影响）");
                ui.label(RichText::new("3️⃣ 启动隧道").strong().color(self.fg(Color32::from_rgb(255, 220, 130))));
                ui.label("在「隧道管理」点「▶ 启动」运行；隧道运行时修改配置需要重启才能生效");
                ui.label(RichText::new("✅ 完成后别人访问 服务器IP:remotePort 即可进入你本地的服务器").strong().color(self.fg(Color32::from_rgb(140, 220, 140))));
                ui.add_space(3.0);
                ui.label(RichText::new("注意").strong().color(self.fg(Color32::from_rgb(255, 170, 120))));
                ui.label("每个 Frp 隧道 [[proxies]] 里的 name 应保持不重复；remotePort 请使用 frps 服务器上未被占用的端口");
                ui.add_space(2.0);
            });
    }

    /// 隧道配置弹窗：基础（备注名/映射端口/服务器端口）+ 高级（frpc.toml）
    fn ui_tunnel_edit_window(&mut self, ctx: &egui::Context) {
        let Some(i) = self.tunnel_edit_idx else { return };
        if i >= self.cfg.tunnels.len() {
            self.tunnel_edit_idx = None;
            return;
        }
        let mut close = false;
        let mut save = false;
        let mut edit_advanced = self.tunnel_edit_advanced;
        let running = self
            .tunnel_runtimes
            .get(i)
            .and_then(|r| r.proc.as_ref())
            .map(|p| process::is_running(p))
            .unwrap_or(false);
        // 草稿跨帧复用：仅在打开时从 cfg 拷贝一次，编辑期间不再重建
        // （否则 TextEdit 输入会因下一帧重建被覆盖，表现为"输入框无法输入"）
        if self.tunnel_edit_draft.is_none() {
            let mut draft = self.cfg.tunnels[i].clone();
            // 高级/简易联动：进入时若 cfg 有端口而基础字段为 0，先同步一次
            if draft.frp_local_port == 0 || draft.frp_remote_port == 0 {
                let (l, r) = parse_frp_ports(&draft.cfg);
                if l != 0 {
                    draft.frp_local_port = l;
                }
                if r != 0 {
                    draft.frp_remote_port = r;
                }
            }
            self.tunnel_edit_draft = Some(draft);
        }
        let is_light = self.theme_is_light();
                    let fg = |c: egui::Color32| if is_light { theme::light_adapt(c) } else { c };
        let t = self.tunnel_edit_draft.as_mut().unwrap();
        egui::Window::new(format!("隧道配置（{}）", t.name))
            .collapsible(false)
            .resizable(false)
            .movable(true)
            .show(ctx, |ui| {
                // 简易 / 高级切换
                ui.horizontal(|ui| {
                    ui.radio_value(&mut edit_advanced, false, "简易模式");
                    ui.radio_value(&mut edit_advanced, true, "高级模式");
                });
                ui.separator();
                // 双向联动：渲染前用 frpc.toml 内容同步基础端口（改任一侧，另一侧跟随）
                let (cl, cr) = parse_frp_ports(&t.cfg);
                if cl != 0 {
                    t.frp_local_port = cl;
                }
                if cr != 0 {
                    t.frp_remote_port = cr;
                }
                if !edit_advanced {
                    // 简易模式：类型化字段编辑，改动即时同步到配置
                    ui.horizontal(|ui| {
                        ui.label("隧道备注名:");
                        ui.add(TextEdit::singleline(&mut t.remark).desired_width(220.0));
                    });
                    if t.kind == "rathole" {
                        // Rathole 内核简易编辑（阶段5）
                        ui.horizontal(|ui| {
                            ui.label("服务器地址 server_addr:");
                            ui.add(TextEdit::singleline(&mut t.rh_server_addr).desired_width(180.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("服务器端口 server_port:");
                            ui.add(egui::DragValue::new(&mut t.rh_server_port).range(1..=65535));
                        });
                        ui.horizontal(|ui| {
                            ui.label("隧道名 name:");
                            ui.add(TextEdit::singleline(&mut t.rh_tunnel_name).desired_width(180.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("本地地址 local_addr:");
                            ui.add(TextEdit::singleline(&mut t.rh_local_addr).desired_width(180.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("本地端口 local_port:");
                            ui.add(egui::DragValue::new(&mut t.rh_local_port).range(1..=65535));
                        });
                        ui.horizontal(|ui| {
                            ui.label("远程端口 remote_port:");
                            ui.add(egui::DragValue::new(&mut t.rh_remote_port).range(1..=65535));
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut t.rh_noise, "启用 NOISE 加密");
                        });
                        if t.rh_noise {
                            ui.horizontal(|ui| {
                                ui.label("私钥 private_key:");
                                ui.add(
                                    TextEdit::singleline(&mut t.rh_noise_key)
                                        .password(true)
                                        .desired_width(220.0),
                                );
                            });
                        }
                        ui.label(RichText::new("修改后需重启隧道才能生效").weak().small());
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("frpc.exe 路径（留空自动用共享 frpc.exe）:");
                            ui.add(TextEdit::singleline(&mut t.exe).desired_width(180.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("映射端口（本机 localPort）:");
                            let lp = ui.add(egui::DragValue::new(&mut t.frp_local_port).range(1..=65535));
                            if lp.changed() {
                                t.cfg = set_frp_ports(&t.cfg, t.frp_local_port, t.frp_remote_port);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("服务器端口（对外 remotePort）:");
                            let rp = ui.add(egui::DragValue::new(&mut t.frp_remote_port).range(1..=65535));
                            if rp.changed() {
                                t.cfg = set_frp_ports(&t.cfg, t.frp_local_port, t.frp_remote_port);
                            }
                        });
                        ui.label(RichText::new("修改端口会同步写入 frpc.toml").weak().small());
                    }
                } else if t.kind == "rathole" {
                    ui.label(RichText::new("rathole 内核无 TOML 编辑，请在简易模式中配置各字段。").weak());
                    if t.rh_noise {
                        ui.horizontal(|ui| {
                            ui.label("私钥 private_key:");
                            ui.add(
                                TextEdit::singleline(&mut t.rh_noise_key)
                                    .password(true)
                                    .desired_width(220.0),
                            );
                        });
                    }
                } else {
                    // 高级模式：直接编辑 frpc.toml，改动即时回写基础端口
                    ui.add(
                        TextEdit::multiline(&mut t.cfg)
                            .code_editor()
                            .desired_rows(12)
                            .desired_width(f32::INFINITY),
                    );
                    if ui.add(egui::Button::new("从 frpc.toml 同步端口到基础配置")).clicked() {
                        let (l, r) = parse_frp_ports(&t.cfg);
                        if l != 0 {
                            t.frp_local_port = l;
                        }
                        if r != 0 {
                            t.frp_remote_port = r;
                        }
                    }
                }
                if running {
                    ui.label(RichText::new("🔒 隧道运行中：修改后需重启隧道才能生效").color(fg(Color32::from_rgb(230, 180, 90))));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("保存").clicked() {
                        save = true;
                        close = true;
                    }
                    if ui.button("取消").clicked() {
                        close = true;
                    }
                });
            });
        // 每帧同步模式选择：点击「高级模式」立即持久化，避免下一帧回到简易模式
        self.tunnel_edit_advanced = edit_advanced;
        if close {
            if save {
                let changed_cfg = {
                    let old = &self.cfg.tunnels[i];
                    let t = self.tunnel_edit_draft.as_ref().unwrap();
                    old.remark != t.remark
                        || old.exe != t.exe
                        || old.frp_local_port != t.frp_local_port
                        || old.frp_remote_port != t.frp_remote_port
                        || old.cfg != t.cfg
                        || old.rh_server_addr != t.rh_server_addr
                        || old.rh_server_port != t.rh_server_port
                        || old.rh_tunnel_name != t.rh_tunnel_name
                        || old.rh_local_addr != t.rh_local_addr
                        || old.rh_local_port != t.rh_local_port
                        || old.rh_remote_port != t.rh_remote_port
                        || old.rh_noise != t.rh_noise
                        || old.rh_noise_key != t.rh_noise_key
                };
                if let Some(t) = self.tunnel_edit_draft.take() {
                    self.cfg.tunnels[i] = t;
                }
                self.save_config();
                if running && changed_cfg {
                    let name = self.cfg.tunnels[i].name.clone();
                    self.notify("XMST - 隧道配置已保存", &format!("{name} 运行中，重启隧道后生效"));
                } else {
                    self.set_toast("隧道配置已保存".to_string());
                }
            }
            self.tunnel_edit_draft = None;
            self.tunnel_edit_idx = None;
            self.tunnel_edit_advanced = edit_advanced;
        }
    }

    /// 隧道删除二次确认
    fn ui_tunnel_confirm_delete(&mut self, ctx: &egui::Context) {
        let Some(i) = self.confirm_remove_tunnel else { return };
        if i >= self.cfg.tunnels.len() {
            self.confirm_remove_tunnel = None;
            return;
        }
        let name = self.cfg.tunnels[i].name.clone();
        let mut close = false;
        let mut do_delete = false;
        egui::Window::new("删除隧道")
                .resizable(true)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("确定要删除隧道「{name}」吗？"));
                ui.label(RichText::new("删除后需重新创建；若隧道正在运行会一并停止。").weak());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        close = true;
                    }
                    if ui
                        .button(RichText::new("确认删除").color(self.fg(Color32::from_rgb(230, 120, 120))))
                        .clicked()
                    {
                        do_delete = true;
                        close = true;
                    }
                });
            });
        if close {
            self.confirm_remove_tunnel = None;
        }
        if do_delete {
            self.remove_tunnel(i);
        }
    }

    /// 设置页可折叠区块：左侧折叠箭�?+ 平滑高度动画
    /// 调用方需�?mem::take 移出 settings_sections，结束后放回，避免闭包与 self 借用冲突
    fn setting_section(
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        mut sections: Vec<SettingsSection>,
        id: &'static str,
        title: &str,
        hint: Option<&str>,
        default_open: bool,
        use_anim: bool,
        anim_speed: f32,
        add: impl FnOnce(&mut egui::Ui),
    ) -> Vec<SettingsSection> {
        let idx = match sections.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => {
                sections.push(SettingsSection {
                    id,
                    open: default_open,
                    anim: if default_open { 1.0 } else { 0.0 },
                    last_h: 0.0,
                });
                sections.len() - 1
            }
        };
        // 动画插值（关闭动效时直接跳变，避免展开/折叠仍有延迟卡顿感）
        let target = if sections[idx].open { 1.0 } else { 0.0 };
        let mut anim = sections[idx].anim;
        if use_anim {
            // 动画速度系数 anim_speed（设置页「界面」可调，1.0=默认，越小越慢越平滑）：
            // 展开收敛速率 2.5/s、折叠 4.0/s（乘以 speed）；采用帧率无关指数插值，
            // 保证 60FPS 与低帧率（如 200ms 空闲刷新）下动画推进速度一致，避免"时快时慢"
            let sp = anim_speed.clamp(0.05, 3.0);
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let k_expand = 1.0 - (-2.5 * sp * dt).exp();
            let k_collapse = 1.0 - (-4.0 * sp * dt).exp();
            let k = if target < anim { k_collapse } else { k_expand };
            anim += (target - anim) * k;
            if (anim - target).abs() < 0.02 {
                anim = target;
            } else {
                // 动画未收敛时持续请求重绘，保证插值逐帧推进（否则事件循环空闲时动画会停滞等待输入）
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        } else {
            anim = target;
        }
        sections[idx].anim = anim;

        let open = sections[idx].open;
        // 头部：与隧道管理页（egui 内建 CollapsingHeader）**同款**：
        // 三角箭头 + 标题（无分隔线、无自绘悬停块）——用户反馈"按钮、竖线之类的还是有区别"。
        ui.horizontal(|ui| {
            let arrow = if anim > 0.5 { "⏷" } else { "⏵" };
            if ui
                .selectable_label(false, arrow)
                .on_hover_text("展开/折叠")
                .clicked()
            {
                sections[idx].open = !open;
            }
            if ui
                .selectable_label(false, RichText::new(title).strong())
                .clicked()
            {
                sections[idx].open = !open;
            }
            if let Some(h) = hint {
                ui.label(RichText::new(h).weak().small());
            }
        });
        if anim <= 0.02 {
            return sections;
        }
        let last_h = sections[idx].last_h.max(8.0);
        let max_h = anim * last_h;
        let mut content_h = 0.0_f32;
        // ★ 用与 egui 内建 `CollapsingHeader` 完全相同的做法（分配 + 裁剪子 UI），
        // 不再用 ScrollArea —— ScrollArea 会随高度变化反复重排内容并参与滚动条布局，
        // 这正是"设置折叠始终卡一下、不如隧道管理那边顺滑"的原因。
        let (body_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), max_h),
            egui::Sense::hover(),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(body_rect.intersect(ui.clip_rect()));
        add(&mut child);
        content_h = child.min_rect().height();
        // ★ 折叠"卡一下"的根因：动画期间 `content_h` 取到的是**被 max_height 裁剪后**的高度，
        // 若把它写回 last_h，下一帧的动画目标就变小，于是目标逐帧缩水 → 展开过程一顿一顿。
        // 只在"完全展开"时记录真实内容高度（或首次未知时取一次）。
        if anim >= 0.99 || sections[idx].last_h <= 0.0 {
            sections[idx].last_h = content_h;
        }
        ui.add_space(24.0);
        ui.separator();
        sections
    }

    // ---- Theme system (stage 6) ----

    /// Recompute the target palette from config, advance the interpolation and
    /// apply style + background image to the context. Called every visible frame.
    fn tick_theme(&mut self, ctx: &egui::Context) {
        // 主题模式：日间 / 夜间 / 自定义配色（第三独立模式）。自定义模式按背景 RGB
        // 自动反算文字黑白深浅（见 theme::target_colors 的 luma 分支）。
        let mode_ref: &str = &self.cfg.theme_mode;
        self.theme_target = theme::target_colors(
            mode_ref,
            self.cfg.theme_mode == "custom",
            self.cfg.custom_accent,
            self.cfg.custom_bg,
            self.cfg.custom_highlight,
        );
        if self.cfg.ui_animations {
            // Frame-rate independent exponential interpolation (same formula as
            // the settings collapse animation). k tuned so the cross-fade
            // converges in roughly half a second at anim_speed=1.0.
            let k = 6.0f32 * self.cfg.anim_speed.max(0.05);
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            let f = 1.0 - (-k * dt).exp();
            self.theme_cur = self.theme_cur.lerp(&self.theme_target, f.clamp(0.0, 1.0));
        } else {
            self.theme_cur = self.theme_target;
        }
        let bg_enabled = !self.cfg.bg_image.is_empty() && self.bg_tex.is_some();
        // 背景材质模式：把「桌面捕获底图 + 着色」合成后的**实际可见颜色**交给主题层，
        // 由它自动挑选深/浅前景色 —— 否则在明亮桌面上会出现「浅字 + 亮底」= UI 看不清。
        // 材质浓度：默认跟滑杆走；关闭「配色跟随材质」时抬到 0.60 下限，
        // 保证固定深色主题在明亮桌面上依然可读（否则浅字 + 亮底 = 看不清）。
        let t = self.effective_bg_opacity();
        // ★ 防御：材质模式要求「底图纹理必须存在」。
        // 材质模式下内容面板填充 alpha = 0（靠纹理透出桌面），一旦纹理不存在（刚从托盘恢复、
        // 纹理被丢弃、首帧抓取还没完成），整窗就只剩 clear 色 = **黑屏窗口**。
        // 这里在纹理缺失时把 material 置空 → 主题回落到常规不透明面板，绝不黑屏；
        // 首帧抓取完成后材质自然出现。
        let has_backdrop_tex = self.backdrop.texture().is_some();
        let material = if self.plugin_bg_style != plugins::BgStyle::Default
            && self.cfg.bg_capture_exclusion
            && has_backdrop_tex
        {
            let (mean, _, captures, _, _, _) = self.backdrop.stats();
            if captures > 0 {
                let bg = self.theme_cur.bg;
                let mix = |b: u8, m: u8| {
                    (b as f32 * t + m as f32 * (1.0 - t)).round().clamp(0.0, 255.0) as u8
                };
                // 内容区实际可见色 = 衬底（主题底色 × scrim）叠在材质之上；
                // 前景对比度必须按这个「最终可见色」来定，否则会选错深浅。
                let scrim = self.cfg.bg_content_scrim.clamp(0.0, 0.6);
                let mix2 = |b: u8, m: u8| {
                    (b as f32 * scrim + m as f32 * (1.0 - scrim))
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
                Some(egui::Color32::from_rgb(
                    mix2(bg.r(), mix(bg.r(), mean.0)),
                    mix2(bg.g(), mix(bg.g(), mean.1)),
                    mix2(bg.b(), mix(bg.b(), mean.2)),
                ))
            } else {
                None
            }
        } else {
            None
        };
        let win_r = theme::apply(
            ctx,
            &self.theme_cur,
            self.cfg.round_corners,
            self.cfg.window_round_corners,
            self.cfg.corner_scale,
            self.cfg.window_corner_scale,
            self.cfg.bg_alpha,
            bg_enabled,
            self.plugin_bg_style,
            t,
            material,
            self.cfg.bg_material_auto_contrast,
            self.cfg.bg_content_scrim,
        );
        // 整窗圆角走系统区域（SetWindowRgn）：无边框不透明窗口唯一可靠的圆角方案，
        // 圆角轮廓由系统裁剪（含背景图/面板/内容），不是画在界面上的遮罩。
        self.win_r_points = win_r;
        self.apply_window_round_region(ctx, win_r);
        // 界面字号：以启动时系统 DPI 为基线，按 scaled_font(1.0, ui_font_scale) 缩放
        // （统一辅助函数，11.0..=20.0 钳制与字号比例定义集中在 theme::scaled_font）。
        let want_ppp = self.base_ppp * theme::scaled_font(1.0, self.cfg.ui_font_scale);
        if (ctx.pixels_per_point() - want_ppp).abs() > 0.01 {
            ctx.set_pixels_per_point(want_ppp);
        }
        // Keep repainting at 16ms while the transition is still visible.
        if self.theme_cur.max_diff(&self.theme_target) > 1 {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        // Lazy background texture load (e.g. config loaded before texture ready).
        if self.bg_tex.is_none() && !self.cfg.bg_image.is_empty() {
            self.refresh_bg();
        }
    }

    /// Apply the window outer-corner radius via a Win32 system region.
    /// A rounded region (CreateRoundRectRgn + SetWindowRgn) makes the whole frameless
    /// window render with a rounded silhouette on Windows 10+: content, panels and the
    /// wallpaper are all clipped by the system. Redundant calls are skipped using the
    /// (r_px, w, h) cache. r=0 resets the region (sharp corners).
    fn apply_window_round_region(&mut self, ctx: &egui::Context, r_points: f32) {
        use winapi::shared::windef::RECT;
        use winapi::um::wingdi::CreateRoundRectRgn;
        use winapi::um::winuser::{GetWindowRect, SetWindowRgn};
        let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
        unsafe {
            if hwnd.is_null() {
                return;
            }
            let mut rc: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rc) == 0 {
                return;
            }
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;
            // 供桌面捕获使用（物理像素矩形）
            self.win_rect_px = (rc.left, rc.top, w, h);
            let r_px = (r_points * ctx.pixels_per_point()).round() as i32;
            // F1：窗口位置/大小记忆 —— 几何变化后延迟 ~800ms 落盘（避免拖动过程中每帧写配置）。
            //
            // 两个坑（实测都被用户撞到）：
            // ① `GetWindowRect` 是**外框**矩形（含不可见缩放边框），把它当 inner_size 还原来
            //    每次重启都会变大一圈；且窗口最大化时外框 = 整个屏幕 → 下次启动开出「巨大窗口」。
            //    因此尺寸一律取 egui 的**客户区逻辑尺寸**（`screen_rect`，即 inner_size 语义），
            //    位置仍用外框左上角（winit 的 with_position 就是外框位置）。
            // ② 最大化状态不记尺寸（否则会把屏幕大小当窗口大小存下来）。
            let ppp = ctx.pixels_per_point().max(0.1);
            // 最大化判定：状态位 + 矩形比对显示器工作区（见 window_is_maximized_now 注释）
            let maximized = ctx.input(|i| i.viewport().maximized == Some(true))
                || window_is_maximized_now(hwnd, (rc.left, rc.top, w, h));
            // 拖动/缩放进行中（本帧几何与上一帧不同）不落盘，等稳定 800ms 后再写
            let cur = (rc.left, rc.top, w, h);
            if self.win_saved_rect != cur {
                if self.win_save_at.is_none() {
                    self.win_save_at = Some(std::time::Instant::now());
                } else if self
                    .win_save_at
                    .map(|t| t.elapsed().as_millis() > 800)
                    .unwrap_or(false)
                {
                    self.win_saved_rect = cur;
                    self.win_save_at = None;
                    // 最大化时既不能把屏幕尺寸当窗口尺寸，也不能把最大化位置当普通位置
                    if !maximized {
                        self.cfg.window_pos =
                            [(rc.left as f32 / ppp).round(), (rc.top as f32 / ppp).round()];
                        let sz = ctx.screen_rect().size();
                        if sz.x >= 400.0 && sz.y >= 300.0 {
                            self.cfg.window_size = [sz.x.round(), sz.y.round()];
                        }
                        self.save_config();
                    }
                }
            } else {
                self.win_save_at = None;
            }
            if self.last_win_rgn == (r_px, w, h) {
                return;
            }
            self.last_win_rgn = (r_px, w, h);
            let rgn = if r_px >= 1 {
                CreateRoundRectRgn(0, 0, w + 1, h + 1, r_px * 2, r_px * 2)
            } else {
                std::ptr::null_mut()
            };
            // SetWindowRgn takes ownership of rgn on success; on failure we simply
            // drop it (negligible, and this path is practically unreachable).
            SetWindowRgn(hwnd, rgn, 1);
        }
    }

    /// 确保亚克力噪点纹理已创建（96×96、重复寻址、无 mipmap 的随机灰点）。
    /// 亚克力 = 模糊 + 噪点；本机 DWM accent 不可用，噪点由材质层自己画。
    fn ensure_noise_tex(&mut self, ctx: &egui::Context) {
        if self.noise_tex.is_some() {
            return;
        }
        const N: usize = 96;
        let mut px = Vec::with_capacity(N * N);
        // 简单 LCG：不需要密码学随机，只要稳定且分布均匀
        let mut seed: u32 = 0x9E37_79B9;
        for _ in 0..(N * N) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = ((seed >> 24) & 0xFF) as u8;
            px.push(egui::Color32::from_rgba_unmultiplied(v, v, v, 255));
        }
        let img = egui::ColorImage {
            size: [N, N],
            pixels: px,
        };
        let opts = egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Nearest,
            wrap_mode: egui::TextureWrapMode::Repeat,
            mipmap_mode: None,
        };
        self.noise_tex = Some(ctx.load_texture("xmst_material_noise", img, opts));
    }

    /// Paint the window background on the background layer (behind all panels):
    /// 1) 桌面捕获底图（非默认背景模式）—— 本机唯一可用的半透明/毛玻璃实现；
    /// 2) 用户壁纸（若配置）叠在其上，保持原「cover」铺满行为。
    fn paint_bg(&mut self, ctx: &egui::Context) {
        let rect = ctx.screen_rect();
        let layer = egui::LayerId::background();

        // ---- 1) 桌面捕获底图 ----
        // D11：「抓屏排除」可关闭。关闭时不抓桌面（否则会把本窗口自己抓进底图，形成
        // 自我递归的反馈画面）；此时效果回落到「面板半透明 + DWM 窗口合成」路径
        // —— 在能合成窗口 alpha 的机器上依然有效。
        let capture_on =
            self.plugin_bg_style != plugins::BgStyle::Default && self.cfg.bg_capture_exclusion;
        let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
        if capture_on {
            // 三种模式的观感差异：半透明＝几乎原图轻微模糊；毛玻璃＝高斯模糊；
            // 亚克力＝更强模糊 + 噪点（Windows 亚克力本就是「模糊 + 噪点」两层，
            // 本机 DWM accent 不可用，所以在材质阶段把这一层差异画出来）。
            // blur_px 是在缩小后小图上的盒式模糊半径（点采样 StretchBlt 很快，
            // 质量由这一步补回来）。
            // ★ 「半透明」= 把桌面**原样**透出来、只叠一层底色，所以 ds=1（原生分辨率 /
            //   完全不缩小）且不模糊 —— 这是唯一真正"清晰"的做法。抓屏 60~100ms 全部发生在
            //   工作线程上（见 backdrop.rs），不再卡 UI。
            //   毛玻璃/亚克力才有意缩小 + 盒式模糊；拖动/缩放期间完全冻结（见下方
            //   「拖动/缩放期间的底图策略」），停止后立刻用精细档位补一张。
            let (ds, blur_px, noise): (i32, i32, bool) = match self.plugin_bg_style {
                plugins::BgStyle::Translucent => (1, 0, false),
                plugins::BgStyle::Frosted => (4, 3, false),
                plugins::BgStyle::Acrylic => (8, 4, true),
                plugins::BgStyle::Default => (4, 3, false),
            };
            // 纹理像素上限：ds=1 在 2560×1440 这类窗口下是 3.7M 像素 / 14MB，上传会给
            // UI 线程造成周期性长帧（"时不时闪一下"的来源之一）。按面积把倍率抬到
            // 不超过 4M 像素；清晰度的主要收益来自 ds=4/8 → ds=1/2 这一段。
            const MAX_TEX_PX: f32 = 4_000_000.0;
            let px_area = (self.win_rect_px.2 as f32) * (self.win_rect_px.3 as f32);
            let ds_cap = (px_area / MAX_TEX_PX).sqrt().ceil().max(1.0) as i32;
            let ds = ds.max(ds_cap);
            if !hwnd.is_null() {
                self.backdrop.ensure_exclusion(hwnd as isize, true);
            }
            // ★ 拖动/缩放期间的底图策略（消除闪烁 + 保持跟手）：
            //   上一版拖动中每 ~200ms 重抓一次且切粗档（ds×6）——每次抓屏完成都会上传
            //   一张「清晰度跳变 + 内容滞后于窗口」的新纹理 → 视觉上表现为闪。
            //   改为：拖动/缩放期间**完全冻结**（零抓屏、零上传、无清晰度跳变），
            //   仅用 **UV 偏移**把上一张底图按窗口位移平移（零成本、视觉连续跟手）；
            //   停止后立刻补抓一帧精细档（抓取矩形 == 当前矩形，与偏移后旧图内容对齐，无缝衔接）。
            let rect_now = self.win_rect_px;
            let moving = self.bg_prev_rect != rect_now;
            self.bg_prev_rect = rect_now;
            let now = std::time::Instant::now();
            if moving {
                self.bg_last_move_at = Some(now);
                // 刚结束拖动后的那一帧要立刻补抓（下面 needs_refresh 用）
                self.bg_needs_refresh = true;
            }
            let settling = self
                .bg_last_move_at
                .map(|t| now.duration_since(t) < std::time::Duration::from_millis(160))
                .unwrap_or(false);
            // 保险丝：无论判据为何，冻结都不允许超过 2 秒
            let frozen_too_long = self
                .bg_last_move_at
                .map(|t| now.duration_since(t) > std::time::Duration::from_secs(2))
                .unwrap_or(false);
            // 拖动/缩放中：完全冻结（min_interval 拉长到 1 小时）；停止 160ms 沉降后
            // 立即用精细档位补抓。拖动期间旧纹理靠 UV 偏移跟随，画面不跳、不闪。
            let min_interval = if moving || settling {
                std::time::Duration::from_secs(3600)
            } else if self.bg_needs_refresh || frozen_too_long {
                std::time::Duration::ZERO
            } else {
                std::time::Duration::from_millis(self.bg_cap_interval_ms as u64)
            };
            let updated = self
                .backdrop
                .update(ctx, rect_now, ds, blur_px, min_interval);
            if updated {
                self.bg_needs_refresh = false;
                // 自适应刷新节奏：清晰档（ds=2）单次抓屏可达 ~30ms，固定 120ms 会让静止时
                // 也有约 1/4 的帧被占用。规则：桌面内容在变（均值跳动）→ 120ms 跟住；
                // 内容基本不变 → 逐步放宽到 600ms，省下绝大部分抓屏开销。
                let mean = self.backdrop.stats().0;
                let d = (mean.0 as i32 - self.bg_prev_mean.0 as i32).abs()
                    + (mean.1 as i32 - self.bg_prev_mean.1 as i32).abs()
                    + (mean.2 as i32 - self.bg_prev_mean.2 as i32).abs();
                if d > 6 {
                    self.bg_cap_interval_ms = 120.0;
                } else {
                    self.bg_cap_interval_ms = (self.bg_cap_interval_ms * 1.7).min(600.0);
                }
                self.bg_prev_mean = mean;
            }
            // 材质 UV 偏移：把上次抓取到的桌面图按「窗口相对上次抓取位置的位移」平移，
            // 这样拖动/缩放期间即使完全不抓屏，材质也一直跟着桌面走（零成本、不闪）。
            // 窗口尺寸变化时 UV 域仍按 (0,0)-(1,1) 铺满（拉伸近似，停止后立刻补抓对齐）。
            let (cap_x, cap_y, _cap_w, _cap_h) = self.backdrop.last_rect();
            let (tw, th) = self.backdrop.stats().3;
            let uv_shift = if tw > 0 && th > 0 {
                let dsp = self.backdrop.last_downscale() as f32;
                (
                    (rect_now.0 - cap_x) as f32 / dsp / tw as f32,
                    (rect_now.1 - cap_y) as f32 / dsp / th as f32,
                )
            } else {
                (0.0, 0.0)
            };
            // 诊断：首次抓到 + 之后每 ~40s 记一条，便于用户回传 bg_debug.log 定位
            if updated {
                let n = self.backdrop.stats().2;
                // 前几次都记录（看冷启动/稳态差异），之后每 ~2.4s 记一条
                // （n%20：用于验证「几何变化后底图仍在持续刷新」——冻结 bug 会让它停住）
                if n <= 3 || n % 20 == 0 {
                    let style = self.plugin_bg_style;
                    self.bg_debug_log(style, style, self.effective_bg_opacity(), false, hwnd);
                }
            }
            if let Some(tex) = self.backdrop.texture() {
                let uv = egui::Rect::from_min_max(
                    egui::pos2(uv_shift.0, uv_shift.1),
                    egui::pos2(1.0 + uv_shift.0, 1.0 + uv_shift.1),
                );
                let mut mesh = egui::Mesh::with_texture(tex.id());
                mesh.add_rect_with_uv(
                    rect,
                    uv,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 255),
                );
                let painter = ctx.layer_painter(layer);
                painter.add(egui::Shape::mesh(mesh));
                // 亚克力噪点层（平铺一张重复寻址的小噪点纹理）
                if noise {
                    self.ensure_noise_tex(ctx);
                    if let Some(nt) = self.noise_tex.as_ref() {
                        let reps = (rect.width() / 96.0).max(1.0);
                        let reps_y = (rect.height() / 96.0).max(1.0);
                        let nuv = egui::Rect::from_min_max(
                            egui::pos2(0.0, 0.0),
                            egui::pos2(reps, reps_y),
                        );
                        let mut nm = egui::Mesh::with_texture(nt.id());
                        nm.add_rect_with_uv(
                            rect,
                            nuv,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16),
                        );
                        painter.add(egui::Shape::mesh(nm));
                    }
                }
                // 材质着色：主题底色按材质浓度盖在底图上 —— 这既是「玻璃的颜色」，
                // 也是 theme::apply 用来算前景对比度的那个合成色（两者必须一致）。
                let tint_alpha = (self.effective_bg_opacity() * 255.0).round() as u8;
                if tint_alpha > 0 {
                    let bg = self.theme_cur.bg;
                    painter.rect_filled(
                        rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), tint_alpha),
                    );
                }
            }
        } else {
            // 默认背景、或用户关闭了抓屏排除：撤销「排除抓屏」，避免影响其截屏/共享
            if !hwnd.is_null() {
                self.backdrop.ensure_exclusion(hwnd as isize, false);
            }
        }

        // ---- 2) 用户壁纸（材质模式下滑杆不透明度已决定观感，壁纸会让材质被盖住，故跳过） ----
        if self.plugin_bg_style != plugins::BgStyle::Default {
            return;
        }
        let Some(tex) = self.bg_tex.as_ref() else { return };
        let size = tex.size_vec2();
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let scale = (rect.width() / size.x).max(rect.height() / size.y).max(0.001);
        let dw = size.x * scale;
        let dh = size.y * scale;
        let img_rect = egui::Rect::from_min_size(
            egui::pos2(rect.center().x - dw / 2.0, rect.center().y - dh / 2.0),
            egui::vec2(dw, dh),
        );
        let alpha = self.cfg.bg_alpha.clamp(0.0, 1.0);
        let color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, (alpha * 255.0) as u8);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let mut mesh = egui::Mesh::with_texture(tex.id());
        mesh.add_rect_with_uv(img_rect, uv, color);
        ctx.layer_painter(layer).add(egui::Shape::mesh(mesh));
    }

    /// (Re)load the background image texture from cfg.bg_image on a worker thread.
    /// egui_extras 0.29 `load_image_bytes` decodes bytes -> ColorImage (no ctx);
    /// the texture is then created via `Context::load_texture` (thread-safe).
    fn refresh_bg(&mut self) {
        let p = self.cfg.bg_image.clone();
        if p.is_empty() {
            self.bg_tex = None;
            return;
        }
        let ctx = self.egui_ctx.clone();
        let tex = std::thread::spawn(move || {
            match std::fs::read(&p) {
                Ok(bytes) => egui_extras::image::load_image_bytes(&bytes)
                    .ok()
                    .map(|img| ctx.load_texture("xmst_bg", img, egui::TextureOptions::LINEAR)),
                Err(_) => None,
            }
        })
        .join()
        .ok()
        .flatten();
        self.bg_tex = tex;
    }

    /// Settings page "界面" section: theme mode / presets / custom colors / background / corners.
    fn ui_theme_settings(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new("主题").strong());
        ui.horizontal(|ui| {
            ui.label("主题模式:");
        });
        if ui
            .radio_value(
                &mut self.cfg.theme_mode,
                "light".to_string(),
                "日间",
            )
            .changed()
        {
            self.save_config();
        }
        if ui
            .radio_value(
                &mut self.cfg.theme_mode,
                "dark".to_string(),
                "夜间",
            )
            .changed()
        {
            self.save_config();
        }
        if ui
            .radio_value(
                &mut self.cfg.theme_mode,
                "custom".to_string(),
                "自定义配色",
            )
            .changed()
        {
            self.save_config();
        }
        // 自定义配色作为第三独立主题模式：RGB 即时预览（每帧 target_colors 生效）并自动保存；
        // 文字颜色按背景 RGB 自动反算黑白深浅，保证清晰可读。
        // 界面字号已移至自定义配色下方（见下方「界面字号」块）。
        if self.cfg.theme_mode == "custom" {
            ui.horizontal(|ui| {
                ui.label("强调色 RGB:");
                if ui.add(egui::DragValue::new(&mut self.cfg.custom_accent.0).range(0..=255).speed(1.0)).changed()
                    || ui.add(egui::DragValue::new(&mut self.cfg.custom_accent.1).range(0..=255).speed(1.0)).changed()
                    || ui.add(egui::DragValue::new(&mut self.cfg.custom_accent.2).range(0..=255).speed(1.0)).changed()
                {
                    self.save_config();
                }
            });
            ui.horizontal(|ui| {
                ui.label("背景色 RGB:");
                if ui.add(egui::DragValue::new(&mut self.cfg.custom_bg.0).range(0..=255).speed(1.0)).changed()
                    || ui.add(egui::DragValue::new(&mut self.cfg.custom_bg.1).range(0..=255).speed(1.0)).changed()
                    || ui.add(egui::DragValue::new(&mut self.cfg.custom_bg.2).range(0..=255).speed(1.0)).changed()
                {
                    self.save_config();
                }
            });
            ui.label(RichText::new("配色修改实时生效并自动保存。").weak());
        }
        // 界面字号：位于自定义配色下方（已从主题模式与配色之间移至此）
        ui.label("界面字号:");
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                // 拖动只改待应用值，点「应用」才真正生效并落盘（避免实时缩放）。
                ui.add(
                    egui::Slider::new(&mut self.pending_font_scale, 11.0..=20.0)
                        .fixed_decimals(1)
                        .show_value(true),
                );
                ui.horizontal(|ui| {
                    if ui.button("应用").clicked() {
                        self.cfg.ui_font_scale = self.pending_font_scale.clamp(11.0, 20.0);
                        self.save_config();
                    }
                    if ui.button("重置").clicked() {
                        self.pending_font_scale = 14.0;
                        self.cfg.ui_font_scale = 14.0;
                        self.save_config();
                    }
                    ui.label(
                        RichText::new("预览按当前滑杆值实时绘制")
                            .weak()
                            .small(),
                    );
                });
            });
            // 实时字号预览：直接按滑杆比例绘制，方便判断多大字号合适。
            // 预览区的字号 = 基准字号 × 滑杆/默认，与真实界面同一换算（theme::scaled_font）。
            let k = self.pending_font_scale.clamp(11.0, 20.0) / 14.0;
            egui::Frame::none()
                .fill(self.theme_cur.widget_bg)
                .stroke(egui::Stroke::new(1.0, self.theme_cur.stroke))
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_min_width(220.0);
                    let base = 14.0 * k;
                    ui.label(RichText::new("正文示例").size(theme::scaled_font(base, 14.0)));
                });
        });
        ui.separator();
        ui.label(RichText::new("背景图").strong());
        ui.horizontal(|ui| {
            if ui.button("选择背景图片...").clicked() {
                if let Some(f) = rfd::FileDialog::new()
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                    .pick_file()
                {
                    self.cfg.bg_image = f.to_string_lossy().to_string();
                    self.refresh_bg();
                    self.save_config();
                }
            }
            if ui.button("编辑背景...").clicked() {
                self.bg_edit_open = true;
            }
            if ui.button("清除背景").clicked() {
                self.cfg.bg_image.clear();
                self.bg_tex = None;
                self.save_config();
            }
        });
        if !self.cfg.bg_image.is_empty() {
            ui.horizontal(|ui| {
                ui.label("透明度:");
                if ui.add(egui::Slider::new(&mut self.cfg.bg_alpha, 0.0..=1.0).fixed_decimals(2)).changed() {
                    self.save_config();
                }
            });
            ui.label(RichText::new(format!("当前: {}", self.cfg.bg_image)).weak());
            ui.label(RichText::new("背景图仅显示在内容区；「编辑背景」可预览调整，ESC 退出。").weak());
        }
        ui.separator();
        ui.label(RichText::new("圆角").strong());
        if ui
            .checkbox(&mut self.cfg.round_corners, "启用控件圆角")
            .changed()
        {
            self.save_config();
        }
        if ui
            .checkbox(&mut self.cfg.window_round_corners, "启用窗口圆角")
            .changed()
        {
            self.save_config();
        }
        if self.cfg.round_corners {
            ui.horizontal(|ui| {
                ui.label("控件圆角幅度:");
                if ui
                    .add(egui::Slider::new(&mut self.cfg.corner_scale, 0.0..=3.0).fixed_decimals(1))
                    .changed()
                {
                    self.save_config();
                }
            });
        }
        if self.cfg.window_round_corners {
            ui.horizontal(|ui| {
                ui.label("窗口圆角幅度:");
                if ui
                    .add(egui::Slider::new(&mut self.cfg.window_corner_scale, 0.0..=3.0).fixed_decimals(1))
                    .changed()
                {
                    self.save_config();
                }
            });
        }
        // F1：窗口位置/大小记忆的重置入口。
        // 记忆功能一旦把「巨大尺寸」写进配置，用户需要一条明确的退路
        // （也方便切显示器/拔掉显示器后把窗口找回来）。
        ui.horizontal(|ui| {
            if ui.button("重置窗口位置与大小").clicked() {
                self.cfg.window_pos = [0.0, 0.0];
                self.cfg.window_size = [0.0, 0.0];
                self.save_config();
                self.win_saved_rect = (0, 0, 0, 0);
                self.win_save_at = None;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1180.0, 760.0)));
                self.set_toast("已重置为 1180×760；下次启动也用默认尺寸".to_string());
            }
            ui.label(
                RichText::new("拖动窗口边缘即可缩放（无边框窗口用四边热区）")
                    .weak()
                    .small(),
            );
        });
    }

    /// Background image editor window (preview + alpha + clear). ESC exits.
    fn ui_bg_editor(&mut self, ctx: &egui::Context) {
        if !self.bg_edit_open {
            return;
        }
        // Keep the open flag in a local so `open(&mut ..)` does not borrow `self`
        // while the contents closure needs `&mut self` (E0500).
        let mut open = self.bg_edit_open;
        egui::Window::new("编辑背景图")
                .resizable(true)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                if !self.cfg.bg_image.is_empty() && self.bg_tex.is_some() {
                    let size = self.bg_tex.as_ref().map(|t| t.size_vec2()).unwrap_or_default();
                    ui.label(format!("已加载背景（{} x {}）", size.x as u32, size.y as u32));
                    let img = egui::load::SizedTexture::new(self.bg_tex.as_ref().unwrap().id(), egui::vec2(380.0, 200.0));
                    ui.add(egui::Image::new(img).fit_to_exact_size(egui::vec2(380.0, 200.0)));
                } else if self.cfg.bg_image.is_empty() {
                    ui.label("尚未选择背景图片，点击下方按钮选择。");
                } else {
                    ui.label("背景图片加载失败，请换一张图片。");
                }
                ui.horizontal(|ui| {
                    if ui.button("选择图片...").clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                            .pick_file()
                        {
                            self.cfg.bg_image = f.to_string_lossy().to_string();
                            self.refresh_bg();
                            self.save_config();
                        }
                    }
                    if ui.button("清除背景").clicked() {
                        self.cfg.bg_image.clear();
                        self.bg_tex = None;
                        self.save_config();
                    }
                    if ui.button("关闭").clicked() {
                        self.bg_edit_open = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("透明度:");
                    if ui
                        .add(egui::Slider::new(&mut self.cfg.bg_alpha, 0.0..=1.0).fixed_decimals(2))
                        .changed()
                    {
                        self.save_config();
                    }
                });
                ui.label(RichText::new("提示：按 ESC 或点击「关闭」退出编辑。").weak());
            });
        // Title-bar close button flips the local `open` flag; sync it back.
        self.bg_edit_open = open;
    }

    fn ui_settings(&mut self, ctx: &egui::Context) {
        // 左侧栏（与服务器列表同风格）：按功能类型分组
        egui::SidePanel::left("settings_side")
            .resizable(true)
            .default_width(150.0)
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(ctx.style().visuals.window_fill))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new("设置").strong());
                        ui.separator();
                        let items: [(&str, SettingsSide, &str); 6] = [
                            ("⚙ 通用", SettingsSide::General, "关闭行为 / 自启 / 数据 / Defender"),
                            ("📜 日志", SettingsSide::Logs, "日志显示行数"),
                            ("🎨 界面", SettingsSide::Ui, "语言 / 动效 / 动画速度"),
                            ("☕ Java", SettingsSide::Java, "JVM 参数 / Java 列表"),
                            ("🔔 通知", SettingsSide::Notify, "右下角通知样式"),
                            ("🧪 测试", SettingsSide::Beta, "测试中的功能开关"),
                        ];
                        for (label, side, hint) in items {
                            let resp = ui.selectable_label(self.settings_side == side, label);
                            if resp.clicked() {
                                self.settings_side = side;
                            }
                            if self.settings_side == side {
                                ui.label(RichText::new(hint).weak().small());
                            }
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // 第二批：max-width 1100 左对齐（设置页内容区，ScrollArea 移入容器内）
            let _avail = ui.available_rect_before_wrap();
            let _w = _avail.width().min(1100.0);
            let _centered = egui::Rect::from_min_size(
                egui::pos2(_avail.left(), _avail.top()),
                egui::vec2(_w, _avail.height()),
            );
            let mut _inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(_centered)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .id_salt("page_center_1100"),
            );
            _inner.set_width(_w);
            let ui = &mut _inner;
            egui::ScrollArea::vertical()
                .id_salt("settings_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // 平滑动画统一由「切换动效」总开关控制（原先设置页另有一个开关，已合并）
                    let use_anim = self.cfg.ui_animations;
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("🔍");
                        ui.add(
                            TextEdit::singleline(&mut self.settings_search)
                                .frame(false)
                                .hint_text("Search...")
                                .desired_width(260.0),
                        );
                        if !self.settings_search.is_empty() && ui.button("清除").clicked() {
                            self.settings_search.clear();
                        }
                    });
                    ui.label(RichText::new("设置").strong());
                    ui.separator();
                    let q_search = self.settings_search.trim().to_lowercase();
                    if !q_search.is_empty() {
                        let matched: Vec<SettingsSide> = [
                            SettingsSide::General,
                            SettingsSide::Logs,
                            SettingsSide::Ui,
                            SettingsSide::Java,
                            SettingsSide::Notify,
                            SettingsSide::Beta,
                        ]
                        .into_iter()
                        .filter(|side| {
                            let kw: &[&str] = match side {
                                SettingsSide::General => {
                                    &["通用", "关闭行为", "托盘", "最小化", "自启", "注册表", "数据", "defender", "排除", "病毒"]
                                }
                                SettingsSide::Logs => &["日志", "显示行数", "自动滚动", "滚动", "行数"],
                                SettingsSide::Ui => {
                                    &["界面", "语言", "中文", "english", "动效", "动画", "速度"]
                                }
                                SettingsSide::Java => &["java", "jvm", "参数", "列表"],
                                SettingsSide::Notify => &["通知", "右下角", "气泡", "侧滑", "系统"],
                                SettingsSide::Beta => {
                                    &["测试", "实验", "beta", "实验性", "功能开关", "崩溃分析", "流量显示", "自动备份"]
                                }
                            };
                            kw.iter().any(|k| q_search.contains(k) || k.contains(q_search.as_str()))
                        })
                        .collect();
                        if matched.is_empty() {
                            ui.label(
                                RichText::new("未找到匹配的设置项，试试「日志 / JVM / 通知 / 托盘」等关键词").weak(),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                ui.label("匹配分组:");
                                for side in &matched {
                                    let lbl = match side {
                                        SettingsSide::General => "⚙ 通用",
                                        SettingsSide::Logs => "📜 日志",
                                        SettingsSide::Ui => "🎨 界面",
                                        SettingsSide::Java => "☕ Java",
                                        SettingsSide::Notify => "🔔 通知",
                                        SettingsSide::Beta => "🧪 测试",
                                    };
                                    if ui.button(lbl).clicked() {
                                        self.settings_side = *side;
                                    }
                                }
                            });
                        }
                        ui.separator();
                    }
                    match self.settings_side {
                        SettingsSide::General => {
                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "close", "关闭行为", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                ui.label("点击窗口关闭按钮（×）时的行为");
                                let mut changed = false;
                                if ui.radio_value(&mut self.cfg.close_behavior, "tray".to_string(), "最小化到托盘：关闭按钮把窗口收进系统托盘，服务器保持运行，可随时从托盘恢复").changed() {
                                    changed = true;
                                }
                                if ui.radio_value(&mut self.cfg.close_behavior, "minimize".to_string(), "最小化（工具常驻，从任务栏恢复").changed() {
                                    changed = true;
                                }
                                if ui.radio_value(&mut self.cfg.close_behavior, "exit".to_string(), "彻底关闭").changed() {
                                    changed = true;
                                }
                                if changed {
                                    self.save_config();
                                    self.set_toast("关闭行为设置已保存".to_string());
                                }
                                if self.cfg.close_behavior == "tray" && self.tray.is_none() {
                                    ui.label(RichText::new("⚠️ 系统托盘初始化失败（可能被其他程序占用），暂不可用，请改用“最小化”").color(self.fg(Color32::from_rgb(230, 180, 90))));
                                }
                                ui.label(RichText::new("有服务器运行时：选择“彻底关闭”会先询问是否静默关闭所有服务器后再退出（可取消）；选择“最小化”则无论是否有服务器运行都仅最小化；“最小化到托盘”还会隐藏任务栏按钮，恢复方式：双击/单击托盘图标或托盘菜单“显示主窗口”").weak().small());
                            });
                            self.settings_sections = sections;

                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "autostart", "开机自启", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                let mut autostart = autostart_enabled();
                                if ui.checkbox(&mut autostart, "开机自动启动 XMST").changed() {
                                    let exe = self.exe_path.to_string_lossy().to_string();
                                    if set_autostart(&exe, autostart) {
                                        self.set_toast(if autostart {
                                            "已开启开机自启".to_string()
                                        } else {
                                            "已关闭开机自启".to_string()
                                        });
                                    } else {
                                        self.set_toast("写入注册表失败".to_string());
                                    }
                                }
                                ui.label(RichText::new("通过 HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run 实现；注册表项指向本程序并附带 --autostart 参数，登录后自动启动并显示主窗口").weak().small());
                            });
                            self.settings_sections = sections;

                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "data", "数据与配置", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                ui.label(format!("配置文件: {}", self.config_path.display()));
                                ui.horizontal(|ui| {
                                    if ui.button("💾 保存全部设置").clicked() {
                                        self.save_config();
                                        self.set_toast("设置已保存".to_string());
                                    }
                                });
                            });
                            self.settings_sections = sections;

                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "defender", "Windows Defender", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                ui.label(format!(
                                    "XMST 数据目录: {}（备份日志可能被 Defender 误报，可加入排除项）",
                                    self.data_dir().display()
                                ));
                                ui.horizontal(|ui| {
                                    if ui.button("🛡 仅排除 XMST 数据目录（推荐）").clicked() {
                                        let data = self.data_dir();
                                        let script = format!(
                                            "Add-MpPreference -ExclusionPath '{}'",
                                            data.to_string_lossy().replace('\'', "''")
                                        );
                                        match run_powershell(&script) {
                                            Ok(_) => self.set_toast("已添加 Defender 排除项（推荐方案）".to_string()),
                                            Err(e) => self.set_toast(format!(
                                                "添加排除项失败: {e}\n请在弹出的 UAC 授权窗口点击「是」后重试；若仍失败，请以管理员身份运行 XMST，或在管理员 PowerShell 中手动执行\n{}",
                                                script
                                            )),
                                        }
                                    }
                                    // 高风险操作：与其它按钮同一行、同一高度对齐；
                                    // 只用红色文字表达风险，不再套一个突兀的红框（用户反馈）。
                                    let risk_red = self.fg(Color32::from_rgb(230, 120, 120));
                                    // 三个按钮统一用默认按钮尺寸（此前两个大按钮 add_sized 显得"一小两大"）
                                    if ui
                                        .button(
                                            RichText::new("⚠ 完全禁用 Defender 实时保护（风险）")
                                                .color(risk_red),
                                        )
                                        .clicked()
                                    {
                                        self.confirm_defender_disable = true;
                                    }
                                    if ui.button("打开 Windows 安全中心").clicked() {
                                        // explorer.exe 打开 URI 比 cmd start windowsdefender: 更稳（修复弹错误框）
                                        let opened = std::process::Command::new("explorer.exe")
                                            .arg("windowsdefender:")
                                            .spawn()
                                            .map(|_| true)
                                            .unwrap_or(false);
                                        if !opened {
                                            use std::os::windows::process::CommandExt;
                                            let _ = std::process::Command::new("cmd")
                                                .args(["/C", "start", "", "ms-settings:windowsdefender"])
                                                .creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                                                .spawn();
                                        }
                                    }
                                });
                                ui.label(RichText::new("完全禁用会关闭系统实时病毒防护，请仅在封闭内网或离线环境使用，否则可能导致安全风险").small().color(self.fg(Color32::from_rgb(230, 120, 120))));
                            });
                            self.settings_sections = sections;
                        }
                        SettingsSide::Logs => {
                            // 日志已独立成侧栏「📜 日志」页（工具自身日志 + 相关设置），
                            // 这里只留一个跳转入口，避免同一功能两处维护。
                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "log", "日志", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                ui.label("日志浏览与相关设置已移至侧栏「📜 日志」页（工具自身日志，含来源/级别筛选与导出）。");
                                if ui.button("前往「日志」页").clicked() {
                                    self.nav = Nav::Logs;
                                }
                            });
                            self.settings_sections = sections;
                        }                        SettingsSide::Ui => {
                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "ui", "界面", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                ui.label("语言:");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut self.cfg.lang, "zh".to_string(), "中文");
                                    ui.radio_value(&mut self.cfg.lang, "en".to_string(), "English");
                                });
                                ui.checkbox(&mut self.cfg.ui_animations, "启用切换动效（导航页签 / 折叠 / 按钮过渡）");
                                // 说明：原先此处另有一个「设置页内平滑动画」开关，已并入上面的总开关
                                // （用户反馈：两个开关语义重复且设置页看起来"没有动效"）。
                                ui.label(RichText::new("关闭动效后所有过渡立即完成，后台以最低刷新率运行，进一步降低资源占用").weak().small());
                                ui.label("平滑动画速度:");
                                ui.horizontal(|ui| {
                                    ui.add(egui::Slider::new(&mut self.cfg.anim_speed, 0.1..=2.0).logarithmic(true).fixed_decimals(2).show_value(true));
                                    if ui.button("重置").clicked() {
                                        self.cfg.anim_speed = 1.0;
                                    }
                                });
                                ui.label(RichText::new("数值越大展开/折叠越快，越小越平滑；关闭动效时此项不生效").weak().small());
                                // Stage 6：主题系统 UI（模式/预设/自定义/背景图/圆角）
                                self.ui_theme_settings(ui);
                            });
                            self.settings_sections = sections;
                        }
                        SettingsSide::Java => {
                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "java_jvm", "Java 和 JVM", Some("使用 run.bat 的服务器启动时直接执行 run.bat，Java 以脚本内为准；下面两项仅用于「无 run.bat」的服务器"), true, use_anim, self.cfg.anim_speed, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Java 兜底路径 (留空使用 PATH):");
                                    ui.add(TextEdit::singleline(&mut self.cfg.java_path).desired_width(300.0));
                                    if ui.button("浏览").clicked() {
                                        if let Some(f) = rfd::FileDialog::new().pick_file() {
                                            self.cfg.java_path = f.to_string_lossy().to_string();
                                        }
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("默认 JVM 参数:");
                                    ui.add(TextEdit::singleline(&mut self.cfg.default_jvm_args).desired_width(300.0));
                                });
                            });
                            self.settings_sections = sections;

                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "java_list", "全局 Java 列表（按服务器 MC 版本自动匹配）", Some("使用 run.bat 且服务器未指定 Java 时，按服务器 MC 版本从列表自动选用；也可在服务器「启动脚本」页手动指定"), true, use_anim, self.cfg.anim_speed, |ui| {
                                for (i, h) in self.cfg.java_homes.clone().iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let ver = if h.version.trim().is_empty() {
                                            "?"
                                        } else {
                                            h.version.trim()
                                        };
                                        ui.label(format!("{} [{}] {}", h.name, ver, h.path));
                                        if ui.button("📂 打开目录").clicked() {
                                            explorer_select(&h.path);
                                        }
                                        if ui.button("🗑 删除").clicked() {
                                            self.cfg.java_homes.remove(i);
                                            self.save_config();
                                        }
                                    });
                                }
                                if self.cfg.java_homes.is_empty() {
                                    ui.label(RichText::new("（暂存 Java 条目）").weak());
                                }
                                ui.horizontal(|ui| {
                                    ui.label("名称:");
                                    ui.add(
                                        TextEdit::singleline(&mut self.add_java_name)
                                            .hint_text("如 Java21")
                                            .desired_width(100.0),
                                    );
                                    ui.label("版本:");
                                    ui.add(
                                        TextEdit::singleline(&mut self.add_java_version)
                                            .hint_text("如 21 / 1.21.1")
                                            .desired_width(90.0),
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.label("路径:");
                                    ui.add(
                                        TextEdit::singleline(&mut self.add_java_path)
                                            .hint_text("C:\\Java\\jdk-21\\bin\\java.exe")
                                            .desired_width(300.0),
                                    );
                                    if ui.button("浏览").clicked() {
                                        if let Some(f) = rfd::FileDialog::new().pick_file() {
                                            self.add_java_path = f.to_string_lossy().to_string();
                                        }
                                    }
                                    if ui.button("➕ 添加").clicked() {
                                        let name = self.add_java_name.trim().to_string();
                                        let path = self.add_java_path.trim().to_string();
                                        if name.is_empty() || path.is_empty() {
                                            self.set_toast("名称和路径不能为空".to_string());
                                        } else {
                                            let version = {
                                                let v = self.add_java_version.trim().to_string();
                                                if v.is_empty() {
                                                    None
                                                } else {
                                                    Some(v)
                                                }
                                            };
                                            self.cfg.java_homes.push(JavaHome {
                                                name,
                                                version: version.unwrap_or_default(),
                                                path,
                                            });
                                            self.add_java_name.clear();
                                            self.add_java_version.clear();
                                            self.add_java_path.clear();
                                            self.save_config();
                                            self.set_toast("已添加 Java".to_string());
                                        }
                                    }
                                });
                            });
                            self.settings_sections = sections;
                        }
                        SettingsSide::Notify => {
                            let sections = std::mem::take(&mut self.settings_sections);
                            let sections = Self::setting_section(ctx, ui, sections, "notify", "通知", None, true, use_anim, self.cfg.anim_speed, |ui| {
                                // （已按用户要求删除原说明文本）
                                ui.horizontal(|ui| {
                                    let mut v = self.cfg.sys_notify;
                                    if ui.checkbox(&mut v, "启用 Windows 系统气泡通知（窗口关闭时也显示）").changed() {
                                        self.cfg.sys_notify = v;
                                        self.save_config();
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("弹出方式:");
                                    let mut st = self.cfg.toast_style.clone();
                                    let mut changed_to: Option<String> = None;
                                    if ui.radio_value(&mut st, "slide".to_string(), "右侧滑入").changed() {
                                        changed_to = Some("slide".to_string());
                                    }
                                    if ui.radio_value(&mut st, "fade".to_string(), "淡入淡出").changed() {
                                        changed_to = Some("fade".to_string());
                                    }
                                    if let Some(v) = changed_to {
                                        self.cfg.toast_style = v;
                                        self.save_config();
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("滞留时间(秒):");
                                    let mut d = self.cfg.toast_duration_secs;
                                    if ui.add(egui::DragValue::new(&mut d).range(1.0..=30.0).speed(0.5)).changed() {
                                        self.cfg.toast_duration_secs = d;
                                        self.save_config();
                                    }
                                });
                            });
                            self.settings_sections = sections;
                        }
                        SettingsSide::Beta => {
                            self.ui_settings_beta(ctx, ui, use_anim);
                        }
                    }
                });
        });
        // 完全禁用 Defender 的二次确认弹�?
        if self.confirm_defender_disable {
            let mut open = true;
            egui::Window::new("确认完全禁用 Defender")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("即将执行: Set-MpPreference -DisableRealtimeMonitoring $true");
                    ui.label("后果：Windows 实时病毒防护将被关闭，系统不再自动扫描恶意软件");
                    ui.label("仅建议在封闭内网/离线环境使用。确认继续？");
                    ui.horizontal(|ui| {
                        if ui.button("确认执行").clicked() {
                            match run_powershell("Set-MpPreference -DisableRealtimeMonitoring $true") {
                                Ok(_) => self.set_toast("已完全禁用 Defender 实时保护".to_string()),
                                Err(e) => self.set_toast(format!(
                                    "禁用失败: {e}\n请在弹出的 UAC 授权窗口点击「是」后重试；若仍失败，请以管理员身份运行 XMST，或在管理员 PowerShell 中手动执行\nSet-MpPreference -DisableRealtimeMonitoring $true"
                                )),
                            }
                            self.confirm_defender_disable = false;
                        }
                        if ui.button("取消").clicked() {
                            self.confirm_defender_disable = false;
                        }
                    });
                });
            if !open {
                self.confirm_defender_disable = false;
            }
        }
    }

    /// 设置页「测试中的功能」分组
    fn ui_settings_beta(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, use_anim: bool) {
        let sections = std::mem::take(&mut self.settings_sections);
        let sections = Self::setting_section(
            ctx,
            ui,
            sections,
            "beta",
            "测试中的功能",
            None,
            true,
            use_anim,
            self.cfg.anim_speed,
            |ui| {
                ui.label(
                    RichText::new("以下功能处于测试阶段，可能存在 Bug 或尚未完成，默认禁用。")
                        .color(self.fg(Color32::from_rgb(255, 200, 80))),
                );
                ui.label("启用后功能立即生效，并可能影响服务器运行稳定性；请先在测试环境验证，再决定是否长期开启。");
                ui.separator();
                // 测试功能：名称 + 状态 + 启用/禁用
                // Bug6：网络下载/玩家管理/插件系统已转正式功能（Tool 组默认启用），不再作为测试功能在此展示开关
                // 2026-10-02：特殊功能（Spark 分析）回归测试功能（Beta 组默认禁用），重新在此展示开关
                let beta_list: [(&str, &str); 4] = [
                    (features::BETA_BACKUP, "自动备份"),
                    (features::BETA_CRASH_ANALYSIS, "崩溃报告分析"),
                    (features::BETA_TRAFFIC, "内网穿透流量显示"),
                    (features::BETA_SPECIAL, "特殊功能"),
                ];
                for (id, name) in beta_list {
                    let enabled = features::is_enabled(&self.cfg.features, id);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name.to_string()).strong());
                        if enabled {
                            ui.label(RichText::new("已启用").color(self.fg(Color32::from_rgb(80, 200, 120))));
                        } else {
                            ui.label(RichText::new("已禁用").color(self.fg(Color32::from_rgb(180, 180, 180))));
                        }
                        if enabled {
                            if ui.button("禁用").clicked() {
                                self.set_feature(id, false);
                                self.set_toast(format!("已禁用「{name}」"));
                            }
                        } else if ui.button("启用").clicked() {
                            // 开启需警告确认
                            self.beta_confirm = Some(id.to_string());
                        }
                    });
                }
            },
        );
        self.settings_sections = sections;
        // 开启测试功能警告确认弹窗
        if let Some(id) = &self.beta_confirm {
            let id = id.clone();
            let name = features::meta(&id)
                .map(|m| m.name.to_string())
                .unwrap_or_else(|| id.clone());
            let mut open = true;
            egui::Window::new("启用测试功能确认")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("确定启用测试功能「{name}」？")).strong());
                    ui.add_space(4.0);
                    ui.label("⚠ 该功能仍在测试阶段，可能存在 Bug、数据异常或稳定性问题。");
                    ui.label("启用后：UI 立即恢复显示，后台调度立即恢复执行。");
                    ui.label("您可以在本页面随时一键禁用。");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("确认启用").clicked() {
                            self.set_feature(&id, true);
                            self.set_toast(format!("已启用测试功能「{name}」（请谨慎使用）"));
                            self.beta_confirm = None;
                        }
                        if ui.button("取消").clicked() {
                            self.beta_confirm = None;
                        }
                    });
                });
            if !open {
                self.beta_confirm = None;
            }
        }
    }

    /// 设置功能覆盖状态并立即落盘
    fn set_feature(&mut self, id: &str, enabled: bool) {
        let st = self.cfg.features.entry(id.to_string()).or_default();
        st.enabled = enabled;
        self.save_config();
        // BETA_DOWNLOAD：启用时初始化下载 UI 状态，禁用时释放（零内存占用）
        if id == features::BETA_DOWNLOAD {
            self.dl = if enabled {
                Some(Box::<DlUiState>::default())
            } else {
                None
            };
        }
        // BETA_PLUGINS：启用时初始化插件运行时（扫描 data/plugins），禁用时释放（零加载零轮询）
        if id == features::BETA_PLUGINS {
            if enabled && self.plugins.is_none() {
                let mut pm = plugins::PluginManager::new(
                    self.data_dir().join("plugins"),
                    self.data_dir().join("plugins_cache"),
                    self.cfg.plugin_states.clone(),
                    self.cfg.plugin_configs.clone(),
                );
                let _ = std::fs::create_dir_all(pm.plugins_dir.clone());
                let _ = std::fs::create_dir_all(pm.cache_dir.clone());
                let _ = pm.reload_all();
                self.plugins = Some(pm);
            } else if !enabled {
                self.plugins = None;
                // 插件系统整体关闭：背景效果一并复位，避免 DWM accent / 透明面板残留在窗口上
                let ctx = self.egui_ctx.clone();
                let op = self.plugin_bg_opacity;
                if self.plugin_bg_style != plugins::BgStyle::Default {
                    self.apply_bg(plugins::BgStyle::Default, &ctx, op, None);
                }
            }
        }
    }

    /// 初始化插件运行时（BETA_PLUGINS 启用时执行；默认关闭零加载）
    fn init_plugins(cfg: &config::GlobalConfig) -> Option<plugins::PluginManager> {
        if !features::is_enabled(&cfg.features, features::BETA_PLUGINS) {
            return None;
        }
        // 插件目录随 data_dir() 动态解析，无法在 new() 前直接调用实例方法，
        // 因此这里只登记目录占位，真正目录在 first_plugin_tick 中补齐。
        // 插件单独配置（plugin_configs）随 states 一并传入，脚本可经 xmst_config_get/set 读写。
        Some(plugins::PluginManager::new(
            PathBuf::from("plugins-placeholder"),
            PathBuf::from("plugins_cache-placeholder"),
            cfg.plugin_states.clone(),
            cfg.plugin_configs.clone(),
        ))
    }

    /// 插件事件分发：将本帧收集的 PluginEvt 依次 emit 到插件运行时
    fn emit_plugin_events(&mut self, evts: Vec<PluginEvt>) {
        let Some(pm) = self.plugins.as_mut() else {
            return;
        };
        for ev in evts {
            let (name, args) = match ev {
                PluginEvt::ServerStarted(srv) => {
                    ("server_started", vec![Dynamic::from(srv)])
                }
                PluginEvt::ServerStopped(srv, reason) => {
                    ("server_stopped", vec![Dynamic::from(srv), Dynamic::from(reason)])
                }
                PluginEvt::LogLine(srv, line) => {
                    ("log_line", vec![Dynamic::from(srv), Dynamic::from(line)])
                }
                PluginEvt::PlayerJoined(srv, player) => {
                    ("player_joined", vec![Dynamic::from(srv), Dynamic::from(player)])
                }
                PluginEvt::PlayerLeft(srv, player) => {
                    ("player_left", vec![Dynamic::from(srv), Dynamic::from(player)])
                }
                PluginEvt::BackupDone(srv, file, bytes) => {
                    (
                        "backup_done",
                        vec![Dynamic::from(srv), Dynamic::from(file), Dynamic::from(bytes)],
                    )
                }
            };
            pm.emit(name, args);
        }
    }

    /// 每帧消费插件消息（日志/toast/背景请求）并应用副作用
    fn tick_plugins(&mut self, ctx: &egui::Context) {
        if self.plugins.is_none() {
            return;
        }
        // 背景不透明度的兜底默认值（历史配置可能为 0，见 plugin_opacity_default）
        let default_alpha = self.plugin_opacity_default();
        // 首次 tick：new() 中因 data_dir() 不可用而登记的占位目录，此处迁移到真实目录并扫描
        let placeholder = self
            .plugins
            .as_ref()
            .map(|p| p.plugins_dir.to_string_lossy().contains("placeholder"))
            .unwrap_or(false);
        if placeholder {
            let mut pm = plugins::PluginManager::new(
                self.data_dir().join("plugins"),
                self.data_dir().join("plugins_cache"),
                self.cfg.plugin_states.clone(),
                self.cfg.plugin_configs.clone(),
            );
            let _ = std::fs::create_dir_all(pm.plugins_dir.clone());
            let _ = std::fs::create_dir_all(pm.cache_dir.clone());
            let _ = pm.reload_all();
            self.plugins = Some(pm);
        }
        let (toasts, bg) = {
            let Some(pm) = self.plugins.as_mut() else {
                return;
            };
            pm.tick();
            let toasts = std::mem::take(&mut pm.toasts);
            let bg = pm.bg_request.take();
            (toasts, bg)
        };
        for (title, body) in toasts {
            self.notify(&title, &body);
        }
        // 插件脚本通过 xmst_config_set 修改了自己的配置：同步回 cfg 并落盘
        if let Some(pm) = self.plugins.as_ref() {
            if pm.configs_dirty {
                if let Some(pm) = self.plugins.as_mut() {
                    pm.configs_dirty = false;
                    if let Ok(g) = pm.configs.lock() {
                        self.cfg.plugin_configs = g.clone();
                    }
                    self.save_config();
                }
            }
        }
        // 启动后首次 tick：按「已启用插件」保存的 bg_style / bg_alpha 恢复背景效果。
        // 修复「重启后窗口背景效果丢失」：以前 bg_style 只被卡片 UI 读写，启动时无人应用。
        if !self.plugin_bg_restored {
            self.plugin_bg_restored = true;
            let restored = self
                .plugins
                .as_ref()
                .map(|pm| {
                    let mut found = None;
                    if let Ok(g) = pm.configs.lock() {
                        for inst in pm.instances.iter().filter(|i| i.enabled) {
                            let Some(m) = g.get(&inst.manifest.name) else {
                                continue;
                            };
                            let style = m
                                .get("bg_style")
                                .map(|s| plugins::BgStyle::from_key(s))
                                .unwrap_or_default();
                            if style != plugins::BgStyle::Default {
                                let alpha = m
                                    .get("bg_alpha")
                                    .and_then(|v| v.parse::<f32>().ok())
                                    .unwrap_or(default_alpha);
                                found = Some((
                                    inst.manifest.name.clone(),
                                    style,
                                    Self::norm_bg_opacity(alpha, default_alpha),
                                ));
                            }
                        }
                    }
                    found
                })
                .flatten();
            if let Some((name, style, alpha)) = restored {
                // 启动恢复：静默应用（不弹提示条），避免每次开机都蹦一条像“警告”的提示
                self.bg_suppress_toast = true;
                self.apply_bg(style, ctx, alpha, Some(&name));
                self.bg_suppress_toast = false;
            }
        }
        if let Some((pname, bg)) = bg {
            // 脚本触发：优先取请求插件自己的 bg_alpha（单插件配置区设置值），无则回退全局默认
            let alpha = self
                .plugins
                .as_ref()
                .and_then(|pm| pm.configs.lock().ok())
                .and_then(|g| {
                    g.get(&pname)
                        .and_then(|m| m.get("bg_alpha"))
                        .and_then(|v| v.parse::<f32>().ok())
                })
                .unwrap_or(default_alpha);
            // 脚本事件（含启动时的 on_enabled）触发的应用同样静默
            let na = Self::norm_bg_opacity(alpha, default_alpha);
            self.bg_suppress_toast = true;
            self.apply_bg(bg, ctx, na, Some(&pname));
            self.bg_suppress_toast = false;
        }
    }

    /// 插件背景不透明度的安全默认值。
    /// 历史配置里 `plugin_bg_alpha` 可能被旧的「毛玻璃暗度」滑杆拖到 0（0 = 完全无着色，
    /// 等于「看不到任何效果」），此时若继续沿用 0，新做的三模式也会「点了没反应」。
    fn plugin_opacity_default(&self) -> f32 {
        let v = self.cfg.plugin_bg_alpha;
        if v.is_finite() && v >= 0.05 {
            v.clamp(0.05, 1.0)
        } else {
            0.55
        }
    }

    /// 归一化插件背景不透明度：落在 [0.05, 1.0] 之外（含历史遗留的 0.00）一律取兜底默认值，
    /// 避免「配置里是 0 → 效果等于没有」再次发生。
    fn norm_bg_opacity(v: f32, fallback: f32) -> f32 {
        if v.is_finite() && v >= 0.05 {
            v.clamp(0.05, 1.0)
        } else {
            fallback
        }
    }

    /// 实际用于材质的浓度：
    /// * 默认 = 滑杆值（越底越透，桌面越抢眼）；
    /// * 关闭「配色跟随材质明暗」时抬到 0.60 下限 —— 因为此时界面固定用主题（深色）配色，
    ///   浓度太低会让明亮桌面上变成「浅字 + 亮底」而看不清。
    fn effective_bg_opacity(&self) -> f32 {
        let v = self.plugin_bg_opacity.clamp(0.0, 1.0);
        if self.cfg.bg_material_auto_contrast {
            v
        } else {
            v.max(0.60)
        }
    }

    /// 应用插件请求的窗口背景效果：半透明 / 毛玻璃 / 亚克力 / 恢复默认。
    ///
    /// `opacity` 0.0-1.0 = 窗口不透明度（三种模式共用的滑杆）。
    /// `owner`：请求方插件名（`None` = 保持当前归属，例如滑杆微调、程序启动恢复）。
/// 材质状态（D4 唯一真源）在 `plugin_configs` 中使用的归属名：背景材质统一落到这个名字下，
/// 避免「UI 滑杆写一份、插件配置写另一份」导致重启后生效的不是用户刚调的值。
const MATERIAL_OWNER_FALLBACK: &str = "xmst-frosted-glass-demo";
    /// 归属用于 D2：禁用/卸载该插件时自动回收背景效果，避免「插件停了窗口还透」。
    ///
    /// 透明机制（实测结论见 docs\背景材质失效根因分析_毛玻璃亚克力半透明.md §10）：
    /// 本机 OpenGL 呈现路径不透明，逐像素窗口 alpha / LWA_ALPHA 均无效，
    /// 因此视觉效果由 `backdrop.rs` 的**桌面捕获底图**承担；DWM accent 仅作增强。
    fn apply_bg(
        &mut self,
        style: plugins::BgStyle,
        ctx: &egui::Context,
        opacity: f32,
        owner: Option<&str>,
    ) {
        let opacity = opacity.clamp(0.0, 1.0);
        let hwnd = resolve_main_hwnd(&mut self.hwnd_cache);
        let h = hwnd as isize;
        let tint_c = self.theme_cur.bg;
        let tint = (tint_c.r(), tint_c.g(), tint_c.b());
        let alpha_u8 = (opacity * 255.0).round().clamp(1.0, 255.0) as u8;
        let mut effective = style;
        let mut note = String::new();
        let mut accent_ok = false;

        match style {
            plugins::BgStyle::Default => {
                if !hwnd.is_null() {
                    plugins::clear_accent(h);
                    // 防御性清理：早期版本可能残留过 WS_EX_LAYERED
                    plugins::clear_uniform_alpha(h);
                }
                self.backdrop.ensure_exclusion(h, false);
            }
            plugins::BgStyle::Translucent => {
                // 桌面捕获模式下不需要 DWM accent；清掉可能残留的
                if !hwnd.is_null() {
                    plugins::clear_accent(h);
                }
                self.backdrop.ensure_exclusion(h, true);
            }
            plugins::BgStyle::Frosted | plugins::BgStyle::Acrylic => {
                // 桌面捕获承担主要视觉效果；DWM accent 仍下发一次，
                // 在「窗口 alpha 可合成」的正常本机会话里作为额外增强（此处无效也无害）
                if !hwnd.is_null() {
                    if let Some(state) = style.accent_state() {
                        accent_ok = plugins::apply_accent(h, state, tint, alpha_u8);
                    }
                    if !accent_ok {
                        let alt = if style == plugins::BgStyle::Acrylic {
                            plugins::BgStyle::Frosted
                        } else {
                            plugins::BgStyle::Acrylic
                        };
                        if let Some(state) = alt.accent_state() {
                            if plugins::apply_accent(h, state, tint, alpha_u8) {
                                accent_ok = true;
                                effective = alt;
                                note = "（首选效果不可用，已回退另一档 DWM 模糊）".to_string();
                            }
                        }
                    }
                }
                self.backdrop.ensure_exclusion(h, true);
            }
        }

        self.plugin_bg_style = effective;
        self.plugin_bg_opacity = opacity;
        // D4：把材质状态**只写一份**到插件配置（作为唯一真源），并同步历史字段
        // `plugin_bg_alpha`（老配置兼容）。
        //
        // 这里是真的踩过坑：UI 滑杆写 `plugin_bg_alpha`、恢复流程读插件配置的
        // `bg_alpha`，两边一旦不一致（实测 UI 记 0.55、插件配置仍是 0.32），
        // 启动后生效的就是那个没人再维护的旧值 —— 表现为「我明明调过，重启又不对」。
        if effective != plugins::BgStyle::Default {
            if let Some(name) = self
                .plugin_bg_owner
                .clone()
                .or_else(|| Some(Self::MATERIAL_OWNER_FALLBACK.to_string()))
            {
                let entry = self.cfg.plugin_configs.entry(name).or_default();
                entry.insert("bg_style".to_string(), effective.key().to_string());
                entry.insert("bg_alpha".to_string(), format!("{opacity:.2}"));
            }
        }
        // 注意：`Default`（关闭效果 / 插件被禁用时的效果回收）**不写回**插件配置，
        // 否则"停用插件 / 关掉效果"会把用户选好的模式清成 default，下次启用就没了。
        // 模式是否真正变化：只在变化时重设窗口区域、立即落盘（避免拖动滑杆时闪烁/写盘）
        let style_changed = self.plugin_bg_style != effective;
        self.plugin_bg_style = effective;
        self.cfg.plugin_bg_alpha = opacity;
        // 落盘做去抖：拖动不透明度滑杆时每帧写配置既浪费又会卡顿（D4 唯一真源已保证一致性）
        if style_changed || self.bg_save_at.map(|t| t.elapsed().as_millis() > 600).unwrap_or(true) {
            self.save_config();
            self.bg_save_at = Some(std::time::Instant::now());
        }
        // D2：记录/清理背景效果归属
        if effective == plugins::BgStyle::Default {
            self.plugin_bg_owner = None;
        } else if let Some(name) = owner {
            self.plugin_bg_owner = Some(name.to_string());
        }
        // ★ 关键：DWM accent 会重置窗口区域（SetWindowRgn 的结果被抹掉），而
        // apply_window_round_region 有 (r,w,h) 去重缓存，于是圆角永久变成「尖锐直角」。
        // 这里既失效缓存，也在同一步内立即用当前半径重设区域，不依赖下一帧。
        //
        // 但**只在模式/圆角真正变化时**才重设：SetWindowRgn 会让整窗重绘一次，
        // 挪动不透明度滑杆时每帧都调用就会周期性地"闪一下"（用户反馈的闪烁根因之一）。
        if style_changed {
            self.last_win_rgn = (-1, -1, -1);
        }
        self.apply_window_round_region(ctx, self.win_r_points);
        // 提示条只在本机 DWM 不可用时说明「已改用桌面捕获材质」，
        // 且**启动恢复时不弹**（启动就弹一条带“不可用”字样的提示，用户会以为出错了）。
        if !self.bg_suppress_toast {
            let msg = if !note.is_empty() {
                format!(
                    "窗口背景：{}（不透明度 {:.0}%）｜ 本机 DWM 亚克力不可用，已用桌面捕获材质实现同等效果",
                    effective.label(),
                    opacity * 100.0
                )
            } else {
                format!(
                    "窗口背景：{}（不透明度 {:.0}%）",
                    effective.label(),
                    opacity * 100.0
                )
            };
            self.set_toast(msg);
        }
        self.bg_debug_log(style, effective, opacity, accent_ok, hwnd);
        ctx.request_repaint();
    }

    /// 运行时诊断：把背景模式相关的关键事实追加到 `data\bg_debug.log`。
    /// 这是排查「毛玻璃/亚克力不生效」的唯一可靠手段（DWM accent 是未公开 API，
    /// 失败时既无返回值也无日志；eframe/winit 的 log 在本程序里没有初始化）。
    fn bg_debug_log(
        &self,
        requested: plugins::BgStyle,
        effective: plugins::BgStyle,
        opacity: f32,
        accent_ok: bool,
        hwnd: winapi::shared::windef::HWND,
    ) {
        use std::io::Write;
        let path = self.data_dir().join("bg_debug.log");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // D7：日志轮转（超过 256KB 保留最后 200 行），避免长期运行无限增长
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > 256 * 1024 {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let lines: Vec<&str> = text.lines().collect();
                    let keep: Vec<&str> = lines[lines.len().saturating_sub(200)..].to_vec();
                    let _ = std::fs::write(&path, keep.join("\n") + "\n");
                }
            }
        }
        let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        else {
            return;
        };
        let (bd_mean, bd_samples, bd_count, bd_size, bd_ms, bd_max_ms) = self.backdrop.stats();
        let samples: Vec<String> = bd_samples
            .iter()
            .map(|(r, g, b)| format!("{r},{g},{b}"))
            .collect();
        let _ = writeln!(
            f,
            "[{}] requested={:?} effective={:?} opacity={:.2} accent_ok={} hwnd_null={} layered={} composition={} gl(red,alpha)={:?} ppp={:.2} | backdrop mean={:?} samples=[{}] captures={} tex={}x{} cap_ms={:.2}/max{:.2}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            requested,
            effective,
            opacity,
            accent_ok,
            hwnd.is_null(),
            if hwnd.is_null() { false } else { plugins::is_layered(hwnd as isize) },
            plugins::is_composition_enabled(),
            self.gl_fb_bits,
            self.egui_ctx.pixels_per_point(),
            bd_mean,
            samples.join(" "),
            bd_count,
            bd_size.0,
            bd_size.1,
            bd_ms,
            bd_max_ms,
        );
    }

    /// 导入插件 zip：复制到插件目录并热加载（B5：zip 拖入 / 导入按钮）
    fn import_plugin_zip(&mut self, src: &std::path::Path) -> bool {
        let Some(pm) = self.plugins.as_mut() else {
            self.set_toast("插件系统未启用".to_string());
            return false;
        };
        let fname = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if fname.is_empty() {
            return false;
        }
        let dst = pm.plugins_dir.join(&fname);
        if let Err(e) = std::fs::copy(src, &dst) {
            self.set_toast(format!("导入插件失败: {e}"));
            return false;
        }
        let _ = pm.reload_all();
        self.set_toast(format!("插件「{fname}」已导入并加载"));
        true
    }

    /// 插件管理页（BETA_PLUGINS 启用时可用）
    fn ui_plugins(&mut self, ctx: &egui::Context) {
        if self.plugins.is_none() {
            self.set_toast("请在设置页「测试中的功能」启用「插件系统」".to_string());
            self.nav = Nav::Settings;
            return;
        }
        let bg_now = self.plugin_bg_style;
        let bg_now_opacity = self.plugin_bg_opacity;
        // 卡片滑杆的兜底默认值（历史配置可能是 0.00，见 plugin_opacity_default）
        let default_alpha = self.plugin_opacity_default();
        let plugins_dir = self
            .plugins
            .as_ref()
            .map(|p| p.plugins_dir.display().to_string())
            .unwrap_or_default();
        let mut msgs: Vec<String> = Vec::new();
        // 单插件卡片「配置」区发起的背景效果动作：(插件名, 样式)；不透明度从该插件 configs 读
        let mut bg_action: Option<(String, plugins::BgStyle)> = None;
        // 滑杆即时生效：半透明改覆盖层 alpha，模糊类重设 DWM 着色
        let mut bg_reapply: Option<f32> = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.panel_fill))
            .show(ctx, |ui| {
                // 第二批：max-width 1100 左对齐（插件页，ScrollArea 移入容器内）
                let _avail = ui.available_rect_before_wrap();
                // 左侧留出 14px 内边距：此前内容紧贴侧栏/窗口边缘，文字看起来"贴太近"。
                let _pad = 14.0f32;
                let _w = (_avail.width() - _pad).min(1100.0);
                let _centered = egui::Rect::from_min_size(
                    egui::pos2(_avail.left() + _pad, _avail.top()),
                    egui::vec2(_w, _avail.height()),
                );
                let mut _inner = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(_centered)
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt("page_center_1100"),
                );
                _inner.set_width(_w);
                let ui = &mut _inner;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.heading("插件系统");
                        });
                        ui.label(
                            RichText::new("插件目录：")
                                .color(self.fg(Color32::from_rgb(150, 150, 150)))
                                .monospace(),
                        );
                        ui.label(
                            RichText::new(plugins_dir.as_str())
                                .monospace()
                                .color(self.fg(Color32::from_rgb(180, 180, 180))),
                        );
                        ui.separator();
                        // 工具栏：重新扫描 / 当前背景状态
                        ui.horizontal(|ui| {
                            if ui.button("重新扫描插件目录").clicked() {
                                if let Some(pm) = self.plugins.as_mut() {
                                    let _ = pm.reload_all();
                                }
                            }
                            if ui.button("📥 导入插件").clicked() {
                                if let Some(p) = rfd::FileDialog::new()
                                    .add_filter("zip", &["zip"])
                                    .pick_file()
                                {
                                    self.import_plugin_zip(&p);
                                }
                            }
                            let bg_txt = if bg_now == plugins::BgStyle::Default {
                                "背景效果：默认".to_string()
                            } else {
                                format!(
                                    "背景效果：{}（不透明度 {:.0}%）",
                                    bg_now.label(),
                                    bg_now_opacity * 100.0
                                )
                            };
                            ui.label(
                                RichText::new(bg_txt).color(if bg_now == plugins::BgStyle::Default {
                                    self.fg(Color32::from_rgb(150, 150, 150))
                                } else {
                                    self.fg(Color32::from_rgb(120, 200, 160))
                                }),
                            );
                            // 背景效果配置入口位于每张插件卡片的「配置」区（单插件配置），
                            // 不再放这里的总设置工具栏。
                        });
                        ui.separator();
                        // 插件列表（卡片式：名称 + 状态徽章 + 描述 + 操作右对齐）
                        let mut toggle: Option<(String, bool)> = None;
                        let mut unload_name: Option<String> = None;
                        let names: Vec<String> = match self.plugins.as_mut() {
                            Some(pm) => pm.instances.iter().map(|i| i.manifest.name.clone()).collect(),
                            None => Vec::new(),
                        };
                        if names.is_empty() {
                            ui.label(
                                RichText::new("暂无已加载插件。请将插件 zip 放入上方插件目录后点击「重新扫描插件目录」。")
                                    .color(self.fg(Color32::from_rgb(180, 180, 180))),
                            );
                        }
                        // 选中项同步：若已不存在则清空
                        if let Some(sel) = self.plugin_sel.clone() {
                            if !names.contains(&sel) {
                                self.plugin_sel = None;
                            }
                        }
                        let sel_name = self.plugin_sel.clone();
                        for name in &names {
                            let (ver, desc, enabled, hooks, err) = {
                                let pm = match self.plugins.as_ref() {
                                    Some(pm) => pm,
                                    None => continue,
                                };
                                match pm.instances.iter().find(|i| i.manifest.name == *name) {
                                    Some(i) => (
                                        i.manifest.version.clone(),
                                        i.manifest.description.clone(),
                                        i.enabled,
                                        i.hook_calls.clone(),
                                        i.last_error.clone(),
                                    ),
                                    None => (String::new(), String::new(), false, std::collections::HashMap::new(), None),
                                }
                            };
                            let is_sel = sel_name.as_deref() == Some(name.as_str());
                            let card_bg = if is_sel {
                                Color32::from_rgba_unmultiplied(
                                    self.theme_target.accent.r(),
                                    self.theme_target.accent.g(),
                                    self.theme_target.accent.b(),
                                    14,
                                )
                            } else {
                                Color32::from_rgba_unmultiplied(150, 160, 180, 8)
                            };
                            let card_stroke = if is_sel {
                                self.theme_target.accent
                            } else {
                                Color32::from_rgba_unmultiplied(150, 160, 180, 50)
                            };
                            egui::Frame::none()
                                .fill(card_bg)
                                .stroke(egui::Stroke::new(1.0, card_stroke))
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        // 名称 + 版本（左侧）
                                        ui.label(RichText::new(name.as_str()).strong().size(16.0));
                                        if !ver.is_empty() {
                                            ui.label(RichText::new(format!("v{ver}")).color(self.fg(Color32::from_rgb(150, 150, 150))));
                                        }
                                        // 状态徽章：运行中=绿色实心圆点 / 已停止=红色实心圆点
                                        if enabled {
                                            ui.label(RichText::new("● 运行中").color(self.fg(Color32::from_rgb(80, 200, 120))));
                                        } else {
                                            ui.label(RichText::new("● 已停止").color(self.fg(Color32::from_rgb(220, 90, 90))));
                                        }
                                        // 操作按钮右对齐（启动/停止、卸载）
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button("卸载").clicked() {
                                                unload_name = Some(name.clone());
                                            }
                                            if ui.button(if enabled { "禁用" } else { "启用" }).clicked() {
                                                toggle = Some((name.clone(), !enabled));
                                            }
                                        });
                                    });
                                    // 描述（弱化色、换行完整显示）
                                    if desc.is_empty() {
                                        ui.label(RichText::new("（无描述）").color(self.fg(Color32::from_rgb(150, 150, 150))));
                                    } else {
                                        ui.label(RichText::new(desc).color(self.fg(Color32::from_rgb(170, 170, 170))));
                                    }
                                    // 钩子（弱化小字）
                                    if !hooks.is_empty() {
                                        let mut parts: Vec<String> = hooks
                                            .iter()
                                            .map(|(k, v)| format!("{k} x{v}"))
                                            .collect();
                                        parts.sort();
                                        ui.label(RichText::new(parts.join(" · ")).color(self.fg(Color32::from_rgb(150, 150, 150))).small());
                                    }
                                    if let Some(e) = &err {
                                        ui.label(RichText::new(format!("最近错误：{e}")).color(self.fg(Color32::from_rgb(220, 90, 90))));
                                    }
                                    // 插件单独配置（键值对；脚本内通过 xmst_config_get/set 读写，改动实时落盘）
                                    egui::CollapsingHeader::new(
                                        RichText::new("配置")
                                            .color(self.fg(Color32::from_rgb(170, 170, 170)))
                                            .small(),
                                    )
                                    .id_salt(("plugin_cfg", name.as_str()))
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        // ---- 背景效果（本插件配置，非插件页总设置）----
                                        // bg_style / bg_alpha 为本插件保留键：UI 与脚本（xmst_config_get/set）共用
                                        let (mut bg_style, mut bg_alpha) = {
                                            let g = self
                                                .plugins
                                                .as_ref()
                                                .and_then(|pm| pm.configs.lock().ok());
                                            match g {
                                                Some(g) => {
                                                    let m = g.get(name.as_str());
                                                    let s = m
                                                        .and_then(|m| m.get("bg_style"))
                                                        .cloned()
                                                        .unwrap_or_default();
                                                    let a = m
                                                        .and_then(|m| m.get("bg_alpha"))
                                                        .and_then(|v| v.parse::<f32>().ok())
                                                        .unwrap_or(default_alpha);
                                                    (s, a.max(0.05))
                                                }
                                                None => (String::new(), default_alpha),
                                            }
                                        };
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("背景效果:")
                                                    .small()
                                                    .color(self.fg(Color32::from_rgb(170, 170, 170))),
                                            );
                                            if ui.small_button("半透明").clicked() {
                                                bg_action = Some((
                                                    name.clone(),
                                                    plugins::BgStyle::Translucent,
                                                ));
                                            }
                                            if ui.small_button("毛玻璃").clicked() {
                                                bg_action = Some((
                                                    name.clone(),
                                                    plugins::BgStyle::Frosted,
                                                ));
                                            }
                                            if ui.small_button("亚克力").clicked() {
                                                bg_action = Some((
                                                    name.clone(),
                                                    plugins::BgStyle::Acrylic,
                                                ));
                                            }
                                            if ui.small_button("恢复默认背景").clicked() {
                                                bg_action = Some((
                                                    name.clone(),
                                                    plugins::BgStyle::Default,
                                                ));
                                            }
                                            if !bg_style.is_empty() {
                                                let st = plugins::BgStyle::from_key(&bg_style).label();
                                                ui.label(
                                                    RichText::new(format!("当前：{st}"))
                                                        .small()
                                                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                                                );
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("不透明度:")
                                                    .small()
                                                    .color(self.fg(Color32::from_rgb(170, 170, 170))),
                                            );
                                            if ui
                                                .add(egui::Slider::new(&mut bg_alpha, 0.05..=1.0).fixed_decimals(2))
                                                .changed()
                                            {
                                                // 写回本插件配置（随 configs_dirty 落盘），三种模式共用该值
                                                if let Some(pm) = self.plugins.as_mut() {
                                                    if let Ok(mut g) = pm.configs.lock() {
                                                        g.entry(name.clone())
                                                            .or_default()
                                                            .insert(
                                                                "bg_alpha".to_string(),
                                                                format!("{:.2}", bg_alpha),
                                                            );
                                                    }
                                                    pm.configs_dirty = true;
                                                }
                                                // 任意非默认模式下即时生效：
                                                // 半透明=覆盖层 alpha（theme 每帧读取），模糊类=重设 DWM 着色
                                                if self.plugin_bg_style != plugins::BgStyle::Default {
                                                    bg_reapply = Some(bg_alpha.clamp(0.0, 1.0));
                                                }
                                            }
                                        });
                                        ui.label(
                                            RichText::new("半透明=窗口背后桌面透进来；毛玻璃/亚克力=同一张底图做重度模糊。三者均可调不透明度。")
                                                .weak()
                                                .small(),
                                        );
                                        // 界面配色是否跟随材质明暗：明亮桌面→自动转浅色界面，
                                        // 深色桌面→保持深色。关闭后固定主题配色（材质浓度会被抬到 0.60 下限）。
                                        let mut auto_c = self.cfg.bg_material_auto_contrast;
                                        if ui
                                            .checkbox(
                                                &mut auto_c,
                                                RichText::new(
                                                    "界面配色跟随材质明暗（关闭则固定当前主题配色）",
                                                )
                                                .small()
                                                .color(self.fg(Color32::from_rgb(170, 170, 170))),
                                            )
                                            .changed()
                                        {
                                            self.cfg.bg_material_auto_contrast = auto_c;
                                            self.save_config();
                                            let ctx2 = ctx.clone();
                                            let op = self.plugin_bg_opacity;
                                            if self.plugin_bg_style != plugins::BgStyle::Default {
                                                let style = self.plugin_bg_style;
                                                self.apply_bg(style, &ctx2, op, None);
                                            }
                                        }
                                        // F2 效果可用性自检：Windows 在「透明效果关闭 / 节电 / 远程会话」
                                        // 时会禁用亚克力，而 TranslucentTB 那类工具失败时静默无提示。
                                        // 这里显式列出当前环境里会削弱效果的系统级门槛。
                                        for warn in plugins::material_env_warnings() {
                                            ui.label(
                                                RichText::new(format!("⚠ {warn}"))
                                                    .small()
                                                    .color(Color32::from_rgb(232, 176, 72)),
                                            );
                                        }
                                        // 内容衬底：材质之上再铺一层主题底色的半透明衬底，
                                        // 桌面材质仍可见，但文字有稳定底色（小字可读性关键）。
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("内容衬底:")
                                                    .small()
                                                    .color(self.fg(Color32::from_rgb(170, 170, 170))),
                                            );
                                            let mut scrim = self.cfg.bg_content_scrim;
                                            if ui
                                                .add(
                                                    egui::Slider::new(&mut scrim, 0.0..=0.6)
                                                        .fixed_decimals(2),
                                                )
                                                .changed()
                                            {
                                                self.cfg.bg_content_scrim = scrim;
                                                self.save_config();
                                                if self.plugin_bg_style != plugins::BgStyle::Default {
                                                    let ctx2 = ctx.clone();
                                                    let op = self.plugin_bg_opacity;
                                                    let style = self.plugin_bg_style;
                                                    self.apply_bg(style, &ctx2, op, None);
                                                }
                                            }
                                            ui.label(
                                                RichText::new("越大文字越清晰、桌面越淡")
                                                    .weak()
                                                    .small(),
                                            );
                                        });
                                        // D11：抓屏排除开关。桌面捕获式背景必须让本窗口对「抓屏」
                                        // 不可见，否则会把界面自己抓进底图形成反馈；
                                        // 代价是本工具会从用户的截屏/录屏/共享屏幕中消失。
                                        let mut excl = self.cfg.bg_capture_exclusion;
                                        if ui
                                            .checkbox(
                                                &mut excl,
                                                RichText::new(
                                                    "开启效果时把本工具排除在截屏/录屏之外（避免背景自我递归）",
                                                )
                                                .small()
                                                .color(self.fg(Color32::from_rgb(170, 170, 170))),
                                            )
                                            .changed()
                                        {
                                            self.cfg.bg_capture_exclusion = excl;
                                            self.save_config();
                                            let ctx2 = ctx.clone();
                                            let op = self.plugin_bg_opacity;
                                            if self.plugin_bg_style != plugins::BgStyle::Default {
                                                let style = self.plugin_bg_style;
                                                self.apply_bg(style, &ctx2, op, None);
                                            }
                                        }
                                        ui.separator();
                                        let entries: Vec<(String, String)> = {
                                            let g = self
                                                .plugins
                                                .as_ref()
                                                .and_then(|pm| pm.configs.lock().ok());
                                            match g {
                                                Some(g) => g
                                                    .get(name.as_str())
                                                    .map(|m| {
                                                        m.iter()
                                                            .map(|(k, v)| (k.clone(), v.clone()))
                                                            .collect()
                                                    })
                                                    .unwrap_or_default(),
                                                None => Vec::new(),
                                            }
                                        };
                                        if entries.is_empty() {
                                            ui.label(
                                                RichText::new("（暂无配置项，插件可通过 xmst_config_set 写入）")
                                                    .color(self.fg(Color32::from_rgb(150, 150, 150)))
                                                    .small(),
                                            );
                                        }
                                        let mut dirty = false;
                                        let mut del_key: Option<String> = None;
                                        let mut upsert: Option<(String, String)> = None;
                                        for (k, v) in &entries {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(k.as_str())
                                                        .monospace()
                                                        .color(self.fg(Color32::from_rgb(170, 170, 170))),
                                                );
                                                let mut val_edit = v.clone();
                                                let resp = ui.add(
                                                    egui::TextEdit::singleline(&mut val_edit)
                                                        .desired_width(200.0)
                                                        .hint_text("值"),
                                                );
                                                if resp.changed() {
                                                    upsert = Some((k.clone(), val_edit));
                                                    dirty = true;
                                                }
                                                if ui.small_button("删除").clicked() {
                                                    del_key = Some(k.clone());
                                                    dirty = true;
                                                }
                                            });
                                        }
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(&mut self.plugin_cfg_new_key)
                                                    .hint_text("新配置项 key")
                                                    .desired_width(140.0),
                                            );
                                            if ui.button("添加配置项").clicked() {
                                                let k = std::mem::take(&mut self.plugin_cfg_new_key);
                                                if !k.trim().is_empty() {
                                                    upsert = Some((k, String::new()));
                                                    dirty = true;
                                                }
                                            }
                                        });
                                        if dirty {
                                            if let Some(pm) = self.plugins.as_mut() {
                                                if let Ok(mut g) = pm.configs.lock() {
                                                    let m = g.entry(name.clone()).or_default();
                                                    if let Some(k) = del_key {
                                                        m.remove(&k);
                                                    }
                                                    if let Some((k, v)) = upsert {
                                                        m.insert(k, v);
                                                    }
                                                }
                                                pm.configs_dirty = true;
                                            }
                                        }
                                    });
                                });
                            ui.add_space(10.0);
                        }
                        // 应用启停（即时热开关）
                        if let Some((name, on)) = toggle {
                            let res = match self.plugins.as_mut() {
                                Some(pm) => pm.set_enabled(&name, on),
                                None => Err("插件运行时未就绪".to_string()),
                            };
                            match res {
                                Ok(()) => {
                                    msgs.push(format!("插件「{name}」已{}", if on { "启用" } else { "禁用" }));
                                    // ★ 启用状态落盘（修复「每次重进插件都变回关闭、需要重新开启」）：
                                    // 以前只改了 PluginManager.states，cfg.plugin_states 从不写回，
                                    // reload_all 的 unwrap_or(false) 让插件每次启动都按禁用加载。
                                    if let Some(pm) = self.plugins.as_ref() {
                                        self.cfg.plugin_states = pm.states.clone();
                                    }
                                    self.save_config();
                                    if on {
                                        // 单播 on_enabled（D9）：脚本可立即应用自己的背景效果。
                                        // 用 emit_to 而不是广播，否则「启用 A」会把所有插件的
                                        // on_enabled 都触发一遍（各自重设一次窗口背景）。
                                        if let Some(pm) = self.plugins.as_mut() {
                                            pm.emit_to(&name, "enabled", Vec::new());
                                        }
                                    } else {
                                        // D2：被禁用的插件若正在持有窗口背景效果 → 立即回收
                                        if self.plugin_bg_owner.as_deref() == Some(name.as_str()) {
                                            let ctx = self.egui_ctx.clone();
                                            let op = self.plugin_bg_opacity;
                                            self.apply_bg(plugins::BgStyle::Default, &ctx, op, None);
                                            msgs.push(format!(
                                                "插件「{name}」持有窗口背景效果，已自动恢复默认背景"
                                            ));
                                        }
                                    }
                                }
                                Err(e) => msgs.push(format!("插件操作失败：{e}")),
                            }
                        }
                        if let Some(name) = unload_name {
                            if let Some(pm) = self.plugins.as_mut() {
                                pm.unload(&name);
                            }
                            if let Some(pm) = self.plugins.as_ref() {
                                self.cfg.plugin_states = pm.states.clone();
                            }
                            self.save_config();
                            // D2：卸载的插件若正在持有背景效果 → 立即回收
                            if self.plugin_bg_owner.as_deref() == Some(name.as_str()) {
                                let ctx = self.egui_ctx.clone();
                                let op = self.plugin_bg_opacity;
                                self.apply_bg(plugins::BgStyle::Default, &ctx, op, None);
                                msgs.push(format!("插件「{name}」已卸载，窗口背景已恢复默认"));
                            } else {
                                msgs.push(format!("插件「{name}」已卸载"));
                            }
                        }
                        // 插件日志（可折叠）
                        ui.separator();
                        let log_lines: Vec<String> = match self.plugins.as_ref() {
                            Some(pm) => pm.log_lines.iter().rev().take(60).cloned().collect(),
                            None => Vec::new(),
                        };
                        egui::CollapsingHeader::new(
                            RichText::new(format!("插件日志（{}）", log_lines.len()))
                                .strong()
                                .size(15.0),
                        )
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("plugin_log_scroll")
                                .max_height(220.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    if log_lines.is_empty() {
                                        ui.label("（暂无日志）");
                                    } else {
                                        for line in &log_lines {
                                            ui.label(RichText::new(line.as_str()).monospace().size(12.0));
                                        }
                                    }
                                });
                        });
                        ui.add_space(16.0);
                    });
            });
        for m in msgs {
            self.set_toast(m);
        }
        if let Some((pname, bg)) = bg_action {
            // 不透明度取该插件配置区的 bg_alpha（无则安全默认值）。
            // 历史配置里可能是 0.00（旧「毛玻璃暗度」滑到底 = 完全无效果），
            // 这种情况下沿用 0 会让人以为「点了没反应」，故抬到默认值并写回配置，
            // 保证「滑杆显示的值」与「实际生效的浓度」一致。
            let stored = self
                .plugins
                .as_ref()
                .and_then(|pm| pm.configs.lock().ok())
                .and_then(|g| {
                    g.get(&pname)
                        .and_then(|m| m.get("bg_alpha"))
                        .and_then(|v| v.parse::<f32>().ok())
                })
                .unwrap_or(default_alpha);
            let alpha = Self::norm_bg_opacity(stored, default_alpha);
            self.apply_bg(bg, ctx, alpha, Some(&pname));
            // 背景效果状态写回该插件配置区（bg_style 保留键），脚本端可用 xmst_config_get 读到，
            // 下次启动也由 tick_plugins 的恢复逻辑读回
            if let Some(pm) = self.plugins.as_mut() {
                if let Ok(mut g) = pm.configs.lock() {
                    let m = g.entry(pname.clone()).or_default();
                    m.insert("bg_style".to_string(), bg.key().to_string());
                    // 非默认模式把实际生效的不透明度写回（含把历史 0.00 纠正为默认值），
                    // 保证卡片滑杆显示值与真实浓度一致
                    if bg != plugins::BgStyle::Default {
                        m.insert("bg_alpha".to_string(), format!("{alpha:.2}"));
                    }
                }
                pm.configs_dirty = true;
            }
        }
        if let Some(a) = bg_reapply {
            // 滑杆即时生效：任一种非默认模式都要重新应用（半透明改的是窗口级均匀 alpha，
            // 模糊类改的是 DWM 着色 —— 两者都不是主题层每帧自动读取的）
            self.plugin_bg_opacity = a.clamp(0.0, 1.0);
            if self.plugin_bg_style != plugins::BgStyle::Default {
                let style = self.plugin_bg_style;
                self.apply_bg(style, ctx, a, None); // owner 不变
            } else {
                ctx.request_repaint();
            }
        }
    }
}

/// 插件事件总线：本帧待分发事件（避免在借用 runtimes 时调用 self.plugins）
enum PluginEvt {
    ServerStarted(String),
    ServerStopped(String, String),
    LogLine(String, String),
    PlayerJoined(String, String),
    PlayerLeft(String, String),
    BackupDone(String, String, i64),
}

impl App {
    // ==================== B7 创建服务器 ====================

    /// 创建服务器回传：版本列表 / 下载进度，完成后自动建服
    fn tick_create_server(&mut self) {
        let Some(cs) = self.create_server.as_mut() else { return };
        // 版本列表回传
        if let Some(shared) = cs.version_shared.take() {
            let done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
            if let Some(res) = done {
                match res {
                    Ok(v) => {
                        cs.versions = v;
                        cs.version_error = None;
                        // 自动选中最新版本
                        if let Some(first) = cs.versions.first() {
                            cs.selected_version = first.clone();
                        }
                    }
                    Err(e) => cs.version_error = Some(e),
                }
                cs.version_busy = false;
            } else {
                cs.version_shared = Some(shared);
            }
        }
        // 下载进度回传
        if let Some(shared) = cs.progress_shared.take() {
            let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
            cs.progress = snap;
            if cs.progress.done {
                if let Some(e) = &cs.progress.error {
                    cs.last_error = Some(format!("下载失败: {e}"));
                    cs.downloading = false;
                } else {
                    self.finish_create_server();
                }
            } else {
                cs.progress_shared = Some(shared);
            }
        }
    }

    /// 后台线程拉取服务端版本列表（B7）
    fn start_create_versions(&mut self) {
        let Some(cs) = self.create_server.as_mut() else { return };
        if cs.version_busy {
            return;
        }
        cs.version_busy = true;
        cs.version_error = None;
        cs.selected_version.clear();
        let kind = cs.kind;
        let shared: std::sync::Arc<std::sync::Mutex<Option<Result<Vec<String>, String>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        cs.version_shared = Some(shared.clone());
        let cctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let res = server_download::fetch_versions(kind);
            *shared.lock().unwrap() = Some(res);
            cctx.request_repaint();
        });
    }

    /// 后台线程下载服务端 jar（B7），完成后由 tick_create_server 建服
    fn start_create_download(&mut self) {
        let (kind, version, folder, last_error) = {
            let Some(cs) = self.create_server.as_mut() else { return };
            if cs.downloading || cs.selected_version.is_empty() {
                return;
            }
            let name = sanitize_server_name(&cs.folder_name);
            let folder = if name.is_empty() || name == "server" {
                format!("server-{}", cs.selected_version)
            } else {
                name
            };
            (cs.kind, cs.selected_version.clone(), folder, cs.last_error.clone())
        };
        let _ = last_error;
        let dir = self.data_dir().join("servers").join(&folder);
        let file_name = format!(
            "{}-{}.jar",
            server_download::file_prefix(kind),
            version
        );
        let url = match server_download::resolve_download_url(kind, &version) {
            Ok(u) => u,
            Err(e) => {
                if let Some(cs) = self.create_server.as_mut() {
                    cs.last_error = Some(format!("获取下载地址失败: {e}"));
                }
                return;
            }
        };
        let shared: std::sync::Arc<std::sync::Mutex<download::DlProgress>> =
            std::sync::Arc::new(std::sync::Mutex::new(download::DlProgress::default()));
        {
            let Some(cs) = self.create_server.as_mut() else { return };
            cs.downloading = true;
            cs.finished = false;
            cs.last_error = None;
            cs.progress = download::DlProgress::default();
            cs.progress_shared = Some(shared.clone());
        }
        let dest = dir.join(&file_name);
        let cctx = self.egui_ctx.clone();
        let kind_label = kind.label().to_string();
        self.notify("开始下载服务端", &format!("{kind_label} {version}"));
        std::thread::spawn(move || {
            let client = download::new_client();
            let res = download::download_file_parallel(&client, &url, &dest, 0, &mut |d, t, ph| {
                if let Ok(mut g) = shared.lock() {
                    g.downloaded = d;
                    g.total = t;
                    g.phase = ph;
                }
            });
            if let Ok(mut g) = shared.lock() {
                match res {
                    Ok(()) => {
                        g.phase = "done";
                        g.done = true;
                    }
                    Err(e) => {
                        g.phase = "error";
                        g.error = Some(e);
                        g.done = true;
                    }
                }
            }
            cctx.request_repaint();
        });
    }

    /// 下载完成：写 eula、创建 ServerConfig、加入列表并选中（B7）
    fn finish_create_server(&mut self) {
        let (folder, jar_name, kind, mc_ver) = {
            let Some(cs) = self.create_server.as_mut() else { return };
            let name = sanitize_server_name(&cs.folder_name);
            let folder = if name.is_empty() || name == "server" {
                format!("server-{}", cs.selected_version)
            } else {
                name
            };
            let jar_name = format!(
                "{}-{}.jar",
                server_download::file_prefix(cs.kind),
                cs.selected_version
            );
            (folder, jar_name, cs.kind, cs.selected_version.clone())
        };
        let dir = self.data_dir().join("servers").join(&folder);
        // 目标目录已存在同名服务器：提示并放弃（避免覆盖）
        let norm = |p: &std::path::Path| -> String {
            p.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_lowercase()
        };
        if self.cfg.servers.iter().any(|s| norm(std::path::Path::new(&s.dir)) == norm(&dir)) {
            if let Some(cs) = self.create_server.as_mut() {
                cs.last_error = Some("该文件夹名已在服务器列表中，请更换".to_string());
                cs.downloading = false;
            }
            return;
        }
        if std::fs::create_dir_all(&dir).is_err() {
            if let Some(cs) = self.create_server.as_mut() {
                cs.last_error = Some("创建服务器目录失败".to_string());
                cs.downloading = false;
            }
            return;
        }
        // eula 自动同意
        let _ = std::fs::write(dir.join("eula.txt"), "eula=true\n");
        let mut sc = ServerConfig::default();
        sc.name = folder.clone();
        sc.dir = dir;
        sc.mc_version = Some(mc_ver.clone());
        sc.launch_cmd = if matches!(kind, server_download::ServerKind::Forge | server_download::ServerKind::NeoForge) {
            format!("{{java}} {{jvm}} -jar {jar_name} --installServer")
        } else {
            format!("{{java}} {{jvm}} -jar {jar_name} nogui")
        };
        sc.backup.last_backup = Some(Local::now().to_rfc3339());
        sc.download_count = 1;
        sc.first_download_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        self.cfg.servers.push(sc);
        self.runtimes.push(ServerRuntime::default());
        self.selected_server = Some(self.cfg.servers.len() - 1);
        self.save_config();
        if let Some(cs) = self.create_server.as_mut() {
            cs.finished = true;
            cs.downloading = false;
            cs.created_idx = Some(self.cfg.servers.len() - 1);
        }
        let tip = if matches!(kind, server_download::ServerKind::Forge | server_download::ServerKind::NeoForge) {
            format!("服务器「{folder}」创建完成（{mc_ver}）。首次启动将运行安装器，安装完成后请到服务器设置把启动命令改为服务端 jar / run.bat")
        } else {
            format!("服务器「{folder}」创建完成（{mc_ver}），eula 已自动同意")
        };
        self.set_toast(tip);
        self.notify("服务器创建完成", &format!("{folder}（{mc_ver}）"));
    }

    /// 创建服务器弹窗（B7）
    fn ui_create_server(&mut self, ctx: &egui::Context) {
        let mut kind_changed = false;
        let mut do_versions = false;
        let mut do_download = false;
        let mut do_start = false;
        let mut do_close = false;
        let mut open = self.create_server.is_some();
        let is_light = self.theme_is_light();
                    let fg = |c: egui::Color32| if is_light { theme::light_adapt(c) } else { c };
        if let Some(cs) = self.create_server.as_mut() {
            egui::Window::new("创建新服务器")
                .resizable(true)
                .collapsible(false)
                .resizable(false)
                .default_width(460.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    // 服务端类型
                    ui.horizontal(|ui| {
                        ui.label("服务端类型：");
                        egui::ComboBox::from_id_salt("cs_kind")
                            .selected_text(cs.kind.label())
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for k in server_download::ServerKind::all() {
                                    if ui.selectable_value(&mut cs.kind, k, k.label()).changed() {
                                        kind_changed = true;
                                    }
                                }
                            });
                    });
                    // 版本
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("版本：");
                        if cs.version_busy {
                            ui.spinner();
                            ui.label("加载中…");
                        }
                        if !cs.version_busy && ui.button("刷新版本").clicked() {
                            do_versions = true;
                        }
                    });
                    if let Some(e) = &cs.version_error {
                        ui.colored_label(fg(Color32::from_rgb(255, 90, 90)), format!("版本获取失败: {e}"));
                    }
                    if !cs.versions.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label("搜索：");
                            ui.add(egui::TextEdit::singleline(&mut cs.version_filter).hint_text("输入版本号过滤").desired_width(220.0));
                            // 默认隐藏快照版（快照不适合长期开服），可一键显示
                            ui.checkbox(&mut cs.show_snapshot, "显示快照版")
                                .on_hover_text("默认只列出正式版；勾选后显示 snapshot/预发布版");
                        });
                        let filtered = cs.filtered_versions();
                        egui::ComboBox::from_id_salt("cs_version")
                            .selected_text(if cs.selected_version.is_empty() {
                                "请选择版本"
                            } else {
                                &cs.selected_version
                            })
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for v in &filtered {
                                    if ui.selectable_label(cs.selected_version == *v, v).clicked() {
                                        cs.selected_version = v.clone();
                                    }
                                }
                            });
                        if filtered.is_empty() {
                            ui.label(
                                RichText::new("（当前过滤条件下没有正式版；可勾选「显示快照版」）")
                                    .weak()
                                    .small(),
                            );
                        }
                    }
                    // 文件夹名
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("文件夹名：");
                        ui.add(egui::TextEdit::singleline(&mut cs.folder_name).hint_text("留空自动生成 server-<版本>").desired_width(240.0));
                    });
                    // 下载进度
                    if cs.downloading {
                        ui.add_space(6.0);
                        ui.add(
                            egui::ProgressBar::new(cs.progress.fraction())
                                .text(format!("{:.1}%", cs.progress.percent())),
                        );
                        ui.label(
                            RichText::new(cs.progress.phase.to_string())
                                .color(fg(Color32::from_rgb(150, 150, 150))),
                        );
                    }
                    if let Some(e) = &cs.last_error {
                        ui.add_space(6.0);
                        ui.colored_label(fg(Color32::from_rgb(255, 90, 90)), e);
                    }
                    if cs.finished {
                        ui.add_space(6.0);
                        ui.label(RichText::new("下载完成，服务器已创建。").color(fg(Color32::from_rgb(120, 220, 120))));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if cs.finished {
                            if ui.button("▶ 启动服务器").clicked() {
                                do_start = true;
                            }
                            if ui.button("关闭").clicked() {
                                do_close = true;
                            }
                        } else {
                            let can_dl = !cs.downloading && !cs.selected_version.is_empty();
                            if ui.add_enabled(can_dl, egui::Button::new("开始下载")).clicked() {
                                do_download = true;
                            }
                            if ui.button("取消").clicked() {
                                do_close = true;
                            }
                        }
                    });
                });
        } else {
            return;
        }
        // 用户点击窗口 X 关闭按钮：egui 会将 open 置 false，此时关闭弹窗状态
        if !open {
            self.create_server = None;
        }
        if kind_changed {
            if let Some(cs) = self.create_server.as_mut() {
                cs.versions.clear();
                cs.selected_version.clear();
                cs.version_filter.clear();
                cs.version_error = None;
                cs.version_busy = false;
                cs.version_shared = None;
            }
            self.start_create_versions();
        }
        if do_versions {
            self.start_create_versions();
        }
        if do_download {
            self.start_create_download();
        }
        if do_start {
            let idx = self.create_server.as_ref().and_then(|c| c.created_idx);
            self.create_server = None;
            if let Some(i) = idx {
                self.selected_server = Some(i);
                self.start_server(i);
            }
        }
        if do_close {
            self.create_server = None;
        }
    }

    fn tick_download(&mut self) {
        if self.dl.is_none() {
            return;
        }
        let mut wake = false;
        let mut toasts: Vec<String> = Vec::new();
        // 图标解码结果（块内只解码不碰 self.egui_ctx，避免与 dl 借用冲突）
        let mut icon_ready: Vec<(String, egui::ColorImage)> = Vec::new();
        {
            let dl = self.dl.as_mut().unwrap();
            // custom download callback（图1 自定义下载标签页）
            if let Some(shared) = dl.custom_shared.take() {
                let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
                dl.custom_progress = snap;
                if dl.custom_progress.done {
                    let msg = if let Some(e) = &dl.custom_progress.error {
                        format!("自定义下载失败: {e}")
                    } else {
                        format!(
                            "自定义下载完成: {}",
                            Path::new(&dl.custom_dest_dir)
                                .join(&dl.custom_file_name)
                                .display()
                        )
                    };
                    dl.custom_busy = false;
                    toasts.push(msg.clone());
                    dl.dl_log.push(msg);
                    wake = true;
                } else {
                    dl.custom_shared = Some(shared);
                }
            }
            // mod download callback
            if let Some(shared) = dl.mod_dl_shared.take() {
                let snap = shared.lock().map(|g| g.clone()).unwrap_or_default();
                dl.mod_dl_progress = snap;
                if dl.mod_dl_progress.done {
                    let msg = if let Some(e) = &dl.mod_dl_progress.error {
                        format!("模组下载失败: {e}")
                    } else {
                        format!("模组已安装: {}", dl.mod_download_name)
                    };
                    dl.mod_dl_busy = false;
                    toasts.push(msg.clone());
                    dl.dl_log.push(msg.clone());
                    wake = true;
                } else {
                    dl.mod_dl_shared = Some(shared);
                }
            }
            // mod search callback
            if let Some(shared) = dl.mod_search_shared.take() {
                let done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
                if let Some(res) = done {
                    match res {
                        Ok(res) => {
                            dl.mod_results = res.hits;
                            dl.mod_total_hits = res.total_hits;
                            dl.dl_log.push("模组搜索完成".to_string());
                        }
                        Err(e) => {
                            dl.mod_search_error = Some(e);
                            dl.dl_log.push("模组搜索失败".to_string());
                        }
                    }
                    dl.mod_search_busy = false;
                    wake = true;
                } else {
                    dl.mod_search_shared = Some(shared);
                }
            }
            // favorites callback（图2 收藏夹）
            if let Some(shared) = dl.mod_fav_shared.take() {
                let done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
                if let Some(res) = done {
                    match res {
                        Ok(v) => {
                            dl.mod_fav_hits = v;
                            dl.dl_log.push("收藏夹已加载".to_string());
                        }
                        Err(e) => {
                            dl.mod_fav_error = Some(e);
                            dl.dl_log.push("收藏夹加载失败".to_string());
                        }
                    }
                    dl.mod_fav_busy = false;
                    wake = true;
                } else {
                    dl.mod_fav_shared = Some(shared);
                }
            }
            // mod versions callback（详情页版本列表）
            if let Some(shared) = dl.mod_ver_shared.take() {
                let done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
                if let Some(res) = done {
                    match res {
                        Ok(v) => {
                            dl.mod_versions = v;
                            dl.dl_log.push("版本列表已加载".to_string());
                        }
                        Err(e) => {
                            dl.mod_ver_error = Some(e);
                            dl.dl_log.push("版本列表加载失败".to_string());
                        }
                    }
                    dl.mod_ver_busy = false;
                    wake = true;
                } else {
                    dl.mod_ver_shared = Some(shared);
                }
            }
            // 详情页简介翻译 callback
            if let Some(shared) = dl.mod_translate_shared.take() {
                let done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
                if let Some(res) = done {
                    match res {
                        Ok(t) => {
                            dl.mod_translate_result = Some(t);
                            dl.mod_translate_hidden = false;
                            dl.dl_log.push("简介翻译完成".to_string());
                        }
                        Err(e) => {
                            dl.mod_translate_error = Some(e);
                            dl.dl_log.push("简介翻译失败".to_string());
                        }
                    }
                    dl.mod_translate_busy = false;
                    wake = true;
                } else {
                    dl.mod_translate_shared = Some(shared);
                }
            }
            // 详情页"打开页面" source_url 拉取回调：成功后打开系统浏览器
            if let Some(shared) = dl.mod_open_shared.take() {
                let done = shared.lock().map(|mut g| g.take()).unwrap_or(None);
                if let Some(res) = done {
                    match res {
                        Ok(url) => {
                            let url_ref: &str = &url;
                            let _ = std::process::Command::new("cmd")
                                .args(["/C", "start", "", url_ref])
                                .spawn();
                            dl.dl_log.push(format!("已打开页面 {url}"));
                        }
                        Err(e) => {
                            dl.dl_log.push(format!("打开页面失败: {e}"));
                        }
                    }
                    wake = true;
                } else {
                    dl.mod_open_shared = Some(shared);
                }
            }
            // 模组图标下载 callback：解码字节并缓存纹理（图2 网站图标）
            if !dl.mod_icon_shared.is_empty() {
                let done: Vec<(String, Result<Vec<u8>, String>)> = dl
                    .mod_icon_shared
                    .iter()
                    .filter_map(|(id, s)| {
                        s.lock()
                            .ok()
                            .and_then(|mut g| g.take())
                            .map(|r| (id.clone(), r))
                    })
                    .collect();
                for (id, res) in done {
                    dl.mod_icon_shared.remove(&id);
                    dl.mod_icon_pending.remove(&id);
                    match res {
                        Ok(bytes) => {
                            if let Ok(img) = egui_extras::image::load_image_bytes(&bytes) {
                                icon_ready.push((id, img));
                            } else {
                                // 解码失败（如格式不支持）记入失败集，避免每帧重下
                                dl.mod_icon_failed.insert(id);
                            }
                        }
                        Err(_) => {
                            dl.mod_icon_failed.insert(id);
                        }
                    }
                }
            }
            // MC 百科中文名查询回传：命中入缓存 + 百科 URL；未收录记入失败集（显示"译"按钮）
            if !dl.mod_mcmod_shared.is_empty() {
                let done: Vec<(String, Result<(String, String), String>)> = dl
                    .mod_mcmod_shared
                    .iter()
                    .filter_map(|(t, s)| {
                        s.lock()
                            .ok()
                            .and_then(|mut g| g.take())
                            .map(|r| (t.clone(), r))
                    })
                    .collect();
                for (t, res) in done {
                    dl.mod_mcmod_shared.remove(&t);
                    dl.mod_mcmod_pending.remove(&t);
                    match res {
                        Ok((cn, url)) => {
                            let cn = cn.trim().to_string();
                            if !cn.is_empty() {
                                dl.mod_cn_name.insert(t.clone(), cn);
                                dl.mod_mcmod_url.insert(t, url);
                            } else {
                                dl.mod_cn_failed.insert(t);
                            }
                        }
                        Err(_) => {
                            dl.mod_cn_failed.insert(t);
                        }
                    }
                }
                wake = true;
            }
            // 模组标题中文名翻译回传：成功入缓存，失败记入失败集（显示"译"按钮）
            if !dl.mod_cn_shared.is_empty() {
                let done: Vec<(String, Result<String, String>)> = dl
                    .mod_cn_shared
                    .iter()
                    .filter_map(|(t, s)| {
                        s.lock()
                            .ok()
                            .and_then(|mut g| g.take())
                            .map(|r| (t.clone(), r))
                    })
                    .collect();
                for (t, res) in done {
                    dl.mod_cn_shared.remove(&t);
                    dl.mod_cn_pending.remove(&t);
                    match res {
                        Ok(cn) => {
                            let cn = cn.trim().to_string();
                            if !cn.is_empty() && cn != t {
                                dl.mod_cn_name.insert(t, cn);
                            } else {
                                dl.mod_cn_failed.insert(t);
                            }
                        }
                        Err(_) => {
                            dl.mod_cn_failed.insert(t);
                        }
                    }
                }
                wake = true;
            }
        }
        // 图标纹理上屏（脱离 dl 借用后再访问 self.egui_ctx）
        for (id, img) in icon_ready {
            let name = format!("mod_icon_{}", id);
            let tex = self
                .egui_ctx
                .load_texture(&name, img, egui::TextureOptions::LINEAR);
            self.dl.as_mut().unwrap().mod_icon_tex.insert(id, tex);
            wake = true;
        }
        for t in toasts {
            self.set_toast(t);
        }
        if wake {
            // 下载完成唤醒一次 repaint（托盘态不轮询进度，仅此一次）
            self.egui_ctx.request_repaint();
        }
    }

    /// 启动自定义下载线程（图1 自定义下载标签页：任意 URL → 保存目录）
    fn dl_start_custom_download(&mut self) {
        let (url, dest_dir, file_name, threads) = {
            let dl = self.dl.as_mut().unwrap();
            (
                dl.custom_url.trim().to_string(),
                dl.custom_dest_dir.trim().to_string(),
                dl.custom_file_name.trim().to_string(),
                dl.custom_threads.clone(),
            )
        };
        if url.is_empty() || dest_dir.is_empty() || file_name.is_empty() {
            return;
        }
        let threads = threads.trim().parse::<u64>().unwrap_or(0);
        let shared = std::sync::Arc::new(std::sync::Mutex::new(download::DlProgress::default()));
        {
            let dl = self.dl.as_mut().unwrap();
            dl.custom_busy = true;
            dl.custom_progress = download::DlProgress::default();
            dl.custom_shared = Some(std::sync::Arc::clone(&shared));
            dl.dl_log.push(format!(
                "开始自定义下载: {file_name}（线程数 {threads}）"
            ));
        }
        let client = download::new_client();
        let dest = std::path::PathBuf::from(&dest_dir).join(&file_name);
        std::thread::spawn(move || {
            let result = download::download_file_parallel(&client, &url, &dest, threads, &mut |d, t, p| {
                if let Ok(mut g) = shared.lock() {
                    g.downloaded = d;
                    g.total = t;
                    g.phase = p;
                    if p == "done" {
                        g.done = true;
                    }
                }
            });
            if let Err(e) = result {
                if let Ok(mut g) = shared.lock() {
                    g.phase = "error";
                    g.error = Some(e);
                    g.done = true;
                }
            }
        });
    }

    /// 启动模组下载线程（jar 落服务器 mods/ 目录）
    fn dl_start_mod_download(&mut self) {
        let (url, dest_path) = {
            let dl = self.dl.as_mut().unwrap();
            (dl.mod_download_target.clone(), dl.mod_download_name.clone())
        };
        if url.is_empty() || dest_path.is_empty() {
            return;
        }
        let dest = std::path::PathBuf::from(&dest_path);
        let shared = std::sync::Arc::new(std::sync::Mutex::new(download::DlProgress::default()));
        {
            let dl = self.dl.as_mut().unwrap();
            dl.mod_dl_busy = true;
            dl.mod_dl_progress = download::DlProgress::default();
            dl.mod_dl_shared = Some(std::sync::Arc::clone(&shared));
            let name = dest
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            dl.dl_log.push(format!("开始安装模组: {name}"));
        }
        let client = download::new_client();
        std::thread::spawn(move || {
            let result = download::download_file_parallel(&client, &url, &dest, 0, &mut |d, t, p| {
                if let Ok(mut g) = shared.lock() {
                    g.downloaded = d;
                    g.total = t;
                    g.phase = p;
                    if p == "done" {
                        g.done = true;
                    }
                }
            });
            if let Err(e) = result {
                if let Ok(mut g) = shared.lock() {
                    g.phase = "error";
                    g.error = Some(e);
                    g.done = true;
                }
            }
        });
    }

    /// 网络下载页（B4 服务端下载 + Modrinth 模组下载/安装）
    fn ui_download(&mut self, ctx: &egui::Context) {
        if self.dl.is_none() {
            self.set_toast("请在设置页「测试中的功能」启用「网络下载」".to_string());
            self.nav = Nav::Settings;
            return;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.panel_fill))
            .show(ctx, |ui| {
                // 第二批：max-width 1100 左对齐（下载页，ScrollArea 移入容器内）
                let _avail = ui.available_rect_before_wrap();
                // 左侧留出 14px 内边距：此前内容紧贴侧栏/窗口边缘，文字看起来"贴太近"。
                let _pad = 14.0f32;
                let _w = (_avail.width() - _pad).min(1100.0);
                let _centered = egui::Rect::from_min_size(
                    egui::pos2(_avail.left() + _pad, _avail.top()),
                    egui::vec2(_w, _avail.height()),
                );
                let mut _inner = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(_centered)
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt("page_center_1100"),
                );
                _inner.set_width(_w);
                let ui = &mut _inner;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        ui.heading("下载");
                        ui.separator();
                        self.ui_dl_custom(ui);
                        ui.separator();
                        self.ui_dl_modrinth(ui);
                        ui.separator();
                        self.ui_dl_log(ui);
                        ui.add_space(16.0);
                    });
            });
    }

    /// 下载区块（图1：任意 URL 自定义下载；服务端下载已并入「创建服务器」）
    fn ui_dl_custom(&mut self, ui: &mut egui::Ui) {
        self.ui_dl_custom_tab(ui);
    }



    /// 图1「自定义下载」标签页：URL + 文件名 + 保存目录 + 线程数 + 开始下载
    fn ui_dl_custom_tab(&mut self, ui: &mut egui::Ui) {
        let is_light = self.theme_is_light();
                    let fg = |c: egui::Color32| if is_light { theme::light_adapt(c) } else { c };
        let dl = self.dl.as_mut().unwrap();
        // 两列网格表单（行1：下载地址|文件名；行2：保存目录|线程数）
        egui::Grid::new("dl_custom_form_grid")
            .num_columns(4)
            .spacing([12.0, 10.0])
            .show(ui, |ui| {
                ui.label("下载地址");
                ui.add(
                    egui::TextEdit::singleline(&mut dl.custom_url)
                        .hint_text("https://…")
                        .min_size(egui::vec2(380.0, 26.0)),
                );
                ui.label("文件名");
                ui.add(
                    egui::TextEdit::singleline(&mut dl.custom_file_name)
                        .hint_text("如 server.jar")
                        .min_size(egui::vec2(200.0, 26.0)),
                );
                ui.end_row();
                ui.label("保存目录");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut dl.custom_dest_dir)
                            .hint_text("选择保存目录")
                            .min_size(egui::vec2(260.0, 26.0)),
                    );
                    if ui.button("选择目录…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            dl.custom_dest_dir = p.display().to_string();
                        }
                    }
                });
                ui.label("线程数");
                ui.add(
                    egui::TextEdit::singleline(&mut dl.custom_threads)
                        .min_size(egui::vec2(70.0, 26.0)),
                );
                ui.end_row();
            });
        if dl.custom_busy {
            // 两段式下载行：左侧进度信息区 + 竖分割线 + 右侧固定 200px 操作列
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let left_w = (ui.available_width() - 212.0).max(140.0);
                    ui.set_width(left_w);
                    ui.add(
                        egui::ProgressBar::new(dl.custom_progress.fraction())
                            .desired_width(left_w)
                            .text(format!("{:.1}%", dl.custom_progress.percent())),
                    );
                    let (total_txt, done_txt) = match dl.custom_progress.total {
                        Some(t) => (fmt_size(t), fmt_size(dl.custom_progress.downloaded)),
                        None => ("未知大小".to_string(), fmt_size(dl.custom_progress.downloaded)),
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("阶段: {}", dl.custom_progress.phase))
                                .size(11.0)
                                .color(fg(Color32::from_rgb(150, 150, 150))),
                        );
                        ui.label(
                            RichText::new(format!("{done_txt} / {total_txt}"))
                                .size(11.0)
                                .color(fg(Color32::from_rgb(150, 150, 150))),
                        );
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    let btn = egui::Button::new("📂 打开目录")
                        .min_size(egui::vec2(96.0, 26.0));
                    if ui.add(btn).clicked() {
                        if !dl.custom_dest_dir.trim().is_empty() {
                            explorer_select(&dl.custom_dest_dir);
                        }
                    }
                });
            });
        }
        let can_dl = !dl.custom_busy
            && !dl.custom_url.trim().is_empty()
            && !dl.custom_dest_dir.trim().is_empty()
            && !dl.custom_file_name.trim().is_empty();
        if ui
            .add_enabled(can_dl, egui::Button::new("开始下载"))
            .clicked()
        {
            self.dl_start_custom_download();
        }
    }

    /// Modrinth 模组/插件/数据包下载/安装区块（B6：类型 + 标签 + 排序 + 目标归类）
    /// 下载页 - 模组社区（图2：左侧导航 + 搜索筛选 + 卡片列表）
    fn ui_dl_modrinth(&mut self, ui: &mut egui::Ui) {
        egui::SidePanel::left("dl_comm_nav")
            .resizable(false)
            .exact_width(150.0)
            .frame(egui::Frame::side_top_panel(&ui.ctx().style()).fill(ui.ctx().style().visuals.window_fill))
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                let nav_items: &[(&str, ModCommunityNav)] = &[
                    ("模组", ModCommunityNav::Mod),
                    ("插件", ModCommunityNav::Plugin),
                    ("数据包", ModCommunityNav::Datapack),
                ];
                let cur = self.dl.as_ref().unwrap().mod_nav;
                for (label, val) in nav_items {
                    let sel = cur == *val;
                    if ui.selectable_label(sel, *label).clicked() {
                        let d = self.dl.as_mut().unwrap();
                        d.mod_nav = *val;
                        d.mod_project_type = val.project_type().to_string();
                        d.mod_results.clear();
                        // 阶段16①：点击左侧分类即收起详情页（清空详情状态，列表切换到该分类重新加载）
                        d.mod_detail_id = None;
                        d.mod_detail_hit = None;
                        d.mod_versions.clear();
                        d.mod_ver_busy = false;
                        d.mod_ver_error = None;
                        d.mod_ver_shared = None;
                        d.mod_ver_loader = "全部".to_string();
                        d.mod_detail_mc = String::new();
                        d.mod_ver_preview = None;
                        d.mod_ver_log_open = None;
                        d.mod_translate_result = None;
                        d.mod_translate_hidden = false;
                        d.mod_translate_busy = false;
                        d.mod_translate_error = None;
                        d.mod_translate_shared = None;
                        // 切换分类后自动搜索该分类（空关键词浏览模式）
                        if !val.project_type().is_empty() {
                            self.dl_mod_search_start();
                        }
                    }
                }
                ui.add_space(8.0);
                let sel = cur == ModCommunityNav::Favorites;
                if ui.selectable_label(sel, "收藏栏").clicked() {
                    let d = self.dl.as_mut().unwrap();
                    d.mod_nav = ModCommunityNav::Favorites;
                    d.mod_project_type = "".to_string();
                    d.mod_results.clear();
                }
            });
        ui.separator();
        match self.dl.as_ref().unwrap().mod_nav {
            ModCommunityNav::Favorites => {
                self.ui_dl_favorites(ui);
            }
            _ => {
                self.ui_dl_community_search(ui);
            }
        }
    }


    /// 收藏夹页（图2：收藏的模组卡片列表）
    fn ui_dl_favorites(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("收藏夹").strong().size(18.0));
            if ui.button("刷新").clicked() {
                self.dl_fav_load_start();
            }
            if self.dl.as_ref().unwrap().mod_fav_busy {
                ui.spinner();
            }
        });
        if let Some(e) = &self.dl.as_ref().unwrap().mod_fav_error {
            ui.colored_label(self.fg(Color32::from_rgb(220, 80, 80)), format!("加载失败: {e}"));
        }
        let favs = self.cfg.mod_favorites.clone();
        if favs.is_empty() && !self.dl.as_ref().unwrap().mod_fav_busy {
            ui.add_space(8.0);
            ui.label(
                RichText::new("暂无收藏。在模组 / 插件 / 数据包社区卡片上点 ☆ 即可收藏。")
                    .color(self.fg(Color32::from_rgb(150, 150, 150))),
            );
            return;
        }
        let mut install: Option<String> = None;
        let mut unfav: Option<String> = None;
        let mut detail: Option<modrinth::ModrinthHit> = None;
        egui::ScrollArea::vertical()
            .id_salt("dl_fav_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 详情展开面板（版本选择）
                if self.dl.as_ref().unwrap().mod_detail_id.is_some() {
                    self.ui_mod_detail_header(ui);
                    ui.separator();
                    self.ui_mod_detail_body(ui);
                    ui.separator();
                }
                let hits = self.dl.as_ref().unwrap().mod_fav_hits.clone();
                if hits.is_empty() && !self.dl.as_ref().unwrap().mod_fav_busy {
                    self.dl_fav_load_start();
                }
                for r in &hits {
                    let act = self.ui_mod_card(ui, r);
                    match act {
                        ModCardAction::Install => install = Some(r.id.clone()),
                        ModCardAction::FavToggle => unfav = Some(r.id.clone()),
                        ModCardAction::Detail => detail = Some(r.clone()),
                        ModCardAction::None => {}
                    }
                }
            });
        if let Some(pid) = install {
            self.mod_install_project(&pid);
        }
        if let Some(hit) = detail {
            self.dl_mod_ver_load_start(&hit.id, &hit);
        }
        if let Some(pid) = unfav {
            self.cfg.mod_favorites.retain(|x| x != &pid);
            self.save_config();
            self.dl.as_mut().unwrap().mod_fav_hits.retain(|h| h.id != pid);
            self.set_toast("已取消收藏".to_string());
        }
    }

    /// 启动收藏夹详情加载线程（逐个 project_id 拉详情）
    fn dl_fav_load_start(&mut self) {
        let ids: Vec<String> = self.cfg.mod_favorites.clone();
        let dl = self.dl.as_mut().unwrap();
        if dl.mod_fav_busy {
            return;
        }
        if ids.is_empty() {
            dl.mod_fav_hits.clear();
            return;
        }
        dl.mod_fav_busy = true;
        dl.mod_fav_error = None;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
        dl.mod_fav_shared = Some(std::sync::Arc::clone(&shared));
        std::thread::spawn(move || {
            let mut out = Vec::with_capacity(ids.len());
            let mut err: Option<String> = None;
            for id in &ids {
                match modrinth::project_by_id(id) {
                    Ok(h) => out.push(h),
                    Err(e) => {
                        err = Some(format!("{id}: {e}"));
                        break;
                    }
                }
            }
            let res = match err {
                Some(e) => Err(e),
                None => Ok(out),
            };
            *shared.lock().unwrap() = Some(res);
        });
    }

    /// 启动一次模组社区搜索（busy 时置 pending，完成后用最新参数自动重搜；空关键词也可浏览）
    fn dl_mod_search_start(&mut self) {
        let dl = self.dl.as_mut().unwrap();
        if dl.mod_search_busy {
            dl.mod_search_pending = true;
            return;
        }
        dl.mod_search_busy = true;
        dl.mod_search_pending = false;
        dl.mod_search_error = None;
        // 新搜索重置图标失败集，允许重新拉取
        dl.mod_icon_failed.clear();
        let q2 = dl.mod_query.trim().to_string();
        let pt2 = dl.mod_project_type.clone();
        let mc2 = dl.mod_mc_version.trim().to_string();
        let l2 = dl.mod_loader.clone();
        let tags2 = dl.mod_tags.trim().to_string();
        let sort2 = dl.mod_sort.clone();
        let limit = dl.mod_page_size;
        let offset = dl.mod_page * dl.mod_page_size as i64;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
        dl.mod_search_shared = Some(std::sync::Arc::clone(&shared));
        let client = download::new_client();
        std::thread::spawn(move || {
            let r = modrinth::search_mods(&q2, &pt2, &mc2, &l2, &tags2, &sort2, limit, offset);
            *shared.lock().unwrap() = Some(r);
        });
    }

    /// 启动详情页版本列表加载（记录当前展开项目；busy 时忽略）
    fn dl_mod_ver_load_start(&mut self, pid: &str, hit: &modrinth::ModrinthHit) {
        let dl = self.dl.as_mut().unwrap();
        dl.mod_detail_id = Some(pid.to_string());
        dl.mod_detail_hit = Some(hit.clone());
        // 反馈③：记录打开详情时的搜索 MC 版本，用于版本列表置顶高亮
        dl.mod_detail_mc = dl.mod_mc_version.trim().to_string();
        // 反馈④：打开详情时加载器筛选跟随搜索指定加载器
        // （若该模组无对应加载器版本，版本列表加载后会自动重置为"全部"）
        dl.mod_ver_loader = dl.mod_loader.clone();
        if dl.mod_ver_busy {
            return;
        }
        dl.mod_ver_busy = true;
        dl.mod_ver_error = None;
        dl.mod_versions.clear();
        let pid2 = pid.to_string();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
        dl.mod_ver_shared = Some(std::sync::Arc::clone(&shared));
        std::thread::spawn(move || {
            let r = modrinth::project_versions(&pid2);
            *shared.lock().unwrap() = Some(r);
        });
    }

    /// 社区搜索页（图2 主界面：搜索栏 + 两行筛选 + 卡片网格 + 返回顶部）
    fn ui_dl_community_search(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        // 首次进入本页做一次空关键词搜索：直接显示热门模组，而不是空白一片
        // （Modrinth 支持空查询，按下载量排序即可）。
        if !self
            .dl
            .as_ref()
            .map(|d| d.mod_search_started)
            .unwrap_or(true)
        {
            if let Some(d) = self.dl.as_mut() {
                d.mod_search_started = true;
                if d.mod_sort.is_empty() {
                    d.mod_sort = "downloads".to_string();
                }
            }
            self.dl_mod_search_start();
        }
        // 过滤栏统一间距（控件间距 8px）
        ui.style_mut().spacing.item_spacing.y = 8.0;
        ui.style_mut().spacing.item_spacing.x = 8.0;
        // 搜索栏（图2 顶部）：放大镜 + Search... + 回车搜索（高 26px）
        let mut do_search = false;
        ui.horizontal(|ui| {
            ui.label("🔍");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.dl.as_mut().unwrap().mod_query)
                    .hint_text("Search...")
                    .min_size(egui::vec2(300.0, 26.0)),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                do_search = true;
                resp.request_focus();
            }
            if ui
                .add_sized([64.0, 26.0], egui::Button::new("搜索"))
                .clicked()
            {
                do_search = true;
            }
            if self.dl.as_ref().unwrap().mod_search_busy {
                ui.spinner();
            }
        });
        // 过滤区独立成框（来源/标签/排序/MC版本/加载器；目标位置移入详情页下载区）
        egui::Frame::group(ui.style())
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("过滤").strong().size(13.0));
                // 筛选行1：来源 / 标签（下拉选择）/ 排序（控件高 26px）
                ui.horizontal(|ui| {
                    ui.label("来源");
            egui::ComboBox::from_id_salt("dl_comm_source")
                .selected_text("Modrinth")
                .height(26.0)
                .show_ui(ui, |ui| {
                    ui.selectable_label(true, "Modrinth");
                });
            ui.label("标签");
            let tags = self.dl.as_ref().unwrap().mod_tags.clone();
            egui::ComboBox::from_id_salt("dl_comm_tags")
                .selected_text(if tags.is_empty() {
                    "选择标签"
                } else {
                    tags.as_str()
                })
                .width(150.0)
                .height(26.0)
                .show_ui(ui, |ui| {
                    for t in modrinth::category_options() {
                        if ui.selectable_label(tags == t, t.clone()).clicked() {
                            self.dl.as_mut().unwrap().mod_tags = t.to_string();
                            do_search = true;
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(tags.is_empty(), "清除标签").clicked() {
                        self.dl.as_mut().unwrap().mod_tags.clear();
                        do_search = true;
                    }
                });
            ui.label("排序");
            let sort = self.dl.as_ref().unwrap().mod_sort.clone();
            egui::ComboBox::from_id_salt("dl_comm_sort")
                .selected_text(match sort.as_str() {
                    "downloads" => "下载量",
                    "follows" => "关注",
                    "newest" => "最新",
                    "updated" => "最近",
                    _ => "默认（相关）",
                })
                .height(26.0)
                .show_ui(ui, |ui| {
                    for (label, val) in [
                        ("默认（相关）", "relevance"),
                        ("下载量", "downloads"),
                        ("关注", "follows"),
                        ("最新", "newest"),
                        ("最近", "updated"),
                    ] {
                        if ui
                            .selectable_value(
                                &mut self.dl.as_mut().unwrap().mod_sort,
                                val.to_string(),
                                label,
                            )
                            .changed()
                        {
                            do_search = true;
                        }
                    }
                });
        });
        // 筛选行2：MC 版本 / 加载器 / 目标位置模式（控件高 26px）
        ui.horizontal(|ui| {
            ui.label("MC 版本");
            ui.add(
                egui::TextEdit::singleline(&mut self.dl.as_mut().unwrap().mod_mc_version)
                    .hint_text("任意")
                    .min_size(egui::vec2(110.0, 26.0)),
            );
            ui.label("加载器");
            let loader = self.dl.as_ref().unwrap().mod_loader.clone();
            egui::ComboBox::from_id_salt("dl_comm_loader")
                .selected_text(loader.clone())
                .height(26.0)
                .show_ui(ui, |ui| {
                    for l in modrinth::loader_options() {
                        if ui
                            .selectable_value(
                                &mut self.dl.as_mut().unwrap().mod_loader,
                                l.to_string(),
                                l,
                            )
                            .changed()
                        {
                            do_search = true;
                        }
                    }
                });
                // 显示模组数量（1-20）：变更即重搜（回第 1 页）
                ui.label("显示模组数量");
                if ui
                    .add_sized(
                        [64.0, 26.0],
                        egui::DragValue::new(&mut self.dl.as_mut().unwrap().mod_page_size)
                            .range(1..=20),
                    )
                    .changed()
                {
                    do_search = true;
                }
            });
            });
        if do_search {
            self.dl.as_mut().unwrap().mod_page = 0;
            self.dl_mod_search_start();
        }
        // 上一次搜索进行中参数再次变化（如拖拽"显示模组数量"）时：等其完成后立即用最新参数重搜
        if self.dl.as_ref().unwrap().mod_search_pending
            && !self.dl.as_ref().unwrap().mod_search_busy
        {
            self.dl.as_mut().unwrap().mod_search_pending = false;
            self.dl_mod_search_start();
        }
        if let Some(e) = &self.dl.as_ref().unwrap().mod_search_error {
            ui.colored_label(self.fg(Color32::from_rgb(220, 80, 80)), format!("搜索失败: {e}"));
        }
        // 结果区：详情展开时头固定 + body 滚动（左版本列表/右预览）；否则卡片网格
        let results = self.dl.as_ref().unwrap().mod_results.clone();
        let mut install: Option<String> = None;
        let mut favtoggle: Option<String> = None;
        let mut detail: Option<modrinth::ModrinthHit> = None;
        let mut scroll_top = false;
        if self.dl.as_ref().unwrap().mod_detail_id.is_some() {
            // 固定头：详情头 + 简介 + 翻译 + 加载器筛选（常驻顶部，可随时收起）
            self.ui_mod_detail_header(ui);
            ui.separator();
            // body：左版本列表 / 右预览，独立滚动
            egui::ScrollArea::vertical()
                .id_salt("dl_detail_body")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.ui_mod_detail_body(ui);
                });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("dl_comm_results")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // 顶部锚点（“返回顶部”按钮消费）
                    if self.dl.as_ref().unwrap().mod_scroll_top {
                        ui.scroll_to_cursor(Some(egui::Align::Min));
                        self.dl.as_mut().unwrap().mod_scroll_top = false;
                    }
                    if results.is_empty() {
                        if self.dl.as_ref().unwrap().mod_search_busy {
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("搜索中…");
                            });
                        } else {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new("暂无结果。输入关键词或调整筛选后点“搜索”。")
                                    .color(self.fg(Color32::from_rgb(150, 150, 150))),
                            );
                        }
                        return;
                    }
                    let total_hits = self.dl.as_ref().unwrap().mod_total_hits;
                    let shown = if total_hits > 0 {
                        total_hits
                    } else {
                        results.len() as i64
                    };
                    ui.label(format!("共 {shown} 个结果"));
                    // 竖向单列列表：每行一个模组条目（不横铺网格）
                    for r in &results {
                        let act = self.ui_mod_row(ui, r);
                        match act {
                            ModCardAction::Install => install = Some(r.id.clone()),
                            ModCardAction::FavToggle => {
                                favtoggle = Some(r.id.clone())
                            }
                            ModCardAction::Detail => detail = Some(r.clone()),
                            ModCardAction::None => {}
                        }
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("返回顶部").clicked() {
                    scroll_top = true;
                }
                ui.separator();
                // 分页：上一页 / 页码 / 下一页（切换时回顶部并重新搜索）
                let page = self.dl.as_ref().unwrap().mod_page;
                let page_size = self.dl.as_ref().unwrap().mod_page_size as i64;
                let total_hits = self.dl.as_ref().unwrap().mod_total_hits;
                let est_total = if total_hits > 0 {
                    total_hits
                } else {
                    results.len() as i64
                };
                let total_pages = ((est_total + page_size - 1) / page_size).max(1);
                if ui
                    .add_enabled(page > 0, egui::Button::new("上一页"))
                    .clicked()
                {
                    self.dl.as_mut().unwrap().mod_page -= 1;
                    self.dl.as_mut().unwrap().mod_scroll_top = true;
                    self.dl_mod_search_start();
                }
                ui.label(format!("第 {} / {total_pages} 页", page + 1));
                if ui
                    .add_enabled(page + 1 < total_pages, egui::Button::new("下一页"))
                    .clicked()
                {
                    self.dl.as_mut().unwrap().mod_page += 1;
                    self.dl.as_mut().unwrap().mod_scroll_top = true;
                    self.dl_mod_search_start();
                }
            });
        }
        if scroll_top {
            self.dl.as_mut().unwrap().mod_scroll_top = true;
        }
        if let Some(pid) = install {
            self.mod_install_project(&pid);
        }
        if let Some(hit) = detail {
            self.dl_mod_ver_load_start(&hit.id, &hit);
        }
        if let Some(pid) = favtoggle {
            self.mod_fav_toggle(&pid);
        }
        // 反馈①：下载进度条已移至详情页下载区下方（见 ui_mod_detail_body），此处不再渲染
    }

    /// 目标位置明细：选服务器自动归类 或 自选目录
    fn ui_dl_mod_target(&mut self, ui: &mut egui::Ui) {
        let use_server = self.dl.as_ref().unwrap().mod_target_use_server;
        if use_server {
            let servers = self.cfg.servers.clone();
            let cur = self.dl.as_ref().unwrap().mod_target_dir.clone();
            let names: Vec<String> = servers
                .iter()
                .map(|s| {
                    s.dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                })
                .collect();
            ui.horizontal(|ui| {
                ui.label("安装到");
                egui::ComboBox::from_id_salt("dl_target_server")
                    .selected_text(
                        names
                            .iter()
                            .find(|n| {
                                cur.ends_with(&format!("\\{}", n))
                                    || cur.ends_with(&format!("/{}", n))
                            })
                            .cloned()
                            .unwrap_or_else(|| {
                                if cur.is_empty() {
                                    "（未选择服务器）".to_string()
                                } else {
                                    "（自定义目录）".to_string()
                                }
                            }),
                    )
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            if ui.selectable_label(false, n).clicked() {
                                self.dl.as_mut().unwrap().mod_target_dir =
                                    servers[i].dir.display().to_string();
                                // 功能优化：选定服务器即**自动套用它识别出的加载器与 MC 版本**
                                self.apply_platform_from_dir(&servers[i].dir);
                            }
                        }
                    });
                ui.label(
                    RichText::new("将按类型自动安装到 <服务器>/mods、plugins 或 world/datapacks")
                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
            });
            // 识别结果显示 + 一键重新识别
            let cur_dir = self
                .dl
                .as_ref()
                .map(|d| d.mod_target_dir.clone())
                .unwrap_or_default();
            if !cur_dir.trim().is_empty() {
                let info = serverinfo::detect(std::path::Path::new(cur_dir.trim()));
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("识别到目标：{}", info.summary()))
                            .color(self.fg(Color32::from_rgb(120, 200, 255))),
                    );
                    if ui
                        .small_button("重新识别并套用")
                        .on_hover_text(info.evidence.join("\n"))
                        .clicked()
                    {
                        self.apply_platform_from_dir(std::path::Path::new(cur_dir.trim()));
                    }
                    if !info.kind.is_modded() {
                        ui.label(
                            RichText::new("该目标不是模组加载器（原版/纯插件端），模组无法加载")
                                .color(Color32::from_rgb(240, 176, 96)),
                        );
                    }
                });
            }
        } else {
            ui.horizontal(|ui| {
                ui.label("安装到");
                ui.add(
                    egui::TextEdit::singleline(&mut self.dl.as_mut().unwrap().mod_target_dir)
                        .desired_width(280.0),
                );
                if ui.button("浏览…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.dl.as_mut().unwrap().mod_target_dir = p.display().to_string();
                    }
                }
                ui.label(
                    RichText::new("jar 将安装到 <目录>/mods、plugins 或 world/datapacks（按类型）")
                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
            });
        }
    }

    /// 依据目标目录识别服务端平台，并把结果**套用到下载页**（加载器 + MC 版本）。
    ///
    /// 只在识别到模组加载器时改加载器；MC 版本识别到就填（原版也能识别版本，
    /// 便于用户至少把版本选对）。返回识别结果供调用方展示。
    fn apply_platform_from_dir(&mut self, dir: &Path) -> serverinfo::PlatformInfo {
        let info = serverinfo::detect(dir);
        if let Some(dl) = self.dl.as_mut() {
            if let Some(loader) = info.kind.modrinth_loader() {
                dl.mod_loader = loader.to_string();
            }
            if let Some(v) = &info.mc_version {
                dl.mod_mc_version = v.clone();
            }
        }
        info
    }

    /// 每个可见帧调用：崩溃分析小窗（服务端异常退出后自动弹出）。
    fn ui_crash_report(&mut self, ctx: &egui::Context) {
        let Some((idx, name, finding)) = self.crash_report.clone() else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new("⚠ 崩溃原因分析")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("{name} 异常退出"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(format!("分析来源：{}", finding.source))
                        .weak()
                        .small(),
                );
                if !finding.summary.is_empty() {
                    ui.label(RichText::new(&finding.summary).small());
                }
                ui.separator();
                if finding.causes.is_empty() {
                    ui.label("日志里没有识别出明确的崩溃原因（可能是被强杀/断电，或崩溃信息不完整）。");
                    ui.label(RichText::new("可以到「日志」页查看完整输出，或把 crash-reports 里的文件发我。").weak().small());
                } else {
                    for c in &finding.causes {
                        egui::Frame::none()
                            .fill(self.theme_cur.widget_bg)
                            .stroke(egui::Stroke::new(1.0, self.theme_cur.stroke))
                            .rounding(6.0)
                            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&c.title).strong());
                                if !c.suspects.is_empty() {
                                    ui.label(
                                        RichText::new(format!(
                                            "涉及：{}",
                                            c.suspects.join("、")
                                        ))
                                        .color(Color32::from_rgb(240, 176, 96)),
                                    );
                                }
                                ui.label(RichText::new(format!("建议：{}", c.advice)).small());
                                // 一键跳转到需要修改的文件/目录（文件在资源管理器中选中，目录直接打开）
                                if let Some(rel) = &c.path {
                                    let full = self
                                        .cfg
                                        .servers
                                        .get(idx)
                                        .map(|s| s.dir.join(rel))
                                        .unwrap_or_default();
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            RichText::new(format!("📄 {rel}"))
                                                .small()
                                                .color(self.fg(Color32::from_rgb(120, 200, 255))),
                                        );
                                        if c.path_broken {
                                            ui.label(
                                                RichText::new("（内容不是合法 JSON，极可能就是它）")
                                                    .small()
                                                    .color(Color32::from_rgb(255, 120, 120)),
                                            );
                                        } else if !full.exists() {
                                            ui.label(
                                                RichText::new("（文件不存在，可能是同级目录/文件名不同）")
                                                    .small()
                                                    .weak(),
                                            );
                                        }
                                        if ui.button("跳转到该位置").clicked() {
                                            if full.exists() {
                                                reveal_in_explorer(&full);
                                            } else {
                                                // 文件不存在时打开它的上一级目录，避免"跳转无效"
                                                let parent = full
                                                    .parent()
                                                    .map(|p| p.to_path_buf())
                                                    .unwrap_or_default();
                                                self.open_folder(&parent);
                                            }
                                        }
                                        if ui.button("复制路径").clicked() {
                                            ctx.copy_text(full.display().to_string());
                                            self.set_toast("路径已复制".to_string());
                                        }
                                    });
                                }
                                if !c.evidence.is_empty() {
                                    ui.collapsing("原始日志行", |ui| {
                                        for e in &c.evidence {
                                            ui.label(RichText::new(e).monospace().small());
                                        }
                                    });
                                }
                            });
                        ui.add_space(4.0);
                    }
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("📂 打开 config 目录").clicked() {
                        if let Some(s) = self.cfg.servers.get(idx) {
                            self.open_folder(&s.dir.join("config"));
                        }
                    }
                    // 模组服务端设置也常放在 world/ 下（如 Carpet 系），或直接写在根目录
                    if ui.button("📂 打开 world 目录").clicked() {
                        if let Some(s) = self.cfg.servers.get(idx) {
                            self.open_folder(&s.dir.join("world"));
                        }
                    }
                    if ui.button("📂 打开服务器根目录").clicked() {
                        if let Some(s) = self.cfg.servers.get(idx) {
                            self.open_folder(&s.dir);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("📂 打开日志目录").clicked() {
                        if let Some(s) = self.cfg.servers.get(idx) {
                            self.open_folder(&s.dir.join("logs"));
                        }
                    }
                    if ui.button("📂 打开崩溃报告目录").clicked() {
                        if let Some(s) = self.cfg.servers.get(idx) {
                            self.open_folder(&s.dir.join("crash-reports"));
                        }
                    }
                    if ui.button("重新分析").clicked() {
                        if let Some(s) = self.cfg.servers.get(idx) {
                            if let Some(f) = crashscan::analyze(&s.dir) {
                                self.crash_report = Some((idx, name.clone(), f));
                            } else {
                                self.set_toast("未在日志中发现可识别的崩溃原因".to_string());
                            }
                        }
                    }
                    if ui.button("关闭").clicked() {
                        close = true;
                    }
                });
            });
        if close || !open {
            self.crash_report = None;
        }
    }

    /// 工具自身日志页（📜 日志）：日志库浏览 + 过滤 + 相关设置。
    ///
    /// 这是 **XMST 工具自己的运行日志**（服务器输出也汇总进同一个库，以来源标记区分），
    /// 因此不按服务器拆页签，筛选靠「来源」下拉完成。原「设置 → 日志」整段已并入本页。
    fn ui_logs_page(&mut self, ctx: &egui::Context) {
        let mut query = self.log_query.clone();
        let mut level = self.log_level.clone();
        let mut src_filter = self.log_src.clone();
        let mut follow = self.log_follow;
        let mut row_h = self.log_row_h;
        let mut do_clear = false;
        let mut do_export = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.panel_fill))
            .show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                let pad = 14.0f32;
                let w = (avail.width() - pad).min(1100.0);
                let mut inner = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            egui::pos2(avail.left() + pad, avail.top()),
                            egui::vec2(w, avail.height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt("logs_page"),
                );
                let ui = &mut inner;
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("日志");
                    ui.label(
                        RichText::new("XMST 工具自身的运行日志（含服务器输出汇总，按来源标记）")
                            .weak()
                            .small(),
                    );
                });
                ui.separator();
                // ---- 统计卡 ----
                let (count, rows) = match &self.logdb {
                    Some(db) => (db.count(), db.recent(800, None)),
                    None => (0, Vec::new()),
                };
                let sources: Vec<String> = {
                    let mut v: Vec<String> = rows.iter().map(|r| r.src.clone()).collect();
                    v.sort();
                    v.dedup();
                    v
                };
                ui.horizontal_wrapped(|ui| {
                    for (t, v) in [
                        ("日志总条数", format!("{count}")),
                        ("库上限（轮转）", "50000".to_string()),
                        (
                            "最近写入",
                            rows.first().map(|r| r.ts.clone()).unwrap_or_else(|| "暂无".into()),
                        ),
                    ] {
                        egui::Frame::none()
                            .fill(self.theme_cur.widget_bg)
                            .stroke(egui::Stroke::new(1.0, self.theme_cur.stroke))
                            .rounding(8.0)
                            .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(t).weak().small());
                                    ui.label(RichText::new(v).strong());
                                });
                            });
                    }
                });
                ui.add_space(6.0);
                // ---- 过滤栏 ----
                ui.horizontal_wrapped(|ui| {
                    ui.label("🔍");
                    ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text("搜索关键字（消息或来源）")
                            .desired_width(240.0),
                    );
                    ui.label("级别");
                    egui::ComboBox::from_id_salt("log_level")
                        .selected_text(level.clone())
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for l in ["全部", "信息", "警告", "错误"] {
                                ui.selectable_value(&mut level, l.to_string(), l);
                            }
                        });
                    ui.label("来源");
                    egui::ComboBox::from_id_salt("log_src")
                        .selected_text(src_filter.clone())
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut src_filter, "全部".to_string(), "全部");
                            for s in &sources {
                                ui.selectable_value(&mut src_filter, s.clone(), s);
                            }
                        });
                    ui.checkbox(&mut follow, "跟随最新");
                    ui.label("行高");
                    egui::ComboBox::from_id_salt("log_rowh")
                        .selected_text(if row_h < 1.5 { "紧凑" } else { "舒适" })
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut row_h, 1.0, "紧凑");
                            ui.selectable_value(&mut row_h, 1.6, "舒适");
                        });
                    if ui.button("🗑 清空日志库").clicked() {
                        do_clear = true;
                    }
                    if ui.button("💾 导出…").clicked() {
                        do_export = true;
                    }
                });
                ui.separator();
                // ---- 主日志区（等宽、级别着色、点击复制）----
                let q = query.trim().to_lowercase();
                let lv = level.clone();
                let mut shown = 0usize;
                egui::ScrollArea::vertical()
                    .id_salt("logs_scroll")
                    .auto_shrink([false, false])
                    .stick_to_bottom(follow)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if rows.is_empty() {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(
                                    "暂无日志。启动服务器 / 隧道后，输出会自动汇总到这里。",
                                )
                                .weak(),
                            );
                        }
                        for r in rows.iter() {
                            if src_filter != "全部" && r.src != src_filter {
                                continue;
                            }
                            let lvl_ok = match lv.as_str() {
                                "警告" => r.line.contains("WARN") || r.line.contains("警告"),
                                "错误" => {
                                    r.line.contains("ERROR")
                                        || r.line.contains("FATAL")
                                        || r.line.contains("Exception")
                                        || r.line.contains("失败")
                                        || r.line.contains("异常")
                                }
                                _ => true,
                            };
                            if !lvl_ok {
                                continue;
                            }
                            if !q.is_empty()
                                && !r.line.to_lowercase().contains(&q)
                                && !r.src.to_lowercase().contains(&q)
                            {
                                continue;
                            }
                            if shown >= 500 {
                                break;
                            }
                            shown += 1;
                            let lvl_col = if r.line.contains("ERROR")
                                || r.line.contains("FATAL")
                                || r.line.contains("Exception")
                            {
                                Color32::from_rgb(235, 110, 110)
                            } else if r.line.contains("WARN") {
                                Color32::from_rgb(235, 190, 90)
                            } else {
                                self.theme_cur.text
                            };
                            let row_resp = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(
                                    RichText::new(&r.ts)
                                        .monospace()
                                        .small()
                                        .color(self.theme_cur.weak),
                                );
                                ui.label(
                                    RichText::new(format!("[{}]", r.src))
                                        .monospace()
                                        .small()
                                        .color(self.fg(Color32::from_rgb(120, 180, 255))),
                                );
                                ui.label(
                                    RichText::new(&r.line)
                                        .monospace()
                                        .color(lvl_col),
                                );
                            });
                            let rr = row_resp.response.interact(egui::Sense::click());
                            if rr.clicked() {
                                ui.ctx().copy_text(r.line.clone());
                                self.set_toast("已复制该行日志".to_string());
                            }
                        }
                        if shown == 0 && !rows.is_empty() {
                            ui.add_space(8.0);
                            ui.label(RichText::new("当前过滤条件下没有日志").weak());
                        } else if shown >= 500 {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("仅显示最近 500 行（不影响库中已存数据）")
                                    .weak()
                                    .small(),
                            );
                        }
                    });
                ui.separator();
                // ---- 相关设置（原「设置 → 日志」）----
                ui.horizontal(|ui| {
                    ui.label("最大显示行数");
                    if ui
                        .add(egui::DragValue::new(&mut self.cfg.max_log_lines).range(100..=20000))
                        .changed()
                    {
                        self.save_config();
                    }
                    ui.label(
                        RichText::new("行（仅影响显示条数；日志库按 50000 行轮转）")
                            .weak()
                            .small(),
                    );
                });
            });
        self.log_query = query;
        self.log_level = level;
        self.log_src = src_filter;
        self.log_follow = follow;
        self.log_row_h = row_h;
        if do_clear {
            if let Some(db) = self.logdb.as_mut() {
                db.clear();
            }
            self.set_toast("已清空日志库".to_string());
        }
        if do_export {
            self.export_logs();
        }
    }

    /// 导出日志库为文本文件（写到 data\logs\export_<时间>.log）。
    fn export_logs(&mut self) {
        let Some(db) = self.logdb.as_ref() else {
            self.set_toast("日志库不可用".to_string());
            return;
        };
        let rows = db.recent(50000, None);
        let dir = self.data_dir().join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!(
            "export_{}.log",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ));
        let mut out = String::with_capacity(rows.len() * 80);
        for r in &rows {
            out.push_str(&format!("[{}] [{}] {}\n", r.ts, r.src, r.line));
        }
        match std::fs::write(&path, out) {
            Ok(_) => {
                self.set_toast(format!("已导出 {} 条到 {}", rows.len(), path.display()));
                self.open_folder(&dir);
            }
            Err(e) => self.set_toast(format!("导出失败：{e}")),
        }
    }

    /// 安装模组：取最新版下载地址 -> 归类到目标目录 -> 启动下载
    fn mod_install_project(&mut self, pid: &str) {
        let dir = self.dl.as_ref().unwrap().mod_target_dir.trim().to_string();
        if dir.is_empty() {
            self.set_toast("请先选择目标服务器或填写目标目录".to_string());
            return;
        }
        let (mc2, l2, pt2) = {
            let d = self.dl.as_ref().unwrap();
            (
                d.mod_mc_version.trim().to_string(),
                d.mod_loader.clone(),
                d.mod_project_type.clone(),
            )
        };
        match modrinth::latest_file(pid, &mc2, &l2) {
            Ok((url, fname, _size)) => {
                let sub = match pt2.as_str() {
                    "plugin" => "plugins",
                    // 反馈②：数据包下载到 <服务器>/world/datapacks（Minecraft 标准数据包目录）
                    "datapack" => "world/datapacks",
                    _ => "mods",
                };
                let target = std::path::Path::new(&dir).join(sub);
                if let Err(e) = std::fs::create_dir_all(&target) {
                    self.set_toast(format!("创建 {sub} 目录失败: {e}"));
                } else {
                    let d = self.dl.as_mut().unwrap();
                    d.mod_download_target = url;
                    d.mod_download_name = target.join(fname).display().to_string();
                    self.dl_start_mod_download();
                }
            }
            Err(e) => {
                self.set_toast(format!("获取下载地址失败: {e}"));
            }
        }
    }

    /// 收藏 / 取消收藏（图2 收藏夹）
    fn mod_fav_toggle(&mut self, pid: &str) {
        let favs = &mut self.cfg.mod_favorites;
        if favs.iter().any(|x| x == pid) {
            favs.retain(|x| x != pid);
            self.set_toast("已取消收藏".to_string());
        } else {
            favs.push(pid.to_string());
            self.set_toast("已加入收藏".to_string());
        }
        self.save_config();
    }

    /// 是否已收藏
    fn is_mod_faved(&self, pid: &str) -> bool {
        self.cfg.mod_favorites.iter().any(|x| x == pid)
    }

    /// 模组卡片（图2：图标/标题/描述/分类/兼容性/下载量/更新时间/来源 + 安装 + 收藏）
    /// 点击卡片主体 -> 返回 Detail（展开版本列表）
    /// 截断文本到指定字符数（保留省略号），避免窄卡片内横向布局被撑爆导致竖排
    fn truncate_str(s: &str, max_chars: usize) -> String {
        let n = s.chars().count();
        if n <= max_chars {
            s.to_string()
        } else {
            let mut out: String = s.chars().take(max_chars).collect();
            out.push('…');
            out
        }
    }

    /// 确保模组图标已触发下载（有 icon_url 且未缓存/未在途时后台拉取；图2 从网站获取）
    fn mod_icon_ensure(&mut self, r: &modrinth::ModrinthHit) {
        let id = r.id.clone();
        if r.icon_url.trim().is_empty() {
            return;
        }
        let dl = self.dl.as_mut().unwrap();
        if dl.mod_icon_tex.contains_key(&id)
            || dl.mod_icon_pending.contains(&id)
            || dl.mod_icon_failed.contains(&id)
        {
            return;
        }
        let url = r.icon_url.clone();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None::<Result<Vec<u8>, String>>));
        let shared2 = std::sync::Arc::clone(&shared);
        std::thread::spawn(move || {
            let r = (|| -> Result<Vec<u8>, String> {
                let client = download::new_client();
                let resp = client.get(&url).send().map_err(|e| e.to_string())?;
                let bytes = resp.bytes().map_err(|e| e.to_string())?;
                Ok(bytes.to_vec())
            })();
            *shared2.lock().unwrap() = Some(r);
        });
        dl.mod_icon_shared.insert(id.clone(), shared);
        dl.mod_icon_pending.insert(id);
    }

    /// 绘制模组图标：有缓存纹理显示网站图片，否则首字母色块占位
    fn mod_icon_ui(&mut self, ui: &mut egui::Ui, r: &modrinth::ModrinthHit, size: f32) {
        self.mod_icon_ensure(r);
        if let Some(tex) = self.dl.as_ref().unwrap().mod_icon_tex.get(&r.id).cloned() {
            ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(size, size)));
            return;
        }
        let letter = r
            .title
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "#".to_string());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let color = mod_icon_color(&r.id);
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(8.0), color);
        let mut font_size = size * 0.47;
        let galley = loop {
            let g = ui.fonts(|f| {
                f.layout_no_wrap(
                    letter.clone(),
                    egui::FontId::proportional(font_size),
                    Color32::WHITE,
                )
            });
            if g.size().x <= rect.width() - 4.0 || font_size <= 9.0 {
                break g;
            }
            font_size -= 1.0;
        };
        ui.painter()
            .galley(rect.center() - galley.size() * 0.5, galley, Color32::WHITE);
    }

    /// title 是否已含中文字符（含 CJK 即视为中文名，不再翻译）
    fn is_cjk_title(s: &str) -> bool {
        s.chars().any(|c| {
            let u = c as u32;
            (0x2E80..0xA000).contains(&u)
                || (0xF900..0xFB00).contains(&u)
                || (0xFE30..0xFE50).contains(&u)
        })
    }

    /// 确保模组标题中文名已触发获取（英文标题后台查 MC 百科，中文标题直接视为已译）。
    /// 优先级：MC 百科（正确中文名）→ 仅百科未收录时才由"译"按钮手动走在线翻译。
    fn mod_cn_ensure(&mut self, r: &modrinth::ModrinthHit) {
        if Self::is_cjk_title(&r.title) {
            return;
        }
        let dl = self.dl.as_mut().unwrap();
        if dl.mod_cn_name.contains_key(&r.title)
            || dl.mod_mcmod_url.contains_key(&r.title)
            || dl.mod_cn_pending.contains(&r.title)
            || dl.mod_mcmod_pending.contains(&r.title)
            || dl.mod_cn_failed.contains(&r.title)
        {
            return;
        }
        // 1) 内置 MC 百科离线库优先：按 slug 精确查表（与 PCL-CE 同源，完全离线、结果稳定）
        if let Some(e) = dl
            .mcmod_db
            .lookup_modrinth(&r.slug)
            .or_else(|| dl.mcmod_db.lookup_curseforge(&r.slug))
        {
            dl.mod_mcmod_url.insert(
                r.title.clone(),
                format!("https://www.mcmod.cn/class/{}.html", e.wiki_id),
            );
            // 百科有中文译名则直接落缓存；无译名（如 Fabric API）保持英文标题，仍提供百科入口
            if !e.cn_name.is_empty() {
                dl.mod_cn_name.insert(r.title.clone(), e.cn_name.clone());
            }
            return;
        }
        // 2) 离线库未命中 -> 后台在线查 MC 百科搜索页兜底（原逻辑）
        let t = r.title.clone();
        let shared =
            std::sync::Arc::new(std::sync::Mutex::new(None::<Result<(String, String), String>>));
        let shared2 = std::sync::Arc::clone(&shared);
        let t2 = t.clone();
        std::thread::spawn(move || {
            let r = modrinth::fetch_mcmod_name(&t2);
            *shared2.lock().unwrap() = Some(r);
        });
        dl.mod_mcmod_shared.insert(t.clone(), shared);
        dl.mod_mcmod_pending.insert(t);
    }

    /// 手动在线翻译（"译"按钮触发）：仅 MC 百科未收录时调用，走 Google + MyMemory 兜底
    fn mod_cn_translate_now(&mut self, title: &str) {
        let dl = self.dl.as_mut().unwrap();
        if dl.mod_cn_name.contains_key(title) || dl.mod_cn_pending.contains(title) {
            return;
        }
        let t = title.to_string();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None::<Result<String, String>>));
        let shared2 = std::sync::Arc::clone(&shared);
        let t2 = t.clone();
        std::thread::spawn(move || {
            let r = modrinth::translate_best(&t2);
            *shared2.lock().unwrap() = Some(r);
        });
        dl.mod_cn_shared.insert(t.clone(), shared);
        dl.mod_cn_pending.insert(t);
    }

    /// 标题显示信息：返回 (显示名, 是否已有中文名, 是否翻译中, 是否自动翻译失败)
    fn mod_cn_info(&self, title: &str) -> (String, bool, bool, bool) {
        let dl = self.dl.as_ref().unwrap();
        if let Some(cn) = dl.mod_cn_name.get(title) {
            (cn.clone(), true, false, false)
        } else if dl.mod_cn_pending.contains(title) || dl.mod_mcmod_pending.contains(title) {
            (title.to_string(), false, true, false)
        } else if dl.mod_cn_failed.contains(title) {
            (title.to_string(), false, false, true)
        } else {
            (title.to_string(), false, false, false)
        }
    }

    fn ui_mod_card(&mut self, ui: &mut egui::Ui, r: &modrinth::ModrinthHit) -> ModCardAction {
        let mut act = ModCardAction::None;
        let frame = egui::Frame::none()
            .fill(ui.visuals().extreme_bg_color)
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::same(10.0));
        let mut fav_rect = egui::Rect::NOTHING;
        // 非跳转区矩形：标题/译文 label 与"译"重试按钮（点击不进入详情）
        let mut title_rect = egui::Rect::NOTHING;
        let mut retry_rect = egui::Rect::NOTHING;
        let inner = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            // 第一行：网站图标（或色块占位）+ 标题/收藏/作者 + 下载量
            ui.horizontal(|ui| {
                self.mod_icon_ui(ui, r, 34.0);
                ui.vertical(|ui| {
                    // 标题/作者均截断，防止窄卡内右对齐按钮挤压导致竖排
                    ui.horizontal(|ui| {
                        // 中文名：自动后台翻译，失败则显示"译"按钮手动重试
                        self.mod_cn_ensure(r);
                        let (disp, has_cn, pending, failed) = self.mod_cn_info(&r.title);
                        let disp_show = if has_cn { disp.as_str() } else { &r.title };
                        let title_resp = ui.label(
                            RichText::new(Self::truncate_str(disp_show, 20))
                                .strong()
                                .size(14.0),
                        );
                        title_rect = title_resp.rect;
                        // 收藏挪到前面（紧跟标题）
                        let fav_resp =
                            ui.button(if self.is_mod_faved(&r.id) { "★" } else { "☆" });
                        fav_rect = fav_resp.rect;
                        if fav_resp.clicked() {
                            act = ModCardAction::FavToggle;
                        }
                        // 无中文名时：翻译中显示"…"，自动翻译失败显示"译"按钮
                        if pending {
                            ui.label(RichText::new("…").color(self.fg(Color32::from_rgb(140, 140, 140))));
                        } else if failed {
                            let retry_resp = ui.button("译");
                            retry_rect = retry_resp.rect;
                            if retry_resp.clicked() {
                                self.dl.as_mut().unwrap().mod_cn_failed.remove(&r.title);
                                self.mod_cn_translate_now(&r.title);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{} 下载", fmt_count(r.downloads)))
                                    .size(11.0)
                                    .color(self.fg(Color32::from_rgb(150, 150, 150))),
                            );
                        });
                    });
                    if !r.author.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "by {}",
                                Self::truncate_str(&r.author, 30)
                            ))
                            .size(11.0)
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                        );
                    }
                });
            });
            ui.add_space(4.0);
            // 描述（截断）
            let desc = r.description.replace('\n', " ");
            let n = desc.chars().count();
            let desc_show = if n > 90 {
                let mut s: String = desc.chars().take(90).collect();
                s.push('…');
                s
            } else {
                desc
            };
            ui.label(
                RichText::new(desc_show)
                    .size(12.0)
                    .color(self.fg(Color32::from_rgb(170, 170, 170))),
            );
            ui.add_space(4.0);
            // 分类标签 + 版本兼容
            ui.horizontal_wrapped(|ui| {
                for c in r.categories.iter().take(3) {
                    ui.label(
                        RichText::new(format!("#{c}"))
                            .size(11.0)
                            .color(self.fg(Color32::from_rgb(120, 180, 255))),
                    );
                }
                let ver = r
                    .mc_versions
                    .iter()
                    .rev()
                    .find(|v| v.contains('.'))
                    .map(|s| s.as_str())
                    .unwrap_or("");
                if !ver.is_empty() {
                    ui.label(
                        RichText::new(ver.to_string())
                            .size(11.0)
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                    );
                }
            });
            ui.add_space(4.0);
            // 底部：更新时间 + 来源（安装已移入详情页下载）
            ui.horizontal(|ui| {
                let d = &r.date_modified;
                let dstr = if d.len() >= 10 { &d[..10] } else { d };
                ui.label(
                    RichText::new(format!("更新 {dstr} · Modrinth"))
                        .size(11.0)
                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
            });
        });
        // 卡片主体点击 -> 展开详情；收藏按钮=收藏；标题/译文与"译"按钮=不进入详情（修复点击名称穿透进详情）
        let card_resp = inner.response.interact(egui::Sense::click());
        if card_resp.clicked() {
            let pos = card_resp.interact_pointer_pos();
            if pos.map_or(false, |p| fav_rect.contains(p)) {
                act = ModCardAction::FavToggle;
            } else if pos.map_or(false, |p| title_rect.contains(p) || retry_rect.contains(p)) {
                act = ModCardAction::None;
            } else {
                act = ModCardAction::Detail;
            }
        }
        act
    }

    /// 行式列表条目（搜索结果竖向单列列表用）：
    /// 左侧色块图标，中间标题/作者/描述/标签，右侧收藏 + 安装按钮。
    /// 整行点击展开详情；按钮优先消费点击。
    fn ui_mod_row(&mut self, ui: &mut egui::Ui, r: &modrinth::ModrinthHit) -> ModCardAction {
        let mut act = ModCardAction::None;
        let frame = egui::Frame::none()
            .fill(ui.visuals().extreme_bg_color)
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::same(8.0));
        let mut fav_rect = egui::Rect::NOTHING;
        // 非跳转区矩形：标题/译文 label 与"译"重试按钮（点击不进入详情）
        let mut title_rect = egui::Rect::NOTHING;
        let mut retry_rect = egui::Rect::NOTHING;
        let inner = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // 网站图标（或色块占位）
                self.mod_icon_ui(ui, r, 34.0);
                // 中间信息区：标题 + 收藏 + 作者 + 更新/下载量，第二行描述，第三行标签/版本
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        // 中文名：自动后台翻译，失败则显示"译"按钮手动重试
                        self.mod_cn_ensure(r);
                        let (disp, has_cn, pending, failed) = self.mod_cn_info(&r.title);
                        let disp_show = if has_cn { disp.as_str() } else { &r.title };
                        let title_resp = ui.label(
                            RichText::new(Self::truncate_str(disp_show, 24))
                                .strong()
                                .size(14.0),
                        );
                        title_rect = title_resp.rect;
                        // 收藏挪到前面（紧跟标题）
                        let fav_resp =
                            ui.button(if self.is_mod_faved(&r.id) { "★" } else { "☆" });
                        fav_rect = fav_resp.rect;
                        if fav_resp.clicked() {
                            act = ModCardAction::FavToggle;
                        }
                        // 无中文名时：翻译中显示"…"，自动翻译失败显示"译"按钮
                        if pending {
                            ui.label(RichText::new("…").color(self.fg(Color32::from_rgb(140, 140, 140))));
                        } else if failed {
                            let retry_resp = ui.button("译");
                            retry_rect = retry_resp.rect;
                            if retry_resp.clicked() {
                                self.dl.as_mut().unwrap().mod_cn_failed.remove(&r.title);
                                self.mod_cn_translate_now(&r.title);
                            }
                        }
                        if !r.author.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "by {}",
                                    Self::truncate_str(&r.author, 20)
                                ))
                                .size(11.0)
                                .color(self.fg(Color32::from_rgb(150, 150, 150))),
                            );
                        }
                        let d = &r.date_modified;
                        let dstr = if d.len() >= 10 { &d[..10] } else { d };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "更新 {dstr} · {} 下载",
                                    fmt_count(r.downloads)
                                ))
                                .size(11.0)
                                .color(self.fg(Color32::from_rgb(150, 150, 150))),
                            );
                        });
                    });
                    let desc = r.description.replace('\n', " ");
                    let n = desc.chars().count();
                    let desc_show = if n > 60 {
                        let mut s: String = desc.chars().take(60).collect();
                        s.push('…');
                        s
                    } else {
                        desc
                    };
                    if !desc_show.is_empty() {
                        ui.label(
                            RichText::new(desc_show)
                                .size(12.0)
                                .color(self.fg(Color32::from_rgb(170, 170, 170))),
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        for c in r.categories.iter().take(3) {
                            ui.label(
                                RichText::new(format!("#{c}"))
                                    .size(11.0)
                                    .color(self.fg(Color32::from_rgb(120, 180, 255))),
                            );
                        }
                        let ver = r
                            .mc_versions
                            .iter()
                            .rev()
                            .find(|v| v.contains('.'))
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        if !ver.is_empty() {
                            ui.label(
                                RichText::new(ver.to_string())
                                    .size(11.0)
                                    .color(self.fg(Color32::from_rgb(150, 150, 150))),
                            );
                        }
                    });
                });
                // 收藏已移至标题右侧；安装已移入详情页下载，右侧区删除
            });
        });
        // 整行点击 -> 展开详情；收藏按钮=收藏；标题/译文与"译"按钮=不进入详情（修复点击名称穿透进详情）
        let row_resp = inner.response.interact(egui::Sense::click());
        if row_resp.clicked() {
            let pos = row_resp.interact_pointer_pos();
            if pos.map_or(false, |p| fav_rect.contains(p)) {
                act = ModCardAction::FavToggle;
            } else if pos.map_or(false, |p| title_rect.contains(p) || retry_rect.contains(p)) {
                act = ModCardAction::None;
            } else {
                act = ModCardAction::Detail;
            }
        }
        act
    }

    /// 详情展开面板（点击卡片后显示）：项目信息 + 版本列表
    /// 版本列表按 MC 版本分组（PCL 式），加载器筛选；点文件项直接安装该版本。
    /// 详情固定头（常驻顶部，可随时收起）：标题/作者/下载量 + 打开页面 + 简介 + 翻译 + 加载器筛选
    fn ui_mod_detail_header(&mut self, ui: &mut egui::Ui) {
        let Some(hit) = self.dl.as_ref().unwrap().mod_detail_hit.clone() else {
            return;
        };
        // 详情头：图标 + 标题/作者/下载量 + 简介（图标样式与列表一致）
        self.mod_cn_ensure(&hit);
        let (disp, has_cn, _, _) = self.mod_cn_info(&hit.title);
        let title_show = if has_cn { disp.as_str() } else { &hit.title };
        ui.horizontal(|ui| {
            self.mod_icon_ui(ui, &hit, 40.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title_show).strong().size(16.0));
                    if !hit.author.is_empty() {
                        ui.label(
                            RichText::new(format!("by {}", hit.author))
                                .size(12.0)
                                .color(self.fg(Color32::from_rgb(150, 150, 150))),
                        );
                    }
                    ui.label(
                        RichText::new(format!("{} 下载", fmt_count(hit.downloads)))
                            .size(12.0)
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                    );
                });
                // 简介（完整显示）
                let desc = hit.description.trim().to_string();
                if !desc.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&desc)
                                .size(12.0)
                                .color(self.fg(Color32::from_rgb(170, 170, 170))),
                        )
                        .wrap(),
                    );
                }
            });
        });
        // 译文（简介下方、按钮上方；绿色无“译文:”前缀；隐藏后不渲染但保留缓存）
        if let Some(t) = &self.dl.as_ref().unwrap().mod_translate_result {
            if !self.dl.as_ref().unwrap().mod_translate_hidden {
                ui.add(
                    egui::Label::new(
                        RichText::new(t.as_str())
                            .size(12.0)
                            .color(self.fg(Color32::from_rgb(140, 190, 140))),
                    )
                    .wrap(),
                );
            }
        }
        // 操作行：收起 / 打开仓库 / 翻译简介（收起、打开仓库与翻译同排）
        ui.horizontal(|ui| {
            if ui.button("收起").clicked() {
                let d = self.dl.as_mut().unwrap();
                d.mod_detail_id = None;
                d.mod_detail_hit = None;
                d.mod_versions.clear();
                d.mod_ver_loader = "全部".to_string();
                d.mod_translate_result = None;
                d.mod_translate_hidden = false;
                d.mod_translate_busy = false;
                d.mod_translate_error = None;
                d.mod_translate_shared = None;
                d.mod_ver_log_open = None;
            }
            // 打开项目页面（系统浏览器）：优先官网 source_url（GitHub 等），无则异步拉取详情后回传
            if ui.button("打开仓库").clicked() {
                self.dl_mod_open_page(&hit);
            }
            // 跳转 MC 百科：仅百科已命中中文名时显示（百科页含完整资料与正确译名）
            let mcmod_url = self.dl.as_ref().unwrap().mod_mcmod_url.get(&hit.title).cloned();
            if let Some(u) = mcmod_url {
                if ui.button("MC百科").clicked() {
                    let _ = std::process::Command::new("cmd")
                        .args(["/C", "start", "", &u])
                        .spawn();
                }
            }
            if self.dl.as_ref().unwrap().mod_translate_busy {
                ui.spinner();
                ui.label(
                    RichText::new("翻译中…").color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
            } else if self.dl.as_ref().unwrap().mod_translate_result.is_some() {
                // 已有缓存（含隐藏后）：仅切换显示/隐藏，直接复用缓存不再调接口
                let hidden = self.dl.as_ref().unwrap().mod_translate_hidden;
                if ui
                    .button(if hidden { "翻译简介" } else { "隐藏翻译" })
                    .clicked()
                {
                    let d = self.dl.as_mut().unwrap();
                    d.mod_translate_hidden = !d.mod_translate_hidden;
                    d.mod_translate_error = None;
                }
            } else if ui.button("翻译简介").clicked() {
                let text = hit.description.trim().to_string();
                if !text.is_empty() {
                    let shared = std::sync::Arc::new(std::sync::Mutex::new(
                        None::<Result<String, String>>,
                    ));
                    let shared2 = std::sync::Arc::clone(&shared);
                    std::thread::spawn(move || {
                        let r = modrinth::translate_best(&text);
                        *shared2.lock().unwrap() = Some(r);
                    });
                    let d = self.dl.as_mut().unwrap();
                    d.mod_translate_shared = Some(shared);
                    d.mod_translate_busy = true;
                    d.mod_translate_error = None;
                    d.mod_translate_hidden = false;
                }
            }
        });
        if let Some(e) = &self.dl.as_ref().unwrap().mod_translate_error {
            ui.colored_label(
                self.fg(Color32::from_rgb(220, 120, 80)),
                format!("翻译失败: {e}"),
            );
        }
        ui.add_space(4.0);
        // 加载器筛选行（仅显示该模组版本实际存在的加载器；无对应版本的加载器不出现）
        ui.horizontal(|ui| {
            ui.label("加载器");
            let versions = self.dl.as_ref().unwrap().mod_versions.clone();
            let avail_loaders = {
                let mut seen: Vec<String> = Vec::new();
                for v in &versions {
                    for l in &v.loaders {
                        if !l.is_empty() && !seen.iter().any(|s| s == l) {
                            seen.push(l.clone());
                        }
                    }
                }
                let order = ["fabric", "forge", "neoforge", "quilt"];
                let mut ordered: Vec<String> = order
                    .iter()
                    .filter(|l| seen.iter().any(|s| s.as_str() == **l))
                    .map(|s| s.to_string())
                    .collect();
                for s in &seen {
                    if !ordered.iter().any(|o| o == s) {
                        ordered.push(s.clone());
                    }
                }
                ordered
            };
            let loader = self.dl.as_ref().unwrap().mod_ver_loader.clone();
            // 当前选中加载器不在实际集合（版本刷新/切换项目后）时重置为全部
            if loader != "全部" && !avail_loaders.iter().any(|l| l == &loader) {
                self.dl.as_mut().unwrap().mod_ver_loader = "全部".to_string();
            }
            let loader = self.dl.as_ref().unwrap().mod_ver_loader.clone();
            egui::ComboBox::from_id_salt("dl_detail_loader")
                .selected_text(loader.clone())
                .show_ui(ui, |ui| {
                    for l in std::iter::once("全部".to_string()).chain(avail_loaders) {
                        if ui
                            .selectable_label(loader == l, l.clone())
                            .clicked()
                        {
                            self.dl.as_mut().unwrap().mod_ver_loader = l;
                        }
                    }
                });
            if self.dl.as_ref().unwrap().mod_ver_busy {
                ui.spinner();
                ui.label(RichText::new("加载版本…").color(self.fg(Color32::from_rgb(150, 150, 150))));
            }
            if let Some(e) = &self.dl.as_ref().unwrap().mod_ver_error {
                ui.colored_label(self.fg(Color32::from_rgb(220, 80, 80)), format!("版本加载失败: {e}"));
            }
            // 阶段16②：显示快照版本开关（默认关=隐藏 snapshot- 开头版本；点击切换）
            let show_snap = self.dl.as_ref().unwrap().mod_show_snapshot;
            if ui
                .selectable_label(show_snap, if show_snap { "✓ 显示快照版本" } else { "显示快照版本" })
                .on_hover_text("默认隐藏以 snapshot- 开头的快照版本，点击切换显示/隐藏")
                .clicked()
            {
                self.dl.as_mut().unwrap().mod_show_snapshot = !show_snap;
            }
        });
    }

    /// 详情滚动 body：左版本列表（MC 版本组可折叠）/ 右预览（点击版本行显示更新日志）
    fn ui_mod_detail_body(&mut self, ui: &mut egui::Ui) {
        let all_versions = self.dl.as_ref().unwrap().mod_versions.clone();
        // 阶段16②：默认隐藏 snapshot- 开头快照版本（开关开启后显示全部）
        let show_snap = self.dl.as_ref().unwrap().mod_show_snapshot;
        let versions: Vec<_> = if show_snap {
            all_versions.clone()
        } else {
            all_versions
                .iter()
                .filter(|v| {
                    let fname = v
                        .files
                        .first()
                        .map(|f| f.filename.clone())
                        .unwrap_or_default();
                    !v.version_number.starts_with("snapshot-") && !fname.starts_with("snapshot-")
                })
                .cloned()
                .collect()
        };
        if versions.is_empty() {
            // 全量非空但被快照过滤清空：提示用户可开启开关（避免误报“暂无版本信息”）
            if !all_versions.is_empty() {
                ui.label(
                    RichText::new("当前仅有快照版本，点击「显示快照版本」可查看")
                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
                return;
            }
            if !self.dl.as_ref().unwrap().mod_ver_busy
                && self.dl.as_ref().unwrap().mod_ver_error.is_none()
            {
                ui.label(
                    RichText::new("暂无版本信息")
                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
            }
            return;
        }
        let loader = self.dl.as_ref().unwrap().mod_ver_loader.clone();
        let mut groups = group_mod_versions(&versions, &loader);
        // 反馈③：搜索指定 MC 版本时，将该版本组置顶（稳定排序保持其余组原顺序）
        let mc_ref = self.dl.as_ref().unwrap().mod_detail_mc.clone();
        if !mc_ref.is_empty() {
            groups.sort_by_key(|(k, _)| if k.as_str() == mc_ref.as_str() { 0 } else { 1 });
        }
        if groups.is_empty() {
            ui.label(
                RichText::new("该加载器下无可用版本")
                    .color(self.fg(Color32::from_rgb(180, 160, 60))),
            );
            return;
        }
        let total = ui.available_width();
        let left_w = (total - 16.0) * 0.60;
        let mut install: Option<(String, String)> = None;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    // 左：版本列表（MC 版本组，CollapsingHeader 可折叠）
                    for (mc, vs) in &groups {
                    let is_other = mc == "其他";
                    // 反馈③：与搜索 MC 版本匹配的组置顶并橙色高亮
                    let is_match = mc.as_str() == mc_ref.as_str();
                    let head = egui::CollapsingHeader::new(
                        RichText::new(format!("{mc}（{} 个版本）", vs.len()))
                            .strong()
                            .size(13.0)
                            .color(if is_match {
                                self.fg(Color32::from_rgb(255, 170, 60))
                            } else if is_other {
                                self.fg(Color32::from_rgb(200, 170, 60))
                            } else {
                                self.fg(Color32::from_rgb(120, 180, 255))
                            }),
                    )
                    .id_salt(("dl_mc_group", mc))
                    .default_open(true);
                    head.show(ui, |ui| {
                        for v in vs {
                            let fname = v
                                .files
                                .first()
                                .map(|f| f.filename.clone())
                                .unwrap_or_else(|| v.version_number.clone());
                            let fname_show = Self::truncate_str(&fname, 40);
                            let preview_open = self
                                .dl
                                .as_ref()
                                .unwrap()
                                .mod_ver_preview
                                .as_ref()
                                .map(|p| p.id == v.id)
                                .unwrap_or(false);
                            // 反馈③：搜索命中的 MC 版本组内版本行橙色高亮
                            let row_text: egui::WidgetText = if is_match {
                                RichText::new(&fname_show)
                                    .color(self.fg(Color32::from_rgb(255, 170, 60)))
                                    .strong()
                                    .into()
                            } else {
                                fname_show.into()
                            };
                            ui.horizontal(|ui| {
                                let (ch, ch_color) = match v.channel.as_str() {
                                    "release" => ("R", Color32::from_rgb(90, 180, 110)),
                                    "beta" => ("B", Color32::from_rgb(220, 160, 70)),
                                    _ => ("A", Color32::from_rgb(210, 90, 90)),
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(18.0, 18.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .rect_filled(rect, egui::Rounding::same(4.0), ch_color);
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    ch,
                                    egui::FontId::proportional(10.0),
                                    Color32::WHITE,
                                );
                                // 点击文件名 -> 右侧预览
                                if ui.selectable_label(preview_open, row_text.clone()).clicked() {
                                    self.dl.as_mut().unwrap().mod_ver_preview = Some((*v).clone());
                                }
                            });
                        }
                    });
                }
            });
            ui.vertical(|ui| {
                // 右：版本详情预览（点击左侧版本行显示）
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("版本详情").strong().size(14.0));
                    if self.dl.as_ref().unwrap().mod_ver_preview.is_some() {
                        if ui.button("关闭预览").clicked() {
                            self.dl.as_mut().unwrap().mod_ver_preview = None;
                        }
                    }
                });
                ui.separator();
                let Some(pv) = self.dl.as_ref().unwrap().mod_ver_preview.clone() else {
                    ui.label(
                        RichText::new("点击左侧版本查看更新日志")
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                    );
                    return;
                };
                ui.label(RichText::new(&pv.version_number).strong().size(15.0));
                ui.label(
                    RichText::new(format!(
                        "channel: {} · {} 下载",
                        pv.channel,
                        fmt_count(pv.downloads)
                    ))
                    .size(12.0)
                    .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
                let dstr = if pv.date_published.len() >= 10 {
                    &pv.date_published[..10]
                } else {
                    &pv.date_published
                };
                ui.label(
                    RichText::new(format!("更新时间 {dstr}"))
                        .size(12.0)
                        .color(self.fg(Color32::from_rgb(150, 150, 150))),
                );
                if let Some(f) = pv.files.first() {
                    ui.label(
                        RichText::new(format!("文件：{}", f.filename))
                            .size(12.0)
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                    );
                }
                ui.add_space(6.0);
                ui.label(RichText::new("更新日志").strong().size(13.0));
                let cl = pv.changelog.trim();
                if cl.is_empty() {
                    ui.label(
                        RichText::new("该版本无更新日志")
                            .color(self.fg(Color32::from_rgb(150, 150, 150))),
                    );
                } else {
                    ui.add(
                        egui::Label::new(
                            RichText::new(strip_html(cl))
                                .size(12.0)
                                .color(self.fg(Color32::from_rgb(190, 190, 190))),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(8.0);
                // 目标位置（下载前选择：自选目录 / 选服务器自动归类）
                ui.horizontal(|ui| {
                    let use_server = self.dl.as_ref().unwrap().mod_target_use_server;
                    if ui.selectable_label(!use_server, "自选目录").clicked() {
                        self.dl.as_mut().unwrap().mod_target_use_server = false;
                    }
                    if ui.selectable_label(use_server, "选服务器自动归类").clicked() {
                        self.dl.as_mut().unwrap().mod_target_use_server = true;
                    }
                });
                self.ui_dl_mod_target(ui);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("下载").clicked() {
                        if let Some(f) = pv.files.first() {
                            install = Some((f.url.clone(), f.filename.clone()));
                        }
                    }
                    if ui.button("打开页面").clicked() {
                        if let Some(hit2) = self.dl.as_ref().unwrap().mod_detail_hit.clone() {
                            self.dl_mod_open_page(&hit2);
                        }
                    }
                });
                // 反馈①：下载进度条从页面最底部移至下载区下方
                // 阶段16③：进度条总宽缩短约 30%（按可用宽度比例，避免撑满右栏）
                let dl = self.dl.as_ref().unwrap();
                if dl.mod_dl_busy {
                    let pb_w = (ui.available_width() * 0.70).max(160.0);
                    ui.add(
                        egui::ProgressBar::new(dl.mod_dl_progress.fraction())
                            .desired_width(pb_w)
                            .text(format!("{:.1}%", dl.mod_dl_progress.percent())),
                    );
                    if !dl.mod_download_name.is_empty() {
                        ui.label(
                            RichText::new(&dl.mod_download_name)
                                .size(11.0)
                                .color(self.fg(Color32::from_rgb(150, 150, 150))),
                        );
                    }
                }
            });
        });
        if let Some((url, fname)) = install {
            self.mod_install_file(&url, &fname);
        }
    }

    /// 打开项目页面：source_url 已有时直接打开；否则异步拉取项目详情回传 source_url
    fn dl_mod_open_page(&mut self, hit: &modrinth::ModrinthHit) {
        let src = hit.source_url.trim().to_string();
        if !src.is_empty() {
            let url = if src.starts_with("http://") || src.starts_with("https://") {
                src
            } else {
                format!("https://{src}")
            };
            let url_ref: &str = &url;
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", url_ref])
                .spawn();
            return;
        }
        // 无 source_url：后台拉取 project/{id} 拿 source_url 后打开（失败则打开 Modrinth 页）
        let pid = hit.id.clone();
        let slug = if hit.slug.is_empty() {
            hit.id.clone()
        } else {
            hit.slug.clone()
        };
        let shared = std::sync::Arc::new(std::sync::Mutex::new(None::<Result<String, String>>));
        let shared2 = std::sync::Arc::clone(&shared);
        std::thread::spawn(move || {
            let r = match modrinth::project_by_id(&pid) {
                Ok(p) => {
                    let s = p.source_url.trim().to_string();
                    if s.is_empty() {
                        Err("项目未提供官网地址".to_string())
                    } else {
                        Ok(if s.starts_with("http://") || s.starts_with("https://") {
                            s
                        } else {
                            format!("https://{s}")
                        })
                    }
                }
                Err(e) => Err(e.to_string()),
            };
            *shared2.lock().unwrap() = Some(r);
        });
        let d = self.dl.as_mut().unwrap();
        d.mod_open_shared = Some(shared);
        // 兜底：source_url 拉取失败时打开 Modrinth 页（异步结果 Err 时由 tick 处理）
        let fallback = format!("https://modrinth.com/mod/{slug}");
        let _ = fallback;
    }

    /// 直接安装指定版本文件（详情页点文件项）：归类到目标目录后启动下载
    fn mod_install_file(&mut self, url: &str, fname: &str) {
        let dir = self.dl.as_ref().unwrap().mod_target_dir.trim().to_string();
        if dir.is_empty() {
            self.set_toast("请先选择目标服务器或填写目标目录".to_string());
            return;
        }
        let pt2 = self.dl.as_ref().unwrap().mod_project_type.clone();
        let sub = match pt2.as_str() {
            "plugin" => "plugins",
            // 反馈②：数据包下载到 <服务器>/world/datapacks（Minecraft 标准数据包目录）
            "datapack" => "world/datapacks",
            _ => "mods",
        };
        let target = std::path::Path::new(&dir).join(sub);
        if let Err(e) = std::fs::create_dir_all(&target) {
            self.set_toast(format!("创建 {sub} 目录失败: {e}"));
            return;
        }
        let d = self.dl.as_mut().unwrap();
        d.mod_download_target = url.to_string();
        d.mod_download_name = target.join(fname).display().to_string();
        self.dl_start_mod_download();
    }

    /// 下载日志
    fn ui_dl_log(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("下载日志").strong());
        let log = self.dl.as_ref().unwrap().dl_log.clone();
        if log.is_empty() {
            ui.label(RichText::new("暂无记录").color(self.fg(Color32::from_rgb(150, 150, 150))));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("dl_log")
            .max_height(160.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &log {
                    ui.monospace(line);
                }
            });
    }
}

/// 简�?Base64 编码（RFC 4648，无外部依赖�?
fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// UTF-16LE �?Base64（PowerShell -EncodedCommand 格式�?
fn utf16le_b64(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    b64_encode(&bytes)
}

/// 当前进程是否管理�?
fn is_admin() -> bool {
    std::process::Command::new("powershell")
        .creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
        ])
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("True")
        })
        .unwrap_or(false)
}

/// 执行 PowerShell 脚本�?EncodedCommand 传递，避免转义问题）�?
/// 非管理员时自动通过 UAC 提权运行（弹出授权框，等待完成），返�?stdout 或错误信息�?
fn run_powershell(script: &str) -> Result<String, String> {
    let encoded = utf16le_b64(script);
    if is_admin() {
        let out = std::process::Command::new("powershell")
            .creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if err.is_empty() {
                format!("命令执行失败（状（{})", out.status.code().unwrap_or(-1))
            } else {
                err
            })
        }
    } else {
        // 提权执行：Start-Process -Verb RunAs -Wait -PassThru，输出子进程退出码
        let inner = format!(
            "$p = Start-Process -FilePath 'powershell' -Verb RunAs -Wait -PassThru -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{encoded}'; if ($p) {{ exit $p.ExitCode }} else {{ exit 1 }}"
        );
        let out = std::process::Command::new("powershell")
            .creation_flags(0x0800_0000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .args(["-NoProfile", "-NonInteractive", "-Command", &inner])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if err.is_empty() {
                "提权执行失败（可能已取消授权）".to_string()
            } else {
                err
            })
        }
    }
}

/// 根据 Minecraft 日志行级别返回颜�?
/// 兼容两种日志级别写法�?
///   旧格�?[12:34:56] [INFO/FML] ...
///   新格�?[20:31:15] [main/INFO] ...（Fabric / NeoForge 默认�?
fn log_line_color(line: &str, light: bool) -> Color32 {
    let level = line
        .split(']')
        .skip(1)
        .find_map(|seg| {
            let s = seg.trim().trim_start_matches('[').to_uppercase();
            let hit = s
                .split('/')
                .find(|p| matches!(*p, "INFO" | "WARN" | "ERROR" | "FATAL" | "DEBUG"));
            hit.map(|p| p.to_string())
        })
        .unwrap_or_default();
    let c = match level.as_str() {
        "INFO" => Color32::from_rgb(205, 220, 240),
        "WARN" => Color32::from_rgb(245, 200, 80),
        "ERROR" => Color32::from_rgb(255, 95, 95),
        "FATAL" => Color32::from_rgb(255, 60, 60),
        "DEBUG" => Color32::from_rgb(150, 170, 210),
        _ => Color32::from_rgb(185, 185, 185),
    };
    if light { theme::light_adapt(c) } else { c }
}

/// 实时性能监视条：CPU% / 内存折线图，仅服务器运行时显示真实数�?
/// 常驻性能横栏：与页签同级的一条固定高度横栏，不折叠、不随内容伸�?
fn show_perf_bar(ui: &mut egui::Ui, rt: &ServerRuntime, running: bool) {
    let (cpu, mem) = rt.perf.last().unwrap_or((0.0, 0.0));
    let h = 170.0;
    let (resp, painter) =
        ui.allocate_painter(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    let rect = resp.rect;
    let card = ui.visuals().widgets.noninteractive.bg_fill;
    painter.rect_filled(rect, 6.0, card);
    painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color));
    painter.text(
        egui::pos2(rect.left() + 12.0, rect.top() + 9.0),
        egui::Align2::LEFT_TOP,
        format!(
            "📊 进程性能　{}　CPU {:.0}%　内存 {:.0} MB",
            if running { "● 运行中" } else { "○ 未运行" },
            cpu,
            mem
        ),
        egui::FontId::proportional(13.0),
        ui.visuals().text_color(),
    );
    if running && rt.perf.samples.len() >= 2 {
        let plot = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 10.0, rect.top() + 34.0),
            egui::pos2(rect.right() - 10.0, rect.bottom() - 28.0),
        );
        painter.rect_filled(plot, 3.0, ui.visuals().extreme_bg_color);
        let samples: Vec<(f32, f32)> = rt
            .perf
            .samples
            .iter()
            .map(|s| (s.cpu_pct, s.mem_mb))
            .collect();
        let n = samples.len() as f32;
        let max_mem = samples
            .iter()
            .map(|(_, m)| *m)
            .fold(0.0_f32, f32::max)
            .max(1.0_f32);
        let x = |i: usize| plot.left() + 2.0 + (i as f32 / (n - 1.0_f32)) * (plot.width() - 4.0);
        let to_y = |v: f32, max: f32| plot.bottom() - 2.0 - (v / max).min(1.0_f32) * (plot.height() - 4.0);
        for k in 1..3 {
            let y = plot.top() + (plot.height() * k as f32 / 3.0_f32);
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
            );
        }
        let cpu_pts: Vec<egui::Pos2> = samples
            .iter()
            .enumerate()
            .map(|(i, s)| egui::pos2(x(i), to_y(s.0, 100.0)))
            .collect();
        painter.add(egui::Shape::line(
            cpu_pts,
            egui::Stroke::new(3.0_f32, Color32::from_rgb(80, 200, 120)),
        ));
        let mem_pts: Vec<egui::Pos2> = samples
            .iter()
            .enumerate()
            .map(|(i, s)| egui::pos2(x(i), to_y(s.1, max_mem)))
            .collect();
        painter.add(egui::Shape::line(
            mem_pts,
            egui::Stroke::new(3.0_f32, Color32::from_rgb(120, 180, 255)),
        ));
        // 图例：彩色方�?+ 文字（CPU �?/ 内存蓝）
        let leg_y = rect.bottom() - 9.0;
        let mut leg_x = rect.left() + 12.0;
        let sw = 10.0_f32;
        let sh = 10.0_f32;
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(leg_x, leg_y - sh), egui::vec2(sw, sh)),
            2.0,
            Color32::from_rgb(80, 200, 120),
        );
        leg_x += sw + 5.0;
        painter.text(
            egui::pos2(leg_x, leg_y),
            egui::Align2::LEFT_BOTTOM,
            "CPU% (0-100%)",
            egui::FontId::proportional(12.0),
            Color32::from_rgb(150, 160, 170),
        );
        leg_x += ui.fonts(|f| f.layout_no_wrap("CPU% (0-100%)".to_string(), egui::FontId::proportional(12.0), Color32::WHITE).size().x) + 16.0;
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(leg_x, leg_y - sh), egui::vec2(sw, sh)),
            2.0,
            Color32::from_rgb(120, 180, 255),
        );
        leg_x += sw + 5.0;
        painter.text(
            egui::pos2(leg_x, leg_y),
            egui::Align2::LEFT_BOTTOM,
            "内存(MB, 按本次采样最大值自适应)   （实时采样）",
            egui::FontId::proportional(12.0),
            Color32::from_rgb(150, 160, 170),
        );
    } else {
        painter.text(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 9.0),
            egui::Align2::LEFT_BOTTOM,
            "服务器未运行，无性能数据",
            egui::FontId::proportional(12.0),
            Color32::from_rgb(140, 145, 150),
        );
    }
}

/// 读取本地 frpc 版本（运�?frpc.exe --version�?
fn frp_local_version(frp_dir: &std::path::Path) -> Option<String> {
    let frpc = frp_dir.join("frpc.exe");
    if !frpc.exists() {
        return None;
    }
    let out = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new(&frpc)
            .arg("--version")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?
    };
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    let ver = text
        .split_whitespace()
        .find(|w| w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .unwrap_or(text)
        .to_string();
    Some(ver)
}

/// 下载 frpc.exe �?frp_dir（官方源 + 镜像按序尝试），成功后仅保留 frpc.exe�?
/// progress 回调用于向前端日志推送当前步骤（第几个源/下载/解压等），避免用户只能干等�?
fn download_frpc(
    frp_dir: &std::path::Path,
    mut progress: impl FnMut(&str),
) -> Result<(), String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let zip = frp_dir.join("frp.zip");
    let extract = frp_dir.join("extract");
    let sources = [
        "https://github.com/fatedier/frp/releases/latest/download/frp_windows_amd64.zip",
        "https://ghfast.top/https://github.com/fatedier/frp/releases/latest/download/frp_windows_amd64.zip",
        "https://gh-proxy.com/https://github.com/fatedier/frp/releases/latest/download/frp_windows_amd64.zip",
        "https://mirror.ghproxy.com/https://github.com/fatedier/frp/releases/latest/download/frp_windows_amd64.zip",
    ];
    let mut last_err = String::new();
    for (i, url) in sources.iter().enumerate() {
        progress(&format!("下下载 {}/{}：{url}", i + 1, sources.len()));
        let script = format!(
            "$ErrorActionPreference='Stop'\n[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12\nInvoke-WebRequest -Uri '{url}' -OutFile '{}' -UseBasicParsing\nExpand-Archive -Path '{}' -DestinationPath '{}' -Force\n",
            zip.to_string_lossy().replace('\'', "''"),
            zip.to_string_lossy().replace('\'', "''"),
            extract.to_string_lossy().replace('\'', "''")
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .output();
        match out {
            Ok(o) if o.status.success() => {
                progress("下载完成，正在解压");
                // zip 内目录形�?frp_0.6x_windows_amd64/frpc.exe，递归查找
                let found = find_file_recursive(&extract, "frpc.exe");
                if let Some(src) = found {
                    let dst = frp_dir.join("frpc.exe");
                    let _ = std::fs::remove_file(&dst);
                    std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
                    let _ = std::fs::remove_file(&zip);
                    let _ = std::fs::remove_dir_all(&extract);
                    return Ok(());
                }
                last_err = "压缩包内未找到 frpc.exe".to_string();
            }
            Ok(o) => {
                last_err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if last_err.is_empty() {
                    last_err = format!("下载失败（HTTP 状态 {}", o.status.code().unwrap_or(-1)) + ")";
                }
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(format!("所有下载源均失败：{last_err}"))
}

fn find_file_recursive(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().map(|n| n == name).unwrap_or(false) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 生成集中管理�?frpc.toml（多隧道合并），返回配置文件路径
fn write_frpc_toml_file(
    frp_dir: &std::path::Path,
    server: &str,
    token: &str,
    tunnels: &[(String, String, u16, u16)],
) -> Result<std::path::PathBuf, String> {
    let (host, port) = match server.trim().rsplit_once(':') {
        Some((h, p)) => (h.trim().to_string(), p.trim().parse::<u16>().unwrap_or(7000)),
        None => (server.trim().to_string(), 7000),
    };
    let mut s = String::new();
    s.push_str(&format!("serverAddr = \"{host}\"\n"));
    s.push_str(&format!("serverPort = {port}\n"));
    if !token.trim().is_empty() {
        s.push_str(&format!("auth.token = \"{}\"\n", token.trim()));
    }
    s.push('\n');
    for (name, ptype, local, remote) in tunnels {
        s.push_str("[[proxies]]\n");
        s.push_str(&format!("name = \"{}\"\n", sanitize_toml_str(name)));
        s.push_str(&format!("type = \"{ptype}\"\n"));
        s.push_str("localIP = \"127.0.0.1\"\n");
        s.push_str(&format!("localPort = {local}\n"));
        s.push_str(&format!("remotePort = {remote}\n\n"));
    }
    let path = frp_dir.join("frpc.toml");
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(path)
}

fn sanitize_toml_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ")
}

/// 从 frpc.toml 文本解析 serverAddr（简单行扫描，忽略注释）
fn parse_frp_server_addr(cfg: &str) -> String {
    cfg.lines()
        .find_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            if let Some(rest) = l.strip_prefix("serverAddr") {
                let rest = rest.trim_start().trim_start_matches('=').trim();
                Some(rest.trim_matches('"').to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// 从 frpc.toml 文本解析 localPort / remotePort（简单行扫描）
fn parse_frp_ports(cfg: &str) -> (u16, u16) {
    let mut local = 0u16;
    let mut remote = 0u16;
    for l in cfg.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with("localPort") {
            if let Some(rest) = l.split('=').nth(1) {
                local = rest.trim().parse().unwrap_or(0);
            }
        } else if l.starts_with("remotePort") {
            if let Some(rest) = l.split('=').nth(1) {
                remote = rest.trim().parse().unwrap_or(0);
            }
        }
    }
    (local, remote)
}

/// 从 frpc.toml 文本解析 admin API 端口（webServer.port 新版 / adminPort 旧版，兼容）
fn parse_frp_admin_port(cfg: &str) -> Option<u16> {
    for l in cfg.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with("webServer.port") || l.starts_with("adminPort") {
            if let Some(rest) = l.split('=').nth(1) {
                let p: u16 = rest.trim().parse().ok()?;
                if p != 0 {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 从 frpc.toml 文本解析 localPort（本地服务端口），与 parse_frp_ports 同源
fn parse_frp_local_port(cfg: &str) -> Option<u16> {
    for l in cfg.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with("localPort") {
            if let Some(rest) = l.split('=').nth(1) {
                let p: u16 = rest.trim().parse().ok()?;
                if p != 0 {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 从 frpc.toml 文本解析 serverPort / server_port（frps 服务端端口），frpc 主连接固定连向该端口
fn parse_frp_server_port(cfg: &str) -> Option<u16> {
    for l in cfg.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with("serverPort") || l.starts_with("server_port") {
            if let Some(rest) = l.split('=').nth(1) {
                let p: u16 = rest.trim().parse().ok()?;
                if p != 0 {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 递归统计目录占用字节数（后台线程调用，避免大 world 卡 UI）
fn dir_size_bytes(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let p = e.path();
            let meta = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                total = total.saturating_add(dir_size_bytes(&p));
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// 当前秒时间戳（f64）
fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// 字节数人类可读格式化（1024 进制）
fn fmt_bytes(b: u64) -> String {
    if b >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", b as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024 * 1024 {
        format!("{:.2} MB", b as f64 / (1024.0 * 1024.0))
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}

/// frpc 进程级流量统计（IPv4+IPv6 汇总）：统计 frpc 进程与 frps 服务端（serverPort）之间
/// 全部 ESTAB 连接的累计收发字节。返回 (下行, 上行)：
/// - 下行 = frpc 从 frps 接收的字节（外网→内网，对应 frp trafficIn）
/// - 上行 = frpc 发送给 frps 的字节（内网→外网，对应 frp trafficOut）
/// UDP 隧道不建立 TCP 连接，恒返回 (0, 0)。
/// 依赖 GetPerTcpConnectionEStats（Vista+，无需管理员，但仅统计到本用户权限可见的连接）。
/// IPv6 表读取失败时静默忽略（frps 走 IPv4 的场景不受影响）。
fn query_frpc_traffic(frpc_pid: u32, server_port: u16) -> Result<(u64, u64), String> {
    let (v4_in, v4_out) = query_frpc_traffic_v4(frpc_pid, server_port)?;
    let (v6_in, v6_out) = query_frpc_traffic_v6(frpc_pid, server_port).unwrap_or((0, 0));
    Ok((v4_in + v6_in, v4_out + v6_out))
}

/// IPv4 实现
fn query_frpc_traffic_v4(frpc_pid: u32, server_port: u16) -> Result<(u64, u64), String> {
    use std::mem::{size_of, zeroed};
    use winapi::shared::iprtrmib::TCP_TABLE_OWNER_PID_ALL;
    use winapi::shared::tcpmib::{
        MIB_TCPROW, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_ESTAB,
    };
    use winapi::shared::tcpestats::{
        TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData,
    };
    use winapi::shared::winerror::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use winapi::shared::ws2def::AF_INET;
    use winapi::um::iphlpapi::{GetExtendedTcpTable, GetPerTcpConnectionEStats};

    // 1) 枚举 IPv4 TCP 连接表
    let mut size: u32 = 0;
    let mut buf: Vec<u8> = Vec::new();
    let mut ret = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if (ret == NO_ERROR || ret == ERROR_INSUFFICIENT_BUFFER) && size > 0 {
        buf.resize(size as usize, 0);
        ret = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr() as *mut _,
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
    }
    if ret != NO_ERROR {
        return Err(format!("GetExtendedTcpTable: {ret}"));
    }
    let table = unsafe { &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
    let rows = unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };

    let mut tin = 0u64;
    let mut tout = 0u64;
    for r in rows {
        if r.dwState != MIB_TCP_STATE_ESTAB {
            continue;
        }
        if r.dwOwningPid != frpc_pid {
            continue;
        }
        // MIB 表端口按网络字节序（大端）存储
        let rport = u16::from_be((r.dwRemotePort & 0xFFFF) as u16);
        if rport != server_port {
            continue;
        }
        // MIB_TCPROW(=MIB_TCPROW_LH) 与 MIB_TCPROW_OWNER_PID 前 20 字节布局一致，直接复用行内存
        let row_ptr = r as *const MIB_TCPROW_OWNER_PID as *mut MIB_TCPROW;
        let mut rw: TCP_ESTATS_DATA_RW_v0 = unsafe { zeroed() };
        rw.EnableCollection = 1;
        let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { zeroed() };
        let rc = unsafe {
            GetPerTcpConnectionEStats(
                row_ptr,
                TcpConnectionEstatsData,
                &mut rw as *mut _ as *mut u8,
                0,
                size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                &mut rod as *mut _ as *mut u8,
                0,
                size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
                std::ptr::null_mut(),
                0,
                0,
            )
        };
        if rc == NO_ERROR {
            // frpc 从 frps 接收 = 下行（外网→本地服务），发送给 frps = 上行（本地服务→外网）
            tin = tin.saturating_add(rod.DataBytesIn);
            tout = tout.saturating_add(rod.DataBytesOut);
        }
    }
    Ok((tin, tout))
}

/// IPv6 实现：frps 若解析出 IPv6 地址或 frpc 启用 IPv6 优先，主连接会走 IPv6 表
fn query_frpc_traffic_v6(frpc_pid: u32, server_port: u16) -> Result<(u64, u64), String> {
    use std::mem::{size_of, zeroed};
    use winapi::shared::iprtrmib::TCP_TABLE_OWNER_PID_ALL;
    use winapi::shared::tcpmib::{
        MIB_TCP6ROW, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCP_STATE_ESTAB,
    };
    use winapi::shared::tcpestats::{
        TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData,
    };
    use winapi::shared::winerror::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use winapi::shared::ws2def::AF_INET6;
    use winapi::um::iphlpapi::{GetExtendedTcpTable, GetPerTcpConnectionEStats};

    let mut size: u32 = 0;
    let mut buf: Vec<u8> = Vec::new();
    let mut ret = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if (ret == NO_ERROR || ret == ERROR_INSUFFICIENT_BUFFER) && size > 0 {
        buf.resize(size as usize, 0);
        ret = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr() as *mut _,
                &mut size,
                0,
                AF_INET6 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
    }
    if ret != NO_ERROR {
        return Err(format!("GetExtendedTcpTable(v6): {ret}"));
    }
    let table = unsafe { &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID) };
    let rows =
        unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };

    let mut tin = 0u64;
    let mut tout = 0u64;
    for r in rows {
        if r.dwState != MIB_TCP_STATE_ESTAB || r.dwOwningPid != frpc_pid {
            continue;
        }
        // MIB 表端口按网络字节序（大端）存储
        let rport = u16::from_be((r.dwRemotePort & 0xFFFF) as u16);
        if rport != server_port {
            continue;
        }
        // MIB_TCP6ROW(=MIB_TCP6ROW_LH) 与 MIB_TCP6ROW_OWNER_PID 前部布局一致，直接复用行内存
        // GetPerTcpConnectionEStats 参数类型为 *mut MIB_TCPROW_LH，此处按 C 头文件约定强转
        let row_ptr = r as *const MIB_TCP6ROW_OWNER_PID as *mut winapi::shared::tcpmib::MIB_TCPROW_LH;
        let mut rw: TCP_ESTATS_DATA_RW_v0 = unsafe { zeroed() };
        rw.EnableCollection = 1;
        let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { zeroed() };
        let rc = unsafe {
            GetPerTcpConnectionEStats(
                row_ptr,
                TcpConnectionEstatsData,
                &mut rw as *mut _ as *mut u8,
                0,
                size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                &mut rod as *mut _ as *mut u8,
                0,
                size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
                std::ptr::null_mut(),
                0,
                0,
            )
        };
        if rc == NO_ERROR {
            tin = tin.saturating_add(rod.DataBytesIn);
            tout = tout.saturating_add(rod.DataBytesOut);
        }
    }
    Ok((tin, tout))
}

/// 流量统计诊断：返回人类可读的排查信息（PID / serverPort / 匹配 ESTAB 连接数 / EStats 成功数 / 累计字节）。
/// 用于帮助定位"流量恒 0"的具体环节（进程 PID 拿不到 / serverPort 解析失败 / 连接不匹配 / EStats 读取失败）。
fn query_frpc_traffic_diag(frpc_pid: u32, server_port: u16) -> String {
    use std::mem::{size_of, zeroed};
    use winapi::shared::iprtrmib::TCP_TABLE_OWNER_PID_ALL;
    use winapi::shared::tcpmib::{
        MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_ESTAB,
    };
    use winapi::shared::tcpestats::{
        TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData,
    };
    use winapi::shared::winerror::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use winapi::shared::ws2def::{AF_INET, AF_INET6};
    use winapi::um::iphlpapi::{GetExtendedTcpTable, GetPerTcpConnectionEStats};

    let mut out = String::new();
    out.push_str(&format!("诊断对象：frpc PID={frpc_pid}，serverPort={server_port}\n"));
    let mut tin = 0u64;
    let mut tout = 0u64;
    let mut matched = 0usize;
    let mut estats_ok = 0usize;
    // IPv4
    {
        let mut size: u32 = 0;
        let mut buf: Vec<u8> = Vec::new();
        let mut ret = unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if (ret == NO_ERROR || ret == ERROR_INSUFFICIENT_BUFFER) && size > 0 {
            buf.resize(size as usize, 0);
            ret = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr() as *mut _,
                    &mut size,
                    0,
                    AF_INET as u32,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
        }
        if ret == NO_ERROR {
            let table = unsafe { &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
            let rows =
                unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };
            for r in rows {
                if r.dwState != MIB_TCP_STATE_ESTAB || r.dwOwningPid != frpc_pid {
                    continue;
                }
                let rport = u16::from_be((r.dwRemotePort & 0xFFFF) as u16);
                if rport != server_port {
                    continue;
                }
                matched += 1;
                let row_ptr = r as *const MIB_TCPROW_OWNER_PID as *mut winapi::shared::tcpmib::MIB_TCPROW;
                let mut rw: TCP_ESTATS_DATA_RW_v0 = unsafe { zeroed() };
                rw.EnableCollection = 1;
                let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { zeroed() };
                let rc = unsafe {
                    GetPerTcpConnectionEStats(
                        row_ptr,
                        TcpConnectionEstatsData,
                        &mut rw as *mut _ as *mut u8,
                        0,
                        size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                        &mut rod as *mut _ as *mut u8,
                        0,
                        size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
                        std::ptr::null_mut(),
                        0,
                        0,
                    )
                };
                if rc == NO_ERROR {
                    estats_ok += 1;
                    tin = tin.saturating_add(rod.DataBytesIn);
                    tout = tout.saturating_add(rod.DataBytesOut);
                }
            }
            out.push_str(&format!("IPv4 表：OK\n"));
        } else {
            out.push_str(&format!("IPv4 表：GetExtendedTcpTable 失败 ret={ret}\n"));
        }
    }
    // IPv6
    {
        let mut size: u32 = 0;
        let mut buf: Vec<u8> = Vec::new();
        let mut ret = unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                AF_INET6 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if (ret == NO_ERROR || ret == ERROR_INSUFFICIENT_BUFFER) && size > 0 {
            buf.resize(size as usize, 0);
            ret = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr() as *mut _,
                    &mut size,
                    0,
                    AF_INET6 as u32,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
        }
        if ret == NO_ERROR {
            let table = unsafe { &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID) };
            let rows =
                unsafe { std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize) };
            for r in rows {
                if r.dwState != MIB_TCP_STATE_ESTAB || r.dwOwningPid != frpc_pid {
                    continue;
                }
                let rport = u16::from_be((r.dwRemotePort & 0xFFFF) as u16);
                if rport != server_port {
                    continue;
                }
                matched += 1;
                let row_ptr =
                    r as *const MIB_TCP6ROW_OWNER_PID as *mut winapi::shared::tcpmib::MIB_TCPROW_LH;
                let mut rw: TCP_ESTATS_DATA_RW_v0 = unsafe { zeroed() };
                rw.EnableCollection = 1;
                let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { zeroed() };
                let rc = unsafe {
                    GetPerTcpConnectionEStats(
                        row_ptr,
                        TcpConnectionEstatsData,
                        &mut rw as *mut _ as *mut u8,
                        0,
                        size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                        &mut rod as *mut _ as *mut u8,
                        0,
                        size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
                        std::ptr::null_mut(),
                        0,
                        0,
                    )
                };
                if rc == NO_ERROR {
                    estats_ok += 1;
                    tin = tin.saturating_add(rod.DataBytesIn);
                    tout = tout.saturating_add(rod.DataBytesOut);
                }
            }
            out.push_str(&format!("IPv6 表：OK\n"));
        } else {
            out.push_str(&format!("IPv6 表：GetExtendedTcpTable 失败 ret={ret}（忽略）\n"));
        }
    }
    out.push_str(&format!(
        "匹配 ESTAB 连接：{matched} 条；EStats 读取成功：{estats_ok} 条\n累计字节：下行(外网→内) {}，上行(内→外网) {}\n",
        fmt_bytes(tin),
        fmt_bytes(tout)
    ));
    if matched == 0 {
        out.push_str("→ 未匹配到 frpc→frps 主连接。可能原因：frps 用的不是该 serverPort；frpc 以其他进程/方式启动（非 XMST 托管）；隧道类型为 UDP（不建 TCP 连接）。\n");
    } else if estats_ok == 0 {
        out.push_str("→ 连接已匹配但 EStats 读取失败。可能原因：权限受限（建议以管理员运行 XMST 再试）、内核 TCP 统计未启用（等待 2 个采样周期）。\n");
    }
    out
}

/// 替换 frpc.toml 中的 localPort / remotePort 数值（保留原缩进与格式）
fn set_frp_ports(cfg: &str, local: u16, remote: u16) -> String {
    let mut out = String::with_capacity(cfg.len() + 16);
    for l in cfg.lines() {
        let t = l.trim_start();
        if t.starts_with("localPort") || t.starts_with("remotePort") {
            let indent = &l[..l.len() - t.len()];
            let key = t.split('=').next().unwrap_or("localPort").trim();
            let val = if t.starts_with("localPort") { local } else { remote };
            out.push_str(&format!("{indent}{key} = {val}\n"));
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// 可拖动分隔条：上下拖动调整编辑区高度（h 实时更新）。
/// 最小 40px，不设上限（允许拉倒最大，配合整页滚动可查看全部内容）。
/// 使用起始高度+指针位移计算，避免逐帧累加 drag_delta 造成漂移。
fn draggable_divider(ui: &mut egui::Ui, h: &mut f32, drag_start: &mut Option<(f32, f32)>) {
    let (sep_rect, sep_resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 14.0),
        egui::Sense::drag(),
    );
    let sep_hover = sep_resp.hovered() || sep_resp.dragged();
    let sep_stroke = if sep_hover {
        egui::Stroke::new(2.0, Color32::from_rgb(120, 200, 255))
    } else {
        egui::Stroke::new(2.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
    };
    ui.painter().hline(sep_rect.x_range(), sep_rect.center().y, sep_stroke);
    if sep_resp.drag_started() {
        *drag_start = Some((*h, sep_resp.interact_pointer_pos().map(|p| p.y).unwrap_or(0.0)));
    }
    if sep_resp.dragged() {
        if let (Some((start_h, start_y)), Some(cur)) =
            (*drag_start, sep_resp.interact_pointer_pos())
        {
            *h = (start_h + (cur.y - start_y)).max(40.0);
        }
    } else {
        *drag_start = None;
    }
    if sep_hover {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
}

/// 日志渲染：按行着�?+ 滚动条自动贴底（显示最新日志）
/// scroll_to_end：本帧有新增日志或首次显示时�?true，强制滚动到底部
/// 返回日志区域的点击响应（用于"点击日志聚焦命令输入�?�?
fn show_colored_log(
    ui: &mut egui::Ui,
    log: &str,
    scroll_to_end: bool,
    force_bottom: bool,
    max_height: Option<f32>,
) -> egui::Response {
    use egui::text::{LayoutJob, TextFormat};
    use egui::FontId;

    // 只渲染最�?RENDER_MAX 行：缓冲再大也只排版可视窗口，避免每帧全量文本整形导致卡顿�?
    // 老日志无用（用户偏好从最底看最新），滚动条即反映该渲染窗口�?
    const RENDER_MAX: usize = 1200;
    let lines: Vec<&str> = log.lines().collect();
    let start = lines.len().saturating_sub(RENDER_MAX);

    let mono = FontId::monospace(12.0);
    let mut job = LayoutJob::default();
    for line in lines.iter().skip(start) {
        let color = log_line_color(line, !ui.visuals().dark_mode);
        job.append(
            line,
            0.0,
            TextFormat {
                font_id: mono.clone(),
                color,
                ..Default::default()
            },
        );
        job.append(
            "\n",
            0.0,
            TextFormat {
                font_id: mono.clone(),
                ..Default::default()
            },
        );
    }
    let mut area = egui::ScrollArea::vertical()
        .id_salt("show_colored_log")
        .auto_shrink([false, false]);
    if let Some(h) = max_height {
        area = area.max_height(h);
    }
    if scroll_to_end || force_bottom {
        // 贴底即可，不要用 f32::MAX 偏移：那会让滚动条极小且内容不可见（切回日志空白 bug�?
        area = area.stick_to_bottom(true);
    }
    area.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(job))
    })
    .inner
}

// ---------- server.properties 编辑辅助 ----------

/// 解析 Java properties 文本为键值对（保留全部键，忽略空行与注释）�?
fn parse_properties(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        if let Some(eq) = line.find(['=', ':']) {
            let k = line[..eq].trim().to_string();
            let v = line[eq + 1..].trim().to_string();
            if !k.is_empty() {
                m.insert(k, v);
            }
        } else if !line.is_empty() {
            m.insert(line.to_string(), String::new());
        }
    }
    m
}

/// 将键值对格式化为 properties 文本（每�?key=value）�?
fn format_properties(m: &std::collections::BTreeMap<String, String>) -> String {
    let mut s = String::new();
    for (k, v) in m {
        s.push_str(k);
        s.push('=');
        s.push_str(v);
        s.push('\n');
    }
    s
}

/// server.properties 类型化编辑区：返回修改后的键值对�?
/// 常见键以中文标签 + 类型化控件呈现；未列出的键原样保留在 map 中�?
fn show_server_props_typed(
    ui: &mut egui::Ui,
    map: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    let mut m = map.clone();
    let g = &mut m;

    // 文本�?
    fn row_text(
        ui: &mut egui::Ui,
        m: &mut std::collections::BTreeMap<String, String>,
        label: &str,
        key: &str,
        def: &str,
    ) {
        ui.label(label);
        let mut v = m.entry(key.to_string()).or_insert_with(|| def.to_string());
        ui.add(TextEdit::singleline(v).desired_width(260.0));
        ui.end_row();
    }
    // 布尔�?
    fn row_bool(
        ui: &mut egui::Ui,
        m: &mut std::collections::BTreeMap<String, String>,
        label: &str,
        key: &str,
        def: &str,
    ) {
        ui.label(label);
        let mut v = m.entry(key.to_string()).or_insert_with(|| def.to_string());
        let mut b = v == "true";
        if ui.checkbox(&mut b, "").changed() {
            *v = if b { "true".to_string() } else { "false".to_string() };
        }
        ui.end_row();
    }
    // 整数�?
    fn row_int(
        ui: &mut egui::Ui,
        m: &mut std::collections::BTreeMap<String, String>,
        label: &str,
        key: &str,
        def: i64,
        range: std::ops::RangeInclusive<i64>,
    ) {
        ui.label(label);
        let mut v = m.entry(key.to_string()).or_insert_with(|| def.to_string());
        let mut n: i64 = v.parse().unwrap_or(def);
        if ui.add(egui::DragValue::new(&mut n).range(range).speed(1)).changed() {
            *v = n.to_string();
        }
        ui.end_row();
    }
    // 下拉�?
    fn row_select(
        ui: &mut egui::Ui,
        m: &mut std::collections::BTreeMap<String, String>,
        label: &str,
        key: &str,
        def: &str,
        opts: &[&str],
        id: &str,
    ) {
        ui.label(label);
        let mut v = m.entry(key.to_string()).or_insert_with(|| def.to_string());
        let mut sel = opts
            .iter()
            .position(|o| *o == v.as_str())
            .unwrap_or_else(|| opts.iter().position(|o| *o == def).unwrap_or(0));
        egui::ComboBox::from_id_salt(id)
            .selected_text(opts[sel])
            .show_ui(ui, |ui| {
                for (i, o) in opts.iter().enumerate() {
                    if ui.selectable_label(sel == i, *o).clicked() {
                        sel = i;
                    }
                }
            });
        *v = opts[sel].to_string();
        ui.end_row();
    }

    egui::Grid::new("sp_props_grid")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            row_text(ui, g, "服务器端口", "server-port", "25565");
            row_text(ui, g, "服务器 IP（留空全部）", "server-ip", "");
            row_text(ui, g, "世界名称", "level-name", "world");
            row_text(ui, g, "世界种子", "level-seed", "");
            row_text(ui, g, "MMOTD 展示", "motd", "A Minecraft Server");
            row_text(ui, g, "资源包 URL（可选）", "resource-pack", "");
            row_int(ui, g, "最大玩家数", "max-players", 20, 1..=9999);
            row_int(ui, g, "视图距离（区块）", "view-distance", 10, 3..=32);
            row_int(ui, g, "模拟距离（区块）", "simulation-distance", 10, 3..=32);
            row_int(ui, g, "出生点保保护半径", "spawn-protection", 16, 0..=1000);
            row_int(ui, g, "网络压缩阈值（-1 关闭）", "network-compression-threshold", 256, -1..=9999);
            row_bool(ui, g, "在线模式（正版验证）", "online-mode", "true");
            row_bool(ui, g, "白名单", "white-list", "false");
            row_bool(ui, g, "允许 PVP", "pvp", "true");
            row_bool(ui, g, "极限模式（锁定困难）", "hardcore", "false");
            row_bool(ui, g, "允许飞行", "allow-flight", "false");
            row_bool(ui, g, "允许命令方块", "enable-command-block", "false");
            row_bool(ui, g, "强制安全档案", "enforce-secure-profile", "true");
            row_bool(ui, g, "启用 RCON 远程控制", "enable-rcon", "false");
            row_text(ui, g, "RCON 密码", "rcon.password", "");
            row_int(ui, g, "RCON 端口", "rcon.port", 25575, 1..=65535);
            row_bool(ui, g, "启用查询（GameSpy4）", "enable-query", "false");
            row_select(ui, g, "游戏模式", "gamemode", "survival", &["survival", "creative", "adventure", "spectator"], "sp_gamemode");
            row_select(ui, g, "难度", "difficulty", "easy", &["peaceful", "easy", "normal", "hard"], "sp_difficulty");
            row_select(ui, g, "默认玩家权限", "op-permission-level", "4", &["1", "2", "3", "4"], "sp_opperm");
        });
    m
}

/// 读取 JSON 数组文件；不存在/损坏返回空列表
fn load_json_list(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
        .unwrap_or_default()
}

/// 原子写入 JSON 数组文件（先写临时文件再改名，避免服务器运行中读一半）
fn save_json_list(path: &std::path::Path, list: &[serde_json::Value]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// 毫秒时间戳 → 本地时间 "YYYY-MM-DD HH:MM"（0/负数返回 "—"）
fn fmt_ms_time(ms: i64) -> String {
    if ms <= 0 {
        return "—".to_string();
    }
    let secs = ms / 1000;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, m) = ((rem / 3600) as u32, ((rem % 3600) / 60) as u32);
    // 从 1970-01-01 推算日期（公历）
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let yd = if leap { 366 } else { 365 };
        if d < yd {
            break;
        }
        d -= yd;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mdays = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1usize;
    let mut dd = d;
    for (i, md) in mdays.iter().enumerate() {
        if dd < *md {
            mo = i + 1;
            break;
        }
        dd -= md;
    }
    format!("{y:04}-{mo:02}-{:02} {h:02}:{m:02}", dd + 1)
}

/// 离线模式 UUID：MD5("OfflinePlayer:" + name)，按 UUID 标准格式输出（v3 风格）
fn offline_uuid(name: &str) -> String {
    let mut input = String::from("OfflinePlayer:");
    input.push_str(name);
    let digest = md5_hex(input.as_bytes());
    // 8-4-4-4-12，并设置版本位 3（UUID v3）
    let mut bytes = [0u8; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&digest[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    bytes[6] = (bytes[6] & 0x0F) | 0x30;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// 纯 Rust MD5（RFC 1321），用于离线 UUID 生成，避免新增依赖
fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = tmp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let out = [a0.to_le_bytes(), b0.to_le_bytes(), c0.to_le_bytes(), d0.to_le_bytes()].concat();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// 崩溃来源列表：(kind, 相对路径)，kind: 0=latest.log, 1=crash-reports/*.txt, 2=hs_err_pid*.log
/// 相对路径基于服务器根目录（dir），可直接 dir.join(rel) 得到真实路径。
fn collect_crash_sources(dir: &std::path::Path) -> Vec<(u8, String)> {
    let mut out: Vec<(u8, String)> = Vec::new();
    // 0) latest.log
    if dir.join("logs").join("latest.log").exists() {
        out.push((0, "logs/latest.log".to_string()));
    }
    // 1) crash-reports/*.txt，按修改时间倒序（最新在前）
    let cr_dir = dir.join("crash-reports");
    if cr_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&cr_dir) {
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".txt"))
                .collect();
            files.sort_by_key(|e| {
                e.metadata().and_then(|m| m.modified()).map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                }).unwrap_or(0)
            });
            files.reverse();
            for e in files.into_iter().take(12) {
                out.push((1, format!("crash-reports/{}", e.file_name().to_string_lossy())));
            }
        }
    }
    // 2) hs_err_pid*.log：根目录 + logs/ 下
    let mut hs: Vec<std::path::PathBuf> = Vec::new();
    for base in [dir.to_path_buf(), dir.join("logs")] {
        if let Ok(entries) = std::fs::read_dir(&base) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("hs_err_pid") && name.ends_with(".log") {
                    hs.push(e.path());
                }
            }
        }
    }
    hs.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).map(|t| {
        t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }).unwrap_or(0));
    hs.reverse();
    for p in hs.into_iter().take(6) {
        let rel = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        out.push((2, rel));
    }
    out
}

/// 读取文件尾部（按字节截取），供日志/报告分析使用。
fn tail_of_file(path: &std::path::Path, bytes: usize) -> String {
    std::fs::read(path).ok().map(|buf| {
        let start = buf.len().saturating_sub(bytes);
        String::from_utf8_lossy(&buf[start..]).into_owned()
    }).unwrap_or_default()
}

/// 分析 latest.log 尾部，返回关键错误行与判定；无命中返回 None。
/// 输出使用行首标记：[H]标题 [E]错误/堆栈 [W]判定 [S]建议 [M]模组 [I]信息，UI 按标记着色。
fn analyze_latest_log_tail(dir: &std::path::Path) -> Option<String> {
    let log_path = dir.join("logs").join("latest.log");
    if !log_path.exists() {
        return None;
    }
    let tail = tail_of_file(&log_path, 256 * 1024);
    let has = |text: &str, pats: &[&str]| -> bool { pats.iter().any(|p| text.contains(p)) };
    let mut hits: Vec<String> = Vec::new();
    for line in tail.lines().rev().take(400) {
        let l = line.trim();
        if l.contains("[Server thread/ERROR]")
            || l.contains("[main/ERROR]")
            || l.contains("Exception in thread")
            || l.contains("Caused by:")
            || l.contains("at net.minecraft")
            || l.contains("at java.lang.Thread.run")
        {
            hits.push(l.to_string());
            if hits.len() >= 30 {
                break;
            }
        }
    }
    if hits.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("[H] latest.log 关键错误行（尾部）\n");
    for h in hits.iter().take(12) {
        out.push_str(&format!("  [E] {}\n", truncate_str(h, 220)));
    }
    if hits.len() > 12 {
        out.push_str(&format!("  [I] …（另有 {} 条）\n", hits.len() - 12));
    }
    // 模组级定位（加载失败 / 依赖缺失场景，PCL 同款思路）
    let mods = parse_mods_from_log(&tail);
    if !mods.is_empty() {
        out.push_str("  [M] 可疑模组:\n");
        for m in mods.iter().take(8) {
            out.push_str(&format!("    [M] {m}\n"));
        }
    }
    if has(&tail, &["OutOfMemoryError", "There is insufficient memory", "unable to create native thread"]) {
        out.push_str("  [W] 判定：内存不足（OutOfMemoryError / 无法分配内存）\n");
        out.push_str("  [S] 建议：调高 JVM -Xmx 内存，或减少同时加载的模组数量；检查系统可用内存。\n");
    }
    if has(&tail, &["NoSuchMethodError", "NoClassDefFoundError", "ClassNotFoundException"]) {
        out.push_str("  [W] 判定：类/方法缺失——通常是模组版本冲突或依赖缺失\n");
        out.push_str("  [S] 建议：核对冲突模组版本，补齐前置模组，或移除不兼容模组。\n");
    }
    if has(&tail, &["DuplicateModsException", "duplicate entries", "two mods with the same name"]) {
        out.push_str("  [W] 判定：重复模组（同名模组被加载两次）\n");
        out.push_str("  [S] 建议：在 mods 目录删除重复的 .jar 文件。\n");
    }
    if has(&tail, &["Failed to create client thread", "java.lang.IllegalStateException"]) {
        out.push_str("  [W] 判定：客户端线程初始化失败（常见于模组注入崩溃）\n");
        out.push_str("  [S] 建议：查看 Caused by 链定位具体模组，尝试移除最近新增的模组。\n");
    }
    Some(out)
}

/// 从 latest.log 尾部提取可疑模组名（模组加载失败 / 依赖缺失场景）
fn parse_mods_from_log(tail: &str) -> Vec<String> {
    let mut mods: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let n = name
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim_matches(|c| c == ':' || c == ',' || c == ';')
            .to_string();
        if !n.is_empty() && !mods.contains(&n) {
            mods.push(n);
        }
    };
    for line in tail.lines() {
        let l = line.trim();
        // "Failed to load mod <name>" / "Failed to load mod <name> (from mods/xxx.jar)"
        if let Some(idx) = l.find("Failed to load mod") {
            let rest = &l[idx + "Failed to load mod".len()..];
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c| c == '(' || c == ')' || c == '"');
            if !name.is_empty() && name != "from" {
                push(name);
            }
        }
        // "Missing or unsupported mandatory dependencies: <mod1>, <mod2>"
        if let Some(idx) = l.find("Missing or unsupported mandatory dependencies") {
            let rest = &l[idx..];
            for seg in rest.split_whitespace().skip_while(|s| !s.contains(':')) {
                let s = seg
                    .trim_matches(|c| c == ':' || c == ',' || c == ';' || c == '(' || c == ')');
                if !s.is_empty() && !s.contains('.') {
                    push(s);
                }
            }
        }
        // "Mod Loading Exception" / "Error loading mod" 行尾常带模组名
        if l.contains("Mod Loading Exception") || l.contains("Error loading mod") {
            let rest = l.split("Exception").next().unwrap_or("");
            let last = rest.split_whitespace().last().unwrap_or("");
            if !last.is_empty() && !last.starts_with("java") {
                push(last);
            }
        }
        // "Mod <name> requires version" / "Mod <name> could not be loaded"
        if l.contains(" requires version") || l.contains(" could not be loaded") || l.contains(" is missing") {
            if let Some(idx) = l.find("Mod ") {
                let after = &l[idx + 4..];
                let name = after.split_whitespace().next().unwrap_or("");
                if !name.is_empty() && !name.starts_with('<') && !name.starts_with('>') {
                    push(name);
                }
            }
        }
    }
    mods
}

/// 分析单个 crash-reports/*.txt 报告，返回摘要；无关键内容返回 None。
fn analyze_crash_report_file(path: &std::path::Path) -> Option<String> {
    let txt = std::fs::read_to_string(path).unwrap_or_default();
    if txt.trim().is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("[H] crash-reports 报告\n");
    out.push_str(&format!("  [I] 文件: {}\n", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    let mut found = false;
    for line in txt.lines().take(60) {
        let l = line.trim();
        if l.starts_with("Description:")
            || l.starts_with("java.")
            || l.starts_with("at ")
            || l.starts_with("Caused by:")
            || l.starts_with("-- Head")
            || l.starts_with("-- Affected")
        {
            out.push_str(&format!("  [E] {}\n", truncate_str(l, 220)));
            found = true;
        }
    }
    if !found {
        return None;
    }
    let low = txt.to_lowercase();
    if has_any(&txt, &["OutOfMemoryError"]) {
        out.push_str("  [W] 判定：内存不足崩溃\n");
        out.push_str("  [S] 建议：调高 -Xmx / 减少模组 / 检查泄漏（如区块卸载异常）。\n");
    }
    if has_any(&low, &["couldn't load mod", "missing dependency", "missing mod"]) {
        out.push_str("  [W] 判定：模组加载失败或缺少前置\n");
        out.push_str("  [S] 建议：按报告补装前置模组，或移除报错模组。\n");
    }
    Some(out)
}

fn has_any(text: &str, pats: &[&str]) -> bool {
    pats.iter().any(|p| text.contains(p))
}

/// 分析单个 hs_err_pid*.log（JVM 致命错误），返回摘要；无关键内容返回 None。
fn analyze_hs_err_file(path: &std::path::Path) -> Option<String> {
    let txt = std::fs::read_to_string(path).unwrap_or_default();
    if txt.trim().is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("[H] JVM hs_err 日志\n");
    out.push_str(&format!("  [I] 文件: {}\n", path.display()));
    let mut pf = String::new();
    let mut found = false;
    for line in txt.lines().take(80) {
        let l = line.trim();
        if l.starts_with("# Problematic frame:") {
            pf = l.to_string();
        }
        if l.starts_with("# ") && l.contains("Native frame") && pf.is_empty() {
            pf = l.to_string();
        }
        if l.starts_with("# There is insufficient memory") || l.contains("insufficient memory") {
            out.push_str("  [W] 判定：系统/Java 内存不足导致 JVM 直接崩溃\n");
            out.push_str("  [S] 建议：增大 -Xmx 与系统虚拟内存，检查是否同时运行了多个重型程序。\n");
        }
        if l.starts_with("siginfo:")
            || l.starts_with("# Problematic frame:")
            || l.starts_with("Current thread")
            || l.starts_with("# Native frames:")
        {
            out.push_str(&format!("  [E] {}\n", truncate_str(l, 220)));
            found = true;
        }
    }
    if !pf.is_empty() {
        out.push_str(&format!("  [E] {}\n", truncate_str(&pf, 220)));
        out.push_str("  [W] 判定：JVM 原生层崩溃（native crash）\n");
        out.push_str("  [S] 建议：优先升级显卡驱动 / 检查内存条与超频稳定性；若为 Java 启动器层错误，尝试更换 JVM 发行版。\n");
        found = true;
    }
    if !found {
        return None;
    }
    Some(out)
}

/// 分析单个崩溃来源（kind 与 rel 来自 collect_crash_sources）。
fn analyze_crash_single(dir: &std::path::Path, kind: u8, rel: &str) -> String {
    let path = dir.join(rel);
    let res = match kind {
        0 => analyze_latest_log_tail(dir),
        1 => analyze_crash_report_file(&path),
        2 => analyze_hs_err_file(&path),
        _ => None,
    };
    match res {
        Some(t) => format!("{}\n[I] —— 分析完成：以上判定基于日志模式匹配，仅供参考。", t.trim_end()),
        None => format!("[I] 未在 {} 中找到可分析的崩溃证据（无关键错误行/堆栈）。\n[I] 若服务器仍在运行，请先停止后重试。", rel),
    }
}

/// 崩溃报告离线分析：读取 latest.log 尾部、crash-reports 最新报告、hs_err_pid 日志，
/// 用模式匹配定位崩溃原因并给出建议（参考 PCL 的日志/报告解析思路）。
fn analyze_crash_reports(dir: &std::path::Path) -> String {
    let mut out = String::new();
    let mut found_any = false;

    // 1) latest.log 尾部
    if let Some(t) = analyze_latest_log_tail(dir) {
        found_any = true;
        out.push_str(&t);
    }

    // 2) crash-reports 最新报告（最多 3 份）
    let cr_dir = dir.join("crash-reports");
    if cr_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&cr_dir) {
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".txt"))
                .collect();
            files.sort_by_key(|e| {
                e.metadata().and_then(|m| m.modified()).map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                }).unwrap_or(0)
            });
            files.reverse();
            for f in files.into_iter().take(3) {
                if let Some(t) = analyze_crash_report_file(&f.path()) {
                    found_any = true;
                    out.push_str(&format!("\n{}", t.trim_end()));
                }
            }
        }
    }

    // 3) hs_err_pid*（JVM 致命错误，最多 3 份）
    let mut hs_paths: Vec<std::path::PathBuf> = Vec::new();
    for base in [dir.to_path_buf(), dir.join("logs")] {
        if let Ok(entries) = std::fs::read_dir(&base) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("hs_err_pid") && name.ends_with(".log") {
                    hs_paths.push(e.path());
                }
            }
        }
    }
    hs_paths.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).map(|t| {
        t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }).unwrap_or(0));
    hs_paths.reverse();
    for p in hs_paths.into_iter().take(3) {
        if let Some(t) = analyze_hs_err_file(&p) {
            found_any = true;
            out.push_str(&format!("\n{}", t.trim_end()));
        }
    }

    if !found_any {
        out.push_str("[I] 未在服务器目录下找到可分析的崩溃证据（latest.log / crash-reports / hs_err_pid 均无内容）。\n");
        out.push_str("[I] 若服务器仍在运行，请先停止后重试；或确认目录正确（应指向服务器根目录，含 logs/ 与 mods/）。");
    } else {
        out.push_str("\n[I] —— 分析完成：以上判定基于日志模式匹配，仅供参考；如为模组兼容问题，建议核对各模组对应版本的兼容性。");
    }
    out
}


/// 截断长文本到指定字符数（按 char 安全截断）
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}
/// 按行首标记着色渲染崩溃分析文本：[H]标题 [E]错误/堆栈 [W]判定 [S]建议 [M]模组 [I]信息。
fn render_analysis_text(ui: &mut egui::Ui, text: &str) {
    let light = !ui.visuals().dark_mode;
    for line in text.lines() {
        let t = line.trim_start();
        if t.is_empty() {
            ui.add_space(2.0);
            continue;
        }
        let (body, color, strong, size) = if let Some(rest) = t.strip_prefix("[H] ") {
            (rest, Color32::from_rgb(255, 200, 100), true, 14.0)
        } else if let Some(rest) = t.strip_prefix("[E] ") {
            (rest, Color32::from_rgb(255, 110, 110), false, 12.0)
        } else if let Some(rest) = t.strip_prefix("[W] ") {
            (rest, Color32::from_rgb(255, 200, 80), false, 12.0)
        } else if let Some(rest) = t.strip_prefix("[S] ") {
            (rest, Color32::from_rgb(140, 220, 140), false, 12.0)
        } else if let Some(rest) = t.strip_prefix("[M] ") {
            (rest, Color32::from_rgb(130, 190, 255), false, 12.0)
        } else if let Some(rest) = t.strip_prefix("[I] ") {
            (rest, Color32::from_rgb(165, 165, 165), false, 12.0)
        } else {
            (t, Color32::from_rgb(205, 205, 205), false, 12.0)
        };
        let color = if light { theme::light_adapt(color) } else { color };
        let mut rt = RichText::new(body).color(color).monospace().size(size);
        if strong {
            rt = rt.strong();
        }
        ui.label(rt);
    }
}