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
//!   - xmst_set_bg(style: String)       -> "acrylic" | "translucent" | "default"
//!   - xmst_resource_dir() -> String    -> extracted resource dir of this plugin

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use rhai::{Dynamic, Engine, Scope, AST};

pub const MAX_LOG_LINES: usize = 200;
pub const DEFAULT_MAX_OPS: u64 = 10_000;
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
}

/// Messages sent from engine-registered API closures to the manager.
#[derive(Debug, Clone)]
pub enum PluginMsg {
    Log(String),
    Toast(String, String),
    SetBg(BgStyle),
}

/// Requested frosted-glass / acrylic window background effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgStyle {
    /// Transparent window + Win10 acrylic blur (best effort, Win10 1803+).
    Acrylic,
    /// Transparent window + translucent dark overlay (universal fallback).
    Translucent,
    /// Default opaque background (restore).
    Default,
}

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
    pub bg_request: Option<BgStyle>,
    pub toasts: Vec<(String, String)>,
    tx: mpsc::Sender<PluginMsg>,
    rx: mpsc::Receiver<PluginMsg>,
    pub states: HashMap<String, bool>,
}

thread_local! {
    /// Current plugin name during an event dispatch (used by resource API).
    static CURRENT_PLUGIN: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf, cache_dir: PathBuf, states: HashMap<String, bool>) -> Self {
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
        let dest = self.cache_dir.join(&manifest.name);
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
        let (engine, op_count) =
            Self::build_engine(&manifest, self.tx.clone(), self.cache_dir.clone());
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
        let fn_name = format!("on_{}", event);
        let targets: Vec<String> = self
            .instances
            .iter()
            .filter(|i| i.enabled)
            .map(|i| i.manifest.name.clone())
            .collect();
        for name in targets {
            let idx = match self.instances.iter().position(|i| i.manifest.name == name) {
                Some(i) => i,
                None => continue,
            };
            CURRENT_PLUGIN.with(|c| *c.borrow_mut() = name.clone());
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
        }
        CURRENT_PLUGIN.with(|c| c.borrow_mut().clear());
    }

    /// Drain API messages (log / toast / bg request) produced by script calls.
    pub fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                PluginMsg::Log(s) => self.push_log(s),
                PluginMsg::Toast(title, body) => self.toasts.push((title, body)),
                PluginMsg::SetBg(bg) => self.bg_request = Some(bg),
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
    ) -> (Engine, Arc<AtomicU64>) {
        let mut engine = Engine::new();
        let max_ops = manifest.max_operations.unwrap_or(DEFAULT_MAX_OPS);
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
        // xmst_log(msg)
        {
            let tx = tx.clone();
            engine.register_fn("xmst_log", move |msg: String| {
                let _ = tx.send(PluginMsg::Log(msg));
            });
        }
        // xmst_toast(title, body)
        {
            let tx = tx.clone();
            engine.register_fn("xmst_toast", move |title: String, body: String| {
                let _ = tx.send(PluginMsg::Toast(title, body));
            });
        }
        // xmst_set_bg(style)
        {
            let tx = tx.clone();
            engine.register_fn("xmst_set_bg", move |style: String| {
                let bg = match style.as_str() {
                    "acrylic" => BgStyle::Acrylic,
                    "translucent" => BgStyle::Translucent,
                    _ => BgStyle::Default,
                };
                let _ = tx.send(PluginMsg::SetBg(bg));
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
                cache.join(name).to_string_lossy().to_string()
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

/// Windows 10+ acrylic blur behind the window frame.
/// Uses SetWindowCompositionAttribute loaded dynamically from user32 so the
/// exe keeps running on older systems (returns false when unavailable).
/// ACCENT_ENABLE_ACRYLICBLURBEHIND only works while the window is transparent
/// (ViewportCommand::Transparent(true)) and the compositor allows it.
#[cfg(windows)]
pub fn apply_acrylic(hwnd: isize, enable: bool) -> bool {
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
    const ACCENT_DISABLED: u32 = 0;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

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
        let mut accent = AccentPolicy {
            accent_state: if enable {
                ACCENT_ENABLE_ACRYLICBLURBEHIND
            } else {
                ACCENT_DISABLED
            },
            accent_flags: 0,
            // AABBGGRR: 0xCC alpha -> ~80% dark tint behind the blur.
            gradient_color: if enable { 0xCC_00_00_00 } else { 0 },
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
pub fn apply_acrylic(_hwnd: isize, _enable: bool) -> bool {
    false
}
