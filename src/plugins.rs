//! Plugin system (Phase 4): rhai host + event bus + zip hot-loading.
//!
//! A plugin is a ZIP archive placed under `data/plugins/` containing:
//!   - `manifest.json` : { name, version, entry, description, events[],
//!                         max_operations, resources[] }
//!   - `<entry>.rhai`  : rhai script implementing `on_<event>(...)` hooks
//!   - optional resource files (extracted to cache dir, exposed via
//!     `xmst_resource_dir()`)
//!
//! Event hooks are wired from main.rs: server_started / server_stopped /
//! log_line / player_joined / player_left / backup_done. A hook is invoked
//! only when the compiled AST actually defines a matching `on_<event>` fn.
//!
//! Safety rails:
//!   - `max_operations`: operation counter checked by Engine::on_progress;
//!     an over-limit script is aborted with an error (no infinite loop).
//!   - Per-plugin isolated Engine instance: scripts cannot pollute each other.
//!   - Feature-gated: the whole runtime is created only when BETA_PLUGINS is
//!     enabled (zero load / zero polling when disabled).
//!
//! Script API (registered on every plugin engine):
//!   - xmst_log(msg: String)            -> append a line to the plugin log
//!   - xmst_toast(title, body: String)  -> show a tool notification
//!   - xmst_set_bg(style: String)       -> "translucent" | "frosted" | "acrylic" | "default"
//!   - xmst_resource_dir() -> String    -> extracted resource dir of this plugin

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use rhai::{Dynamic, Engine, Scope, AST};

pub const MAX_LOG_LINES: usize = 200;
pub const DEFAULT_MAX_OPS: u64 = 10_000;

/// 背景材质的**系统级门槛自检**：返回需要提示用户的条目（空 = 一切正常）。
///
/// 依据（Microsoft 文档 + TranslucentTB 生态的一致结论）：Windows 会在
/// 「透明效果关闭 / 节电模式 / 远程桌面会话」时禁用亚克力，而 TranslucentTB
/// 那类工具**不做任何检测**、失败时静默无提示。这里主动检测并告知用户，
/// 避免把「系统策略禁用」误判成「工具坏了」。
#[cfg(windows)]
pub fn material_env_warnings() -> Vec<String> {
    let mut out = Vec::new();
    // 1) 透明效果开关（HKCU\...\Themes\Personalize\EnableTransparency）
    if let Some(v) = reg_dword_hkcu(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        "EnableTransparency",
    ) {
        if v == 0 {
            out.push(
                "系统「透明效果」已关闭（设置 → 个性化 → 颜色）→ 系统级亚克力被禁用，当前用桌面捕获材质"
                    .to_string(),
            );
        }
    }
    unsafe {
        // 2) 远程桌面会话（Windows 会禁用窗口透明/亚克力）
        if winapi::um::winuser::GetSystemMetrics(winapi::um::winuser::SM_REMOTESESSION) != 0 {
            out.push("当前是远程桌面会话 → Windows 会禁用窗口透明/亚克力".to_string());
        }
        // 3) 节电模式。注意 winapi 0.3.9 的 SYSTEM_POWER_STATUS 把 Win8+ 的
        //    SystemStatusFlag 仍命名为 Reserved1（同一字节位置），非 0 即正在节电。
        let mut st: winapi::um::winbase::SYSTEM_POWER_STATUS = std::mem::zeroed();
        if winapi::um::winbase::GetSystemPowerStatus(&mut st) != 0 && st.Reserved1 != 0 {
            out.push("系统处于节电模式 → 透明/亚克力被自动禁用".to_string());
        }
    }
    out
}

#[cfg(not(windows))]
pub fn material_env_warnings() -> Vec<String> {
    Vec::new()
}

/// 读 HKCU 下的 DWORD（失败返回 None）。
#[cfg(windows)]
fn reg_dword_hkcu(path: &str, name: &str) -> Option<u32> {
    use winapi::um::winreg::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    unsafe {
        let key: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let val: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut data: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        let mut ty: u32 = 0;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_DWORD,
            &mut ty,
            &mut data as *mut u32 as *mut _,
            &mut len,
        );
        if rc == 0 {
            Some(data)
        } else {
            None
        }
    }
}

/// 插件名 → 安全的路径片段（缓存目录 / 资源目录共用）。
///
/// 插件名来自 zip 内的 manifest，属于不可信输入：直接 `join(name)` 会让
/// `:` `*` `?` 等 Windows 非法字符或超长名导致 `create_dir_all` 静默失败
/// （只留在插件日志里），也会让 `xmst_resource_dir()` 返回宿主拿不到的路径。
pub fn cache_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "plugin".to_string();
    }
    out.truncate(64);
    out
}
/// Plugin descriptor parsed from `manifest.json`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub entry: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub max_operations: Option<u64>,
    #[serde(default)]
    pub resources: Vec<String>,
    /// D3 权限声明。留空 = **兼容模式：授予全部权限**（并在插件日志里记一条提醒），
    /// 因此历史插件不会被这个新字段破坏；显式声明后只授予列出的权限。
    /// 已定义的权限：
    /// * `window.background` —— `xmst_set_bg` / `xmst_apply_saved_bg`（改窗口背景/材质）
    /// * `notify`            —— `xmst_toast`（弹提示）
    /// * `config`            —— `xmst_config_get` / `xmst_config_set`（读写自己的配置）
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// D3：权限名常量（避免各处写裸字符串）。
pub const PERM_BACKGROUND: &str = "window.background";
pub const PERM_NOTIFY: &str = "notify";
pub const PERM_CONFIG: &str = "config";

impl PluginManifest {
    /// 是否授予某权限。`permissions` 为空 = 兼容模式（全部授予）。
    pub fn allows(&self, perm: &str) -> bool {
        self.permissions.is_empty() || self.permissions.iter().any(|p| p == perm)
    }

    /// 是否处于「未声明 permissions」的兼容模式。
    pub fn perms_legacy(&self) -> bool {
        self.permissions.is_empty()
    }
}

/// Messages sent from engine-registered API closures to the manager.
#[derive(Debug, Clone)]
pub enum PluginMsg {
    Log(String),
    Toast(String, String),
    /// (plugin_name, style): the requesting plugin name lets the main thread
    /// pick the per-plugin bg_alpha config instead of the global default.
    SetBg(String, BgStyle),
    /// Script mutated its own config via xmst_config_set; main thread persists it.
    ConfigDirty,
}

/// Requested window background effect. Four distinct modes so the UI can offer
/// 半透明 / 毛玻璃 / 亚克力 independently, each with its own opacity value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BgStyle {
    /// Default opaque background (restore).
    #[default]
    Default,
    /// In-app translucent dark overlay (alpha from the plugin opacity value).
    /// Needs no DWM support at all: the central content area is drawn with
    /// alpha = opacity, so the desktop shows through dimmed.
    Translucent,
    /// DWM `ACCENT_ENABLE_BLURBEHIND` (Gaussian blur behind, Win10 any build).
    Frosted,
    /// DWM `ACCENT_ENABLE_ACRYLICBLURBEHIND` (blur + noise, Win10 1803+).
    Acrylic,
}

impl BgStyle {
    /// Accepts the persisted / script-facing key.
    pub fn from_key(s: &str) -> Self {
        match s {
            "translucent" => Self::Translucent,
            "frosted" => Self::Frosted,
            "acrylic" => Self::Acrylic,
            _ => Self::Default,
        }
    }

    /// Persisted / script-facing key (round-trips through `from_key`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Translucent => "translucent",
            Self::Frosted => "frosted",
            Self::Acrylic => "acrylic",
            Self::Default => "default",
        }
    }

    /// Chinese UI label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Translucent => "半透明",
            Self::Frosted => "毛玻璃",
            Self::Acrylic => "亚克力",
            Self::Default => "默认",
        }
    }

    /// DWM accent state that implements this style, if any.
    pub fn accent_state(self) -> Option<u32> {
        match self {
            Self::Frosted => Some(ACCENT_ENABLE_BLURBEHIND),
            Self::Acrylic => Some(ACCENT_ENABLE_ACRYLICBLURBEHIND),
            _ => None,
        }
    }

    /// True when the style needs per-pixel window alpha (transparent surface).
    pub fn needs_transparency(self) -> bool {
        self != Self::Default
    }
}

/// `AccentState` values (undocumented API, reverse-engineered from twinui.pdb).
pub const ACCENT_DISABLED: u32 = 0;
/// Plain Gaussian blur behind the window (works on every Win10 build).
pub const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
/// Acrylic: blurred wallpaper sample + noise layer (Win10 1803+; ignored by
/// some drivers/Win11 builds — callers must handle `false`).
pub const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

/// A loaded plugin instance (one isolated Engine per plugin).
pub struct PluginInstance {
    pub manifest: PluginManifest,
    pub source: PathBuf,
    pub enabled: bool,
    pub engine: Engine,
    pub entry_ast: AST,
    pub scope: Scope<'static>,
    /// Reset before every event dispatch; incremented by on_progress.
    pub op_count: Arc<AtomicU64>,
    pub last_error: Option<String>,
    pub hook_calls: HashMap<String, u64>,
}

/// Owns all plugin instances, the plugin log and the message pump.
pub struct PluginManager {
    pub plugins_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub instances: Vec<PluginInstance>,
    /// name -> source zip path (kept after unload for instant re-enable).
    pub sources: HashMap<String, PathBuf>,
    pub log: String,
    pub log_lines: Vec<String>,
    /// (plugin_name, style) requested by the last script call; consumed by main.
    pub bg_request: Option<(String, BgStyle)>,
    pub toasts: Vec<(String, String)>,
    tx: mpsc::Sender<PluginMsg>,
    rx: mpsc::Receiver<PluginMsg>,
    pub states: HashMap<String, bool>,
    /// Plugin-specific key/value configs (name -> map), shared with engines.
    pub configs: std::sync::Arc<std::sync::Mutex<HashMap<String, HashMap<String, String>>>>,
    /// Set when a script mutated its config; main thread syncs to disk.
    pub configs_dirty: bool,
}

thread_local! {
    /// Current plugin name during an event dispatch (used by resource API).
    static CURRENT_PLUGIN: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

impl PluginManager {
    pub fn new(
        plugins_dir: PathBuf,
        cache_dir: PathBuf,
        states: HashMap<String, bool>,
        configs: HashMap<String, HashMap<String, String>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            plugins_dir,
            cache_dir,
            instances: Vec::new(),
            sources: HashMap::new(),
            log: String::new(),
            log_lines: Vec::new(),
            bg_request: None,
            toasts: Vec::new(),
            tx,
            rx,
            states,
            configs: std::sync::Arc::new(std::sync::Mutex::new(configs)),
            configs_dirty: false,
        }
    }

    /// Full rescan: drop everything, load every `*.zip` in plugins_dir.
    /// A plugin is enabled when its name is registered as enabled in states.
    pub fn reload_all(&mut self) -> Result<usize, String> {
        let zips = self.scan_zips();
        self.instances.clear();
        self.sources.clear();
        self.push_log(format!("重新扫描插件目录：{}", self.plugins_dir.display()));
        let mut ok = 0usize;
        for zip_path in zips {
            let enabled = self
                .manifest_name_of(&zip_path)
                .and_then(|n| self.states.get(&n).copied())
                .unwrap_or(false);
            match self.load_zip(&zip_path, enabled) {
                Ok(name) => {
                    ok += 1;
                    self.push_log(format!("[{}] 插件已加载（enabled={enabled}）", name));
                }
                Err(e) => self.push_log(format!("加载失败 {}：{e}", zip_path.display())),
            }
        }
        if ok == 0 {
            self.push_log("未发现有效插件（请放置 zip 到插件目录）".to_string());
        }
        Ok(ok)
    }

    /// List candidate zip files in the plugin directory.
    pub fn scan_zips(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.plugins_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case("zip"))
                    .unwrap_or(false)
                {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// Peek the manifest name of a zip without installing it.
    fn manifest_name_of(&self, zip_path: &Path) -> Option<String> {
        let file = std::fs::File::open(zip_path).ok()?;
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let mut mf = archive.by_name("manifest.json").ok()?;
        let mut json = String::new();
        mf.read_to_string(&mut json).ok()?;
        serde_json::from_str::<PluginManifest>(&json).ok().map(|m| m.name)
    }

    /// Load one plugin zip into a fresh isolated engine.
    pub fn load_zip(&mut self, zip_path: &Path, enabled: bool) -> Result<String, String> {
        let file = std::fs::File::open(zip_path)
            .map_err(|e| format!("打开失败 {zip_path:?}：{e}"))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("zip 解析失败 {zip_path:?}：{e}"))?;

        let mut mf = archive
            .by_name("manifest.json")
            .map_err(|_| format!("缺少 manifest.json（{zip_path:?}）"))?;
        let mut json = String::new();
        mf.read_to_string(&mut json)
            .map_err(|e| format!("manifest 读取失败：{e}"))?;
        let manifest: PluginManifest = serde_json::from_str(&json)
            .map_err(|e| format!("manifest 解析失败：{e}"))?;
        drop(mf); // release the ZipFile borrow before other archive access
        if manifest.name.trim().is_empty() {
            return Err("manifest.name 为空".to_string());
        }
        if manifest.entry.trim().is_empty() {
            return Err(format!("{}: manifest.entry 为空", manifest.name));
        }
        // Entry must end with .rhai (append suffix if omitted).
        let entry_file = if manifest.entry.ends_with(".rhai") {
            manifest.entry.clone()
        } else {
            format!("{}.rhai", manifest.entry)
        };
        let mut ef = archive
            .by_name(&entry_file)
            .map_err(|_| format!("{}: 缺少入口脚本 {entry_file}", manifest.name))?;
        let mut script = String::new();
        ef.read_to_string(&mut script)
            .map_err(|e| format!("{}: 入口脚本读取失败：{e}", manifest.name))?;
        drop(ef); // release the ZipFile borrow before iterating the archive

        // Extract resources (everything except manifest + entry scripts).
        let dest = self.cache_dir.join(cache_key(&manifest.name));
        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::create_dir_all(&dest);
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = f.name().to_string();
            if name == "manifest.json" || name.ends_with(".rhai") {
                continue;
            }
            // Path traversal guard: never write outside the plugin cache dir.
            if name.contains("..") || Path::new(&name).is_absolute() {
                continue;
            }
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_ok() {
                let target = dest.join(&name);
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(target, buf);
            }
        }

        // Build a fresh engine with the shared script API.
        let (engine, op_count) = Self::build_engine(
            &manifest,
            self.tx.clone(),
            self.cache_dir.clone(),
            self.configs.clone(),
        );
        let entry_ast = engine
            .compile(&script)
            .map_err(|e| format!("{}: 脚本编译失败：{e}", manifest.name))?;
        let scope = Scope::<'static>::new();

        // Replace any previous instance of the same plugin name.
        if let Some(pos) = self.instances.iter().position(|i| i.manifest.name == manifest.name) {
            self.instances.remove(pos);
        }
        self.sources.insert(manifest.name.clone(), zip_path.to_path_buf());
        self.states.insert(manifest.name.clone(), enabled);
        self.instances.push(PluginInstance {
            manifest: manifest.clone(),
            source: zip_path.to_path_buf(),
            enabled,
            engine,
            entry_ast,
            scope,
            op_count,
            last_error: None,
            hook_calls: HashMap::new(),
        });
        Ok(manifest.name)
    }

    /// Instant on/off switch: enable reloads (or loads) the plugin, disable
    /// unloads it from memory immediately.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        if enabled {
            if let Some(inst) = self.instances.iter_mut().find(|i| i.manifest.name == name) {
                if inst.enabled {
                    return Ok(()); // already enabled
                }
                // 已加载但处于禁用状态：直接置为启用，无需重载
                inst.enabled = true;
                self.states.insert(name.to_string(), true);
                self.push_log(format!("[{name}] 插件已启用"));
                return Ok(());
            }
            let zip = self
                .sources
                .get(name)
                .cloned()
                .ok_or_else(|| format!("插件「{name}」未找到 zip，请先重新扫描"))?;
            self.load_zip(&zip, true)?;
            self.push_log(format!("[{name}] 插件已启用（热加载）"));
            Ok(())
        } else {
            // 禁用不卸载实例：仅标记 enabled=false，列表保持可见（重扫才显示的旧行为是 bug）
            if let Some(inst) = self.instances.iter_mut().find(|i| i.manifest.name == name) {
                inst.enabled = false;
                self.states.insert(name.to_string(), false);
                self.push_log(format!("[{name}] 插件已禁用"));
            } else {
                self.states.insert(name.to_string(), false);
            }
            Ok(())
        }
    }

    /// Remove an instance from memory (source zip is kept for re-enable).
    pub fn unload(&mut self, name: &str) {
        if let Some(pos) = self.instances.iter().position(|i| i.manifest.name == name) {
            let removed = self.instances.remove(pos);
            self.states.insert(name.to_string(), false);
            self.push_log(format!("[{}] 插件已卸载", removed.manifest.name));
        }
    }

    /// Dispatch an event to every enabled plugin. Missing hooks are silently
    /// skipped (ErrorFunctionNotFound), other errors are logged on the plugin.
    pub fn emit(&mut self, event: &str, args: Vec<Dynamic>) {
        if self.instances.is_empty() {
            return;
        }
        let targets: Vec<String> = self
            .instances
            .iter()
            .filter(|i| i.enabled)
            .map(|i| i.manifest.name.clone())
            .collect();
        for name in targets {
            self.emit_to(&name, event, args.clone());
        }
    }

    /// 只向**指定插件**派发事件（单播）。
    ///
    /// `emit` 是广播：例如「启用插件 A」时若用广播，所有已启用插件的 `on_enabled`
    /// 都会被触发（各自重设一次窗口背景），语义与副作用都不对。启用/禁用这类
    /// 「针对某个插件」的事件应该用单播。
    pub fn emit_to(&mut self, name: &str, event: &str, args: Vec<Dynamic>) {
        let fn_name = format!("on_{}", event);
        let idx = match self.instances.iter().position(|i| i.manifest.name == name) {
            Some(i) => i,
            None => return,
        };
        if !self.instances[idx].enabled {
            return;
        }
        CURRENT_PLUGIN.with(|c| *c.borrow_mut() = name.to_string());
        let inst = &mut self.instances[idx];
        inst.op_count.store(0, Ordering::Relaxed);
        let result = call_hook(&inst.engine, &mut inst.scope, &inst.entry_ast, &fn_name, &args);
        match result {
            Ok(_) => {
                *inst.hook_calls.entry(fn_name.clone()).or_insert(0) += 1;
                inst.last_error = None;
            }
            Err(e) => {
                // Ignore "function not found": the plugin simply does not
                // implement this event hook.
                if !matches!(&*e, rhai::EvalAltResult::ErrorFunctionNotFound(_, _)) {
                    let msg = e.to_string();
                    inst.last_error = Some(msg.clone());
                    let tx = self.tx.clone();
                    let _ = tx.send(PluginMsg::Log(format!(
                        "[{}] {fn_name} 执行出错：{msg}",
                        inst.manifest.name
                    )));
                }
            }
        }
        CURRENT_PLUGIN.with(|c| c.borrow_mut().clear());
    }

    /// Drain API messages (log / toast / bg request) produced by script calls.
    pub fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                PluginMsg::Log(s) => self.push_log(s),
                PluginMsg::Toast(title, body) => self.toasts.push((title, body)),
                PluginMsg::SetBg(name, bg) => self.bg_request = Some((name, bg)),
                PluginMsg::ConfigDirty => self.configs_dirty = true,
            }
        }
        if self.toasts.len() > 10 {
            let over = self.toasts.len() - 10;
            self.toasts.drain(..over);
        }
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.instances.iter().any(|i| i.manifest.name == name && i.enabled)
    }

    pub fn enabled_count(&self) -> usize {
        self.instances.iter().filter(|i| i.enabled).count()
    }

    fn push_log(&mut self, line: String) {
        self.log_lines.push(line);
        if self.log_lines.len() > MAX_LOG_LINES {
            let over = self.log_lines.len() - MAX_LOG_LINES;
            self.log_lines.drain(..over);
        }
        self.log = self.log_lines.join("\n");
    }

    /// Build an isolated Engine with the shared script API. The op counter is
    /// shared with the instance so it can be reset before every dispatch.
    fn build_engine(
        manifest: &PluginManifest,
        tx: mpsc::Sender<PluginMsg>,
        cache_dir: PathBuf,
        configs: std::sync::Arc<std::sync::Mutex<HashMap<String, HashMap<String, String>>>>,
    ) -> (Engine, Arc<AtomicU64>) {
        let mut engine = Engine::new();
        // 上限保护：清单填 1e12 会让 rhai 长时间占满 UI 线程
    let max_ops = manifest.max_operations.unwrap_or(DEFAULT_MAX_OPS).min(200_000);
        // Official operation cap first (rhai aborts with ErrorTooManyOperations).
        engine.set_max_operations(max_ops);
        let op_count = Arc::new(AtomicU64::new(0));
        {
            let oc = op_count.clone();
            engine.on_progress(move |_progress| {
                let n = oc.fetch_add(1, Ordering::Relaxed) + 1;
                if n > max_ops {
                    Some(format!("max_operations exceeded ({max_ops})").into())
                } else {
                    None
                }
            });
        }
        // xmst_log(msg) —— 无特权要求（写插件自己的日志）
        {
            let tx = tx.clone();
            engine.register_fn("xmst_log", move |msg: String| {
                let _ = tx.send(PluginMsg::Log(msg));
            });
        }
        // D3 权限门控：未声明 permissions 的插件按兼容模式授予全部权限（并在日志里提醒），
        // 显式声明后只有列出的权限可用 —— 越权调用会被拒绝并写一条插件日志。
        let allow_notify = manifest.allows(PERM_NOTIFY);
        let allow_bg = manifest.allows(PERM_BACKGROUND);
        let allow_config = manifest.allows(PERM_CONFIG);
        if manifest.perms_legacy() {
            let tx = tx.clone();
            let _ = tx.send(PluginMsg::Log(
                "[权限] manifest 未声明 permissions → 按兼容模式授予全部权限；\
                 建议显式声明 window.background / notify / config"
                    .to_string(),
            ));
        }
        let deny = |tx: &mpsc::Sender<PluginMsg>, what: &str, perm: &str| {
            let _ = tx.send(PluginMsg::Log(format!(
                "[权限] 拒绝 {what}：manifest 未声明 \"{perm}\" 权限"
            )));
        };
        // xmst_toast(title, body) —— 需 notify
        {
            let tx = tx.clone();
            let tx_deny = tx.clone();
            engine.register_fn("xmst_toast", move |title: String, body: String| {
                if allow_notify {
                    let _ = tx.send(PluginMsg::Toast(title, body));
                } else {
                    deny(&tx_deny, "xmst_toast", PERM_NOTIFY);
                }
            });
        }
        // xmst_set_bg(style) —— 需 window.background
        {
            let tx = tx.clone();
            let tx_deny = tx.clone();
            engine.register_fn("xmst_set_bg", move |style: String| {
                if !allow_bg {
                    deny(&tx_deny, "xmst_set_bg", PERM_BACKGROUND);
                    return;
                }
                let bg = BgStyle::from_key(style.as_str());
                let name = CURRENT_PLUGIN.with(|c| c.borrow().clone());
                let _ = tx.send(PluginMsg::SetBg(name, bg));
            });
        }
        // xmst_config_get(key) -> String  /  xmst_config_set(key, value) —— 需 config
        {
            let cfg = configs.clone();
            let tx = tx.clone();
            engine.register_fn("xmst_config_get", move |key: String| -> String {
                if !allow_config {
                    deny(&tx, "xmst_config_get", PERM_CONFIG);
                    return String::new();
                }
                let name = CURRENT_PLUGIN.with(|c| c.borrow().clone());
                if name.is_empty() {
                    return String::new();
                }
                cfg.lock()
                    .unwrap()
                    .get(&name)
                    .and_then(|m| m.get(&key))
                    .cloned()
                    .unwrap_or_default()
            });
        }
        {
            let cfg = configs.clone();
            let tx = tx.clone();
            let tx_deny = tx.clone();
            engine.register_fn("xmst_config_set", move |key: String, value: String| {
                if !allow_config {
                    deny(&tx_deny, "xmst_config_set", PERM_CONFIG);
                    return;
                }
                let name = CURRENT_PLUGIN.with(|c| c.borrow().clone());
                if !name.is_empty() {
                    cfg.lock().unwrap_or_else(|e| e.into_inner()).entry(name).or_default().insert(key, value);
                    let _ = tx.send(PluginMsg::ConfigDirty);
                }
            });
        }
        // xmst_resource_dir() -> String
        {
            let cache = cache_dir.clone();
            engine.register_fn("xmst_resource_dir", move || -> String {
                let name = CURRENT_PLUGIN.with(|c| c.borrow().clone());
                if name.is_empty() {
                    return String::new();
                }
                cache.join(cache_key(&name)).to_string_lossy().to_string()
            });
        }
        (engine, op_count)
    }
}

/// Invoke a rhai hook with 0..=3 arguments. rhai's FuncArgs is implemented
/// for tuples/arrays with a fixed length, so a match over the arg count keeps
/// the public event API ergonomic (all XMST events pass <= 3 strings).
fn call_hook(
    engine: &Engine,
    scope: &mut Scope<'static>,
    ast: &AST,
    fn_name: &str,
    args: &[Dynamic],
) -> Result<Dynamic, Box<rhai::EvalAltResult>> {
    match args.len() {
        0 => engine.call_fn::<Dynamic>(scope, ast, fn_name, ()),
        1 => engine.call_fn::<Dynamic>(scope, ast, fn_name, (args[0].clone(),)),
        2 => engine.call_fn::<Dynamic>(
            scope,
            ast,
            fn_name,
            (args[0].clone(), args[1].clone()),
        ),
        3 => engine.call_fn::<Dynamic>(
            scope,
            ast,
            fn_name,
            (args[0].clone(), args[1].clone(), args[2].clone()),
        ),
        _ => Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
            Dynamic::from(format!("事件参数过多：{}", args.len())),
            rhai::Position::NONE,
        ))),
    }
}

/// Apply a DWM accent policy (`SetWindowCompositionAttribute`, undocumented API
/// loaded dynamically from user32 so the exe keeps running on older systems).
///
/// * `state`: `ACCENT_*` constant (0 = disabled/clear).
/// * `tint`  : AABBGGRR tint; RGB comes from the app theme background and the
///             alpha is the user's opacity. `ACCENT_ENABLE_ACRYLICBLURBEHIND`
///             refuses to blur with a zero alpha, so an enabled accent is
///             always given at least `1` here (a fully transparent tint still
///             blurs, which is what the user asks for at opacity 0).
///
/// The accent only shows through where the app itself paints with alpha < 255:
/// egui's panel fill is set to alpha 0 by `theme::apply` in blur modes, and the
/// window must be created with `ViewportBuilder::with_transparent(true)`.
///
/// Returns `false` when the API is unavailable or the call is refused, so the
/// caller can fall back instead of silently showing a transparent window.
#[cfg(windows)]
pub fn apply_accent(hwnd: isize, state: u32, tint: (u8, u8, u8), alpha: u8) -> bool {
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct WINDOWCOMPOSITIONATTRIBDATA {
        attrib: u32,
        pv_data: *mut std::ffi::c_void,
        cb_data: usize,
    }
    const WCA_ACCENT_POLICY: u32 = 19;

    if hwnd == 0 {
        return false;
    }
    unsafe {
        let name: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        let user32 = LoadLibraryW(name.as_ptr());
        if user32.is_null() {
            return false;
        }
        let addr = GetProcAddress(
            user32,
            "SetWindowCompositionAttribute\0".as_ptr() as *const i8,
        );
        if addr.is_null() {
            return false;
        }
        type SwcaFn = unsafe extern "system" fn(isize, *mut WINDOWCOMPOSITIONATTRIBDATA) -> i32;
        let swca: SwcaFn = std::mem::transmute(addr);
        let gradient_color = if state == ACCENT_DISABLED {
            0
        } else {
            let a = alpha.max(1) as u32;
            (a << 24) | ((tint.2 as u32) << 16) | ((tint.1 as u32) << 8) | (tint.0 as u32)
        };
        let mut accent = AccentPolicy {
            accent_state: state,
            // AccentFlags 语义是逆向所得：置 2 表示「GradientColor 参与着色」。
            // 参照 TranslucentTB（Win10 唯一可用路径）：acrylic(state 4) 用 0，
            // 其余状态（含毛玻璃 state 3）用 2。此前我们用固定 0，毛玻璃的着色可能因此被忽略。
            accent_flags: if state == ACCENT_ENABLE_ACRYLICBLURBEHIND { 0 } else { 2 },
            gradient_color,
            animation_id: 0,
        };
        let mut data = WINDOWCOMPOSITIONATTRIBDATA {
            attrib: WCA_ACCENT_POLICY,
            pv_data: &mut accent as *mut _ as *mut std::ffi::c_void,
            cb_data: std::mem::size_of::<AccentPolicy>(),
        };
        swca(hwnd, &mut data) != 0
    }
}

#[cfg(not(windows))]
pub fn apply_accent(_hwnd: isize, _state: u32, _tint: (u8, u8, u8), _alpha: u8) -> bool {
    false
}

/// Remove any accent policy (back to a plain, fully transparent window).
#[cfg(windows)]
pub fn clear_accent(hwnd: isize) -> bool {
    apply_accent(hwnd, ACCENT_DISABLED, (0, 0, 0), 0)
}

#[cfg(not(windows))]
pub fn clear_accent(_hwnd: isize) -> bool {
    false
}

/// `DwmIsCompositionEnabled` (dwmapi.dll, loaded dynamically): diagnostics only.
#[cfg(windows)]
pub fn is_composition_enabled() -> bool {
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};
    unsafe {
        let name: Vec<u16> = "dwmapi.dll\0".encode_utf16().collect();
        let dwm = LoadLibraryW(name.as_ptr());
        if dwm.is_null() {
            return false;
        }
        let addr = GetProcAddress(dwm, "DwmIsCompositionEnabled\0".as_ptr() as *const i8);
        if addr.is_null() {
            return false;
        }
        type Fn = unsafe extern "system" fn(*mut i32) -> i32;
        let f: Fn = std::mem::transmute(addr);
        let mut enabled: i32 = 0;
        f(&mut enabled) == 0 && enabled != 0
    }
}

#[cfg(not(windows))]
pub fn is_composition_enabled() -> bool {
    false
}

/// `WS_EX_LAYERED` present? Layered windows break DWM blur/backdrop compositing,
/// so this is logged as a diagnostic (winit must not set it for our window).
#[cfg(windows)]
pub fn is_layered(hwnd: isize) -> bool {
    use winapi::um::winuser::{GetWindowLongW, GWL_EXSTYLE, WS_EX_LAYERED};
    if hwnd == 0 {
        return false;
    }
    unsafe { (GetWindowLongW(hwnd as _, GWL_EXSTYLE) & WS_EX_LAYERED as i32) != 0 }
}

#[cfg(not(windows))]
pub fn is_layered(_hwnd: isize) -> bool {
    false
}

/// 清除整窗均匀 alpha（`WS_EX_LAYERED`）并强制一次 frame change。
///
/// 背景：一度尝试用 `WS_EX_LAYERED + LWA_ALPHA` 做「整窗半透明」，但实测
/// `SetLayeredWindowAttributes` 返回 TRUE 而像素毫无变化 —— 分层 alpha 只作用于
/// GDI 重定向表面，GPU 呈现的 OpenGL 内容走的是另一条合成路径。故不再主动设置，
/// 只保留本函数做**防御性清理**（旧版本或外部工具可能留下该扩展样式）。
#[cfg(windows)]
pub fn clear_uniform_alpha(hwnd: isize) -> bool {
    use winapi::um::winuser::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_LAYERED,
    };
    if hwnd == 0 {
        return false;
    }
    unsafe {
        let h = hwnd as _;
        let ex = GetWindowLongW(h, GWL_EXSTYLE);
        if (ex & WS_EX_LAYERED as i32) != 0 {
            SetWindowLongW(h, GWL_EXSTYLE, ex & !(WS_EX_LAYERED as i32));
            SetWindowPos(
                h,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        true
    }
}

#[cfg(not(windows))]
pub fn clear_uniform_alpha(_hwnd: isize) -> bool {
    false
}
