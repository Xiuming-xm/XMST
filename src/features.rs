//! 功能注册表：为 XMST 所有功能/选项分配稳定语义化 ID。
//!
//! 设计目的：
//! 1. 每个功能有唯一稳定 ID（语义化 slug，如 `server.backup.auto`），代码/配置/日志统一引用；
//! 2. 用户对功能的个性化覆盖（启用/禁用/显示/排序/样式）全部以 ID 为键持久化，
//!    缺省时用默认值，不膨胀配置文件；
//! 3. 后续自定义 UI（布局/拖拽排序/样式/显隐）直接基于本注册表驱动，无需再动硬编码。
//!
//! 当前使用者：
//! - 「测试中的功能」开关：列表完全由本注册表（`FeatureGroup::Beta`）生成，
//!   Beta 分组功能默认禁用，启用需警告确认；关闭 = enabled=false，
//!   UI 隐藏 + 后台调度拦截，不会出现"不显示但还在执行"。
//!
//! 新功能上线时必须在此登记一条 FeatureMeta（含 `desc` 一句话说明），
//! 否则无法被个性化系统识别，也不会出现在开关列表里。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 功能分组
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureGroup {
    /// 服务器管理（概览/状态/文件/备份/设置等）
    Server,
    /// 内网穿透（仪表盘/创建/管理/日志/流量）
    Tunnel,
    /// 设置页分组
    Settings,
    /// 全局工具/导航（导航入口、通知、托盘等）
    Tool,
    /// 测试中的功能（存在 Bug/未完成，默认禁用）
    Beta,
}

impl FeatureGroup {
    pub fn label(&self) -> &'static str {
        match self {
            FeatureGroup::Server => "服务器",
            FeatureGroup::Tunnel => "内网穿透",
            FeatureGroup::Settings => "设置",
            FeatureGroup::Tool => "工具",
            FeatureGroup::Beta => "测试中的功能",
        }
    }

    /// 分组展示顺序（升序），供注册表驱动的列表排序使用
    pub fn index(&self) -> u8 {
        match self {
            FeatureGroup::Beta => 0,
            FeatureGroup::Server => 1,
            FeatureGroup::Tunnel => 2,
            FeatureGroup::Settings => 3,
            FeatureGroup::Tool => 4,
        }
    }
}

/// 功能元数据（编译期静态登记，不可变）
pub struct FeatureMeta {
    /// 稳定语义化唯一 ID
    pub id: &'static str,
    /// 显示名
    pub name: &'static str,
    /// 一句话说明（开关列表里的"风险/说明"小字，必须填写）
    pub desc: &'static str,
    /// 所属分组
    pub group: FeatureGroup,
    /// 默认启用（false=默认禁用，仅测试功能使用）
    pub default_enabled: bool,
    /// 默认可见（false=默认隐藏 UI）
    pub default_visible: bool,
    /// 是否需要重启程序才完全生效（true 时界面标注"下次启动生效"）
    ///
    /// 目前所有已登记项都在运行时读取开关（后台调度每轮重新判断），
    /// 因此全部为 false（界面显示"立即生效"）；该字段留给将来
    /// "只能在启动时一次性装配"的功能。
    pub restart_required: bool,
    /// 默认排序（同组内升序）
    pub order: u32,
}

/// 用户覆盖状态（持久化到 GlobalConfig.features，缺省用默认值）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureState {
    /// 是否启用：false = UI 隐藏 + 后台调度拦截（彻底禁用）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 是否显示：false = 仅隐藏 UI，功能照常运行
    #[serde(default = "default_true")]
    pub visible: bool,
    /// 排序覆盖（None=默认顺序）；自定义 UI 预留
    #[serde(default)]
    pub order: Option<i32>,
}

fn default_true() -> bool {
    true
}

impl Default for FeatureState {
    fn default() -> Self {
        Self {
            enabled: true,
            visible: true,
            order: None,
        }
    }
}

/// ===== 测试中的功能 ID（Beta，默认禁用） =====
pub const BETA_BACKUP: &str = "beta.server.backup"; // 自动备份
pub const BETA_CRASH_ANALYSIS: &str = "beta.server.crash_analysis"; // 崩溃报告分析
pub const BETA_TRAFFIC: &str = "beta.tunnel.traffic"; // 内网穿透流量显示
pub const BETA_DOWNLOAD: &str = "beta.network.download"; // 网络下载（服务端下载 + 模组下载）
pub const BETA_PLAYERS: &str = "beta.server.players"; // 玩家管理（在线/白名单/封禁/OP 四 Tab）
pub const BETA_PLUGINS: &str = "beta.server.plugins"; // 插件系统（rhai 宿主 + zip 热加载 + 毛玻璃示范包）
pub const BETA_RATHOLE: &str = "beta.tunnel.rathole"; // 穿透内核 rathole（纯 Rust 反向代理）
pub const BETA_REMOTE_BACKUP: &str = "beta.server.remote_backup"; // 备份远端转存（本地/UNC/WebDAV）
pub const BETA_SPECIAL: &str = "beta.server.special"; // 特殊功能（Spark 分析）总开关【2026-10-02 移回测试功能，默认禁用】
pub const FEATURE_SPARK: &str = "server.special.spark"; // Spark 分析子开关【随总开关 BETA_SPECIAL 显示】
pub const BETA_MOD_UPDATE: &str = "beta.server.mod_update"; // 模组检查更新（SHA1 指纹 + modid 兜底尚未完成，默认禁用）

/// ===== 阶段 3 新功能 ID（默认启用，普通功能开关） =====

pub const FEATURE_FORCE_STOP_CONFIRM: &str = "server.force_stop_confirm"; // B3 强停二次确认（默认开）

/// ===== 全量注册表 =====
pub const REGISTRY: &[FeatureMeta] = &[
    // ---------- 服务器 ----------
    FeatureMeta {
        id: "server.overview",
        name: "概览",
        desc: "服务器首页：启动/停止、核心信息与常用快捷入口。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 10,
    },
    FeatureMeta {
        id: "server.status",
        name: "状态",
        desc: "实时查看进程 CPU/内存占用与运行时长。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 20,
    },
    FeatureMeta {
        id: "server.files",
        name: "文件",
        desc: "在界面里浏览、编辑服务器目录下的文件。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 30,
    },
    FeatureMeta {
        id: "server.backup",
        name: "自动功能",
        desc: "备份与自动重启等自动能力的集中入口。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 40,
    },
    FeatureMeta {
        id: "server.auto_restart",
        name: "自动重启",
        desc: "服务器异常退出后按倒计时自动重新启动。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 50,
    },
    FeatureMeta {
        id: "server.crash_restart",
        name: "崩溃重启",
        desc: "检测到崩溃后自动重启（反复崩溃时建议关闭）。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 51,
    },
    FeatureMeta {
        id: "server.whitelist",
        name: "白名单/黑名单管理",
        desc: "管理白名单、封禁与 OP 名单，改动直接写入服务端文件。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 60,
    },
    FeatureMeta {
        id: "server.properties",
        name: "服务器设置",
        desc: "图形化编辑 server.properties 与常用启动参数。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 61,
    },
    FeatureMeta {
        id: "server.scripts",
        name: "启动脚本",
        desc: "编辑 run.bat 与 JVM 参数等启动脚本。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 62,
    },
    FeatureMeta {
        id: "server.java",
        name: "Java 设置",
        desc: "为每台服务器指定 Java 路径与内存参数。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 63,
    },
    FeatureMeta {
        id: FEATURE_FORCE_STOP_CONFIRM,
        name: "强停二次确认",
        desc: "强制结束服务器进程前弹窗确认，避免误点丢档。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 64,
    },
    FeatureMeta {
        id: "server.tps",
        name: "服务端性能(TPS/MSPT)",
        desc: "从日志解析 TPS/MSPT 指标并展示变化。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 70,
    },
    // ---------- 内网穿透 ----------
    FeatureMeta {
        id: "tunnel.dashboard",
        name: "仪表盘",
        desc: "内网穿透总览：隧道状态与连接信息。",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 10,
    },
    FeatureMeta {
        id: "tunnel.create",
        name: "创建隧道",
        desc: "新建 frp/rathole 隧道并映射本地端口。",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 20,
    },
    FeatureMeta {
        id: "tunnel.manage",
        name: "隧道管理",
        desc: "启停、编辑与删除已有隧道。",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 30,
    },
    FeatureMeta {
        id: "tunnel.logs",
        name: "隧道日志",
        desc: "查看穿透内核输出的运行日志。",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 40,
    },
    FeatureMeta {
        id: "tunnel.tutorial",
        name: "教程",
        desc: "从零开始的内网穿透图文步骤说明。",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 50,
    },
    FeatureMeta {
        id: "tunnel.frpc",
        name: "frpc 管理",
        desc: "下载、更新或指定 frpc 可执行文件。",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 60,
    },
    // ---------- 设置 ----------
    FeatureMeta {
        id: "settings.general",
        name: "通用",
        desc: "全局通用设置（启动行为、默认值等）。",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 10,
    },
    FeatureMeta {
        id: "settings.logs",
        name: "日志",
        desc: "日志采集与日志库相关设置。",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 20,
    },
    FeatureMeta {
        id: "settings.ui",
        name: "界面",
        desc: "主题、字号、动效与布局设置。",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 30,
    },
    FeatureMeta {
        id: "settings.java",
        name: "Java",
        desc: "全局默认 Java 环境与内存参数。",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 40,
    },
    FeatureMeta {
        id: "settings.notify",
        name: "通知",
        desc: "系统通知与弹窗提醒的开关。",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 50,
    },
    // ---------- 工具/导航 ----------
    FeatureMeta {
        id: "tool.nav.server",
        name: "服务器导航",
        desc: "左侧导航栏的服务器入口。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 10,
    },
    FeatureMeta {
        id: "tool.nav.tunnel",
        name: "内网穿透导航",
        desc: "左侧导航栏的内网穿透入口。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 20,
    },
    FeatureMeta {
        id: "tool.nav.settings",
        name: "设置导航",
        desc: "左侧导航栏的设置入口。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 30,
    },
    FeatureMeta {
        id: "tool.sys_notify",
        name: "系统右下角通知",
        desc: "关服、崩溃等事件用 Windows 气泡通知提醒。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 40,
    },
    // ---------- 测试中的功能（默认禁用，启用需警告确认） ----------
    FeatureMeta {
        id: BETA_BACKUP,
        name: "自动备份",
        desc: "按策略自动生成世界与配置快照；启用后会定时占用磁盘与磁盘 IO。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 10,
    },
    FeatureMeta {
        id: BETA_CRASH_ANALYSIS,
        name: "崩溃报告分析",
        desc: "自动读取 crash-reports，给出根因归类与可疑文件定位。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 20,
    },
    FeatureMeta {
        id: BETA_TRAFFIC,
        name: "内网穿透流量显示",
        desc: "在穿透页面显示实时上下行流量与累计用量。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 30,
    },
    FeatureMeta {
        id: BETA_DOWNLOAD,
        name: "网络下载（服务端下载 + 模组下载）",
        desc: "内置服务端核心与模组下载（含 Modrinth 搜索与翻译）。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 60,
    },
    FeatureMeta {
        id: BETA_PLAYERS,
        name: "玩家管理（在线/白名单/封禁/OP）",
        desc: "在线玩家、白名单、封禁与 OP 四个页签的集中管理。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 70,
    },
    FeatureMeta {
        id: BETA_PLUGINS,
        name: "插件系统（rhai + 热加载）",
        desc: "用 rhai 脚本扩展 XMST，支持 zip 热加载与自定义界面效果。",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 80,
    },
    FeatureMeta {
        id: BETA_RATHOLE,
        name: "穿透内核 rathole（纯 Rust 反向代理）",
        desc: "启用后可创建 rathole 隧道替代 frp（需自备 rathole 服务端）。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 70,
    },
    FeatureMeta {
        id: BETA_REMOTE_BACKUP,
        name: "备份远端转存（本地/UNC/WebDAV）",
        desc: "把备份额外转存到远端目录或网络位置，实现异地留存。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 80,
    },
    FeatureMeta {
        id: BETA_SPECIAL,
        name: "特殊功能",
        desc: "特殊功能页总开关（当前包含 Spark 性能分析），默认禁用。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 90,
    },
    FeatureMeta {
        id: BETA_MOD_UPDATE,
        name: "模组检查更新",
        desc: "读取 mods 内模组版本并与 Modrinth 比对，只报告可更新版本，不自动替换。",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        restart_required: false,
        order: 91,
    },
    FeatureMeta {
        id: FEATURE_SPARK,
        name: "Spark 分析",
        desc: "解析 Spark 报告，定位卡顿的模组、维度与实体。",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        restart_required: false,
        order: 91,
    },
];

/// 按 ID 查询元数据
pub fn meta(id: &str) -> Option<&'static FeatureMeta> {
    REGISTRY.iter().find(|m| m.id == id)
}

/// 取某分组内的注册项，按 `order` 升序（order 相同时按 id 兜底，保证顺序稳定）
pub fn items_in_group(group: FeatureGroup) -> Vec<&'static FeatureMeta> {
    let mut v: Vec<&'static FeatureMeta> = REGISTRY.iter().filter(|m| m.group == group).collect();
    v.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(b.id)));
    v
}

/// 全部注册项：先按分组顺序、再按 `order` 升序排列（设置页只想看一遍登记情况时用）
pub fn all_items_ordered() -> Vec<&'static FeatureMeta> {
    let mut v: Vec<&'static FeatureMeta> = REGISTRY.iter().collect();
    v.sort_by(|a, b| {
        a.group
            .index()
            .cmp(&b.group.index())
            .then_with(|| a.order.cmp(&b.order))
            .then_with(|| a.id.cmp(b.id))
    });
    v
}

/// 开关生效时机提示文案（注册表驱动，避免界面里再写死一份清单）
pub fn apply_note(m: &FeatureMeta) -> &'static str {
    if m.restart_required {
        "下次启动生效"
    } else {
        "立即生效"
    }
}

/// 是否启用（有覆盖时用覆盖值，无覆盖时用注册表默认值）。
///
/// 未登记的 ID 一律视为**禁用**：ID 写错时功能应当"不出现"，
/// 而不是"开关关掉了、界面照样显示"（后者会让门控看起来失效）。
pub fn is_enabled(features: &HashMap<String, FeatureState>, id: &str) -> bool {
    features
        .get(id)
        .map(|s| s.enabled)
        .unwrap_or_else(|| meta(id).map(|m| m.default_enabled).unwrap_or(false))
}

/// 开关对应的界面入口提示：告诉用户在哪个页面的哪个位置能看到该功能
/// （开关在「设置 → 测试功能」页，功能本身在别的页面时最需要这行字）。
/// 返回空串表示该项没有独立界面入口。
pub fn entry_hint(id: &str) -> &'static str {
    match id {
        BETA_BACKUP => "入口：服务器 → 自动功能页（「快照备份」与「快照 / 备份列表」），服务器列表右键「📦 立即备份」",
        BETA_CRASH_ANALYSIS => "入口：服务器 → 服务器状态页底部的「崩溃报告分析」区块",
        BETA_TRAFFIC => "入口：内网穿透 → 仪表盘 / 隧道管理（实时流量、累计用量与「🩺 诊断流量」）",
        BETA_RATHOLE => "入口：内网穿透 → 创建隧道 → 「穿透内核」里的 Rathole 选项与对应表单",
        BETA_REMOTE_BACKUP => "入口：服务器 → 自动功能 → 存储与远程（远端备份目标 / 账号 / 重试次数），快照列表里的「转存」按钮",
        BETA_SPECIAL => "入口：服务器顶部页签「特殊功能」（首次开启后需切换到该页签）",
        BETA_MOD_UPDATE => "入口：服务器 → 文件浏览 → mods 页（工具栏「🔄 检查更新」与每个模组行的「🔄 更新」）",
        BETA_DOWNLOAD => "入口：左侧导航「下载」；文件浏览 → mods 页的「⬇️ 下载模组」",
        BETA_PLAYERS => "入口：服务器顶部页签「玩家管理」（在线 / 白名单 / 封禁 / OP / 属性）",
        BETA_PLUGINS => "入口：左侧导航「插件」（插件管理页与背景效果设置）",
        FEATURE_SPARK => "入口：服务器 → 特殊功能 → Spark 性能分析（随「特殊功能」总开关显示）",
        _ => "",
    }
}

/// 是否可见（无覆盖时按默认值）
pub fn is_visible(features: &HashMap<String, FeatureState>, id: &str) -> bool {
    features
        .get(id)
        .map(|s| s.visible)
        .unwrap_or_else(|| meta(id).map(|m| m.default_visible).unwrap_or(true))
}

#[cfg(test)]
mod registry_selfcheck {
    use super::*;

    /// 注册表完整性：ID 唯一、非空、有说明文案；Beta 组必须全部能在开关列表里出现。
    #[test]
    fn registry_ids_unique_and_described() {
        let mut seen: Vec<&str> = Vec::new();
        for m in REGISTRY {
            assert!(!m.id.trim().is_empty(), "注册项 ID 不能为空");
            assert!(!m.name.trim().is_empty(), "{} 缺少显示名", m.id);
            assert!(!m.desc.trim().is_empty(), "{} 缺少一句话说明", m.id);
            assert!(!seen.contains(&m.id), "注册项 ID 重复：{}", m.id);
            seen.push(m.id);
        }
        // 测试功能列表由 Beta 组生成：确认三个"曾经找不到入口"的项都在其中
        let beta: Vec<&str> = items_in_group(FeatureGroup::Beta).iter().map(|m| m.id).collect();
        for id in [BETA_MOD_UPDATE, BETA_RATHOLE, BETA_REMOTE_BACKUP] {
            assert!(beta.contains(&id), "Beta 组缺少 {id}");
        }
        // 分组排序稳定：同组内 order 单调不减
        let all = all_items_ordered();
        for w in all.windows(2) {
            if w[0].group == w[1].group {
                assert!(w[0].order <= w[1].order, "同组 order 未升序：{}", w[1].id);
            }
        }
    }

    /// 分组与默认值一致：items_in_group 覆盖本组全部项；默认禁用的项只允许出现在测试功能分组。
    #[test]
    fn group_and_defaults_consistent() {
        for m in REGISTRY {
            assert!(
                items_in_group(m.group).iter().any(|x| x.id == m.id),
                "{} 未出现在其分组的 items_in_group 结果里",
                m.id
            );
            // 默认禁用的项 = 测试功能；反过来测试功能必须默认禁用（避免"看不到开关却已开启"）
            assert!(
                m.default_enabled || m.group == FeatureGroup::Beta,
                "{} 默认禁用但不在测试功能分组",
                m.id
            );
        }
        let beta = items_in_group(FeatureGroup::Beta);
        assert!(!beta.is_empty(), "测试功能分组不能为空");
        for m in &beta {
            assert_eq!(m.group, FeatureGroup::Beta, "{} 分组与列表不一致", m.id);
            assert!(!m.default_enabled, "测试功能 {} 必须默认禁用", m.id);
            assert!(
                !entry_hint(m.id).is_empty(),
                "测试功能 {} 缺少「开启后在哪看到」的入口提示",
                m.id
            );
        }
    }

    /// Beta 常量与注册表一一对应：常量写错（或漏登记）时这里直接失败。
    #[test]
    fn beta_constants_registered() {
        for id in [
            BETA_BACKUP,
            BETA_CRASH_ANALYSIS,
            BETA_TRAFFIC,
            BETA_DOWNLOAD,
            BETA_PLAYERS,
            BETA_PLUGINS,
            BETA_RATHOLE,
            BETA_REMOTE_BACKUP,
            BETA_SPECIAL,
            BETA_MOD_UPDATE,
            FEATURE_SPARK,
            FEATURE_FORCE_STOP_CONFIRM,
        ] {
            assert!(meta(id).is_some(), "常量 {id} 未登记到 REGISTRY");
        }
    }

    /// set/get 往返：不依赖全局配置，直接用覆盖表验证读取结果与写入一致。
    #[test]
    fn enabled_roundtrip_with_overrides() {
        let mut map: HashMap<String, FeatureState> = HashMap::new();
        for m in REGISTRY {
            // 无覆盖：按注册表默认值
            assert_eq!(is_enabled(&map, m.id), m.default_enabled, "{} 默认值不符", m.id);
            // 写入与默认相反的值：立即读到新值（不存在"启动时缓存"）
            map.insert(
                m.id.to_string(),
                FeatureState {
                    enabled: !m.default_enabled,
                    visible: m.default_visible,
                    order: None,
                },
            );
            assert_eq!(is_enabled(&map, m.id), !m.default_enabled, "{} 覆盖未生效", m.id);
        }
        // 未登记 ID：一律禁用，避免 ID 拼错时功能意外开启
        assert!(!is_enabled(&map, "not.registered.id"));
        assert!(meta("not.registered.id").is_none());
    }
}
