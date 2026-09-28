//! Spark 输出文件解析与分析（.sparkhealth / .sparkprofile）
//!
//! 两种文件均为 gzip 压缩的 protobuf（lucko/spark 官方 schema）：
//! - .sparkhealth → HealthData（spark.proto）：TPS/MSPT/CPU/内存/GC/实体/区块/玩家等概览指标 + 时间窗口序列
//! - .sparkprofile → SamplerData（spark_sampler.proto）：线程调用树（self time）、按模组/插件来源聚合的 CPU 占比
//!
//! 字段含义参考 lucko/spark 官方 Java 反序列化实现与 spark-profiler-mcp（同一 protobuf 格式）。

include!(concat!(env!("OUT_DIR"), "/spark.rs"));

use egui::{Color32, RichText};
use flate2::read::GzDecoder;
use prost::Message;
use std::collections::HashMap;
use std::io::Read;

/// TPS 低于该值判定为卡顿（警示着色）
pub const TPS_WARN: f64 = 18.0;
/// MSPT 高于该值判定为卡顿（警示着色）
pub const MSPT_WARN: f64 = 50.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SparkFileKind {
    Health,
    Profile,
}

/// 概览卡片：核心指标 + 阈值着色
#[derive(Clone)]
pub struct OverviewCard {
    pub label: &'static str,
    pub value: String,
    /// true = 警示着色（低于/高于阈值）
    pub warn: bool,
    /// 预留：健康着色
    pub ok: bool,
}

impl OverviewCard {
    fn new(label: &'static str, value: String, warn: bool) -> Self {
        Self { label, value, warn, ok: false }
    }
}

/// 模组/插件性能占用（按 class_sources 聚合自耗时采样）
#[derive(Clone)]
pub struct ModPerf {
    pub name: String,
    pub self_time: f64,
    pub pct: f64,
}

/// 卡顿热点：调用树中自耗时最高的方法
#[derive(Clone)]
pub struct HotSpot {
    pub class: String,
    pub method: String,
    pub self_time: f64,
    pub pct: f64,
    /// 模组/插件来源
    pub source: String,
    /// 类型提示（实体/区块/红石等）
    pub tag: String,
}

/// 解析后的完整分析摘要（UI 渲染唯一数据源）
#[derive(Clone)]
pub struct SparkSummary {
    pub kind: SparkFileKind,
    pub file_name: String,
    /// 概览卡片
    pub cards: Vec<OverviewCard>,
    /// TPS 波动序列 (x, tps)
    pub tps_series: Vec<(f64, f64)>,
    /// MSPT 波动序列 (x, mspt)
    pub mspt_series: Vec<(f64, f64)>,
    /// 序列 x 轴单位说明（"窗口" / "时间(s)"）
    pub series_unit: &'static str,
    /// 模组性能 TOP
    pub mod_top: Vec<ModPerf>,
    /// 卡顿热点 TOP
    pub hot_top: Vec<HotSpot>,
    /// 元信息行
    pub meta_lines: Vec<String>,
}

impl SparkSummary {
    fn new(kind: SparkFileKind, file_name: String) -> Self {
        Self {
            kind,
            file_name,
            cards: Vec::new(),
            tps_series: Vec::new(),
            mspt_series: Vec::new(),
            series_unit: "窗口",
            mod_top: Vec::new(),
            hot_top: Vec::new(),
            meta_lines: Vec::new(),
        }
    }
}

/// UI 缓存状态：某文件的一次解析结果
#[derive(Clone)]
pub struct SparkAnalysisState {
    pub path: String,
    pub summary: Option<SparkSummary>,
    pub parse_error: Option<String>,
}

/// 解析 spark 输出文件（自动识别 .sparkhealth / .sparkprofile，gzip 解压 + protobuf 解码）
pub fn parse_spark_file(path: &str) -> Result<SparkSummary, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败：{e}"))?;
    // spark --save-to-file 输出为 gzip 压缩的 protobuf；容错：非 gzip 时按原始字节解码
    let raw = gunzip_or_orig(&bytes);
    let lower = path.to_lowercase();
    let file_name = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_string();
    if lower.ends_with(".sparkhealth") {
        let h = HealthData::decode(raw.as_slice())
            .map_err(|e| format!("HealthData 解码失败：{e}（文件可能损坏或 spark 版本不兼容）"))?;
        Ok(build_health_summary(h, file_name))
    } else if lower.ends_with(".sparkprofile") {
        let p = SamplerData::decode(raw.as_slice())
            .map_err(|e| format!("SamplerData 解码失败：{e}（文件可能损坏或 spark 版本不兼容）"))?;
        Ok(build_sampler_summary(p, file_name))
    } else {
        Err("不支持的文件类型（仅支持 .sparkhealth / .sparkprofile）".to_string())
    }
}

/// gzip 魔数判断解压；失败则回退原始字节
fn gunzip_or_orig(data: &[u8]) -> Vec<u8> {
    if data.len() > 2 && data[0] == 0x1f && data[1] == 0x8b {
        let mut dec = GzDecoder::new(data);
        let mut out = Vec::new();
        if dec.read_to_end(&mut out).is_ok() && !out.is_empty() {
            return out;
        }
    }
    data.to_vec()
}

// ---------------------------------------------------------------- health

fn build_health_summary(h: HealthData, file_name: String) -> SparkSummary {
    let mut s = SparkSummary::new(SparkFileKind::Health, file_name);
    if let Some(md) = &h.metadata {
        if let Some(pm) = &md.platform_metadata {
            let mc = if pm.minecraft_version.is_empty() {
                "?".to_string()
            } else {
                pm.minecraft_version.clone()
            };
            s.meta_lines.push(format!(
                "平台：{} {}（MC {}）",
                pm.name, pm.version, mc
            ));
            if !pm.spark_version.is_empty() {
                s.meta_lines.push(format!("Spark 版本：{}", pm.spark_version));
            }
            if !pm.brand.is_empty() {
                s.meta_lines.push(format!("品牌：{}", pm.brand));
            }
        }
        if md.generated_time > 0 {
            s.meta_lines.push(format!(
                "生成时间：{}",
                fmt_ts_millis(md.generated_time)
            ));
        }
        if let Some(ps) = &md.platform_statistics {
            build_platform_cards(&mut s, ps, &md.system_statistics);
        }
    }
    collect_windows(&h.time_window_statistics, &mut s);
    if s.tps_series.is_empty() || s.mspt_series.is_empty() {
        if let Some(md) = &h.metadata {
            collect_metrics(&md.metrics, &mut s);
        }
    }
    s
}

// ---------------------------------------------------------------- sampler

fn build_sampler_summary(p: SamplerData, file_name: String) -> SparkSummary {
    let mut s = SparkSummary::new(SparkFileKind::Profile, file_name);
    if let Some(md) = &p.metadata {
        if let Some(pm) = &md.platform_metadata {
            let mc = if pm.minecraft_version.is_empty() {
                "?".to_string()
            } else {
                pm.minecraft_version.clone()
            };
            s.meta_lines.push(format!(
                "平台：{} {}（MC {}）",
                pm.name, pm.version, mc
            ));
            if !pm.spark_version.is_empty() {
                s.meta_lines.push(format!("Spark 版本：{}", pm.spark_version));
            }
        }
        if md.start_time > 0 && md.end_time > 0 {
            s.meta_lines.push(format!(
                "采样区间：{} → {}（{} 秒）",
                fmt_ts_millis(md.start_time),
                fmt_ts_millis(md.end_time),
                (md.end_time - md.start_time).max(0) / 1000
            ));
        }
        if md.interval > 0 {
            s.meta_lines.push(format!(
                "采样间隔：{}ms，共 {} ticks",
                md.interval, md.number_of_ticks
            ));
        }
        let mode = match md.sampler_mode {
            1 => "ALLOCATION（内存分配采样）",
            _ => "EXECUTION（执行耗时采样）",
        };
        let engine = match md.sampler_engine {
            1 => "ASYNC",
            _ => "JAVA",
        };
        s.meta_lines.push(format!("采样模式：{mode} / 引擎：{engine}"));
        if !md.comment.is_empty() {
            s.meta_lines.push(format!("备注：{}", md.comment));
        }
        if let Some(ps) = &md.platform_statistics {
            build_platform_cards(&mut s, ps, &md.system_statistics);
        }
    }
    collect_windows(&p.time_window_statistics, &mut s);
    if s.tps_series.is_empty() || s.mspt_series.is_empty() {
        if let Some(md) = &p.metadata {
            collect_metrics(&md.metrics, &mut s);
        }
    }
    aggregate_tree(&p, &mut s);
    s
}

// ---------------------------------------------------------------- 概览卡片

fn build_platform_cards(
    s: &mut SparkSummary,
    ps: &PlatformStatistics,
    ss: &Option<SystemStatistics>,
) {
    // TPS
    if let Some(tps) = &ps.tps {
        s.cards.push(OverviewCard::new(
            "TPS (1m/5m/15m)",
            format!("{:.1} / {:.1} / {:.1}", tps.last1m, tps.last5m, tps.last15m),
            tps.last1m < TPS_WARN,
        ));
    }
    // MSPT
    if let Some(mspt) = &ps.mspt {
        let v = |f: Option<&RollingAverageValues>, g: fn(&RollingAverageValues) -> f64| {
            f.map(g).unwrap_or(f64::NAN)
        };
        let mean = v(mspt.last1m.as_ref(), |x| x.mean);
        let median = v(mspt.last1m.as_ref(), |x| x.median);
        let p95 = v(mspt.last1m.as_ref(), |x| x.percentile95);
        let max = v(mspt.last1m.as_ref(), |x| x.max);
        s.cards.push(OverviewCard::new(
            "MSPT (mean/med/p95/max)",
            format!("{:.1} / {:.1} / {:.1} / {:.1} ms", mean, median, p95, max),
            mean > MSPT_WARN,
        ));
    }
    // 堆内存
    if let Some(mem) = &ps.memory {
        if let Some(heap) = &mem.heap {
            let used_mb = heap.used as f64 / 1048576.0;
            let max_mb = heap.max as f64 / 1048576.0;
            let pct = if heap.max > 0 {
                heap.used as f64 / heap.max as f64 * 100.0
            } else {
                0.0
            };
            s.cards.push(OverviewCard::new(
                "堆内存",
                format!("{:.0} / {:.0} MB（{:.0}%）", used_mb, max_mb, pct),
                pct > 85.0,
            ));
        }
    }
    // GC
    if !ps.gc.is_empty() {
        let mut lines = Vec::new();
        let mut warn = false;
        for (name, gc) in &ps.gc {
            lines.push(format!("{name}: {:.1}ms/次", gc.avg_time));
            if gc.avg_time > 50.0 {
                warn = true;
            }
        }
        s.cards.push(OverviewCard::new("GC", lines.join("；"), warn));
    }
    // 玩家
    if ps.player_count >= 0 {
        s.cards.push(OverviewCard::new("玩家", ps.player_count.to_string(), false));
    }
    // 世界统计（实体数）
    if let Some(w) = &ps.world {
        s.cards.push(OverviewCard::new("实体", w.total_entities.to_string(), false));
        if !w.worlds.is_empty() {
            let mut lines = Vec::new();
            for wd in &w.worlds {
                lines.push(format!("{}: 实体{}", wd.name, wd.total_entities));
            }
            s.cards.push(OverviewCard::new("世界", lines.join("；"), false));
        }
    }
    // CPU
    if let Some(sys) = ss {
        if let Some(cpu) = &sys.cpu {
            let p = cpu
                .process_usage
                .as_ref()
                .map(|u| pct_fmt(u.last1m))
                .unwrap_or(f64::NAN);
            let sy = cpu
                .system_usage
                .as_ref()
                .map(|u| pct_fmt(u.last1m))
                .unwrap_or(f64::NAN);
            s.cards.push(OverviewCard::new(
                "CPU 进程/系统",
                format!("{:.1}% / {:.1}%", p, sy),
                p > 85.0,
            ));
        }
    }
    // 区块（取窗口序列最后一窗的 chunks）
    let chunks = s
        .mspt_series
        .last()
        .and_then(|_| None)
        .unwrap_or(0);
    if chunks > 0 {
        s.cards.push(OverviewCard::new("区块", chunks.to_string(), false));
    }
}

/// spark CPU usage：0-1 比例与 0-100 百分比两种口径，统一成百分比
fn pct_fmt(v: f64) -> f64 {
    if v <= 1.0 {
        v * 100.0
    } else {
        v
    }
}

// ---------------------------------------------------------------- 时间序列

fn collect_windows(m: &HashMap<i32, WindowStatistics>, s: &mut SparkSummary) {
    if m.is_empty() {
        return;
    }
    let mut keys: Vec<i32> = m.keys().copied().collect();
    keys.sort_unstable();
    for k in &keys {
        let w = &m[k];
        s.tps_series.push((*k as f64, w.tps));
        s.mspt_series.push((*k as f64, w.mspt_median));
    }
    s.series_unit = "窗口";
    // 区块数补充（概览卡用最后窗口值）
    if s.cards.iter().any(|c| c.label == "区块") == false {
        if let Some(w) = m.get(keys.last().unwrap()) {
            if w.chunks > 0 {
                s.cards.push(OverviewCard::new("区块", w.chunks.to_string(), false));
            }
        }
    }
}

fn collect_metrics(m: &Option<Metrics>, s: &mut SparkSummary) {
    let Some(m) = m else { return };
    // TPS 序列
    if s.tps_series.is_empty() {
        if let Some(t) = &m.tps {
            let mut t_ms = t.start_timestamp_ms as f64;
            for (i, v) in t.values.iter().enumerate() {
                s.tps_series.push((t_ms / 1000.0, *v));
                if i < t.timestamp_deltas_ms.len() {
                    t_ms += t.timestamp_deltas_ms[i] as f64;
                }
            }
        }
    }
    // MSPT 序列（tick_duration 的 median）
    if s.mspt_series.is_empty() {
        if let Some(t) = &m.tick_duration {
            let mut t_ms = t.start_timestamp_ms as f64;
            for (i, v) in t.values.iter().enumerate() {
                s.mspt_series.push((t_ms / 1000.0, v.median));
                if i < t.timestamp_deltas_ms.len() {
                    t_ms += t.timestamp_deltas_ms[i] as f64;
                }
            }
        }
    }
    if !s.tps_series.is_empty() || !s.mspt_series.is_empty() {
        s.series_unit = "时间(s)";
    }
}

// ---------------------------------------------------------------- 调用树聚合（.sparkprofile）

/// 遍历 ThreadNode.children 扁平调用树（spark 序列化把每个线程的调用树节点扁平化为数组，
/// 每个节点的 children_refs 指向数组内子节点下标；同一共享节点只出现一次，self time 直接取
/// 节点 times 之和即可避免重复计数），归因到 class_sources 对应的模组/插件。
fn aggregate_tree(p: &SamplerData, s: &mut SparkSummary) {
    let mut nodes: Vec<(f64, String, String, String)> = Vec::new();
    for t in &p.threads {
        for n in &t.children {
            let st: f64 = n.times.iter().sum();
            if st <= 0.0 {
                continue;
            }
            let source = p
                .class_sources
                .get(&n.class_name)
                .cloned()
                .unwrap_or_else(|| "未知".to_string());
            nodes.push((st, n.class_name.clone(), n.method_name.clone(), source));
        }
    }
    if nodes.is_empty() {
        return;
    }
    let total: f64 = nodes.iter().map(|n| n.0).sum();
    if total <= 0.0 {
        return;
    }
    // 模组/插件聚合
    let mut mod_map: HashMap<String, f64> = HashMap::new();
    for (st, _, _, src) in &nodes {
        *mod_map.entry(src.clone()).or_default() += st;
    }
    let mut mods: Vec<(String, f64)> = mod_map.into_iter().collect();
    mods.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (name, st) in mods.into_iter().take(15) {
        s.mod_top.push(ModPerf {
            name,
            self_time: st,
            pct: st / total * 100.0,
        });
    }
    // 卡顿热点 TOP
    nodes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (st, cls, method, src) in nodes.into_iter().take(20) {
        s.hot_top.push(HotSpot {
            tag: type_tag(&cls),
            class: cls,
            method,
            self_time: st,
            pct: st / total * 100.0,
            source: src,
        });
    }
}

/// 类名 → 类型提示（帮助判断卡顿来源类型）
fn type_tag(class: &str) -> String {
    let l = class.to_lowercase();
    let mut tags = Vec::new();
    if l.contains("entity")
        || l.contains("mob")
        || l.contains("villager")
        || l.contains("player")
        || l.contains("animal")
        || l.contains("monster")
        || l.contains("zombie")
        || l.contains("skeleton")
        || l.contains("creeper")
    {
        tags.push("实体");
    }
    if l.contains("chunk") || l.contains("region") {
        tags.push("区块");
    }
    if l.contains("redstone") || l.contains("piston") || l.contains("repeater") || l.contains("comparator") {
        tags.push("红石");
    }
    if l.contains("block")
        || l.contains("tile")
        || l.contains("container")
        || l.contains("furnace")
        || l.contains("chest")
        || l.contains("hopper")
    {
        tags.push("方块");
    }
    if l.contains("pathfinding") || l.contains("navigation") || l.contains("brain") || l.contains("pathfinder") {
        tags.push("AI寻路");
    }
    if l.contains("network") || l.contains("packet") || l.contains("connection") {
        tags.push("网络");
    }
    if l.contains("fluid") || l.contains("liquid") || l.contains("water") || l.contains("lava") {
        tags.push("流体");
    }
    tags.join("/")
}

// ---------------------------------------------------------------- 工具

fn fmt_ts_millis(ms: i64) -> String {
    chrono::DateTime::from_timestamp(ms / 1000, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "-".to_string())
}

// ================================================================ UI 渲染

/// 「特殊功能」页：分析结果渲染（文件列表区点击条目后展示）
pub fn ui_analysis(ui: &mut egui::Ui, state: &SparkAnalysisState) {
    ui.separator();
    let kind = match state.summary.as_ref().map(|s| s.kind) {
        Some(SparkFileKind::Health) => ".sparkhealth",
        Some(SparkFileKind::Profile) => ".sparkprofile",
        None => "",
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("分析结果").strong());
        let name = state
            .path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&state.path);
        ui.label(RichText::new(format!("{name}（{kind}）")).weak().small());
    });

    // 解析失败：明确提示
    if let Some(err) = &state.parse_error {
        ui.colored_label(
            Color32::from_rgb(225, 95, 95),
            RichText::new(format!("解析失败：{err}")).strong(),
        );
        ui.label("提示：文件可能已损坏、不是 spark 输出文件，或 spark 版本 schema 不兼容。");
        return;
    }
    let Some(s) = &state.summary else { return };

    // 元信息
    for l in &s.meta_lines {
        ui.label(RichText::new(l).weak().small());
    }
    ui.add_space(4.0);

    // 概览卡片（阈值着色）
    ui.label(RichText::new("概览").strong());
    if s.cards.is_empty() {
        ui.label(RichText::new("无数据").weak());
    } else {
        egui::Grid::new(("spark_cards", &state.path))
            .num_columns(3)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                for c in &s.cards {
                    ui_card(ui, c);
                }
            });
    }

    // 波动曲线（TPS / MSPT，标注卡顿段）
    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new("TPS / MSPT 波动").strong());
    ui.label(RichText::new("红色区域/红点为低于阈值的卡顿时间段（TPS < 18 或 MSPT > 50）。").weak().small());
    draw_series(
        ui,
        "TPS",
        &s.tps_series,
        Some(TPS_WARN),
        Color32::from_rgb(96, 180, 255),
        s.series_unit,
    );
    draw_series(
        ui,
        "MSPT (ms)",
        &s.mspt_series,
        Some(MSPT_WARN),
        Color32::from_rgb(255, 170, 80),
        s.series_unit,
    );

    // 模组性能 TOP
    ui.separator();
    ui.label(RichText::new("模组性能 TOP").strong());
    if s.mod_top.is_empty() {
        ui.label(RichText::new("无数据").weak());
    } else {
        egui::Grid::new(("spark_mods", &state.path))
            .striped(true)
            .min_col_width(70.0)
            .show(ui, |ui| {
                ui.label(RichText::new("模组/插件").strong());
                ui.label(RichText::new("自耗时").strong());
                ui.label(RichText::new("占比").strong());
                ui.end_row();
                for m in &s.mod_top {
                    ui.label(RichText::new(&m.name).small());
                    ui.label(RichText::new(format!("{:.1}", m.self_time)).small());
                    ui.label(RichText::new(format!("{:.1}%", m.pct)).small());
                    ui.end_row();
                }
            });
    }

    // 卡顿热点 TOP
    ui.separator();
    ui.label(RichText::new("卡顿热点 TOP").strong());
    if s.hot_top.is_empty() {
        ui.label(RichText::new("无数据").weak());
    } else {
        egui::Grid::new(("spark_hot", &state.path))
            .striped(true)
            .min_col_width(70.0)
            .show(ui, |ui| {
                ui.label(RichText::new("方法").strong());
                ui.label(RichText::new("自耗时").strong());
                ui.label(RichText::new("占比").strong());
                ui.label(RichText::new("来源").strong());
                ui.label(RichText::new("类型").strong());
                ui.end_row();
                for h in &s.hot_top {
                    ui.label(RichText::new(format!("{}.{}", h.class, h.method)).small());
                    ui.label(RichText::new(format!("{:.1}", h.self_time)).small());
                    ui.label(RichText::new(format!("{:.1}%", h.pct)).small());
                    ui.label(RichText::new(&h.source).weak().small());
                    if h.tag.is_empty() {
                        ui.label("");
                    } else {
                        ui.label(RichText::new(&h.tag).color(Color32::from_rgb(240, 168, 82)).small());
                    }
                    ui.end_row();
                }
            });
    }
}

fn ui_card(ui: &mut egui::Ui, c: &OverviewCard) {
    let bg = if c.warn {
        Color32::from_rgb(92, 46, 46)
    } else {
        Color32::from_gray(34)
    };
    egui::Frame::none()
        .fill(bg)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.set_min_width(150.0);
            ui.label(RichText::new(c.label).weak().small());
            let color = if c.warn {
                Color32::from_rgb(240, 120, 110)
            } else {
                Color32::from_rgb(212, 222, 232)
            };
            ui.label(RichText::new(&c.value).color(color).strong());
        });
}

/// 简单折线绘制：网格 + 阈值线 + 折线 + 卡顿段高亮
fn draw_series(
    ui: &mut egui::Ui,
    title: &str,
    series: &[(f64, f64)],
    threshold: Option<f64>,
    color: Color32,
    unit: &str,
) {
    if series.is_empty() {
        ui.label(RichText::new(format!("{title}：无数据")).weak());
        return;
    }
    let width = ui.available_width().max(200.0);
    let height = 130.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, Color32::from_gray(26));

    // y 范围
    let mut ymin = f64::MAX;
    let mut ymax = f64::MIN;
    for (_, v) in series {
        ymin = ymin.min(*v);
        ymax = ymax.max(*v);
    }
    if let Some(th) = threshold {
        ymin = ymin.min(th);
        ymax = ymax.max(th);
    }
    ymin = ymin.min(0.0);
    if (ymax - ymin).abs() < 1e-9 {
        ymax = ymin + 1.0;
    }
    let range = (ymax - ymin).max(1e-9);

    // 水平网格
    for i in 0..=4 {
        let y = rect.top() + rect.height() * (i as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.5, Color32::from_gray(45)),
        );
    }

    // 阈值线
    if let Some(th) = threshold {
        let y = rect.bottom() - ((th - ymin) / range * rect.height() as f64) as f32;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, Color32::from_rgb(200, 70, 70)),
        );
        painter.text(
            egui::pos2(rect.left() + 4.0, (y - 13.0).max(rect.top())),
            egui::Align2::LEFT_TOP,
            format!("阈值 {th}"),
            egui::FontId::proportional(10.0),
            Color32::from_rgb(200, 90, 90),
        );
    }

    // 折线
    let n = series.len();
    let step = if n > 1 {
        rect.width() / (n - 1) as f32
    } else {
        rect.width()
    };
    let to_y = |v: f64| rect.bottom() - ((v - ymin) / range * rect.height() as f64) as f32;
    let points: Vec<egui::Pos2> = series
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let x = if n > 1 {
                rect.left() + i as f32 * step
            } else {
                rect.left() + rect.width() / 2.0
            };
            egui::pos2(x, to_y(*v))
        })
        .collect();
    painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(1.6, color)));

    // 卡顿段高亮：低于阈值点标红点 + 相邻点间底部红带
    if let Some(th) = threshold {
        for i in 0..n {
            if series[i].1 < th {
                painter.circle_filled(points[i], 2.2, Color32::from_rgb(230, 90, 90));
                if i + 1 < n && series[i + 1].1 < th {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(points[i].x, rect.top()),
                            egui::pos2(points[i + 1].x, rect.bottom()),
                        ),
                        0.0,
                        Color32::from_rgba_unmultiplied(200, 60, 60, 28),
                    );
                }
            }
        }
    }

    // 标题 + 范围标注
    let start = series.first().map(|p| p.0 as i64).unwrap_or(0);
    let end = series.last().map(|p| p.0 as i64).unwrap_or(0);
    let peak = series
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|p| p.1)
        .unwrap_or(0.0);
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        format!("{title} · {n} 点（{start}~{end} {unit}）峰值 {peak:.1}"),
        egui::FontId::proportional(11.0),
        Color32::from_gray(200),
    );
    ui.add_space(2.0);
}
