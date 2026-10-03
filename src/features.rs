//! 功能注册表：为 XMST 所有功能/选项分配稳定语义化 ID。
//!
//! 设计目的：
//! 1. 每个功能有唯一稳定 ID（语义化 slug，如 `server.backup.auto`），代码/配置/日志统一引用；
//! 2. 用户对功能的个性化覆盖（启用/禁用/显示/排序/样式）全部以 ID 为键持久化，
//!    缺省时用默认值，不膨胀配置文件；
//! 3. 后续自定义 UI（布局/拖拽排序/样式/显隐）直接基于本注册表驱动，无需再动硬编码。
//!
//! 当前使用者：
//! - 「测试中的功能」开关：Beta 分组功能默认禁用，启用需警告确认；
//!   关闭 = enabled=false，UI 隐藏 + 后台调度拦截，不会出现"不显示但还在执行"。
//!
//! 新功能上线时必须在此登记一条 FeatureMeta，否则无法被个性化系统识别。

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
}

/// 功能元数据（编译期静态登记，不可变）
pub struct FeatureMeta {
    /// 稳定语义化唯一 ID
    pub id: &'static str,
    /// 显示名
    pub name: &'static str,
    /// 所属分组
    pub group: FeatureGroup,
    /// 默认启用（false=默认禁用，仅测试功能使用）
    pub default_enabled: bool,
    /// 默认可见（false=默认隐藏 UI）
    pub default_visible: bool,
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
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 10,
    },
    FeatureMeta {
        id: "server.status",
        name: "状态",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 20,
    },
    FeatureMeta {
        id: "server.files",
        name: "文件",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 30,
    },
    FeatureMeta {
        id: "server.backup",
        name: "自动功能",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 40,
    },
    FeatureMeta {
        id: "server.auto_restart",
        name: "自动重启",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 50,
    },
    FeatureMeta {
        id: "server.crash_restart",
        name: "崩溃重启",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 51,
    },
    FeatureMeta {
        id: "server.whitelist",
        name: "白名单/黑名单管理",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 60,
    },
    FeatureMeta {
        id: "server.properties",
        name: "服务器设置",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 61,
    },
    FeatureMeta {
        id: "server.scripts",
        name: "启动脚本",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 62,
    },
    FeatureMeta {
        id: "server.java",
        name: "Java 设置",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 63,
    },
    FeatureMeta {
        id: FEATURE_FORCE_STOP_CONFIRM,
        name: "强停二次确认",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 64,
    },
    FeatureMeta {
        id: "server.tps",
        name: "服务端性能(TPS/MSPT)",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 70,
    },
    // ---------- 内网穿透 ----------
    FeatureMeta {
        id: "tunnel.dashboard",
        name: "仪表盘",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        order: 10,
    },
    FeatureMeta {
        id: "tunnel.create",
        name: "创建隧道",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        order: 20,
    },
    FeatureMeta {
        id: "tunnel.manage",
        name: "隧道管理",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        order: 30,
    },
    FeatureMeta {
        id: "tunnel.logs",
        name: "隧道日志",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        order: 40,
    },
    FeatureMeta {
        id: "tunnel.tutorial",
        name: "教程",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        order: 50,
    },
    FeatureMeta {
        id: "tunnel.frpc",
        name: "frpc 管理",
        group: FeatureGroup::Tunnel,
        default_enabled: true,
        default_visible: true,
        order: 60,
    },
    // ---------- 设置 ----------
    FeatureMeta {
        id: "settings.general",
        name: "通用",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        order: 10,
    },
    FeatureMeta {
        id: "settings.logs",
        name: "日志",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        order: 20,
    },
    FeatureMeta {
        id: "settings.ui",
        name: "界面",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        order: 30,
    },
    FeatureMeta {
        id: "settings.java",
        name: "Java",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        order: 40,
    },
    FeatureMeta {
        id: "settings.notify",
        name: "通知",
        group: FeatureGroup::Settings,
        default_enabled: true,
        default_visible: true,
        order: 50,
    },
    // ---------- 工具/导航 ----------
    FeatureMeta {
        id: "tool.nav.server",
        name: "服务器导航",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 10,
    },
    FeatureMeta {
        id: "tool.nav.tunnel",
        name: "内网穿透导航",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 20,
    },
    FeatureMeta {
        id: "tool.nav.settings",
        name: "设置导航",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 30,
    },
    FeatureMeta {
        id: "tool.sys_notify",
        name: "系统右下角通知",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 40,
    },
    // ---------- 测试中的功能（默认禁用，启用需警告确认） ----------
    FeatureMeta {
        id: BETA_BACKUP,
        name: "自动备份",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 10,
    },
    FeatureMeta {
        id: BETA_CRASH_ANALYSIS,
        name: "崩溃报告分析",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 20,
    },
    FeatureMeta {
        id: BETA_TRAFFIC,
        name: "内网穿透流量显示",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 30,
    },
    FeatureMeta {
        id: BETA_DOWNLOAD,
        name: "网络下载（服务端下载 + 模组下载）",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 60,
    },
    FeatureMeta {
        id: BETA_PLAYERS,
        name: "玩家管理（在线/白名单/封禁/OP）",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 70,
    },
    FeatureMeta {
        id: BETA_PLUGINS,
        name: "插件系统（rhai + 热加载）",
        group: FeatureGroup::Tool,
        default_enabled: true,
        default_visible: true,
        order: 80,
    },
    FeatureMeta {
        id: BETA_RATHOLE,
        name: "穿透内核 rathole（纯 Rust 反向代理）",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 70,
    },
    FeatureMeta {
        id: BETA_REMOTE_BACKUP,
        name: "备份远端转存（本地/UNC/WebDAV）",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 80,
    },
    FeatureMeta {
        id: BETA_SPECIAL,
        name: "特殊功能",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 90,
    },    FeatureMeta {
        id: BETA_MOD_UPDATE,
        name: "模组检查更新",
        group: FeatureGroup::Beta,
        default_enabled: false,
        default_visible: true,
        order: 91,
    },
    FeatureMeta {
        id: FEATURE_SPARK,
        name: "Spark 分析",
        group: FeatureGroup::Server,
        default_enabled: true,
        default_visible: true,
        order: 91,
    },
];

/// 按 ID 查询元数据
pub fn meta(id: &str) -> Option<&'static FeatureMeta> {
    REGISTRY.iter().find(|m| m.id == id)
}

/// 是否启用（无覆盖时按默认值）
pub fn is_enabled(features: &HashMap<String, FeatureState>, id: &str) -> bool {
    features
        .get(id)
        .map(|s| s.enabled)
        .unwrap_or_else(|| meta(id).map(|m| m.default_enabled).unwrap_or(true))
}

/// 是否可见（无覆盖时按默认值）
pub fn is_visible(features: &HashMap<String, FeatureState>, id: &str) -> bool {
    features
        .get(id)
        .map(|s| s.visible)
        .unwrap_or_else(|| meta(id).map(|m| m.default_visible).unwrap_or(true))
}
