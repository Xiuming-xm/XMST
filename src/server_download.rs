//! B4 服务端下载：五大内置数据源（Vanilla / Paper / Fabric / Forge / NeoForge）。
//!
//! 数据源：
//! - Vanilla:  piston-meta.mojang.com version_manifest_v2（取 releases/snapshots，server jar 二次查询）
//! - Paper:    api.papermc.io v2（项目 paper → versions → 最新 build 的 application jar）
//! - Fabric:   meta.fabricmc.net v2（游戏版本 + 最新 loader/installer → server jar）
//! - Forge:    maven.minecraftforge.net（maven-metadata.xml，版本形如 1.20.1-47.3.0）
//! - NeoForge: maven.neoforged.net releases（同 maven-metadata.xml 结构）

use crate::download::{fetch_json, new_client};

/// 服务端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerKind {
    Vanilla,
    Paper,
    Fabric,
    Forge,
    NeoForge,
    Spigot,
    Bukkit,
}

impl ServerKind {
    pub fn label(&self) -> &'static str {
        match self {
            ServerKind::Vanilla => "Vanilla（原版）",
            ServerKind::Paper => "Paper",
            ServerKind::Fabric => "Fabric",
            ServerKind::Forge => "Forge",
            ServerKind::NeoForge => "NeoForge",
            ServerKind::Spigot => "Spigot",
            ServerKind::Bukkit => "Bukkit（CraftBukkit）",
        }
    }

    pub fn all() -> [ServerKind; 7] {
        [
            ServerKind::Vanilla,
            ServerKind::Paper,
            ServerKind::Fabric,
            ServerKind::Forge,
            ServerKind::NeoForge,
            ServerKind::Spigot,
            ServerKind::Bukkit,
        ]
    }
}

/// 拉取某类型的版本列表（阻塞，须放入独立线程）
pub fn fetch_versions(kind: ServerKind) -> Result<Vec<String>, String> {
    let client = new_client();
    match kind {
        ServerKind::Vanilla => {
            let v = fetch_json(&client, "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")?;
            let mut out: Vec<String> = Vec::new();
            if let Some(arr) = v["versions"].as_array() {
                for it in arr {
                    if let (Some(id), Some(ty)) = (it["id"].as_str(), it["type"].as_str()) {
                        if ty == "release" || ty == "snapshot" {
                            out.push(id.to_string());
                        }
                    }
                }
            }
            if out.is_empty() {
                return Err("Vanilla 版本列表为空".to_string());
            }
            Ok(out)
        }
        ServerKind::Paper => {
            // PaperMC 2025 年起 API 迁移至 fill.papermc.io/v3：
            // versions 为按主版本分组的 map，扁平化后过滤 rc/pre 快照，使正式版在前
            let v = fetch_json(&client, "https://fill.papermc.io/v3/projects/paper")?;
            let mut out: Vec<String> = Vec::new();
            if let Some(map) = v["versions"].as_object() {
                for list in map.values() {
                    if let Some(arr) = list.as_array() {
                        for x in arr {
                            if let Some(s) = x.as_str() {
                                if !s.contains("-rc") && !s.contains("-pre") && !s.contains("-snapshot") {
                                    if !out.iter().any(|e| e.as_str() == s) {
                                        out.push(s.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // 语义版本降序（新版在前）
            out.sort_by(|a, b| version_cmp(b, a));
            if out.is_empty() {
                return Err("Paper 版本列表为空".to_string());
            }
            Ok(out)
        }
        ServerKind::Fabric => {
            let v = fetch_json(&client, "https://meta.fabricmc.net/v2/versions/game")?;
            let mut out: Vec<String> = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x["version"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if out.is_empty() {
                return Err("Fabric 版本列表为空".to_string());
            }
            Ok(out)
        }
        ServerKind::Forge => {
            parse_maven_versions(&client, "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml")
        }
        ServerKind::NeoForge => {
            parse_maven_versions(&client, "https://maven.neoforged.net/releases/net/neoforged/forge/maven-metadata.xml")
        }
        ServerKind::Spigot | ServerKind::Bukkit => {
            // Spigot / Bukkit 版本与 MC 版本一致，直接复用 Mojang 版本清单
            let v = fetch_json(&client, "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")?;
            let mut out: Vec<String> = Vec::new();
            if let Some(arr) = v["versions"].as_array() {
                for it in arr {
                    if let (Some(id), Some(ty)) = (it["id"].as_str(), it["type"].as_str()) {
                        if ty == "release" {
                            out.push(id.to_string());
                        }
                    }
                }
            }
            out.reverse();
            if out.is_empty() {
                return Err("Spigot/Bukkit 版本列表为空".to_string());
            }
            Ok(out)
        }
    }
}

/// 解析 maven-metadata.xml 的 <version> 列表（轻量文本解析，不引入 XML 依赖）
fn parse_maven_versions(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<String>, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().map_err(|e| format!("读取响应失败: {e}"))?;
    let mut out: Vec<String> = Vec::new();
    for seg in text.split("<version>").skip(1) {
        if let Some(end) = seg.find("</version>") {
            let ver = seg[..end].trim().to_string();
            if !ver.is_empty() && !out.contains(&ver) {
                out.push(ver);
            }
        }
    }
    out.reverse();
    if out.is_empty() {
        return Err("Maven 版本列表为空".to_string());
    }
    Ok(out)
}

/// 解析最终 jar 下载地址（部分源需二次 API 查询）
pub fn resolve_download_url(kind: ServerKind, version: &str) -> Result<String, String> {
    let client = new_client();
    match kind {
        ServerKind::Vanilla => {
            let man = fetch_json(&client, "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")?;
            let mut version_url: Option<String> = None;
            if let Some(arr) = man["versions"].as_array() {
                for it in arr {
                    if it["id"].as_str() == Some(version) {
                        version_url = it["url"].as_str().map(String::from);
                        break;
                    }
                }
            }
            let version_url = version_url.ok_or_else(|| format!("版本清单中未找到 {version}"))?;
            let detail = fetch_json(&client, &version_url)?;
            detail["downloads"]["server"]["url"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| "该版本无服务端下载（可能为快照）".to_string())
        }
        ServerKind::Paper => {
            let v = fetch_json(
                &client,
                &format!("https://fill.papermc.io/v3/projects/paper/versions/{version}/builds"),
            )?;
            let builds = v.as_array().ok_or("builds 缺失")?;
            let b = builds.first().ok_or("无可用构建")?;
            let name = b["downloads"]["server:default"]["name"]
                .as_str()
                .ok_or("下载文件名缺失")?;
            let url = b["downloads"]["server:default"]["url"]
                .as_str()
                .ok_or("下载地址缺失")?;
            Ok(url.to_string())
        }
        ServerKind::Fabric => {
            let loaders = fetch_json(
                &client,
                &format!("https://meta.fabricmc.net/v2/versions/loader/{version}"),
            )?;
            let loader = loaders
                .as_array()
                .and_then(|a| a.first())
                .and_then(|x| x["loader"]["version"].as_str())
                .ok_or("Fabric loader 缺失")?;
            let installers = fetch_json(&client, "https://meta.fabricmc.net/v2/versions/installer")?;
            let installer = installers
                .as_array()
                .and_then(|a| a.first())
                .and_then(|x| x["version"].as_str())
                .ok_or("Fabric installer 缺失")?;
            Ok(format!(
                "https://meta.fabricmc.net/v2/versions/loader/{version}/{loader}/{installer}/server/jar"
            ))
        }
        ServerKind::Forge => Ok(format!(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/{version}/forge-{version}-installer.jar"
        )),
        ServerKind::NeoForge => Ok(format!(
            "https://maven.neoforged.net/releases/net/neoforged/forge/{version}/forge-{version}-installer.jar"
        )),
        ServerKind::Spigot => Ok(format!(
            "https://download.getbukkit.org/spigot/spigot-{version}.jar"
        )),
        ServerKind::Bukkit => Ok(format!(
            "https://download.getbukkit.org/craftbukkit/craftbukkit-{version}.jar"
        )),
    }
}

/// 语义版本比较：将 "1.21.11" / "1.20.1" 等按数字段比较（主版本可能一位或两位）
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|x| !x.is_empty())
            .filter_map(|x| x.parse::<u64>().ok())
            .collect()
    };
    let (pa, pb) = (parse(a), parse(b));
    for (x, y) in pa.iter().zip(pb.iter()) {
        let ord = x.cmp(y);
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    pa.len().cmp(&pb.len())
}

/// 下载产物文件名前缀（统一 {prefix}-{version}.jar）
pub fn file_prefix(kind: ServerKind) -> &'static str {
    match kind {
        ServerKind::Vanilla => "vanilla",
        ServerKind::Paper => "paper",
        ServerKind::Fabric => "fabric-server",
        ServerKind::Forge => "forge",
        ServerKind::NeoForge => "neoforge",
        ServerKind::Spigot => "spigot",
        ServerKind::Bukkit => "craftbukkit",
    }
}

/// 清理版本字符串中的非法文件名字符
pub fn sanitize_version(version: &str) -> String {
    let mut out = String::with_capacity(version.len());
    for ch in version.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}
