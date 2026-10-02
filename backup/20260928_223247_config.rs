use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::features::FeatureState;

/// 新版配置文件名（XMST）
pub const CONFIG_FILE: &str = "xmst_config.json";
/// 旧版配置文件名（mcsrv 迁移兼容，仅在 CONFIG_FILE 不存在时读取并自动迁移）
pub const OLD_CONFIG_FILE: &str = "mcsrv_config.json";

/// 全局 Java 列表项：按 MC 版本可选用不同 Java
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaHome {
    /// 显示名（唯一标识，服务器通过它引用）
    pub name: String,
    /// Java 主版本号，如 "17" / "21" / "25"
    pub version: String,
    /// java.exe 绝对路径
    pub path: String,
}

impl Default for JavaHome {
    fn default() -> Self {
        Self {
            name: "Java 21".to_string(),
            version: "21".to_string(),
            path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// 默认 Java 可执行文件，留空则用 PATH 中的 java
    #[serde(default)]
    pub java_path: String,
    /// 全局 Java 列表（多版本）
    #[serde(default)]
    pub java_homes: Vec<JavaHome>,
    /// 默认 JVM 参数（如 -Xmx4G -Xms2G）
    #[serde(default)]
    pub default_jvm_args: String,
    /// 日志显示最大行数
    #[serde(default = "default_max_log_lines")]
    pub max_log_lines: usize,
    /// 界面语言: zh / en
    #[serde(default = "default_lang")]
    pub lang: String,
    /// 服务器列表
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
    /// 内网穿透条目
    #[serde(default)]
    pub tunnels: Vec<TunnelConfig>,
    /// Frp 服务端地址（host:port），用于集中生成 frpc.toml
    #[serde(default = "default_frp_server")]
    pub frp_server: String,
    /// Frp 服务端 token
    #[serde(default)]
    pub frp_token: String,
    /// 界面平滑动效开关
    #[serde(default = "default_ui_anim")]
    pub ui_animations: bool,
    /// 设置页内的折叠平滑动画开关（默认关闭，避免设置页折叠卡顿；其它页面仍按 ui_animations 生效）
    #[serde(default = "default_ui_settings_anim")]
    pub ui_settings_anim: bool,
    /// 平滑动画速度系数（越小越慢越平滑，1.0=默认，0.05=几乎瞬间）
    #[serde(default = "default_anim_speed")]
    pub anim_speed: f32,
    /// Theme mode: "dark" (night) / "light" (day)
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// Color preset: "default" / "sealantern" / "dawn" / "twilight" / "forest"
    #[serde(default = "default_theme_preset")]
    pub theme_preset: String,
    /// Whether custom colors override the preset's accent/background
    #[serde(default)]
    pub custom_colors: bool,
    /// Custom accent color (R,G,B)
    #[serde(default = "default_custom_accent")]
    pub custom_accent: (u8, u8, u8),
    /// Custom highlight color (R,G,B), reserved for secondary accent
    #[serde(default = "default_custom_highlight")]
    pub custom_highlight: (u8, u8, u8),
    /// Custom background color (R,G,B)
    #[serde(default = "default_custom_bg")]
    pub custom_bg: (u8, u8, u8),
    /// Background image path (empty = no background image)
    #[serde(default)]
    pub bg_image: String,
    /// Background image opacity 0.0-1.0 (1.0 = fully opaque)
    #[serde(default = "default_bg_alpha")]
    pub bg_alpha: f32,
    /// Corner radius toggle (false = sharp corners)
    #[serde(default = "default_round_corners")]
    pub round_corners: bool,
    /// Corner radius scale (1.0 = default rounding)
    #[serde(default = "default_corner_scale")]
    pub corner_scale: f32,
    /// 通知弹出方式: slide(右下角侧滑) / fade(淡入淡出)
    #[serde(default = "default_toast_style")]
    pub toast_style: String,
    /// 通知滞留时间（秒）
    #[serde(default = "default_toast_duration")]
    pub toast_duration_secs: f32,
    /// 系统级右下角通知（独立于窗口，窗口关闭也显示）
    #[serde(default = "default_sys_notify")]
    pub sys_notify: bool,
    /// 关闭行为: minimize(最小化) / exit(彻底关闭)
    #[serde(default = "default_close_behavior")]
    pub close_behavior: String,
    /// 功能覆盖状态（注册表 features.rs；缺省用默认值）
    #[serde(default)]
    pub features: HashMap<String, FeatureState>,
    /// 插件启用状态（插件系统内每个插件名 -> 是否启用；缺省未登记视为禁用）
    #[serde(default)]
    pub plugin_states: HashMap<String, bool>,
    /// 模组社区收藏（Modrinth project_id 列表，图2 收藏夹导航）
    #[serde(default)]
    pub mod_favorites: Vec<String>,
}

fn default_max_log_lines() -> usize {
    2000
}

fn default_ui_anim() -> bool {
    true
}

fn default_ui_settings_anim() -> bool {
    false
}

fn default_anim_speed() -> f32 {
    1.0
}

fn default_theme_mode() -> String {
    "dark".to_string()
}

fn default_theme_preset() -> String {
    "default".to_string()
}

fn default_custom_accent() -> (u8, u8, u8) {
    (0, 150, 136)
}

fn default_custom_highlight() -> (u8, u8, u8) {
    (120, 230, 160)
}

fn default_custom_bg() -> (u8, u8, u8) {
    (22, 22, 26)
}

fn default_bg_alpha() -> f32 {
    0.35
}

fn default_round_corners() -> bool {
    true
}

fn default_corner_scale() -> f32 {
    1.0
}

fn default_toast_style() -> String {
    "slide".to_string()
}

fn default_toast_duration() -> f32 {
    5.0
}

fn default_sys_notify() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_lang() -> String {
    "zh".to_string()
}

fn default_frp_server() -> String {
    "127.0.0.1:7000".to_string()
}

fn default_close_behavior() -> String {
    "exit".to_string()
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            java_path: String::new(),
            java_homes: Vec::new(),
            default_jvm_args: "-Xmx4G -Xms2G".to_string(),
            max_log_lines: 2000,
            lang: "zh".to_string(),
            servers: Vec::new(),
            tunnels: Vec::new(),
            frp_server: "127.0.0.1:7000".to_string(),
            frp_token: String::new(),
            ui_animations: true,
            ui_settings_anim: false,
            anim_speed: 1.0,
            theme_mode: "dark".to_string(),
            theme_preset: "default".to_string(),
            custom_colors: false,
            custom_accent: (0, 150, 136),
            custom_highlight: (120, 230, 160),
            custom_bg: (22, 22, 26),
            bg_image: String::new(),
            bg_alpha: 0.35,
            round_corners: true,
            corner_scale: 1.0,
            toast_style: "slide".to_string(),
            toast_duration_secs: 5.0,
            sys_notify: true,
            close_behavior: "exit".to_string(),
            features: HashMap::new(),
            plugin_states: HashMap::new(),
            mod_favorites: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 显示名
    pub name: String,
    /// 服务器根目录
    pub dir: PathBuf,
    /// 覆盖全局 Java（可空）
    #[serde(default)]
    pub java_path: Option<String>,
    /// 引用全局 Java 列表项（JavaHome.name，可空；优先于 java_path）
    #[serde(default)]
    pub java_home_id: Option<String>,
    /// 服务器 MC 版本，如 "1.21.1"，用于自动匹配 Java（可空）
    #[serde(default)]
    pub mc_version: Option<String>,
    /// 覆盖全局 JVM 参数（可空，空则用全局）
    #[serde(default)]
    pub jvm_args: Option<String>,
    /// 自定义启动命令模板，{java} {jvm} 会被替换
    pub launch_cmd: String,
    /// 是否被用户收藏（收藏的服务器在列表中置顶）
    #[serde(default)]
    pub favorited: bool,
    /// 开机自启该服务器（--autostart 模式下自动启动）
    #[serde(default)]
    pub autostart_enabled: bool,
    /// 开机自启前等待系统 CPU 空闲（错峰，避免多服同启卡顿）
    #[serde(default = "default_true")]
    pub autostart_cpu_idle: bool,
    /// 备份设置
    pub backup: BackupConfig,
    /// 自动重启设置
    #[serde(default)]
    pub auto_restart: AutoRestartConfig,
    /// 崩溃重启设置（异常退出自动拉起，带次数/等待/熔断）
    #[serde(default)]
    pub crash_restart: CrashRestartConfig,
    /// 仪表盘统计：累计启动次数
    #[serde(default)]
    pub start_count: u64,
    /// 仪表盘统计：累计下载次数（服务端 jar）
    #[serde(default)]
    pub download_count: u64,
    /// 仪表盘统计：首次下载时间（本地时间字符串，空=从未下载）
    #[serde(default)]
    pub first_download_at: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "新服务器".to_string(),
            dir: PathBuf::new(),
            java_path: None,
            java_home_id: None,
            mc_version: None,
            jvm_args: None,
            launch_cmd: "{java} {jvm} -jar server.jar nogui".to_string(),
            favorited: false,
            autostart_enabled: false,
            autostart_cpu_idle: true,
            backup: BackupConfig::default(),
            auto_restart: AutoRestartConfig::default(),
            crash_restart: CrashRestartConfig::default(),
            start_count: 0,
            download_count: 0,
            first_download_at: None,
        }
    }
}

/// 崩溃重启设置（"自动功能"页）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrashRestartConfig {
    /// 是否启用崩溃自动重启
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 单个熔断窗口内最大重启次数（达到后熔断，需等待窗口重置）
    #[serde(default = "default_crash_max")]
    pub max_restarts: u32,
    /// 崩溃后等待秒数再重启
    #[serde(default = "default_crash_wait")]
    pub wait_secs: u64,
    /// 熔断窗口（分钟）：窗口内连续崩溃达到 max_restarts 次即停止
    #[serde(default = "default_crash_circuit")]
    pub circuit_minutes: u64,
}

fn default_crash_max() -> u32 {
    5
}

fn default_crash_wait() -> u64 {
    10
}

fn default_crash_circuit() -> u64 {
    10
}

impl Default for CrashRestartConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_restarts: 5,
            wait_secs: 10,
            circuit_minutes: 10,
        }
    }
}

/// 自动重启设置（"自动功能"页）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoRestartConfig {
    /// 是否启用自动重启
    #[serde(default)]
    pub enabled: bool,
    /// 模式: "interval"=间隔重启 / "daily"=每天定时重启
    #[serde(default = "default_restart_mode")]
    pub mode: String,
    /// 间隔重启的间隔（分钟）
    #[serde(default = "default_restart_interval")]
    pub interval_min: u64,
    /// 每天定时重启时间 "HH:MM"
    #[serde(default = "default_restart_time")]
    pub daily_time: String,
    /// 重启前警告倒计时（秒）
    #[serde(default = "default_restart_warn")]
    pub warn_secs: u64,
    /// 上次重启时间（ISO 字符串；daily 模式同时用于防同日重复触发）
    #[serde(default)]
    pub last_restart: Option<String>,
}

fn default_restart_mode() -> String {
    "interval".to_string()
}

fn default_restart_interval() -> u64 {
    360
}

fn default_restart_time() -> String {
    "04:00".to_string()
}

fn default_restart_warn() -> u64 {
    30
}

impl Default for AutoRestartConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_restart_mode(),
            interval_min: default_restart_interval(),
            daily_time: default_restart_time(),
            warn_secs: default_restart_warn(),
            last_restart: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupConfig {
    pub enabled: bool,
    /// 备份间隔（分钟）
    pub interval_min: u64,
    /// 保留备份数量
    pub keep_count: usize,
    /// 备份限速（MB/s），0 表示不限速
    #[serde(default = "default_throttle")]
    pub throttle_mbps: f64,
    /// 备份内容列表，如 ["world", "config"]
    pub folders: Vec<String>,
    /// 上次备份时间（ISO 字符串）
    pub last_backup: Option<String>,
    /// 备份列表显示分组: "day"=按日 / "month"=按月
    #[serde(default = "default_backup_view")]
    pub view_mode: String,
    /// Memory pressure threshold (%): when system memory load reaches this,
    /// backup interval automatically switches to mem_interval_min.
    #[serde(default = "default_mem_threshold")]
    pub mem_threshold_percent: u8,
    /// Emergency backup interval (minutes) used while memory is under pressure.
    #[serde(default = "default_mem_interval")]
    pub mem_interval_min: u64,
    /// Remote backup target: empty = disabled; local dir / UNC path (e.g. D:\\backup
    /// or \\\\nas\\share) or WebDAV URL (http(s)://...). After a backup finishes,
    /// the zip is copied/uploaded here when non-empty.
    #[serde(default)]
    pub remote_target: String,
    /// WebDAV username (ignored for local/UNC copy)
    #[serde(default)]
    pub remote_user: String,
    /// WebDAV password (ignored for local/UNC copy)
    #[serde(default)]
    pub remote_password: String,
    /// Remaining retry count after a failed remote transfer (decremented per failure,
    /// gives up at 0; tick_backup retries pending entries on later ticks)
    #[serde(default = "default_remote_retry")]
    pub remote_retry: u32,
}

fn default_mem_threshold() -> u8 {
    90
}

fn default_backup_view() -> String {
    "day".to_string()
}

fn default_mem_interval() -> u64 {
    15
}

fn default_remote_retry() -> u32 {
    2
}

fn default_throttle() -> f64 {
    30.0
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_min: 60,
            keep_count: 10,
            throttle_mbps: 30.0,
            folders: vec!["world".to_string(), "config".to_string()],
            last_backup: None,
            view_mode: "day".to_string(),
            mem_threshold_percent: 90,
            mem_interval_min: 15,
            remote_target: String::new(),
            remote_user: String::new(),
            remote_password: String::new(),
            remote_retry: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub name: String,
    /// 穿透工具类型: frp / nps
    pub kind: String,
    /// 可执行文件路径（frpc.exe / nps client）
    pub exe: String,
    /// 配置文件路径（frpc.toml / nps.conf 等）；frp 集中模式下可留空，由工具自动生成
    pub cfg: String,
    /// Frp 本地端口（frp 集中模式）
    #[serde(default = "default_frp_port")]
    pub frp_local_port: u16,
    /// Frp 远程端口（frp 集中模式）
    #[serde(default = "default_frp_port")]
    pub frp_remote_port: u16,
    /// Frp 代理类型: tcp / udp
    #[serde(default = "default_frp_proxy_type")]
    pub frp_proxy_type: String,
    /// 是否自动随对应服务器启动
    pub auto_with_server: bool,
    /// 关联服务器索引（-1 表示独立运行）
    pub bind_server: i32,
    /// 隧道备注名（管理列表显示用，独立于 frpc 代理 name）
    #[serde(default)]
    pub remark: String,
    /// 是否被用户收藏（收藏的隧道在列表中优先显示）
    #[serde(default)]
    pub favorited: bool,
    /// frpc admin API 端口（每隧道唯一，用于流量统计；frpc.toml 无 webServer 时自动注入）
    #[serde(default = "default_admin_port")]
    pub admin_port: u16,
    /// 累计下行流量（字节，不清空持续累计）
    #[serde(default)]
    pub traffic_in_total: u64,
    /// 累计上行流量（字节，不清空持续累计）
    #[serde(default)]
    pub traffic_out_total: u64,
    /// Rathole: server address the client connects to (host only).
    #[serde(default)]
    pub rh_server_addr: String,
    /// Rathole: server port the client connects to.
    #[serde(default = "default_rh_server_port")]
    pub rh_server_port: u16,
    /// Rathole: tunnel name (first service section name under [client]).
    #[serde(default = "default_rh_tunnel_name")]
    pub rh_tunnel_name: String,
    /// Rathole: local forward address (local_addr).
    #[serde(default = "default_rh_local_addr")]
    pub rh_local_addr: String,
    /// Rathole: local forward port (local_port).
    #[serde(default = "default_rh_local_port")]
    pub rh_local_port: u16,
    /// Rathole: remote listening port on server (remote_port).
    #[serde(default = "default_rh_remote_port")]
    pub rh_remote_port: u16,
    /// Rathole: NOISE encryption toggle (default off; when on the server side must
    /// share the same key).
    #[serde(default)]
    pub rh_noise: bool,
    /// Rathole: NOISE shared key (hex string; empty = auto-generate 32 bytes).
    #[serde(default)]
    pub rh_noise_key: String,
}

fn default_admin_port() -> u16 {
    7400
}

fn default_rh_server_port() -> u16 {
    2333
}

fn default_rh_tunnel_name() -> String {
    "xmst".to_string()
}

fn default_rh_local_addr() -> String {
    "127.0.0.1".to_string()
}

fn default_rh_local_port() -> u16 {
    25565
}

fn default_rh_remote_port() -> u16 {
    25565
}

fn default_frp_port() -> u16 {
    25565
}

fn default_frp_proxy_type() -> String {
    "tcp".to_string()
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            name: "新穿透".to_string(),
            kind: "frp".to_string(),
            exe: String::new(),
            cfg: String::new(),
            frp_local_port: 25565,
            frp_remote_port: 25565,
            frp_proxy_type: "tcp".to_string(),
            auto_with_server: false,
            bind_server: -1,
            remark: String::new(),
            favorited: false,
            admin_port: 7400,
            traffic_in_total: 0,
            traffic_out_total: 0,
            rh_server_addr: String::new(),
            rh_server_port: 2333,
            rh_tunnel_name: "xmst".to_string(),
            rh_local_addr: "127.0.0.1".to_string(),
            rh_local_port: 25565,
            rh_remote_port: 25565,
            rh_noise: false,
            rh_noise_key: String::new(),
        }
    }
}

/// 修复「UTF-8 误读 GBK 字节」造成的乱码字符串（Bug5）。
/// 判定：把字符串按 GB18030 重新编码再按 UTF-8 解码；仅当两步均无编码错误、
/// 且结果与原串不同、非空时才替换。正常 UTF-8 中文经 GBK 往返必然产生错误
/// 而不会被替换，因此不会误伤；「GBK 字节被当 UTF-8 读」的脏串恰好能无损还原。
pub fn repair_gbk_mojibake(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let (bytes, _, enc_err) = encoding_rs::GB18030.encode(s);
    if enc_err || bytes.is_empty() {
        return None;
    }
    let (out, _, dec_err) = encoding_rs::UTF_8.decode(&bytes);
    if dec_err || out.contains('\u{FFFD}') {
        return None;
    }
    let out = out.into_owned();
    if out.is_empty() || out == s {
        return None;
    }
    Some(out)
}

/// 修复配置中已知易被错误编码的文本字段（隧道名/备注），返回是否有改动（Bug5）。
/// 磁盘上的历史脏数据（GBK 字节被按 UTF-8 写出）在此一次性还原为正确中文。
pub fn repair_cfg_mojibake(cfg: &mut GlobalConfig) -> bool {
    let mut changed = false;
    for t in cfg.tunnels.iter_mut() {
        for field in [&mut t.name, &mut t.remark] {
            if let Some(v) = repair_gbk_mojibake(field) {
                *field = v;
                changed = true;
            }
        }
    }
    changed
}
