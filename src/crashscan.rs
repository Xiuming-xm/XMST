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
    /// 建议查看/修改的文件或目录（相对服务器目录）；Some 时弹窗给出跳转按钮
    pub path: Option<String>,
    /// 该文件内容不是合法 JSON（极可能就是崩溃元凶，UI 会特别标注）
    pub path_broken: bool,
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
                    path: None,
                    path_broken: false,
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
                            path: None,
                            path_broken: false,
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
                        path: None,
                        path_broken: false,
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
                        path: None,
                        path_broken: false,
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
                        path: None,
                        path_broken: false,
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
                        path: Some("server.properties".to_string()),
                        path_broken: false,
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
                        path: None,
                        path_broken: false,
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
                        path: Some("eula.txt".to_string()),
                        path_broken: false,
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
                        path: None,
                        path_broken: false,
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
                        path: None,
                        path_broken: false,
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
                        path: None,
                        path_broken: false,
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
    // 为每条结论补充"建议查看的数据文件"（用户可直接跳过去改）。
    // 搜索范围不止 config/：carpet 系列等模组把服务端设置放在 **world/** 下，
    // 也有模组直接写在服务器根目录，所以按 config → world → 根目录 → defaultconfigs 依次找。
    if !finding.causes.is_empty() {
        for c in finding.causes.iter_mut() {
            if c.path.is_some() {
                continue;
            }
            let ids: Vec<String> = c
                .suspects
                .iter()
                .map(|s| {
                    s.rsplit_once('（')
                        .map(|(_, r)| r.trim_end_matches('）').to_string())
                        .unwrap_or_else(|| s.clone())
                })
                .collect();
            if let Some((rel, broken)) = find_data_file(dir, &ids) {
                c.path = Some(rel);
                c.path_broken = broken;
            } else if c.title.contains("json")
                || c.title.contains("配置")
                || c.title.to_lowercase().contains("json")
            {
                // 名字对不上时，退一步做**内容级**排查：JSON 配置损坏类崩溃，
                // 直接把"解析失败的那个 json"找出来报给用户（比猜文件名可靠得多）。
                if let Some(rel) = find_broken_json(dir) {
                    c.path = Some(rel);
                    c.path_broken = true;
                }
            }
        }
    }
    if finding.is_empty() {
        None
    } else {
        Some(finding)
    }
}

/// 在 `config/`（含一层子目录）里按模组 id 模糊查找配置文件/目录，返回相对路径。
fn find_config_for(dir: &Path, mod_id: &str) -> Option<String> {
    let key = norm(mod_id);
    if key.len() < 4 {
        return None;
    }
    let cfg = dir.join("config");
    let mut best: Option<(usize, String)> = None;
    let mut scan = |p: &Path| {
        let name = p.file_name()?.to_string_lossy().to_string();
        let n = norm(&name);
        if n.contains(&key) || key.contains(&n) {
            let score = n.len();
            if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                let rel = p
                    .strip_prefix(dir)
                    .map(|r| r.to_string_lossy().to_string())
                    .unwrap_or_else(|_| name.clone());
                best = Some((score, rel));
            }
        }
        Some(())
    };
    if let Ok(rd) = std::fs::read_dir(&cfg) {
        for e in rd.flatten() {
            let p = e.path();
            let _ = scan(&p);
            if p.is_dir() {
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    for e2 in rd2.flatten() {
                        let _ = scan(&e2.path());
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
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

/// 在多个可能的位置查找与模组相关的数据文件，返回 (相对路径, 内容是否不是合法 JSON)。
///
/// 搜索顺序（先找到先用，都是"与模组 id 强匹配"才算命中）：
///   `config/`（含一层子目录）→ `world/`（含一层子目录）→ 服务器根目录 → `defaultconfigs/`
///
/// 匹配规则（旧实现用 `contains` 导致 "carpet" 命中了 "antideath-carpet-addition" 这类误报）：
/// * 归一化后**完全相等** → 最高分；
/// * 一方是另一方的前缀，且长度比 ≥ 0.75 → 次之；
/// * 其余一律不算命中（宁可回退到目录，也不要指错文件）。
/// 另外：如果候选文件本身**解析 JSON 失败**，说明它极可能就是元凶（配置损坏类崩溃），
/// 会被优先选中并在 UI 上标注。
fn find_data_file(dir: &Path, mod_ids: &[String]) -> Option<(String, bool)> {
    let keys: Vec<String> = mod_ids
        .iter()
        .map(|s| norm(s))
        .filter(|k| k.len() >= 4)
        .collect();
    if keys.is_empty() {
        return None;
    }
    let mut best: Option<(f32, String, bool)> = None;
    let mut consider = |p: &Path, best: &mut Option<(f32, String, bool)>| {
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return;
        };
        let stem = name
            .trim_end_matches(".json")
            .trim_end_matches(".toml")
            .trim_end_matches(".properties")
            .to_string();
        let n = norm(&stem);
        if n.len() < 4 {
            return;
        }
        let mut score = 0.0f32;
        for k in &keys {
            // 短/通用 id（如 "carpet"）只接受**完全相等**：否则会把
            // antideath-carpet-addition 这类"名字里恰好含 carpet"的无关模组错认成目标
            // （用户实测到的错误跳转就是它）。
            if k.len() < 10 && n != *k {
                continue;
            }
            if n == *k {
                score = score.max(1.0 + (k.len() as f32) * 0.01);
            } else if n.starts_with(k.as_str()) || k.starts_with(n.as_str()) {
                let ratio = (n.len().min(k.len()) as f32) / (n.len().max(k.len()) as f32);
                if ratio >= 0.75 {
                    score = score.max(0.6 + 0.3 * ratio + (k.len() as f32) * 0.01);
                }
            }
        }
        if score <= 0.0 {
            return;
        }
        // 内容不是合法 JSON → 加权（配置损坏类崩溃的第一嫌疑）
        let broken = p.extension().map(|e| e == "json").unwrap_or(false) && json_is_broken(p);
        if broken {
            score += 0.5;
        }
        let rel = p
            .strip_prefix(dir)
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or(name);
        if best.as_ref().map(|(s, _, _)| score > *s).unwrap_or(true) {
            *best = Some((score, rel, broken));
        }
    };
    for root in ["config", "world", "defaultconfigs", "."] {
        let base = dir.join(root);
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            consider(&p, &mut best);
            if p.is_dir() {
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    for e2 in rd2.flatten() {
                        consider(&e2.path(), &mut best);
                    }
                }
            }
        }
        // 命中且是"坏文件"就不必继续找别的根目录了
        if best.as_ref().map(|(_, _, b)| *b).unwrap_or(false) {
            break;
        }
    }
    best.map(|(_, rel, broken)| (rel, broken))
}

/// 尝试把文件当 JSON 解析；失败返回 true（空文件也算"坏"——实测空配置同样会导致模组崩溃）。
fn json_is_broken(p: &Path) -> bool {
    let Ok(s) = std::fs::read_to_string(p) else {
        return false;
    };
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(t).is_err()
}

/// 扫描 config/、world/（一层）、根目录下的 *.json，返回**第一个解析失败**的相对路径。
///
/// 用于"配置损坏"类崩溃的兜底：即使文件名与模组 id 对不上，坏掉的那个 json 也能被找出来。
/// 空文件同样算坏（实测空配置也会让模组崩溃）。
fn find_broken_json(dir: &Path) -> Option<String> {
    let mut checked = 0;
    // 只扫"模组配置文件该在的地方"：config/（含一层子目录）、世界目录**顶层**
    // （Carpet 等把服务端设置写在 world/ 下，如 world/carpet.conf）、服务器根目录。
    // 故意**不**深入 world/ 子目录 —— 那里是存档数据（advancements/playerdata/stats），
    // 它们的 json 损坏与模组启动崩溃无关，误报会把人带偏（实测踩过）。
    for (root, recurse) in [("config", true), ("world", false), (".", false)] {
        let base = dir.join(root);
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if recurse {
                    dirs.push(p);
                }
                continue;
            }
            if p.extension().map(|x| x == "json").unwrap_or(false) {
                checked += 1;
                if checked > 400 {
                    return None;
                }
                if json_is_broken(&p) {
                    return Some(
                        p.strip_prefix(dir)
                            .map(|r| r.to_string_lossy().to_string())
                            .unwrap_or_else(|_| p.display().to_string()),
                    );
                }
            }
        }
        for d in dirs {
            if let Ok(rd2) = std::fs::read_dir(&d) {
                for e2 in rd2.flatten() {
                    let p = e2.path();
                    if p.is_file() && p.extension().map(|x| x == "json").unwrap_or(false) {
                        checked += 1;
                        if checked > 400 {
                            return None;
                        }
                        if json_is_broken(&p) {
                            return Some(
                                p.strip_prefix(dir)
                                    .map(|r| r.to_string_lossy().to_string())
                                    .unwrap_or_else(|_| p.display().to_string()),
                            );
                        }
                    }
                }
            }
        }
    }
    None
}