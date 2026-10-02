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

/// 概览卡片：核心指标 + 分级着色 + 可展开明细
#[derive(Clone)]
pub struct OverviewCard {
    pub label: &'static str,
    pub value: String,
    /// true = 警示着色（低于/高于阈值，兼容旧调用）
    pub warn: bool,
    /// 预留：健康着色
    pub ok: bool,
    /// 卡顿/健康等级：0=正常，1=偏高（黄），2=严重（红）
    pub level: u8,
    /// 展开后展示的明细行（萌新解读 / 维度明细等）
    pub detail: Vec<String>,
}

impl OverviewCard {
    fn new(label: &'static str, value: String, warn: bool) -> Self {
        let level = if warn { 2 } else { 0 };
        Self { label, value, warn, ok: false, level, detail: Vec::new() }
    }

    fn new_level(label: &'static str, value: String, level: u8) -> Self {
        Self {
            label,
            value,
            warn: level >= 2,
            ok: level == 0,
            level,
            detail: Vec::new(),
        }
    }

    fn detail(mut self, lines: Vec<String>) -> Self {
        self.detail = lines;
        self
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
    /// 堆内存波动序列 (x, used_mb)
    pub heap_series: Vec<(f64, f64)>,
    /// CPU 进程占用波动序列 (x, 百分比)
    pub cpu_series: Vec<(f64, f64)>,
    /// CPU 系统占用波动序列 (x, 百分比)
    pub cpu_sys_series: Vec<(f64, f64)>,
    /// 实体总数波动序列 (x, entities)
    pub entity_series: Vec<(f64, f64)>,
    /// 实体总数（概览卡）
    pub entity_total: i64,
    /// 实体按维度明细（维度ID, 实体数）
    pub entity_worlds: Vec<(String, i64)>,
    /// 序列 x 轴单位说明（"窗口" / "时间(s)"）
    pub series_unit: &'static str,
    /// 模组性能 TOP
    pub mod_top: Vec<ModPerf>,
    /// 卡顿热点 TOP
    pub hot_top: Vec<HotSpot>,
    /// 模组 → 热点方法明细（模组详情展开用；每模组最多保留 20 条）
    pub mod_hots: HashMap<String, Vec<HotSpot>>,
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
            heap_series: Vec::new(),
            cpu_series: Vec::new(),
            cpu_sys_series: Vec::new(),
            entity_series: Vec::new(),
            entity_total: -1,
            entity_worlds: Vec::new(),
            series_unit: "窗口",
            mod_top: Vec::new(),
            hot_top: Vec::new(),
            mod_hots: HashMap::new(),
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
    // TPS（1m 决定分级）
    if let Some(tps) = &ps.tps {
        let level = if tps.last1m < 15.0 {
            2
        } else if tps.last1m < TPS_WARN {
            1
        } else {
            0
        };
        let verdict = match level {
            2 => "严重卡顿：1 分钟平均低于 15，服务器基本跑不动",
            1 => "偏高：接近 18 阈值，可能偶发卡顿掉帧",
            _ => "正常：服务器节奏稳定",
        };
        s.cards.push(OverviewCard::new_level(
            "TPS (1m/5m/15m)",
            format!("{:.1} / {:.1} / {:.1}", tps.last1m, tps.last5m, tps.last15m),
            level,
        )
        .detail(vec![
            format!("1分钟平均 {:.1}", tps.last1m),
            format!("5分钟平均 {:.1}", tps.last5m),
            format!("15分钟平均 {:.1}", tps.last15m),
            format!("判定：{verdict}"),
        ]));
    }
    // MSPT（1m mean 决定分级）
    if let Some(mspt) = &ps.mspt {
        let v = |f: Option<&RollingAverageValues>, g: fn(&RollingAverageValues) -> f64| {
            f.map(g).unwrap_or(f64::NAN)
        };
        let mean = v(mspt.last1m.as_ref(), |x| x.mean);
        let median = v(mspt.last1m.as_ref(), |x| x.median);
        let p95 = v(mspt.last1m.as_ref(), |x| x.percentile95);
        let max = v(mspt.last1m.as_ref(), |x| x.max);
        let level = if mean > 70.0 {
            2
        } else if mean > MSPT_WARN {
            1
        } else {
            0
        };
        let verdict = match level {
            2 => "严重卡顿：每 tick 平均耗时超过 70ms，服务器很吃力",
            1 => "偏高：超过 50ms 阈值，玩家会感受到明显卡顿",
            _ => "正常：每 tick 在 50ms 内，节奏流畅",
        };
        s.cards.push(OverviewCard::new_level(
            "MSPT (mean/med/p95/max)",
            format!("{:.1} / {:.1} / {:.1} / {:.1} ms", mean, median, p95, max),
            level,
        )
        .detail(vec![
            format!("平均 {:.1} ms / 中位数 {:.1} ms", mean, median),
            format!("95% 分位 {:.1} ms / 最高 {:.1} ms", p95, max),
            format!("判定：{verdict}"),
        ]));
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
            let level = if pct > 90.0 {
                2
            } else if pct > 75.0 {
                1
            } else {
                0
            };
            let verdict = match level {
                2 => "严重：堆内存占用超 90%，随时可能触发频繁 GC 卡顿",
                1 => "偏高：超过 75%，建议关注内存增长趋势",
                _ => "正常：内存余量充足",
            };
            s.cards.push(OverviewCard::new_level(
                "堆内存",
                format!("{:.0} / {:.0} MB（{:.0}%）", used_mb, max_mb, pct),
                level,
            )
            .detail(vec![
                format!("已用 {:.0} MB / 上限 {:.0} MB", used_mb, max_mb),
                format!("占用率 {:.0}%", pct),
                format!("判定：{verdict}"),
            ]));
        }
    }
    // GC
    if !ps.gc.is_empty() {
        let mut lines = Vec::new();
        let mut detail = Vec::new();
        let mut warn = false;
        let mut worst = 0.0f64;
        for (name, gc) in &ps.gc {
            lines.push(format!("{name}: {:.1}ms/次", gc.avg_time));
            detail.push(format!("{name}: 平均 {:.1} ms/次", gc.avg_time));
            if gc.avg_time > 50.0 {
                warn = true;
            }
            worst = worst.max(gc.avg_time);
        }
        let level = if worst > 100.0 {
            2
        } else if warn {
            1
        } else {
            0
        };
        s.cards.push(
            OverviewCard::new_level("GC", lines.join("；"), level).detail(detail),
        );
    }
    // 玩家
    if ps.player_count >= 0 {
        s.cards.push(OverviewCard::new("玩家", ps.player_count.to_string(), false));
    }
    // 世界统计（实体总数 + 按维度明细）
    if let Some(w) = &ps.world {
        s.entity_total = w.total_entities as i64;
        let mut detail = Vec::new();
        for wd in &w.worlds {
            s.entity_worlds.push((wd.name.clone(), wd.total_entities as i64));
            detail.push(format!(
                "{}：实体 {}",
                dim_friendly_name(&wd.name),
                wd.total_entities
            ));
        }
        if detail.is_empty() {
            detail.push("无维度细分数据".to_string());
        }
        s.cards.push(OverviewCard::new(
            "实体",
            w.total_entities.to_string(),
            false,
        )
        .detail(detail));
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
            let level = if p > 85.0 {
                2
            } else if p > 60.0 {
                1
            } else {
                0
            };
            let verdict = match level {
                2 => "严重：进程占用 CPU 超 85%，服务器主线程被大量挤占",
                1 => "偏高：超过 60%，可能影响主线程稳定",
                _ => "正常：CPU 占用在健康范围",
            };
            s.cards.push(
                OverviewCard::new_level(
                    "CPU 进程/系统",
                    format!("{:.1}% / {:.1}%", p, sy),
                    level,
                )
                .detail(vec![
                    format!("服务器进程 {:.1}%", p),
                    format!("整机系统 {:.1}%", sy),
                    format!("判定：{verdict}"),
                ]),
            );
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

/// 图表刻度/数值统一格式化：按量级缩写（1000→1.0K、1000000→1.0M），负数保留符号，小数保留合适精度
fn format_tick(v: f64) -> String {
    let neg = v < 0.0;
    let a = v.abs();
    let s = if a >= 1_000_000.0 {
        format!("{:.1}M", a / 1_000_000.0)
    } else if a >= 1_000.0 {
        format!("{:.1}K", a / 1_000.0)
    } else if a.fract().abs() < 1e-9 {
        format!("{}", a as i64)
    } else {
        format!("{:.1}", a)
    };
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

/// 维度 ID → 萌新友好名；未知维度原样显示（适配任意模组维度）
fn dim_friendly_name(id: &str) -> String {
    match id.trim().to_lowercase().as_str() {
        "minecraft:overworld" => "主世界".to_string(),
        "minecraft:the_nether" | "minecraft:nether" => "下界".to_string(),
        "minecraft:the_end" | "minecraft:end" => "末地".to_string(),
        "" => "未知维度".to_string(),
        other => {
            // 去掉 minecraft: 前缀的裸名也做映射（部分整合包数据只有裸名）
            let bare = other.rsplit(':').next().unwrap_or(other);
            match bare {
                "overworld" => "主世界".to_string(),
                "the_nether" | "nether" => "下界".to_string(),
                "the_end" | "end" => "末地".to_string(),
                _ => other.to_string(),
            }
        }
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
        s.cpu_series.push((*k as f64, pct_fmt(w.cpu_process)));
        s.cpu_sys_series.push((*k as f64, pct_fmt(w.cpu_system)));
        s.entity_series.push((*k as f64, w.entities as f64));
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
    // CPU 序列（进程 / 系统，spark 为 0-1 比例，统一转百分比）
    if s.cpu_series.is_empty() {
        if let Some(t) = &m.cpu_usage_process {
            let mut t_ms = t.start_timestamp_ms as f64;
            for (i, v) in t.values.iter().enumerate() {
                s.cpu_series.push((t_ms / 1000.0, pct_fmt(*v)));
                if i < t.timestamp_deltas_ms.len() {
                    t_ms += t.timestamp_deltas_ms[i] as f64;
                }
            }
        }
    }
    if s.cpu_sys_series.is_empty() {
        if let Some(t) = &m.cpu_usage_system {
            let mut t_ms = t.start_timestamp_ms as f64;
            for (i, v) in t.values.iter().enumerate() {
                s.cpu_sys_series.push((t_ms / 1000.0, pct_fmt(*v)));
                if i < t.timestamp_deltas_ms.len() {
                    t_ms += t.timestamp_deltas_ms[i] as f64;
                }
            }
        }
    }
    // 堆内存序列（used → MB）
    if s.heap_series.is_empty() {
        if let Some(t) = &m.memory_usage_heap {
            let mut t_ms = t.start_timestamp_ms as f64;
            for (i, v) in t.values.iter().enumerate() {
                s.heap_series.push((t_ms / 1000.0, v.used as f64 / 1048576.0));
                if i < t.timestamp_deltas_ms.len() {
                    t_ms += t.timestamp_deltas_ms[i] as f64;
                }
            }
        }
    }
    // 实体总数序列
    if s.entity_series.is_empty() {
        if let Some(t) = &m.world_info {
            let mut t_ms = t.start_timestamp_ms as f64;
            for (i, v) in t.values.iter().enumerate() {
                s.entity_series.push((t_ms / 1000.0, v.entities as f64));
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
    for (st, cls, method, src) in nodes.iter().take(20) {
        s.hot_top.push(HotSpot {
            tag: type_tag(cls),
            class: cls.clone(),
            method: method.clone(),
            self_time: *st,
            pct: *st / total * 100.0,
            source: src.clone(),
        });
    }
    // 模组 → 热点明细（详情展开用；每模组最多保留 20 条，按自耗时排序取前 20）
    for (st, cls, method, src) in nodes {
        let v = s.mod_hots.entry(src.clone()).or_default();
        if v.len() >= 20 {
            continue;
        }
        v.push(HotSpot {
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

/// 分析结果预览子页签（仿服务器顶栏可切换式布局，避免单页纵向堆叠在非最大化窗口下超界）
#[derive(PartialEq, Clone, Copy)]
pub enum SparkViewTab {
    Overview,
    Charts,
    Mods,
    Hotspots,
}

impl SparkViewTab {
    pub fn all() -> [SparkViewTab; 4] {
        [
            SparkViewTab::Overview,
            SparkViewTab::Charts,
            SparkViewTab::Mods,
            SparkViewTab::Hotspots,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            SparkViewTab::Overview => "概览",
            SparkViewTab::Charts => "波动曲线",
            SparkViewTab::Mods => "模组性能",
            SparkViewTab::Hotspots => "卡顿热点",
        }
    }
}

/// mods 目录下单个 jar 的定位信息（jar 文件名 + 元数据别名：fabric.mod.json 的 id/name、mods.toml 的 modId/displayName）
#[derive(Clone, Debug)]
pub struct ModJarInfo {
    pub jar: String,
    pub aliases: Vec<String>,
}

/// 根据 spark 归因来源（模组/插件名）在 mods 目录 jar 索引中查找对应 jar 文件名。
/// 匹配打分制（分数越高越精确，取最高分；同分取索引靠前者）：
/// - 3 分：jar 名（去扩展名）/ 别名与来源名完全相等
/// - 2 分：jar 名/别名以来源名为前缀（扩展包自身能精确命中，如 "carpet-extra"）
/// - 1 分：来源名以 jar 名为前缀（扩展包被归因到本体时的次优回退，如 "carpet-extra" → "carpet"）
/// - 0 分：仅双向包含（兜底，短名不再误吞长名）
pub fn match_mod_jar(index: &[ModJarInfo], source: &str) -> Option<String> {
    let q = source.trim().to_lowercase();
    if q.is_empty() || q == "未知" || q == "minecraft" {
        return None;
    }
    let mut best: Option<(i32, String)> = None;
    for info in index {
        let jar_low = info.jar.to_lowercase();
        let jar_stem = jar_low.strip_suffix(".jar").unwrap_or(&jar_low);
        let mut score = score_match(jar_stem, &q);
        for a in &info.aliases {
            score = score.max(score_match(&a.to_lowercase(), &q));
        }
        if score >= 0 {
            let better = match &best {
                Some((bs, _)) => score > *bs,
                None => true,
            };
            if better {
                best = Some((score, info.jar.clone()));
            }
        }
    }
    best.map(|(_, jar)| jar)
}

/// 单个候选名（jar 名或别名）与来源名的匹配得分；完全不匹配返回 -1
fn score_match(candidate: &str, q: &str) -> i32 {
    if candidate.is_empty() {
        return -1;
    }
    if candidate == q {
        return 3;
    }
    if q.chars().count() >= 3 && candidate.starts_with(q) {
        // 候选名以来源名为前缀（如候选 "carpet-extra-1.4.6"，来源 "carpet-extra"）
        return 2;
    }
    if q.starts_with(candidate) {
        // 来源名以候选名为前缀：仅当剩余部分是分隔符时才视为扩展归因到本体
        let rest = &q[candidate.len()..];
        if rest.starts_with('-') || rest.starts_with('_') || rest.starts_with('.') {
            return 1;
        }
    }
    // 兜底：双向包含（过滤过短来源名）
    if q.chars().count() >= 3 && (candidate.contains(q) || q.contains(candidate)) {
        return 0;
    }
    -1
}

/// 定位按钮：解析模组来源 → 对应 mods jar；找不到则禁用
fn locate_btn(
    ui: &mut egui::Ui,
    source: &str,
    jar_index: &[ModJarInfo],
    locate: &mut dyn FnMut(String),
) {
    let Some(jar) = match_mod_jar(jar_index, source) else {
        ui.add_enabled(false, egui::Button::new("定位"))
            .on_hover_text("未在 mods 目录找到对应 jar（可能是插件、内置或已改名）");
        return;
    };
    if ui
        .button("📍 定位")
        .on_hover_text(format!("跳转到文件浏览 mods 目录并定位 {jar}"))
        .clicked()
    {
        locate(jar);
    }
}

/// 「特殊功能」页：分析结果渲染（页签化布局，避免单页纵向堆叠在非最大化窗口下超界）
pub fn ui_analysis(
    ui: &mut egui::Ui,
    state: &SparkAnalysisState,
    view_tab: &mut SparkViewTab,
    mod_detail: &mut Option<String>,
    jar_index: &[ModJarInfo],
    locate: &mut dyn FnMut(String),
) {
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

    // 页签栏（仿服务器顶栏可切换式布局）
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for tab in SparkViewTab::all() {
            let selected = *view_tab == tab;
            let btn = egui::Button::new(RichText::new(tab.label()).strong())
                .selected(selected)
                .min_size(egui::vec2(0.0, 26.0));
            if ui.add(btn).clicked() {
                *view_tab = tab;
                *mod_detail = None;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("「📍 定位」跳转文件浏览对应模组位置")
                    .weak()
                    .small(),
            );
        });
    });
    ui.separator();

    // 元信息（仅概览页展示，避免每页重复占用空间）
    if *view_tab == SparkViewTab::Overview {
        for l in &s.meta_lines {
            ui.label(RichText::new(l).weak().small());
        }
        ui.add_space(4.0);
    }

    let inner = egui::ScrollArea::vertical()
        .id_salt(("spark_view", &state.path))
        .auto_shrink([false, false]);
    match *view_tab {
        SparkViewTab::Overview => {
            inner.show(ui, |ui| {
                ui_overview(ui, s);
            });
        }
        SparkViewTab::Charts => {
            inner.show(ui, |ui| {
                ui.label(RichText::new("TPS / MSPT 波动").strong());
                ui.label(
                    RichText::new("红色区域/红点为低于阈值的卡顿时间段（TPS < 18 或 MSPT > 50）。")
                        .weak()
                        .small(),
                );
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
            });
        }
        SparkViewTab::Mods => {
            inner.show(ui, |ui| {
                ui_mods(ui, s, jar_index, locate, mod_detail);
            });
        }
        SparkViewTab::Hotspots => {
            inner.show(ui, |ui| {
                ui_hotspots(ui, s, jar_index, locate);
            });
        }
    }
}

/// 模组性能页：模组 TOP + 详情展开（分析模组哪部分出问题）
fn ui_mods(
    ui: &mut egui::Ui,
    s: &SparkSummary,
    jar_index: &[ModJarInfo],
    locate: &mut dyn FnMut(String),
    mod_detail: &mut Option<String>,
) {
    ui.label(RichText::new("模组性能 TOP").strong());
    ui.label(
        RichText::new(
            "下面按“谁占用卡顿多”排序：进度条越长表示这个模组越可能是卡顿元凶。点击「定位」跳到对应文件，点击「详情」看它具体卡在哪些方法。",
        )
        .weak()
        .small(),
    );
    if s.mod_top.is_empty() {
        ui.label(RichText::new("无数据").weak());
        return;
    }
    egui::Grid::new(("spark_mods", &s.file_name))
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.label(RichText::new("模组/插件").strong());
            ui.label(RichText::new("占用占比").strong());
            ui.label(RichText::new("自耗时").strong());
            ui.label(RichText::new("操作").strong());
            ui.end_row();
            for m in &s.mod_top {
                ui.label(RichText::new(&m.name).small());
                if m.pct <= 0.0 {
                    // 0% 条目不绘制条身与百分比文本，弱化占位，避免空条视觉噪音
                    ui.label(RichText::new("—").weak().small());
                } else {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add(egui::ProgressBar::new((m.pct / 100.0) as f32)
                            .desired_width(120.0));
                        ui.label(RichText::new(format!("{:.1}%", m.pct)).small());
                    });
                }
                ui.label(RichText::new(format!("{:.1}", m.self_time)).small());
                ui.horizontal(|ui| {
                    locate_btn(ui, &m.name, jar_index, locate);
                    let detail_open = mod_detail.as_deref() == Some(m.name.as_str());
                    if ui
                        .button(if detail_open { "收起" } else { "详情" })
                        .on_hover_text("展开该模组的热点方法明细，定位问题出在模组哪部分")
                        .clicked()
                    {
                        if detail_open {
                            *mod_detail = None;
                        } else {
                            *mod_detail = Some(m.name.clone());
                        }
                    }
                });
                ui.end_row();
            }
        });

    // 详情区：当前选中模组的热点方法明细（表格下方独立展示）
    if let Some(detail) = mod_detail {
        let Some(m) = s.mod_top.iter().find(|m| &m.name == detail) else {
            *mod_detail = None;
            return;
        };
        ui.add_space(6.0);
        ui.separator();
        let mut close_detail = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("「{detail}」热点方法明细")).strong());
            ui.label(
                RichText::new(format!(
                    "自耗时 {:.1}，占比 {:.1}%",
                    m.self_time, m.pct
                ))
                .weak()
                .small(),
            );
            if ui.button("关闭详情").clicked() {
                close_detail = true;
            }
        });
        if close_detail {
            *mod_detail = None;
            return;
        }

        let hits = s.mod_hots.get(detail).cloned().unwrap_or_default();
        if hits.is_empty() {
            ui.label(
                RichText::new(
                    "该模组未产生自耗时热点（耗时可能集中在依赖库、反射调用或已被其他模组归因）。",
                )
                .weak()
                .small(),
            );
            return;
        }
        // 类型分布统计
        let mut tag_map: HashMap<&str, usize> = HashMap::new();
        for h in &hits {
            for t in h.tag.split('/').filter(|t| !t.is_empty()) {
                *tag_map.entry(t).or_default() += 1;
            }
        }
        let mut tags: Vec<(&str, usize)> = tag_map.into_iter().collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1));
        if !tags.is_empty() {
            ui.label(
                RichText::new(format!(
                    "类型分布：{}",
                    tags.iter()
                        .map(|(t, n)| format!("{t}×{n}"))
                        .collect::<Vec<_>>()
                        .join("，")
                ))
                .weak()
                .small(),
            );
        }
        egui::Grid::new(("spark_mod_detail", &s.file_name))
            .min_col_width(60.0)
            .show(ui, |ui| {
                ui.label(RichText::new("方法").strong());
                ui.label(RichText::new("自耗时").strong());
                ui.label(RichText::new("占比").strong());
                ui.label(RichText::new("类型").strong());
                ui.end_row();
                for h in &hits {
                    ui.label(RichText::new(format!("{}.{}", h.class, h.method)).small());
                    ui.label(RichText::new(format!("{:.1}", h.self_time)).small());
                    if h.pct <= 0.0 {
                        // 0% 条目不绘制条身与百分比文本，弱化占位
                        ui.label(RichText::new("—").weak().small());
                    } else {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add(egui::ProgressBar::new((h.pct / 100.0) as f32)
                                .desired_width(90.0));
                            ui.label(RichText::new(format!("{:.1}%", h.pct)).small());
                        });
                    }
                    if h.tag.is_empty() {
                        ui.label("");
                    } else {
                        ui.label(
                            RichText::new(&h.tag)
                                .color(Color32::from_rgb(240, 168, 82))
                                .small(),
                        );
                    }
                    ui.end_row();
                }
            });
        // 一句话定位建议
        let top = &hits[0];
        let advice = if let Some((tag, _)) = tags.first() {
            format!(
                "主要热点集中在「{tag}」相关逻辑（{}.{}，占全文件自耗 {:.1}%），优先检查该部分。",
                top.class, top.method, top.pct
            )
        } else {
            format!(
                "主要热点：{}.{}（占全文件自耗 {:.1}%），优先检查该方法及其调用链。",
                top.class, top.method, top.pct
            )
        };
        ui.add_space(2.0);
        ui.label(RichText::new(advice).color(Color32::from_rgb(180, 220, 160)).small());
    }
}

/// 卡顿热点页：热点 TOP + 逐条定位
fn ui_hotspots(
    ui: &mut egui::Ui,
    s: &SparkSummary,
    jar_index: &[ModJarInfo],
    locate: &mut dyn FnMut(String),
) {
    ui.label(RichText::new("卡顿热点 TOP").strong());
    ui.label(
        RichText::new(
            "这些是“最烧时间”的代码热点，进度条越长越卡。来源显示它属于哪个模组，点击「定位」跳到对应文件。",
        )
        .weak()
        .small(),
    );
    if s.hot_top.is_empty() {
        ui.label(RichText::new("无数据").weak());
        return;
    }
    egui::Grid::new(("spark_hot", &s.file_name))
        .min_col_width(60.0)
        .show(ui, |ui| {
            ui.label(RichText::new("方法").strong());
            ui.label(RichText::new("占比").strong());
            ui.label(RichText::new("自耗时").strong());
            ui.label(RichText::new("来源").strong());
            ui.label(RichText::new("类型").strong());
            ui.label(RichText::new("操作").strong());
            ui.end_row();
            for h in &s.hot_top {
                ui.label(RichText::new(format!("{}.{}", h.class, h.method)).small());
                if h.pct <= 0.0 {
                    // 0% 条目不绘制条身与百分比文本，弱化占位
                    ui.label(RichText::new("—").weak().small());
                } else {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add(egui::ProgressBar::new((h.pct / 100.0) as f32)
                            .desired_width(90.0));
                        ui.label(RichText::new(format!("{:.1}%", h.pct)).small());
                    });
                }
                ui.label(RichText::new(format!("{:.1}", h.self_time)).small());
                ui.label(RichText::new(&h.source).weak().small());
                if h.tag.is_empty() {
                    ui.label("");
                } else {
                    ui.label(
                        RichText::new(&h.tag)
                            .color(Color32::from_rgb(240, 168, 82))
                            .small(),
                    );
                }
                locate_btn(ui, &h.source, jar_index, locate);
                ui.end_row();
            }
        });
}

/// 概览页：结论横幅 + 摘要卡网格 + 可展开指标明细（曲线 / 维度 / 解读）
fn ui_overview(ui: &mut egui::Ui, s: &SparkSummary) {
    ui.label(RichText::new("概览").strong());
    if s.cards.is_empty() {
        ui.label(RichText::new("无数据").weak());
        return;
    }

    // 1) 总体结论横幅：按最高等级指标给出萌新可读的一句话结论
    let mut severe: Vec<&str> = Vec::new();
    let mut warn: Vec<&str> = Vec::new();
    for c in &s.cards {
        match c.level {
            2 => severe.push(c.label),
            1 => warn.push(c.label),
            _ => {}
        }
    }
    let (banner, banner_bg, banner_fg) = if !severe.is_empty() {
        (
            format!("卡顿严重：{} 亮红灯，建议优先排查这几个指标", severe.join("、")),
            Color32::from_rgb(96, 42, 42),
            Color32::from_rgb(240, 130, 120),
        )
    } else if !warn.is_empty() {
        (
            format!("偶有卡顿：{} 偏高，建议留意对应方向", warn.join("、")),
            Color32::from_rgb(90, 74, 34),
            Color32::from_rgb(235, 190, 95),
        )
    } else {
        (
            "运行流畅：各项指标均在健康范围".to_string(),
            Color32::from_rgb(30, 72, 46),
            Color32::from_rgb(140, 215, 165),
        )
    };
    egui::Frame::none()
        .fill(banner_bg)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(format!("【结论】{banner}")).color(banner_fg).strong());
        });
    ui.add_space(6.0);

    // 2) 摘要卡网格（自适应列数：窄窗口 2 列防溢出）
    let spacing = 10.0f32;
    let cols = if ui.available_width() < 560.0 { 2 } else { 3 };
    let col_w = ((ui.available_width() - spacing * (cols as f32 - 1.0)) / cols as f32)
        .max(140.0)
        .min(340.0);
    for chunk in s.cards.chunks(cols) {
        ui.horizontal(|ui| {
            for c in chunk {
                ui.allocate_ui(egui::vec2(col_w, 0.0), |ui| {
                    ui.set_width(col_w);
                    ui_card(ui, c);
                });
            }
        });
    }

    // 3) 指标明细折叠区：点击展开看波动曲线 / 维度明细 / 萌新解读
    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new("指标明细（点击行展开）").strong());
    ui.label(
        RichText::new("展开后可查看该指标这段时间的波动曲线与解读；红色=严重，黄色=偏高，绿色=正常。")
            .weak()
            .small(),
    );

    let find = |prefix: &str| s.cards.iter().find(|c| c.label.starts_with(prefix));

    // TPS
    if let Some(c) = find("TPS") {
        ui_metric_collapse(
            ui,
            ("spark_ov_tps", &s.file_name),
            metric_header("TPS（每秒游戏刻）", c),
            &c.detail,
            &[],
            &s.tps_series,
            "TPS",
            Color32::from_rgb(96, 180, 255),
            Some(TPS_WARN),
            s.series_unit,
            "低于 18 表示卡顿，曲线掉到红线以下就是卡顿时段",
        );
    }
    // MSPT
    if let Some(c) = find("MSPT") {
        ui_metric_collapse(
            ui,
            ("spark_ov_mspt", &s.file_name),
            metric_header("MSPT（每刻耗时）", c),
            &c.detail,
            &[],
            &s.mspt_series,
            "MSPT (ms)",
            Color32::from_rgb(255, 170, 80),
            Some(MSPT_WARN),
            s.series_unit,
            "每刻超过 50ms 玩家就会感到卡顿，数值越高越卡",
        );
    }
    // 堆内存
    if let Some(c) = find("堆内存") {
        ui_metric_collapse(
            ui,
            ("spark_ov_heap", &s.file_name),
            metric_header("堆内存", c),
            &c.detail,
            &[],
            &s.heap_series,
            "堆内存 (MB)",
            Color32::from_rgb(110, 210, 140),
            None,
            s.series_unit,
            "内存占用越高越接近上限，就越容易触发 GC 停顿",
        );
    }
    // CPU
    if let Some(c) = find("CPU") {
        ui_metric_collapse(
            ui,
            ("spark_ov_cpu", &s.file_name),
            metric_header("CPU 占用", c),
            &c.detail,
            &s.cpu_sys_series,
            &s.cpu_series,
            "CPU 进程占用 (%)",
            Color32::from_rgb(200, 140, 255),
            None,
            s.series_unit,
            "进程占用指服务器自己吃掉的 CPU，越高越容易挤占主线程",
        );
    }
    // GC
    if let Some(c) = find("GC") {
        ui_metric_collapse(
            ui,
            ("spark_ov_gc", &s.file_name),
            metric_header("GC（内存回收）", c),
            &c.detail,
            &[],
            &[],
            "",
            Color32::from_gray(160),
            None,
            s.series_unit,
            "GC 每次停顿越长，服务器越容易周期性卡顿",
        );
    }
    // 实体（总共和维度分开）
    if let Some(c) = find("实体") {
        ui_metric_collapse(
            ui,
            ("spark_ov_ent", &s.file_name),
            metric_header("实体", c),
            &c.detail,
            &[],
            &s.entity_series,
            "实体总数",
            Color32::from_rgb(240, 210, 90),
            None,
            s.series_unit,
            "实体越多，每刻 tick 负担越重；展开看哪个维度实体最多",
        );
    }
}

/// 折叠区头文案：标签 + 当前值 + 等级徽标
fn metric_header(label: &str, c: &OverviewCard) -> egui::RichText {
    let color = match c.level {
        2 => Color32::from_rgb(240, 120, 110),
        1 => Color32::from_rgb(235, 190, 90),
        _ => Color32::from_rgb(212, 222, 232),
    };
    let badge = match c.level {
        2 => "⚠ 严重",
        1 => "▲ 偏高",
        _ => "✓ 正常",
    };
    RichText::new(format!("{label} — {}（{badge}）", c.value))
        .color(color)
        .strong()
}

/// 单个指标折叠区：标题 + 明细行 + 一条/两条曲线 + 萌新提示
#[allow(clippy::too_many_arguments)]
fn ui_metric_collapse(
    ui: &mut egui::Ui,
    id: (&str, &str),
    title: egui::RichText,
    detail: &[String],
    series_secondary: &[(f64, f64)],
    series: &[(f64, f64)],
    curve_title: &str,
    curve_color: Color32,
    threshold: Option<f64>,
    unit: &str,
    tip: &str,
) {
    egui::CollapsingHeader::new(title)
        .id_salt(id)
        .default_open(false)
        .show(ui, |ui| {
            for l in detail {
                ui.label(RichText::new(l).small());
            }
            draw_series(ui, "系统占用 (%)", series_secondary, None, Color32::from_rgb(255, 200, 120), unit);
            draw_series(ui, curve_title, series, threshold, curve_color, unit);
            ui.label(RichText::new(format!("提示：{tip}")).weak().small());
        });
}

fn ui_card(ui: &mut egui::Ui, c: &OverviewCard) {
    let (bg, fg) = match c.level {
        2 => (Color32::from_rgb(96, 42, 42), Color32::from_rgb(240, 120, 110)),
        1 => (Color32::from_rgb(90, 74, 34), Color32::from_rgb(235, 190, 90)),
        _ => (Color32::from_gray(34), Color32::from_rgb(212, 222, 232)),
    };
    egui::Frame::none()
        .fill(bg)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            // 宽度受调用方分配约束，防止长文本把卡片撑宽超出窗口右边界
            ui.set_min_width(ui.available_width().min(150.0));
            let mut head = RichText::new(c.label).weak().small();
            if c.level >= 1 {
                head = head.color(fg);
            }
            ui.label(head);
            ui.label(RichText::new(&c.value).color(fg).strong());
            if c.level >= 1 {
                ui.label(
                    RichText::new(if c.level == 2 { "⚠ 严重" } else { "▲ 偏高" })
                        .color(fg)
                        .small(),
                );
            }
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
        // 空图占位：底框 + 空心圆 + 居中弱化文字（不再空白/仅顶部一行小字）
        let width = ui.available_width().max(200.0);
        let height = 130.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, Color32::from_gray(26));
        let center = rect.center();
        painter.circle_stroke(
            center,
            20.0,
            egui::Stroke::new(1.5_f32, Color32::from_gray(90)),
        );
        painter.text(
            center + egui::vec2(0.0, 42.0),
            egui::Align2::CENTER_CENTER,
            "暂无数据",
            egui::FontId::proportional(12.0),
            Color32::from_gray(110),
        );
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
            format!("阈值 {}", format_tick(th)),
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
        format!("{title} · {n} 点（{}~{} {unit}）峰值 {}", format_tick(start as f64), format_tick(end as f64), format_tick(peak)),
        egui::FontId::proportional(11.0),
        Color32::from_gray(200),
    );
    ui.add_space(2.0);
}
