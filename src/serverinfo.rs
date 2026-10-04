//! 服务端平台识别：从服务器目录里的文件推断「模组加载器 / 插件加载器 / 混合端 / 原版」以及 MC 版本。
//!
//! 为什么要做：下载模组时必须先知道目标服务端能吃什么（Fabric 模组 / Forge 模组 / 插件），
//! 以及它的 MC 版本；人工选择容易错。这里全部靠**文件证据**推断，并把证据一并返回，
//! 便于在「概览」里展示、也便于用户判断识别是否正确。
//!
//! 识别优先级（从强到弱）：
//! 1. `libraries/` 下的加载器目录（Fabric/Quilt/Forge/NeoForge）—— 加载器安装后的强特征
//! 2. 启动器文件名（fabric-server-launch.jar、forge-*-universal.jar、paper-*.jar …）
//! 3. 目录里的 `*.jar` 名称关键字（含混合端：mohist / magma / arclight / catserver …）
//! 4. `server.jar` 内部 `version.json`（原版与 Paper 系都带），拿到 MC 版本
//! 5. `logs/latest.log` 首部的 "Starting minecraft server version X"（兜底）
//! 6. `mods/` / `plugins/` 目录是否存在（弱证据，用于区分模组端/插件端）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 服务端平台类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    Paper,
    Spigot,
    Bukkit,
    Purpur,
    Hybrid,
    Unknown,
}

impl PlatformKind {
    pub fn label(self) -> &'static str {
        match self {
            PlatformKind::Vanilla => "原版（Vanilla）",
            PlatformKind::Fabric => "Fabric",
            PlatformKind::Quilt => "Quilt",
            PlatformKind::Forge => "Forge",
            PlatformKind::NeoForge => "NeoForge",
            PlatformKind::Paper => "Paper",
            PlatformKind::Spigot => "Spigot",
            PlatformKind::Bukkit => "CraftBukkit",
            PlatformKind::Purpur => "Purpur",
            PlatformKind::Hybrid => "混合端（模组+插件）",
            PlatformKind::Unknown => "未识别",
        }
    }

    /// 是否为模组加载器（能装模组）。
    pub fn is_modded(self) -> bool {
        matches!(
            self,
            PlatformKind::Fabric
                | PlatformKind::Quilt
                | PlatformKind::Forge
                | PlatformKind::NeoForge
                | PlatformKind::Hybrid
        )
    }

    /// 是否支持 Bukkit 系插件。
    pub fn is_plugin_capable(self) -> bool {
        matches!(
            self,
            PlatformKind::Paper
                | PlatformKind::Spigot
                | PlatformKind::Bukkit
                | PlatformKind::Purpur
                | PlatformKind::Hybrid
        )
    }

    /// 下载页「加载器」下拉框对应的值（`modrinth::loader_options()` 里的写法）。
    pub fn modrinth_loader(self) -> Option<&'static str> {
        match self {
            PlatformKind::Fabric => Some("fabric"),
            PlatformKind::Quilt => Some("quilt"),
            PlatformKind::Forge => Some("forge"),
            PlatformKind::NeoForge => Some("neoforge"),
            PlatformKind::Hybrid => Some("fabric"), // 混合端优先按 Fabric（多数为 Fabric+Paper 组合）
            _ => None,
        }
    }
}

/// 识别结果。
#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub kind: PlatformKind,
    /// MC 版本（如 "1.21.1"），拿不到则 None
    pub mc_version: Option<String>,
    /// 加载器自身版本（Forge/NeoForge/Fabric 能拿到就填）
    pub loader_version: Option<String>,
    /// 主 jar 文件名
    pub main_jar: Option<String>,
    /// 识别依据（展示给用户）
    pub evidence: Vec<String>,
    /// mods 目录里的 jar 数量
    pub mod_count: usize,
    /// plugins 目录里的 jar 数量
    pub plugin_count: usize,
}

impl PlatformInfo {
    pub fn summary(&self) -> String {
        match &self.mc_version {
            Some(v) => format!("{} · MC {}", self.kind.label(), v),
            None => self.kind.label().to_string(),
        }
    }
}

/// 识别结果的别名（对外统一叫法）：与 `PlatformInfo` 是同一个类型。
pub type ServerInfo = PlatformInfo;

/// 从 jar 文件里读 `version.json` 的 `id`/`name`（原版与 Paper 系服务端 jar 都带）。
fn mc_version_from_jar(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let mut entry = zip.by_name("version.json").ok()?;
    let mut s = String::new();
    use std::io::Read;
    entry.read_to_string(&mut s).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    for k in ["id", "name", "release_target"] {
        if let Some(x) = v.get(k).and_then(|x| x.as_str()) {
            if !x.is_empty() {
                return Some(x.to_string());
            }
        }
    }
    None
}

/// jar 里是否含某个路径（用于判断 Fabric/Forge 打进去的启动类）。
fn jar_contains(path: &Path, needle: &str) -> bool {
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(mut zip) = zip::ZipArchive::new(f) else {
        return false;
    };
    (0..zip.len()).any(|i| {
        zip.by_index(i)
            .map(|e| e.name().contains(needle))
            .unwrap_or(false)
    })
}

/// 扫描目录并识别平台。
pub fn detect(dir: &Path) -> PlatformInfo {
    let mut evidence: Vec<String> = Vec::new();
    let mut kind = PlatformKind::Unknown;
    let mut mc_version: Option<String> = None;
    let mut loader_version: Option<String> = None;
    let mut main_jar: Option<String> = None;

    // ---- 1) libraries/ 下的加载器目录（最强的安装特征）----
    let libs = dir.join("libraries");
    if libs.is_dir() {
        let mut seen = |p: &str| libs.join(p).is_dir();
        if seen("net/fabricmc") {
            kind = PlatformKind::Fabric;
            evidence.push("libraries/net/fabricmc（Fabric 安装痕迹）".to_string());
        } else if seen("org/quiltmc") {
            kind = PlatformKind::Quilt;
            evidence.push("libraries/org/quiltmc".to_string());
        } else if seen("net/neoforged") {
            kind = PlatformKind::NeoForge;
            evidence.push("libraries/net/neoforged（NeoForge 安装痕迹）".to_string());
        } else if seen("net/minecraftforge") {
            kind = PlatformKind::Forge;
            evidence.push("libraries/net/minecraftforge（Forge 安装痕迹）".to_string());
        }
        // 加载器版本目录：libraries/net/fabricmc/fabric-loader/<ver>、
        // libraries/org/quiltmc/quilt-loader/<ver>、libraries/net/neoforged/neoforge/<ver>
        for (base, name) in [
            ("net/fabricmc/fabric-loader", "Fabric Loader"),
            ("org/quiltmc/quilt-loader", "Quilt Loader"),
            ("net/neoforged/neoforge", "NeoForge"),
            ("net/neoforged/loader", "NeoForge Loader"),
        ] {
            let p = libs.join(base);
            if let Ok(mut rd) = std::fs::read_dir(&p) {
                if let Some(Ok(e)) = rd.next() {
                    loader_version = Some(e.file_name().to_string_lossy().to_string());
                    evidence.push(format!("{name} {}", loader_version.clone().unwrap_or_default()));
                    break;
                }
            }
        }
        // Forge：libraries/net/minecraftforge/forge/<mcver>-<forgever> 同时给出两个版本
        let forge_p = libs.join("net/minecraftforge/forge");
        if let Ok(mut rd) = std::fs::read_dir(&forge_p) {
            if let Some(Ok(e)) = rd.next() {
                let v = e.file_name().to_string_lossy().to_string();
                let mut it = v.splitn(2, '-');
                let a = it.next().unwrap_or("").to_string();
                if let Some(bv) = it.next() {
                    mc_version = Some(a.clone());
                    loader_version = Some(bv.to_string());
                } else {
                    loader_version = Some(a.clone());
                }
                evidence.push(format!("forge 版本目录 {v}"));
            }
        }
        // 最可靠的 MC 版本来源：libraries/net/minecraft/server/<版本>/（Fabric/Quilt 安装必带）
        let srv_p = libs.join("net/minecraft/server");
        if let Ok(mut rd) = std::fs::read_dir(&srv_p) {
            if let Some(Ok(e)) = rd.next() {
                let v = e.file_name().to_string_lossy().to_string();
                if v.starts_with('1') {
                    evidence.push(format!("libraries/net/minecraft/server/{v}"));
                    mc_version = Some(v);
                }
            }
        }
    }

    // ---- 2) 目录里的 jar：启动器 / 服务端核心 ----
    let mut jars: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.to_lowercase().ends_with(".jar") {
                jars.push(name);
            }
        }
    }
    jars.sort();
    let name_matches = |k: &[&str]| -> Option<String> {
        jars.iter()
            .find(|j| {
                let lj = j.to_lowercase();
                k.iter().any(|x| lj.contains(x))
            })
            .cloned()
    };

    // 混合端关键字（模组+插件）
    if let Some(h) = name_matches(&["mohist", "magma", "arclight", "catserver", "youer", "banner"]) {
        kind = PlatformKind::Hybrid;
        evidence.push(format!("混合端核心 {h}"));
        main_jar = Some(h);
    } else if let Some(f) = name_matches(&["fabric-server-launch", "fabric-server-launcher"]) {
        if kind == PlatformKind::Unknown {
            kind = PlatformKind::Fabric;
        }
        evidence.push(format!("Fabric 启动器 {f}"));
        main_jar = Some(f);
    } else if let Some(q) = name_matches(&["quilt-server-launch"]) {
        if kind == PlatformKind::Unknown {
            kind = PlatformKind::Quilt;
        }
        evidence.push(format!("Quilt 启动器 {q}"));
        main_jar = Some(q);
    } else if let Some(p) = name_matches(&["purpur"]) {
        kind = PlatformKind::Purpur;
        evidence.push(format!("Purpur 核心 {p}"));
        main_jar = Some(p);
    } else if let Some(p) = name_matches(&["paper"]) {
        if kind == PlatformKind::Unknown {
            kind = PlatformKind::Paper;
        }
        evidence.push(format!("Paper 核心 {p}"));
        main_jar = Some(p);
    } else if let Some(s) = name_matches(&["spigot"]) {
        if kind == PlatformKind::Unknown {
            kind = PlatformKind::Spigot;
        }
        evidence.push(format!("Spigot 核心 {s}"));
        main_jar = Some(s);
    } else if let Some(b) = name_matches(&["craftbukkit", "bukkit"]) {
        if kind == PlatformKind::Unknown {
            kind = PlatformKind::Bukkit;
        }
        evidence.push(format!("CraftBukkit 核心 {b}"));
        main_jar = Some(b);
    } else if let Some(f) = name_matches(&["forge"]) {
        if kind == PlatformKind::Unknown {
            kind = PlatformKind::Forge;
        }
        evidence.push(format!("Forge 核心 {f}"));
        main_jar = Some(f);
    } else if let Some(s) = name_matches(&["server.jar", "minecraft_server"]) {
        main_jar = Some(s.clone());
        evidence.push(format!("服务端核心 {s}"));
    }

    // 从文件名里抠 MC 版本（paper-1.21.1-133.jar / forge-1.20.1-47.2.0.jar 等）
    if mc_version.is_none() {
        let re_like: Vec<&str> = vec!["1.7", "1.8", "1.9", "1.10", "1.11", "1.12", "1.13", "1.14", "1.15",
            "1.16", "1.17", "1.18", "1.19", "1.20", "1.21", "1.22"];
        for j in &jars {
            if let Some(v) = find_mc_version_in_name(j, &re_like) {
                mc_version = Some(v);
                break;
            }
        }
    }

    // ---- 3) 主 jar 内 version.json（原版/Paper 都带）----
    if let Some(j) = &main_jar {
        let p = dir.join(j);
        if mc_version.is_none() {
            if let Some(v) = mc_version_from_jar(&p) {
                evidence.push(format!("{j} 内 version.json → {v}"));
                mc_version = Some(v);
            }
        }
        // 原版判定：server.jar 且不含任何加载器类
        if kind == PlatformKind::Unknown {
            let has_fabric = jar_contains(&p, "net/fabricmc/loader");
            let has_forge = jar_contains(&p, "net/minecraftforge/");
            if has_fabric {
                kind = PlatformKind::Fabric;
                evidence.push(format!("{j} 内含 net/fabricmc/loader"));
            } else if has_forge {
                kind = PlatformKind::Forge;
                evidence.push(format!("{j} 内含 net/minecraftforge/"));
            } else {
                kind = PlatformKind::Vanilla;
                evidence.push(format!("{j} 内未发现加载器类 → 判定原版"));
            }
        }
    } else if kind == PlatformKind::Unknown {
        // 没有任何 jar：看 mods/plugins 目录给一个弱判断
        if dir.join("mods").is_dir() {
            kind = PlatformKind::Unknown;
            evidence.push("存在 mods/ 但未找到核心 jar".to_string());
        }
    }

    // ---- 4) logs/latest.log 兜底：版本 + 加载器 ----
    // 各服务端的日志格式完全不同，这里按实际格式分别匹配（实机样本见注释）：
    //   原版/Paper:  "Starting minecraft server version 1.21.1"
    //   Fabric:      "Loading Minecraft 1.21.11 with Fabric Loader 0.19.5"
    //   NeoForge:    "Loading Minecraft 1.21.1 with NeoForge 21.1.72"
    //   Forge:       "Forge Mod Loader version 47.2.0 for Minecraft 1.20.1"
    if mc_version.is_none() || loader_version.is_none() {
        let log = dir.join("logs").join("latest.log");
        if let Ok(s) = std::fs::read_to_string(&log) {
            for line in s.lines().take(600) {
                if mc_version.is_none() {
                    if let Some(v) = mc_version_from_log_line(line) {
                        evidence.push(format!("latest.log → MC {v}"));
                        mc_version = Some(v);
                    }
                }
                if loader_version.is_none() {
                    if let Some(lv) = loader_version_from_log_line(line) {
                        evidence.push(format!("latest.log → {lv}"));
                        loader_version = Some(lv);
                    }
                }
                if mc_version.is_some() && loader_version.is_some() {
                    break;
                }
            }
            if kind == PlatformKind::Unknown {
                let low = s.to_lowercase();
                if low.contains("fabric") {
                    kind = PlatformKind::Fabric;
                    evidence.push("latest.log 提到 fabric".to_string());
                } else if low.contains("neoforge") {
                    kind = PlatformKind::NeoForge;
                    evidence.push("latest.log 提到 neoforge".to_string());
                } else if low.contains("forge") {
                    kind = PlatformKind::Forge;
                    evidence.push("latest.log 提到 forge".to_string());
                } else if low.contains("paper") {
                    kind = PlatformKind::Paper;
                    evidence.push("latest.log 提到 paper".to_string());
                }
            }
        }
    }

    // ---- 4b) versions/<mcver>/（启动器式安装目录）----
    if mc_version.is_none() {
        if let Ok(rd) = std::fs::read_dir(dir.join("versions")) {
            for e in rd.flatten() {
                let v = e.file_name().to_string_lossy().to_string();
                if v.starts_with('1') && v.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    evidence.push(format!("versions/{v}"));
                    mc_version = Some(v);
                    break;
                }
            }
        }
    }

    // ---- 5) mods/ plugins/ 计数 ----
    let count_jars = |sub: &str| -> usize {
        std::fs::read_dir(dir.join(sub))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .to_lowercase()
                            .ends_with(".jar")
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    let mod_count = count_jars("mods");
    let plugin_count = count_jars("plugins");
    if mod_count > 0 {
        evidence.push(format!("mods/ 下 {mod_count} 个 jar"));
    }
    if plugin_count > 0 {
        evidence.push(format!("plugins/ 下 {plugin_count} 个 jar"));
    }
    // 弱证据修正：有 mods 却在原版判定 → 说明可能只是放了 jar；有 plugins 且是原版/未知 → 插件端
    if kind == PlatformKind::Vanilla && plugin_count > 0 {
        evidence.push("原版核心但有 plugins/（需 Paper/Spigot 才能加载）".to_string());
    }

    PlatformInfo {
        kind,
        mc_version,
        loader_version,
        main_jar,
        evidence,
        mod_count,
        plugin_count,
    }
}

/// 版本号提取：从 start 起取连续的 ASCII 数字与 `.`（先去掉前导空白）。
fn digits_dotted(s: &str, start: usize) -> String {
    s.get(start..)
        .unwrap_or("")
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect()
}

/// 从一行服务端日志里取 MC 版本（原版/Paper 与 Fabric/NeoForge 两种格式）。
///
/// 查找结果的字节下标只用于 `get(..)`，needle 长度取自 `len()`，行被截断也不越界。
fn mc_version_from_log_line(line: &str) -> Option<String> {
    const VANILLA: &str = "Starting minecraft server version";
    const LOADING: &str = "Loading Minecraft ";
    if let Some(idx) = line.find(VANILLA) {
        let v = digits_dotted(line, idx + VANILLA.len());
        if !v.is_empty() {
            return Some(v);
        }
    }
    if let Some(idx) = line.find(LOADING) {
        let v = digits_dotted(line, idx + LOADING.len());
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

/// 从一行服务端日志里取加载器版本（Fabric/Quilt/NeoForge/Forge 四种前缀）。
///
/// 前三种只出现在 "Loading Minecraft " 行内；Forge 的独立行也可能出现。
fn loader_version_from_log_line(line: &str) -> Option<String> {
    const LOADING: &str = "Loading Minecraft ";
    const FORGE_OLD: &str = "Forge Mod Loader version ";
    if let Some(idx) = line.find(LOADING) {
        let after = line.get(idx + LOADING.len()..).unwrap_or("");
        for key in ["Fabric Loader ", "NeoForge ", "Quilt Loader "] {
            if let Some(i) = after.find(key) {
                let lv = digits_dotted(after, i + key.len());
                if !lv.is_empty() {
                    return Some(format!("{key}{lv}"));
                }
            }
        }
    }
    if let Some(i) = line.find(FORGE_OLD) {
        let lv = digits_dotted(line, i + FORGE_OLD.len());
        if !lv.is_empty() {
            return Some(format!("{FORGE_OLD}{lv}"));
        }
    }
    None
}

// ---------- 缓存层（渲染路径每帧调用 detect() 会反复读盘，这里做记忆化） ----------

/// detect 缓存：规范化目录 → (写入时刻, 识别结果)
static DETECT_CACHE: OnceLock<Mutex<HashMap<PathBuf, (Instant, PlatformInfo)>>> = OnceLock::new();
/// mods 元数据缓存：规范化目录 → (目录指纹, 写入时刻, 清单)
static MOD_META_CACHE: OnceLock<Mutex<HashMap<PathBuf, ModMetaCacheEntry>>> = OnceLock::new();
/// 单个缓存的条目上限：超出后整表清空（避免服务器很多时无限增长）
const CACHE_MAX_ENTRIES: usize = 64;
/// mods 元数据最长复用时间（即使指纹没变也定期重扫一次，避免极端的"指纹巧合"）
const MOD_META_TTL: Duration = Duration::from_secs(24 * 60 * 60);

fn detect_cache() -> &'static Mutex<HashMap<PathBuf, (Instant, PlatformInfo)>> {
    DETECT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mod_meta_cache() -> &'static Mutex<HashMap<PathBuf, ModMetaCacheEntry>> {
    MOD_META_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 缓存键：规范化的绝对路径，并整体转小写（Windows 路径不区分大小写；
/// 相对路径 / `..` / 短名 8.3 会让同一目录算出不同的键，缓存就形同虚设）。
/// 取不到 canonicalize（目录刚被删除等）时退化成"绝对化 + 小写"。
fn normalize_cache_key(dir: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| {
        if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|c| c.join(dir))
                .unwrap_or_else(|_| dir.to_path_buf())
        }
    });
    PathBuf::from(abs.to_string_lossy().to_lowercase())
}

/// 带 TTL 记忆化的 `detect`：同一目录在 `ttl` 内直接返回缓存（`ttl` 为 0 时等价于强制刷新）。
/// 签名与既有 `detect` 无关，`detect` 行为完全不变。建议 UI 用 3～5 秒。
pub fn detect_cached(dir: &Path, ttl: Duration) -> ServerInfo {
    let key = normalize_cache_key(dir);
    if ttl > Duration::ZERO {
        let cache = detect_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, info)) = cache.get(&key) {
            if at.elapsed() < ttl {
                return info.clone();
            }
        }
    }
    let info = detect(dir);
    {
        let mut cache = detect_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_MAX_ENTRIES && !cache.contains_key(&key) {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), info.clone()));
    }
    info
}

/// 主动失效某个目录的缓存（用户改动目录内容 / 重命名 / 换核心后调用）。
/// 该目录的识别结果与 mods 元数据缓存一起失效。
pub fn invalidate_cache(dir: &Path) {
    let key = normalize_cache_key(dir);
    detect_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
    mod_meta_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
}

/// 清空全部缓存（切换服务器 / 手动刷新时可用）。
pub fn clear_cache() {
    detect_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    mod_meta_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

// ---------- mods/*.jar 元数据扫描（逐个开 jar 是重活，按目录指纹缓存） ----------

/// 单个模组 jar 的元数据。
#[derive(Debug, Clone, Default)]
pub struct ModMeta {
    /// jar 文件名（含扩展名）
    pub file_name: String,
    /// 模组 id（fabric.mod.json 的 id / mods.toml 的 modId）
    pub mod_id: Option<String>,
    /// 显示名（fabric.mod.json 的 name / mods.toml 的 displayName）
    pub name: Option<String>,
    /// 模组版本
    pub version: Option<String>,
    /// 元数据来源：fabric / quilt / forge / neoforge
    pub loader: Option<String>,
}

impl ModMeta {
    /// 兼容"别名列表"用法（modid + 显示名，去重、去空），便于按名字匹配日志/报告。
    pub fn aliases(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for v in [&self.mod_id, &self.name] {
            if let Some(s) = v {
                if !s.is_empty() && !out.contains(s) {
                    out.push(s.clone());
                }
            }
        }
        out
    }
}

struct ModMetaCacheEntry {
    /// 目录指纹：(jar 数量, jar 总字节, 最新 mtime 纳秒)
    fingerprint: (usize, u64, i64),
    at: Instant,
    list: Vec<ModMeta>,
}

/// 元数据文本清洗：空 / 形如 `${version}` `%version%` 的未替换占位符一律当作"没有"。
fn clean_meta_value(v: Option<&str>) -> Option<String> {
    let s = v?.trim();
    if s.is_empty() || s.contains("${") || s.starts_with('%') {
        return None;
    }
    Some(s.to_string())
}

/// 目录指纹：只做一次 read_dir + metadata，比逐个开 jar 便宜得多。
fn mods_fingerprint(dir: &Path) -> (usize, u64, i64) {
    let mut count = 0usize;
    let mut total = 0u64;
    let mut newest = 0i64;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, 0, 0);
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.to_lowercase().ends_with(".jar") {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        count += 1;
        total = total.saturating_add(md.len());
        if let Ok(mtime) = md.modified() {
            if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                let ns = d.as_nanos().min(i64::MAX as u128) as i64;
                if ns > newest {
                    newest = ns;
                }
            }
        }
    }
    (count, total, newest)
}

/// 读单个 jar 的元数据；损坏 / 无元数据的 jar 只返回文件名，绝不 panic。
fn read_mod_meta(path: &Path, file_name: String) -> ModMeta {
    let mut meta = ModMeta {
        file_name,
        ..Default::default()
    };
    let Ok(file) = std::fs::File::open(path) else {
        return meta;
    };
    let Ok(mut z) = zip::ZipArchive::new(file) else {
        return meta;
    };
    // ---- Fabric / Quilt：fabric.mod.json / quilt.mod.json ----
    for (entry, loader) in [("fabric.mod.json", "fabric"), ("quilt.mod.json", "quilt")] {
        let Ok(mut f) = z.by_name(entry) else { continue };
        let mut s = String::new();
        if std::io::Read::read_to_string(&mut f, &mut s).is_err() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
            continue;
        };
        let q = v.get("quilt_loader");
        meta.mod_id = clean_meta_value(
            v.get("id")
                .and_then(|x| x.as_str())
                .or_else(|| q.and_then(|q| q.get("id")).and_then(|x| x.as_str())),
        );
        meta.name = clean_meta_value(
            v.get("name").and_then(|x| x.as_str()).or_else(|| {
                q.and_then(|q| q.get("metadata"))
                    .and_then(|m| m.get("name"))
                    .and_then(|x| x.as_str())
            }),
        );
        meta.version = clean_meta_value(
            v.get("version")
                .and_then(|x| x.as_str())
                .or_else(|| q.and_then(|q| q.get("version")).and_then(|x| x.as_str())),
        );
        meta.loader = Some(loader.to_string());
        break;
    }
    // ---- Forge / NeoForge：META-INF/mods.toml（取第一条 [[mods]]） ----
    if meta.mod_id.is_none() {
        for (entry, loader) in [
            ("META-INF/neoforge.mods.toml", "neoforge"),
            ("META-INF/mods.toml", "forge"),
        ] {
            let Ok(mut f) = z.by_name(entry) else { continue };
            let mut s = String::new();
            if std::io::Read::read_to_string(&mut f, &mut s).is_err() {
                continue;
            }
            if let Ok(v) = toml::from_str::<toml::Value>(&s) {
                if let Some(m) = v
                    .get("mods")
                    .and_then(|m| m.as_array())
                    .and_then(|a| a.first())
                {
                    meta.mod_id = clean_meta_value(m.get("modId").and_then(|x| x.as_str()));
                    meta.name = clean_meta_value(m.get("displayName").and_then(|x| x.as_str()));
                    meta.version = clean_meta_value(m.get("version").and_then(|x| x.as_str()));
                }
            }
            meta.loader = Some(loader.to_string());
            break;
        }
    }
    // ---- 兜底：MANIFEST.MF 的 Implementation-Version ----
    if meta.version.is_none() {
        if let Ok(mut f) = z.by_name("META-INF/MANIFEST.MF") {
            let mut s = String::new();
            if std::io::Read::read_to_string(&mut f, &mut s).is_ok() {
                for line in s.lines() {
                    if let Some(v) = line.strip_prefix("Implementation-Version:") {
                        meta.version = clean_meta_value(Some(v));
                        break;
                    }
                }
            }
        }
    }
    meta
}

/// 真正扫描 mods 目录（不查缓存）：只认 `.jar`，按文件名排序。
fn scan_mod_metadata_uncached(dir: &Path) -> Vec<ModMeta> {
    let mut out: Vec<ModMeta> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.to_lowercase().ends_with(".jar") {
            continue;
        }
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        out.push(read_mod_meta(&p, name));
    }
    out.sort_by(|a, b| {
        a.file_name
            .to_lowercase()
            .cmp(&b.file_name.to_lowercase())
    });
    out
}

/// 扫描 mods 目录内的 jar，读取 fabric.mod.json / mods.toml / META-INF 元数据（modid、名称、版本）。
/// 结果按 (目录 + 目录内文件总大小/mtime 指纹) 缓存，避免每次重开所有 jar：
/// 指纹不变直接复用，指纹变了立即重扫；即使指纹没变，超过 24 小时也会重扫一次。
/// 损坏的 jar 会被跳过（只保留文件名），不会 panic。
pub fn scan_mod_metadata(dir: &Path) -> Vec<ModMeta> {
    let key = normalize_cache_key(dir);
    let fp = mods_fingerprint(dir);
    {
        let cache = mod_meta_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&key) {
            if entry.fingerprint == fp && entry.at.elapsed() < MOD_META_TTL {
                return entry.list.clone();
            }
        }
    }
    let list = scan_mod_metadata_uncached(dir);
    {
        let mut cache = mod_meta_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_MAX_ENTRIES && !cache.contains_key(&key) {
            cache.clear();
        }
        cache.insert(
            key,
            ModMetaCacheEntry {
                fingerprint: fp,
                at: Instant::now(),
                list: list.clone(),
            },
        );
    }
    list
}

/// 从文件名里找形如 1.21 / 1.21.1 的 MC 版本号。
fn find_mc_version_in_name(name: &str, prefixes: &[&str]) -> Option<String> {
    let bytes: Vec<char> = name.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c != '1' || i + 1 >= bytes.len() || bytes[i + 1] != '.' {
            continue;
        }
        // 收集 1.x[.y]
        let mut v = String::from("1.");
        let mut j = i + 2;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            v.push(bytes[j]);
            j += 1;
        }
        if j < bytes.len() && bytes[j] == '.' {
            let mut k = j + 1;
            let mut patch = String::new();
            while k < bytes.len() && bytes[k].is_ascii_digit() {
                patch.push(bytes[k]);
                k += 1;
            }
            if !patch.is_empty() {
                v.push('.');
                v.push_str(&patch);
            }
        }
        if prefixes.iter().any(|p| v.starts_with(p)) {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_name() {
        let p = ["1.7", "1.8", "1.9", "1.10", "1.11", "1.12", "1.13", "1.14", "1.15", "1.16",
            "1.17", "1.18", "1.19", "1.20", "1.21", "1.22"];
        assert_eq!(
            find_mc_version_in_name("paper-1.21.1-133.jar", &p).as_deref(),
            Some("1.21.1")
        );
        assert_eq!(
            find_mc_version_in_name("forge-1.20.1-47.2.0.jar", &p).as_deref(),
            Some("1.20.1")
        );
        assert_eq!(find_mc_version_in_name("server.jar", &p), None);
    }

    #[test]
    fn loader_mapping() {
        assert_eq!(PlatformKind::Fabric.modrinth_loader(), Some("fabric"));
        assert_eq!(PlatformKind::NeoForge.modrinth_loader(), Some("neoforge"));
        assert_eq!(PlatformKind::Vanilla.modrinth_loader(), None);
        assert!(!PlatformKind::Vanilla.is_modded());
        assert!(PlatformKind::Hybrid.is_modded() && PlatformKind::Hybrid.is_plugin_capable());
    }

    /// 日志行截断在 needle 处：不得越界（needle 长度必须取自 len()）
    #[test]
    fn log_line_truncated_at_needle() {
        // needle 紧贴行尾
        assert_eq!(
            mc_version_from_log_line("Starting minecraft server version"),
            None
        );
        assert_eq!(loader_version_from_log_line("Forge Mod Loader version "), None);
        assert_eq!(mc_version_from_log_line("Loading Minecraft "), None);
        assert_eq!(loader_version_from_log_line("Fabric Loader "), None);
        // needle 后只剩空白/半个版本号
        assert_eq!(
            mc_version_from_log_line("[12:00:00] [Server thread/INFO]: Starting minecraft server version   "),
            None
        );
        assert_eq!(
            loader_version_from_log_line("Loading Minecraft 1.21.1 with NeoForge "),
            None
        );
    }

    /// 正常行与紧贴行尾的完整版本号都能取到
    #[test]
    fn log_line_version_extract() {
        assert_eq!(
            mc_version_from_log_line("Starting minecraft server version 1.21.1").as_deref(),
            Some("1.21.1")
        );
        assert_eq!(
            mc_version_from_log_line("Starting minecraft server version 1.20.1\n").as_deref(),
            Some("1.20.1")
        );
        assert_eq!(
            loader_version_from_log_line("Loading Minecraft 1.21.11 with Fabric Loader 0.19.5")
                .as_deref(),
            Some("Fabric Loader 0.19.5")
        );
        assert_eq!(
            loader_version_from_log_line("Forge Mod Loader version 47.2.0 for Minecraft 1.20.1")
                .as_deref(),
            Some("Forge Mod Loader version 47.2.0")
        );
        assert_eq!(
            loader_version_from_log_line("Loading Minecraft 1.21.1 with NeoForge 21.1.72")
                .as_deref(),
            Some("NeoForge 21.1.72")
        );
        // 非 ASCII 前缀不得 panic（按字符边界取）
        assert_eq!(mc_version_from_log_line("中文前缀 Starting minecraft server version 1.21.4").as_deref(), Some("1.21.4"));
        assert_eq!(mc_version_from_log_line("无关中文日志行"), None);
    }
}
