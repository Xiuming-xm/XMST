//! 崩溃根因分析：从 `logs/latest.log` 与 `crash-reports/*.txt` 里提取**可操作**的崩溃原因。
//!
//! 设计原则：只报"能说清楚、且用户能据此行动"的原因 —— 例如「模组 A 缺少前置 B」，
//! 而不是把堆栈原样丢给用户。每条结论都带有**证据行**，便于用户自己核对。
//!
//! 覆盖的实际崩溃类型（按现场日志出现频率排序）：
//! 1. Fabric：`Missing or unsupported mandatory dependencies` / `requires ... which is missing`
//! 2. Forge/NeoForge：`Missing Mods:` / `Mod file X requires Y`
//! 3. Java 版本过低：`UnsupportedClassVersionError` / `class file version NN`
//! 4. 内存不足：`OutOfMemoryError`
//! 5. 端口占用：`Address already in use` / `FAILED TO BIND TO PORT`
//! 6. Mixin 冲突：`Mixin apply failed` / `Mixin transformation ... failed`
//! 7. EULA 未同意：`You need to agree to the EULA`
//! 8. 重复安装 / 客户端模组装进服务端
//! 9. 世界版本不兼容

use std::collections::HashMap;
use std::path::Path;

/// 一条分析结论。
#[derive(Debug, Clone)]
pub struct Cause {
    /// 结论标题（一句话）
    pub title: String,
    /// 可能涉及的模组/插件（已尽力映射成友好名）
    pub suspects: Vec<String>,
    /// 处理建议
    pub advice: String,
    /// 支撑该结论的原始日志行（截断）
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CrashFinding {
    /// 触发分析的文件（相对目录的路径）
    pub source: String,
    pub causes: Vec<Cause>,
    /// 是否是崩溃类日志（false = 只是普通退出，但仍有可疑信息）
    pub crashed: bool,
    /// 崩溃摘要行（crash-report 的 Description / 日志首个 ERROR）
    pub summary: String,
}

impl CrashFinding {
    pub fn is_empty(&self) -> bool {
        self.causes.is_empty() && self.summary.is_empty()
    }
}

/// 读取 mods/ 目录，建立 `mod id -> 友好名(文件名)` 映射（Fabric/Quilt/Forge/NeoForge 四种元数据）。
fn mod_name_map(dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir.join("mods")) else {
        return map;
    };
    for e in rd.flatten() {
        let p = e.path();
        let fname = e.file_name().to_string_lossy().to_string();
        if !fname.to_lowercase().ends_with(".jar") {
            continue;
        }
        map.insert(fname.trim_end_matches(".jar").to_string(), fname.clone());
        let Ok(f) = std::fs::File::open(&p) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(f) else {
            continue;
        };
        // Fabric / Quilt: fabric.mod.json / quilt.mod.json
        for key in ["fabric.mod.json", "quilt.mod.json"] {
            if let Ok(mut entry) = zip.by_name(key) {
                let mut s = String::new();
                use std::io::Read;
                if entry.read_to_string(&mut s).is_ok() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        let id = v.get("id").and_then(|x| x.as_str());
                        let name = v
                            .get("name")
                            .and_then(|x| x.as_str())
                            .or_else(|| {
                                v.get("quilt_loader")
                                    .and_then(|q| q.get("id"))
                                    .and_then(|x| x.as_str())
                            })
                            .unwrap_or("");
                        if let Some(id) = id {
                            let label = if name.is_empty() {
                                id.to_string()
                            } else {
                                format!("{name}（{id}）")
                            };
                            map.insert(id.to_string(), label);
                        }
                    }
                }
                break;
            }
        }
        // Forge / NeoForge: META-INF/mods.toml（简单按行取 modId / displayName）
        let toml_txt: Option<String> = {
            match zip.by_name("META-INF/mods.toml") {
                Ok(mut entry) => {
                    let mut s = String::new();
                    use std::io::Read;
                    if entry.read_to_string(&mut s).is_ok() {
                        Some(s)
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        };
        if let Some(s) = toml_txt {
            let mut cur_id: Option<String> = None;
            let mut cur_name: Option<String> = None;
            for line in s.lines() {
                let l = line.trim();
                if let Some(v) = l.strip_prefix("modId") {
                    cur_id = Some(
                        v.trim_start_matches(['=', ' ', '"'])
                            .trim_matches('"')
                            .to_string(),
                    );
                } else if let Some(v) = l.strip_prefix("displayName") {
                    cur_name = Some(
                        v.trim_start_matches(['=', ' ', '"'])
                            .trim_matches('"')
                            .to_string(),
                    );
                }
                if let (Some(id), Some(nm)) = (&cur_id, &cur_name) {
                    if !id.is_empty() {
                        map.insert(id.clone(), format!("{nm}（{id}）"));
                    }
                    cur_id = None;
                    cur_name = None;
                }
            }
        }
    }
    map
}

/// 在文本里找所有形如 `mod_id` 的标识符，映射成友好名。
fn suspects_in(line: &str, map: &HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut push = |cur: &mut String, out: &mut Vec<String>| {
        if cur.len() >= 3 {
            if let Some(f) = map.get(cur.as_str()) {
                if !out.contains(f) {
                    out.push(f.clone());
                }
            }
        }
        cur.clear();
    };
    for c in line.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
            cur.push(c);
        } else {
            push(&mut cur, &mut out);
        }
    }
    push(&mut cur, &mut out);
    out
}

fn push_cause(causes: &mut Vec<Cause>, c: Cause) {
    // 标题去重（同一类错误可能重复出现多行）
    if causes.iter().any(|x| x.title == c.title) {
        if let Some(existing) = causes.iter_mut().find(|x| x.title == c.title) {
            for s in c.suspects {
                if !existing.suspects.contains(&s) {
                    existing.suspects.push(s);
                }
            }
            for e in c.evidence {
                if existing.evidence.len() < 5 && !existing.evidence.contains(&e) {
                    existing.evidence.push(e);
                }
            }
        }
        return;
    }
    causes.push(c);
}

/// 分析一个服务端目录的最近日志/崩溃报告，给出结论。
pub fn analyze(dir: &Path) -> Option<CrashFinding> {
    let map = mod_name_map(dir);
    // 收集文本源：优先最新 crash-report，其次 latest.log 尾部
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut reports: Vec<std::path::PathBuf> = std::fs::read_dir(dir.join("crash-reports"))
        .ok()
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "txt").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    reports.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
    });
    if let Some(p) = reports.last() {
        if let Ok(s) = std::fs::read_to_string(p) {
            sources.push((
                p.file_name().unwrap_or_default().to_string_lossy().to_string(),
                s,
            ));
        }
    }
    let log_path = dir.join("logs").join("latest.log");
    if let Ok(s) = std::fs::read_to_string(&log_path) {
        // latest.log 可能很大：只取最后 4000 行
        let tail: String = s
            .lines()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        sources.push(("logs/latest.log".to_string(), tail));
    }
    if sources.is_empty() {
        return None;
    }

    let mut finding = CrashFinding::default();
    for (name, text) in &sources {
        // ---- 先用「崩溃报告」的整体结构做一次分析：Exception 行 + 堆栈归属模组 ----
        // 这一步覆盖了大量"模组自身异常/配置损坏"类崩溃（依赖缺失只是其中一类）。
        if text.contains("---- Minecraft Crash Report ----") || text.contains("Description:") {
            if let Some((exc, suspects)) = first_exception(text, &map) {
                finding.summary = exc.clone();
                finding.crashed = true;
                let low = exc.to_lowercase();
                let (title, advice) = if low.contains("jsonsyntaxexception")
                    || low.contains("jsonparseexception")
                    || low.contains("expected begin_array")
                    || low.contains("malformedjson")
                {
                    (
                        "模组配置文件损坏（config 里的 json 格式不正确）".to_string(),
                        "该模组读取自己的配置文件时解析失败。到服务器 config/ 目录把对应模组的\
                         配置文件改名或删除（备份后），重启让它重新生成即可。"
                            .to_string(),
                    )
                } else if low.contains("nosuchmethoderror")
                    || low.contains("noclassdeffounderror")
                    || low.contains("classnotfoundexception")
                    || low.contains("nosuchfielderror")
                {
                    (
                        "模组之间版本不匹配（缺少类/方法）".to_string(),
                        "通常是同系列模组的版本不一致（如 API 与实现版本错开），或缺少前置库。\
                         把这些模组统一升级到同一 MC 版本下的最新版。"
                            .to_string(),
                    )
                } else if low.contains("stackoverflowerror") {
                    (
                        "栈溢出（模组递归或相互冲突）".to_string(),
                        "逐个禁用最近新增/更新过的模组定位；常见于多个优化或兼容模组同时安装。"
                            .to_string(),
                    )
                } else if low.contains("outofmemoryerror") {
                    (
                        "内存不足（堆溢出）".to_string(),
                        "调大 -Xmx（服务器设置里改），或减少常驻大型模组/视距。".to_string(),
                    )
                } else {
                    (
                        format!("模组抛出异常：{}", truncate(&exc)),
                        "按「涉及」里列出的模组检查其配置/版本；也可先禁用该模组确认。".to_string(),
                    )
                };
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title,
                        suspects: suspects.clone(),
                        advice,
                        evidence: vec![exc.clone()],
                    },
                );
                if suspects.is_empty() {
                    push_cause(
                        &mut finding.causes,
                        Cause {
                            title: "无法确定具体模组（堆栈里没有匹配到 mods 目录中的模组）".to_string(),
                            suspects: vec![],
                            advice: "查看完整崩溃报告（下方按钮可打开 crash-reports 目录），\
                                     或把报告发我帮你定位。"
                                .to_string(),
                            evidence: vec![],
                        },
                    );
                }
            }
        }
        let mut crashed = finding.crashed;
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            // ---- 崩溃类型判定 ----
            if l.contains("Missing or unsupported mandatory dependencies")
                || l.contains("requires any version of")
                || l.contains("which is missing")
                || l.contains("Mod resolution failed")
                || l.contains("Incompatible mod set")
                || l.starts_with("Missing Mods:")
                || l.contains("Mod file") && l.contains("requires")
            {
                crashed = true;
                let sus = suspects_in(l, &map);
                let title = if sus.is_empty() {
                    "缺少前置依赖（模组依赖未满足）".to_string()
                } else {
                    format!("缺少前置依赖：{}", sus.join("、"))
                };
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title,
                        suspects: sus,
                        advice: "按提示补装缺失的前置模组（同名版本必须匹配），或把依赖它的模组一起升级/移除。\
                                 用「下载」页按 MC 版本+加载器搜索该前置即可。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("UnsupportedClassVersionError")
                || (l.contains("class file version") && l.contains("Unsupported"))
            {
                crashed = true;
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "Java 版本过低（模组/服务端要求更高版本）".to_string(),
                        suspects: suspects_in(l, &map),
                        advice: "安装与该服务端匹配的 Java（1.20.5+ 需要 Java 21），\
                                 在「设置 → Java 和 JVM」里指定正确的 java.exe。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("OutOfMemoryError") || l.contains("java.lang.OutOfMemoryError") {
                crashed = true;
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "内存不足（堆溢出）".to_string(),
                        suspects: vec![],
                        advice: "调大启动内存：在服务器设置里提高 -Xmx（如 -Xmx4G），\
                                 并检查是否装了过多大型模组。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("Address already in use") || l.contains("FAILED TO BIND TO PORT") {
                crashed = true;
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "端口被占用".to_string(),
                        suspects: vec![],
                        advice: "改 server.properties 的 server-port，或先用「服务器」页找出\
                                 占用该端口的进程（也可能是上一个实例没退干净）。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("Mixin apply failed")
                || l.contains("Mixin transformation")
                || l.contains("MixinApplyError")
            {
                crashed = true;
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "Mixin 冲突（模组之间不兼容）".to_string(),
                        suspects: suspects_in(l, &map),
                        advice: "同一功能不要装多个模组（如多个优化/兼容库）；\
                                 逐个禁用最近新增的模组定位。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("You need to agree to the EULA") {
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "未同意 EULA".to_string(),
                        suspects: vec![],
                        advice: "把 eula.txt 里的 eula=false 改成 true（本工具启动前可一键同意）。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("Duplicate mods") || l.contains("duplicate mod") {
                crashed = true;
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "重复安装模组（同一模组多份 jar）".to_string(),
                        suspects: suspects_in(l, &map),
                        advice: "删除 mods 下多余的旧版本 jar（`.mcsrv_trash` 里可找回）。".to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("client-only")
                || l.contains("is not supported on the dedicated server")
                || l.contains("This mod is client-side only")
            {
                crashed = true;
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "装了仅客户端模组".to_string(),
                        suspects: suspects_in(l, &map),
                        advice: "把仅客户端模组从服务端 mods 目录移除（文件浏览页有「排查客户端模组」）。"
                            .to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if l.contains("level.dat") && (l.contains("incompatible") || l.contains("version")) {
                push_cause(
                    &mut finding.causes,
                    Cause {
                        title: "世界版本不兼容（用旧版本核心打开新版存档）".to_string(),
                        suspects: vec![],
                        advice: "换回原来的服务端版本，或从备份恢复 world（.mcsrv_backups）。".to_string(),
                        evidence: vec![truncate(l)],
                    },
                );
            }
            if finding.summary.is_empty()
                && (l.starts_with("Description:") || l.contains("Encountered an unexpected exception"))
            {
                finding.summary = truncate(l);
            }
        }
        if crashed || !finding.causes.is_empty() {
            finding.source = name.clone();
            if crashed {
                finding.crashed = true;
            }
            break; // 只用最相关的一份来源
        }
    }
    if finding.is_empty() {
        None
    } else {
        Some(finding)
    }
}

fn truncate(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 200 {
        let t: String = s.chars().take(200).collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}

/// 归一化：小写 + 去掉所有非字母数字（用于模糊匹配包名与模组 id）。
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// 把堆栈里的包名模糊匹配到 mods 目录里的某个模组。
///
/// 实机样本（D:\Desktop\Fabric1.21.11 的真实崩溃报告）：
///   堆栈 `com.ayakacraft.carpetayakaaddition.commands.address.AddressManager...`
///   模组 id `carpet-ayaka-addition` → 归一化后 "carpetayakaaddition" 命中包名片段 ✓
fn match_mod_in_stack(line: &str, map: &HashMap<String, String>) -> Option<String> {
    let l = line.trim();
    if !l.starts_with("at ") {
        return None;
    }
    // `at com.foo.bar.Baz.method(...)` → 取类名前的包路径
    let path = l
        .trim_start_matches("at ")
        .split('(')
        .next()
        .unwrap_or("")
        .trim();
    // 跳过 JDK / Minecraft / 加载器自身的帧
    const SKIP: [&str; 9] = [
        "java.",
        "jdk.",
        "javax.",
        "sun.",
        "net.minecraft",
        "com.mojang",
        "net.fabricmc",
        "org.spongepowered",
        "org.objectweb",
    ];
    if SKIP.iter().any(|s| path.starts_with(s)) {
        return None;
    }
    let np = norm(path);
    // 与每个模组 id/名字做双向包含匹配（取最长的命中，避免误配到很短的 id）
    let mut best: Option<(usize, String)> = None;
    for (key, label) in map.iter() {
        let nk = norm(key);
        if nk.len() < 4 {
            continue;
        }
        // 若 key 看起来像文件名（含 . 或 -）也一并归一化比较
        if np.contains(&nk) || nk.contains(&np) {
            let score = nk.len();
            if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                best = Some((score, label.clone()));
            }
        }
    }
    best.map(|(_, l)| l)
}

/// 从一段文本里提取「异常类型 + 位置」以及涉及的模组（只看前若干帧，避免把整个堆栈翻一遍）。
fn first_exception(text: &str, map: &HashMap<String, String>) -> Option<(String, Vec<String>)> {
    let mut exc_line: Option<String> = None;
    let mut suspects: Vec<String> = Vec::new();
    let mut frames = 0;
    for line in text.lines() {
        let l = line.trim();
        if exc_line.is_none() {
            // 异常行：形如 `com.google.gson.JsonSyntaxException: ...`
            if l.contains("Exception") || l.contains("Error:") || l.contains("Error(") {
                if !l.starts_with("at ") && !l.starts_with("Description:") {
                    exc_line = Some(truncate(l));
                }
            }
            continue;
        }
        if l.starts_with("at ") {
            frames += 1;
            if frames > 60 {
                break;
            }
            if suspects.len() < 3 {
                if let Some(m) = match_mod_in_stack(l, map) {
                    if !suspects.contains(&m) {
                        suspects.push(m);
                    }
                }
            }
        } else if frames > 0 && !l.is_empty() {
            break; // 堆栈结束
        }
    }
    exc_line.map(|e| (e, suspects))
}
