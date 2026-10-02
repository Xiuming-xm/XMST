//! MC 百科离线数据库（mcmod.buf 解析结果，随 exe 内嵌，完全离线）。
//!
//! 数据来源：PCL-CE 仓库 `Plain Craft Launcher 2/Resources/mcmod.buf`（GZip+protobuf）。
//! 构建期已解压为裸 protobuf 消息流（resources/mcmod.raw），结构如下：
//! - 外层：连续拼接的消息，每条为 `0a {len} {payload}`（field 1, length-delimited）
//! - 内层 ModTranslation（payload）：
//!   - field 1 (varint)   = WikiId（MC 百科 class ID，可直接拼百科 URL）
//!   - field 2 (string)   = ChineseName（形如 "钠 (Sodium)"；可能为空=百科无中文译名）
//!   - field 3 (string)   = CurseForgeSlug
//!   - field 4 (string)   = ModrinthSlug
//!
//! 查询：按 slug 精确匹配（与 PCL-CE 相同策略），无网络依赖、结果稳定。

use std::collections::HashMap;

const RAW: &[u8] = include_bytes!("../resources/mcmod.raw");

/// 单条百科条目（解析后）
pub struct McmodEntry {
    /// MC 百科 class ID，百科 URL 为 `https://www.mcmod.cn/class/{wiki_id}.html`
    pub wiki_id: i64,
    /// 中文名（已剥去 " (英文)" 括号后缀）；空串 = 百科有收录但无中文译名
    pub cn_name: String,
}

/// 离线百科库：slug -> 条目
pub struct McmodDb {
    by_modrinth: HashMap<String, McmodEntry>,
    by_curseforge: HashMap<String, McmodEntry>,
}

impl McmodDb {
    /// 解析内嵌 protobuf 消息流并构建索引（一次性，纯内存）
    pub fn new() -> Self {
        let mut by_modrinth = HashMap::new();
        let mut by_curseforge = HashMap::new();
        let b = RAW;
        let mut i = 0usize;
        while i + 1 < b.len() {
            if b[i] != 0x0a {
                break;
            }
            let Ok((ln, j)) = read_varint(b, i + 1) else {
                break;
            };
            let end = j.saturating_add(ln as usize);
            if end > b.len() {
                break;
            }
            let (wiki, cn, cf, mr) = parse_inner(&b[j..end]);
            i = end;
            let Some(wiki_id) = wiki else { continue };
            let entry = McmodEntry {
                wiki_id,
                cn_name: strip_en(cn.as_deref().unwrap_or("")),
            };
            if let Some(s) = mr {
                by_modrinth.entry(s).or_insert_with(|| McmodEntry {
                    wiki_id: entry.wiki_id,
                    cn_name: entry.cn_name.clone(),
                });
            }
            if let Some(s) = cf {
                by_curseforge.entry(s).or_insert_with(|| McmodEntry {
                    wiki_id: entry.wiki_id,
                    cn_name: entry.cn_name.clone(),
                });
            }
        }
        Self {
            by_modrinth,
            by_curseforge,
        }
    }

    /// 按 Modrinth slug 精确查询
    pub fn lookup_modrinth(&self, slug: &str) -> Option<&McmodEntry> {
        self.by_modrinth.get(slug)
    }

    /// 按 CurseForge slug 精确查询
    pub fn lookup_curseforge(&self, slug: &str) -> Option<&McmodEntry> {
        self.by_curseforge.get(slug)
    }
}

/// 读取 protobuf varint，返回 (值, 下一个字节下标)；越界返回 Err
fn read_varint(b: &[u8], mut i: usize) -> Result<(u64, usize), ()> {
    let mut r = 0u64;
    let mut shift = 0u32;
    while i < b.len() {
        let x = b[i];
        i += 1;
        r |= ((x & 0x7f) as u64) << shift;
        if x & 0x80 == 0 {
            return Ok((r, i));
        }
        shift += 7;
        if shift >= 64 {
            return Err(());
        }
    }
    Err(())
}

/// 解析内层 ModTranslation 消息，返回 (WikiId, ChineseName, CurseForgeSlug, ModrinthSlug)
fn parse_inner(payload: &[u8]) -> (Option<i64>, Option<String>, Option<String>, Option<String>) {
    let mut wiki = None;
    let mut cn = None;
    let mut cf = None;
    let mut mr = None;
    let mut i = 0usize;
    while i < payload.len() {
        let Ok((tag, ni)) = read_varint(payload, i) else {
            break;
        };
        i = ni;
        let field = (tag >> 3) as u32;
        let wt = (tag & 7) as u32;
        match wt {
            0 => {
                let Ok((v, ni)) = read_varint(payload, i) else {
                    break;
                };
                i = ni;
                if field == 1 {
                    wiki = Some(v as i64);
                }
            }
            2 => {
                let Ok((ln, ni)) = read_varint(payload, i) else {
                    break;
                };
                i = ni;
                let end = i.saturating_add(ln as usize);
                if end > payload.len() {
                    break;
                }
                let s = String::from_utf8_lossy(&payload[i..end]).into_owned();
                i = end;
                match field {
                    2 => cn = Some(s),
                    3 => cf = Some(s),
                    4 => mr = Some(s),
                    _ => {}
                }
            }
            _ => break,
        }
    }
    (wiki, cn, cf, mr)
}

/// 剥去 " (English)" 括号后缀：`钠 (Sodium)` -> `钠`；无括号或剥后为空则原样返回
fn strip_en(cn: &str) -> String {
    let t = cn.trim();
    if let Some(idx) = t.rfind(" (") {
        if t.ends_with(')') {
            let head = t[..idx].trim();
            if !head.is_empty() {
                return head.to_string();
            }
        }
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_db_and_lookup() {
        let db = McmodDb::new();
        // Modrinth slug 命中（钠）
        let e = db.lookup_modrinth("sodium").expect("sodium should hit");
        assert_eq!(e.wiki_id, 2785);
        assert_eq!(e.cn_name, "钠");
        // 仅 CurseForge slug 命中（铁氧体磁芯）
        let e = db.lookup_curseforge("ferritecore").expect("ferritecore cf");
        assert_eq!(e.wiki_id, 3888);
        assert_eq!(e.cn_name, "铁氧体磁芯");
        // 百科收录但无中文译名（Fabric API）
        let e = db.lookup_modrinth("fabric-api").expect("fabric-api hit");
        assert_eq!(e.wiki_id, 3124);
        assert!(e.cn_name.is_empty());
        // 未收录 slug
        assert!(db.lookup_modrinth("this-slug-does-not-exist-xyz").is_none());
    }

    #[test]
    fn strip_brackets() {
        assert_eq!(strip_en("钠 (Sodium)"), "钠");
        assert_eq!(strip_en("[FC] 铁氧体磁芯 (FerriteCore)"), "[FC] 铁氧体磁芯");
        assert_eq!(strip_en("钠 · 扩展 (Sodium Extra)"), "钠 · 扩展");
        assert_eq!(strip_en("工业时代2 (Industrial Craft 2)"), "工业时代2");
        assert_eq!(strip_en("纯中文名"), "纯中文名");
        assert_eq!(strip_en(""), "");
    }
}

