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

use std::path::Path;

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
        // ★ 最可靠的 MC 版本来源：libraries/net/minecraft/server/<版本>/（Fabric/Quilt 安装必带）
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
                    if let Some(idx) = line.find("Starting minecraft server version") {
                        let v: String = line[idx + 32..]
                            .trim()
                            .chars()
                            .take_while(|c| c.is_ascii_digit() || *c == '.')
                            .collect();
                        if !v.is_empty() {
                            evidence.push(format!("latest.log → MC {v}"));
                            mc_version = Some(v);
                        }
                    }
                }
                if line.contains("Loading Minecraft ") {
                    // "Loading Minecraft 1.21.11 with Fabric Loader 0.19.5"
                    let after = line.split("Loading Minecraft ").nth(1).unwrap_or("");
                    let ver: String = after
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    if !ver.is_empty() {
                        if mc_version.is_none() {
                            evidence.push(format!("latest.log → MC {ver}"));
                            mc_version = Some(ver);
                        }
                        if loader_version.is_none() {
                            for key in ["Fabric Loader ", "NeoForge ", "Quilt Loader "] {
                                if let Some(i) = after.find(key) {
                                    let lv: String = after[i + key.len()..]
                                        .chars()
                                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                                        .collect();
                                    if !lv.is_empty() {
                                        evidence.push(format!("latest.log → {key}{lv}"));
                                        loader_version = Some(lv);
                                    }
                                }
                            }
                        }
                    }
                }
                if loader_version.is_none() {
                    if let Some(i) = line.find("Forge Mod Loader version ") {
                        let lv: String = line[i + 24..]
                            .chars()
                            .take_while(|c| c.is_ascii_digit() || *c == '.')
                            .collect();
                        if !lv.is_empty() {
                            evidence.push(format!("latest.log → Forge {lv}"));
                            loader_version = Some(lv);
                        }
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
}
