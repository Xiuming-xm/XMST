//! Modrinth 模组下载/安装（api.modrinth.com/v2）。
//!
//! 流程：search（facets 按 MC 版本 + 加载器过滤）→ project/{id}/version
//! 取最新适配版本 files[].url → 下载 jar 直接落服务器 mods/ 目录。
//! 无需 API Key，搜索限 20 条，排序 downloads 降序。

use crate::download::{fetch_json, new_client};
use serde_json::Value;

/// Modrinth 搜索结果条目（UI 展示用）
#[derive(Debug, Clone, Default)]
pub struct ModrinthHit {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub downloads: i64,
    pub loaders: Vec<String>,
    pub mc_versions: Vec<String>,
    /// 项目图标 URL（图2 卡片图标，暂用首字母色块占位，字段先保留）
    pub icon_url: String,
    /// 作者
    pub author: String,
    /// 最后更新时间（ISO 8601）
    pub date_modified: String,
    /// 展示分类（不含加载器标签，图2 分类标签）
    pub categories: Vec<String>,
    /// 项目官网源码地址（GitHub 等，空=无；来自 project/{id} 详情）
    pub source_url: String,
}

/// 支持的加载器筛选项（Modrinth facet 取值）
pub fn loader_options() -> [&'static str; 5] {
    ["全部", "fabric", "forge", "neoforge", "quilt"]
}

/// 搜索模组（阻塞，须放入独立线程）
/// Search result with total hits for pagination.
#[derive(Debug, Clone, Default)]
pub struct SearchResult {
    pub hits: Vec<ModrinthHit>,
    pub total_hits: i64,
}

/// Search mods (blocking, run in a separate thread).
pub fn search_mods(
    query: &str,
    project_type: &str,
    mc_version: &str,
    loader: &str,
    tags: &str,
    index: &str,
    limit: i32,
    offset: i64,
) -> Result<SearchResult, String> {
    let client = new_client();
    let mut facets: Vec<String> = Vec::new();
    if !project_type.is_empty() {
        facets.push(format!("[\"project_type:{project_type}\"]"));
    }
    if !mc_version.is_empty() {
        facets.push(format!("[\"versions:{mc_version}\"]"));
    }
    if !loader.is_empty() && loader != "全部" {
        facets.push(format!("[\"categories:{loader}\"]"));
    }
    for t in tags.split(',') {
        let t = t.trim();
        if !t.is_empty() {
            facets.push(format!("[\"categories:{t}\"]"));
        }
    }
    let idx = match index {
        "downloads" | "follows" | "newest" | "updated" => index,
        _ => "relevance",
    };
    let limit = limit.clamp(1, 20);
    let offset = offset.max(0);
    let url = if facets.is_empty() {
        format!(
            "https://api.modrinth.com/v2/search?query={}&limit={}&offset={}&index={}",
            urlencode(query),
            limit,
            offset,
            idx
        )
    } else {
        format!(
            "https://api.modrinth.com/v2/search?query={}&limit={}&offset={}&index={}&facets={}",
            urlencode(query),
            limit,
            offset,
            idx,
            urlencode(&format!("[{}]", facets.join(",")))
        )
    };
    let v = fetch_json(&client, &url)?;
    let hits = v["hits"].as_array().ok_or("hits 缺失")?;
    let total_hits = v["total_hits"].as_i64().unwrap_or(hits.len() as i64);
    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let cats = h["display_categories"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        out.push(ModrinthHit {
            id: h["project_id"].as_str().unwrap_or_default().to_string(),
            slug: h["slug"].as_str().unwrap_or_default().to_string(),
            title: h["title"].as_str().unwrap_or_default().to_string(),
            description: h["description"].as_str().unwrap_or_default().to_string(),
            downloads: h["downloads"].as_i64().unwrap_or(0),
            loaders: h["categories"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            mc_versions: h["versions"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            icon_url: h["icon_url"].as_str().unwrap_or_default().to_string(),
            author: h["author"].as_str().unwrap_or_default().to_string(),
            date_modified: h["date_modified"].as_str().unwrap_or_default().to_string(),
            categories: cats,
            source_url: String::new(),
        });
    }
    Ok(SearchResult {
        hits: out,
        total_hits,
    })
}

/// 按 project_id 取项目详情（收藏夹展示用），字段与 search_mods 对齐
pub fn project_by_id(project_id: &str) -> Result<ModrinthHit, String> {
    let client = new_client();
    let v = fetch_json(
        &client,
        &format!("https://api.modrinth.com/v2/project/{project_id}"),
    )?;
    let loader_words = [
        "fabric", "forge", "neoforge", "quilt", "modloader", "bukkit", "spigot", "paper",
        "folia", "purpur", "bungeecord", "velocity",
    ];
    let cats = v["categories"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .filter(|c| !loader_words.contains(&c.as_str()))
                .collect()
        })
        .unwrap_or_default();
    Ok(ModrinthHit {
        id: project_id.to_string(),
        slug: v["slug"].as_str().unwrap_or_default().to_string(),
        title: v["title"].as_str().unwrap_or_default().to_string(),
        description: v["description"].as_str().unwrap_or_default().to_string(),
        downloads: v["downloads"].as_i64().unwrap_or(0),
        loaders: v["loaders"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        mc_versions: v["game_versions"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        icon_url: v["icon_url"].as_str().unwrap_or_default().to_string(),
        author: v["author"].as_str().unwrap_or_default().to_string(),
        date_modified: v["date_modified"].as_str().unwrap_or_default().to_string(),
        categories: cats,
        source_url: v["source_url"].as_str().unwrap_or_default().to_string(),
    })
}

/// Modrinth 项目版本文件项（project/{id}/version 内 files[].url / filename / size）
#[derive(Debug, Clone, Default)]
pub struct ModrinthVersionFile {
    pub url: String,
    pub filename: String,
    pub size: u64,
}

/// Modrinth 项目版本（project/{id}/version 返回项，详情页版本列表用）
#[derive(Debug, Clone, Default)]
pub struct ModrinthVersion {
    pub id: String,
    pub name: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub date_published: String,
    pub downloads: i64,
    /// release / beta / alpha
    pub channel: String,
    pub files: Vec<ModrinthVersionFile>,
    /// 更新日志（HTML，可为空）
    pub changelog: String,
}

/// 拉取项目全量版本列表（GET /v2/project/{id}/version，含 changelog）
pub fn project_versions(project_id: &str) -> Result<Vec<ModrinthVersion>, String> {
    let client = new_client();
    let v = fetch_json(
        &client,
        &format!("https://api.modrinth.com/v2/project/{project_id}/version?include_changelog=true"),
    )?;
    let arr = v.as_array().ok_or("version 响应异常")?;
    let mut out = Vec::with_capacity(arr.len());
    for ver in arr {
        let files = ver["files"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some(ModrinthVersionFile {
                            url: f["url"].as_str()?.to_string(),
                            filename: f["filename"].as_str()?.to_string(),
                            size: f["size"].as_u64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(ModrinthVersion {
            id: ver["id"].as_str().unwrap_or_default().to_string(),
            name: ver["name"].as_str().unwrap_or_default().to_string(),
            version_number: ver["version_number"].as_str().unwrap_or_default().to_string(),
            game_versions: ver["game_versions"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            loaders: ver["loaders"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            date_published: ver["date_published"].as_str().unwrap_or_default().to_string(),
            downloads: ver["downloads"].as_i64().unwrap_or(0),
            channel: ver["version_type"].as_str().unwrap_or_default().to_string(),
            changelog: ver["changelog"].as_str().unwrap_or_default().to_string(),
            files,
        });
    }
    Ok(out)
}

/// 取项目最新版本文件（files[].url / filename / size），未匹配 MC 版本返回 Err
pub fn latest_file(
    project_id: &str,
    mc_version: &str,
    loader: &str,
) -> Result<(String, String, u64), String> {
    let client = new_client();
    let v = fetch_json(
        &client,
        &format!("https://api.modrinth.com/v2/project/{project_id}/version"),
    )?;
    let arr = v.as_array().ok_or("version 响应异常")?;
    for ver in arr {
        let gv = ver["game_versions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let loaders = ver["loaders"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mc_ok = mc_version.is_empty() || gv.iter().any(|x| x == mc_version);
        let loader_ok = loader.is_empty() || loaders.iter().any(|x| x == loader);
        if !mc_ok || !loader_ok {
            continue;
        }
        if let Some(files) = ver["files"].as_array() {
            for f in files {
                if let (Some(url), Some(name)) = (f["url"].as_str(), f["filename"].as_str()) {
                    let size = f["size"].as_u64().unwrap_or(0);
                    return Ok((url.to_string(), name.to_string(), size));
                }
            }
        }
    }
    Err(format!("未找到适配 {mc_version} + {loader} 的版本"))
}

/// 简单 URL 编码（仅转义查询串必要字符）
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 辅助：取 JSON 值工具（供调试打印用）
#[allow(dead_code)]
fn _dump(v: &Value, depth: usize) {
    if depth > 2 {
        return;
    }
    eprintln!("{v:#}");
}

/// 常用分类标签选项（详情/筛选下拉"选择标签"用）
pub fn category_options() -> [&'static str; 15] {
    [
        "optimization", "performance", "decoration", "library", "gameplay",
        "worldgen", "utility", "armor", "food", "magic", "storage", "adventure",
        "technology", "combat", "management",
    ]
}

/// 在线翻译（Google 免费接口，无需 Key；自动检测 -> 简体中文）
/// Google 失败时自动降级 MyMemory 兜底，避免长时间无结果。
pub fn translate_text(text: &str) -> Result<String, String> {
    // 独立短超时 client（10s 连接 / 25s 整体），翻译场景不等 120s
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| format!("构建翻译客户端失败: {e}"))?;
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl=zh-CN&dt=t&q={}",
        urlencode(text)
    );
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("翻译请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("翻译 HTTP {}", resp.status()));
    }
    let v: Value = resp.json().map_err(|e| format!("翻译响应解析失败: {e}"))?;
    let mut out = String::new();
    if let Some(seg) = v.get(0).and_then(|a| a.as_array()) {
        for item in seg {
            if let Some(txt) = item.get(0).and_then(|t| t.as_str()) {
                out.push_str(txt);
            }
        }
    }
    if out.trim().is_empty() {
        Err("翻译结果为空".to_string())
    } else {
        Ok(out)
    }
}

/// 翻译兜底（Google 失败时调用）：MyMemory 免费接口，英文 -> 简体中文。
/// 注意 MyMemory 的 langpair 不接受 auto，源语言固定 en（模组标题基本为英文）。
pub fn translate_text_fallback(text: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| format!("构建翻译客户端失败: {e}"))?;
    let url = format!(
        "https://api.mymemory.translated.net/get?langpair=en|zh-CN&q={}",
        urlencode(text)
    );
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("兜底翻译请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("兜底翻译 HTTP {}", resp.status()));
    }
    let v: Value = resp.json().map_err(|e| format!("兜底翻译响应解析失败: {e}"))?;
    let status = v["responseStatus"].as_i64().unwrap_or(0);
    let out = v["responseData"]["translatedText"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    // MyMemory 错误时 responseStatus != 200，translatedText 可能为错误说明文本
    if status != 200 || out.is_empty() {
        Err(format!("兜底翻译失败(responseStatus={status})"))
    } else {
        Ok(out)
    }
}

/// 按文件 SHA1 查询所属 Modrinth 版本（PCL 式"更新"检测）。
/// 返回 (project_id, version_id, version_number, date_published, primary_file_url, primary_file_name)。
pub fn version_by_sha1(
    sha1: &str,
) -> Result<(String, String, String, String, String, String), String> {
    let client = new_client();
    let body = serde_json::json!({
        "hashes": [format!("sha1:{sha1}")],
        "algorithm": "sha1",
    });
    let resp = client
        .post("https://api.modrinth.com/v2/version_files")
        .json(&body)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: Value = resp.json().map_err(|e| format!("JSON 解析失败: {e}"))?;
    let arr = v.as_array().ok_or("version_files 响应异常")?;
    if arr.is_empty() {
        return Err("该文件不在 Modrinth 上（可能来自第三方源或已下架）".to_string());
    }
    let project_id = arr[0]["project_id"].as_str().unwrap_or_default().to_string();
    let version_id = arr[0]["version_id"].as_str().unwrap_or_default().to_string();
    if project_id.is_empty() || version_id.is_empty() {
        return Err("version_files 响应缺少项目/版本 ID".to_string());
    }
    // GET /v2/version/{id} 拿版本详情
    let vv = fetch_json(
        &client,
        &format!("https://api.modrinth.com/v2/version/{version_id}"),
    )?;
    let version_number = vv["version_number"].as_str().unwrap_or_default().to_string();
    let date_published = vv["date_published"].as_str().unwrap_or_default().to_string();
    let mut furl = String::new();
    let mut fname = String::new();
    if let Some(files) = vv["files"].as_array() {
        for f in files {
            if let (Some(u), Some(n)) = (f["url"].as_str(), f["filename"].as_str()) {
                if f["primary"].as_bool().unwrap_or(false) {
                    furl = u.to_string();
                    fname = n.to_string();
                    break;
                }
            }
        }
        if fname.is_empty() {
            for f in files {
                if let (Some(u), Some(n)) = (f["url"].as_str(), f["filename"].as_str()) {
                    furl = u.to_string();
                    fname = n.to_string();
                    break;
                }
            }
        }
    }
    Ok((
        project_id,
        version_id,
        version_number,
        date_published,
        furl,
        fname,
    ))
}

/// 查询 MC 百科（mcmod.cn）搜索页，取第一个模组条目的中文名与百科链接。
/// 百科返回格式：`<a ... href="https://www.mcmod.cn/class/{id}.html">中文名 (<em>English</em>)</a>`。
/// 成功返回 (中文名, 百科页 URL)；百科未收录中文名（纯英文条目）或请求失败返回 Err。
/// 注意：百科为"正确中文名"首选来源，仅当其未收录时才应降级走在线翻译。
pub fn fetch_mcmod_name(title: &str) -> Result<(String, String), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("构建 MC 百科客户端失败: {e}"))?;
    let url = format!("https://search.mcmod.cn/s?key={}", urlencode(title));
    let resp = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )
        .send()
        .map_err(|e| format!("MC 百科请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("MC 百科 HTTP {}", resp.status()));
    }
    let body = resp
        .text()
        .map_err(|e| format!("MC 百科响应读取失败: {e}"))?;
    let needle = "https://www.mcmod.cn/class/";
    let mut from = 0usize;
    while let Some(rel) = body[from..].find(needle) {
        let p = from + rel + needle.len();
        let Some(id_len) = body[p..].find(".html") else {
            break;
        };
        let id = &body[p..p + id_len];
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            from = p + id_len;
            continue;
        }
        let after = p + id_len + ".html".len();
        let Some(gt_rel) = body[after..].find('>') else {
            break;
        };
        let text_start = after + gt_rel + 1;
        let Some(text_len) = body[text_start..].find("</a>") else {
            break;
        };
        let raw = decode_html(&body[text_start..text_start + text_len]);
        // 形如 "钠 (Sodium)"；无 " (" 或括号为空视为纯英文条目（百科无中文名）
        let (cn, en) = match raw.find(" (") {
            Some(sep) => {
                let cn = raw[..sep].trim().to_string();
                let en = raw[sep + 2..].trim_end_matches(')').trim().to_string();
                (cn, en)
            }
            None => (raw.trim().to_string(), String::new()),
        };
        let has_cn = !en.is_empty() || contains_cjk(&cn);
        if has_cn && !cn.is_empty() {
            return Ok((cn, format!("https://www.mcmod.cn/class/{id}.html")));
        }
        from = text_start + text_len;
    }
    Err("MC 百科未收录该模组的中文名".to_string())
}

/// 简易 HTML 实体解码 + 剥离 <em> 高亮标签（MC 百科搜索页英文名用 <em> 包裹）
fn decode_html(s: &str) -> String {
    s.replace("<em>", "")
        .replace("</em>", "")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// 是否含 CJK 字符（判断百科条目名是否本身已是中文）
fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0x2E80..0xA000).contains(&u)
            || (0xF900..0xFB00).contains(&u)
            || (0xFE30..0xFE50).contains(&u)
    })
}
