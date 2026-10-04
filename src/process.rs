use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use winapi::shared::minwindef::{BOOL, DWORD, LPVOID};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::jobapi::IsProcessInJob;
use winapi::um::jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject};
use winapi::um::processthreadsapi::{GetCurrentProcess, GetExitCodeProcess};
use winapi::um::winnt::{
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
};

/// stderr 末尾保留行数（崩溃诊断要展示"最后 30 行"，这里多留一些便于后续扩展）
const STDERR_TAIL_LINES: usize = 200;

// ==================== 启动诊断日志（data\launch.log） ====================
//
// 目的：定位"工具启动 bat 失败、但同样的命令在 PowerShell 里能成功"这一类问题。
// 每次启动尝试都写一组键值对记录：实际 program/args/cwd、传给子进程的环境变量、
// 本进程完整性级别与是否在 Job 内、CreateProcess 与 Job 分配结果、子进程 PID、
// 前 3 秒内捕获到的 stdout/stderr 首行、退出码（含 0xC0000142 这类 Windows 级错误码）。
//
// 约定：全部 best-effort —— 写日志的任何失败都直接忽略，绝不影响启动流程；
// 文件超过 1 MB 时只保留末尾 500 行（与 integrity_check.log 的裁剪思路一致）。

/// 启动日志超过该大小即裁剪
const LAUNCH_LOG_MAX_BYTES: u64 = 1024 * 1024;
/// 裁剪时保留的末尾行数
const LAUNCH_LOG_KEEP_LINES: usize = 500;
/// 单条记录里环境变量值最多截取的字符数（PATH 可能上千字符，全写进去反而看不清）
const ENV_SNIPPET_CHARS: usize = 200;

/// 启动尝试编号：每调用一次 `LaunchCtx::new` 递增，同一次启动的所有记录共用。
static LAUNCH_SEQ: AtomicU64 = AtomicU64::new(1);

/// `data\launch.log` 的路径（与配置/日志同级的 exe 目录下 data）。
pub fn launch_log_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data")
        .join("launch.log")
}

/// 追加一行启动诊断记录（best-effort：任何一步失败都忽略）。
/// 文件超过 1 MB 时先裁到末尾 500 行再追加。
pub fn launch_log_line(text: &str) {
    use std::io::Write;
    let path = launch_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > LAUNCH_LOG_MAX_BYTES {
            if let Ok(txt) = std::fs::read_to_string(&path) {
                let all: Vec<&str> = txt.lines().collect();
                let keep: Vec<&str> = all
                    .iter()
                    .skip(all.len().saturating_sub(LAUNCH_LOG_KEEP_LINES))
                    .copied()
                    .collect();
                let _ = std::fs::write(&path, keep.join("\n") + "\n");
            }
        }
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(
        f,
        "[{}] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        text
    );
}

/// 写一行"分节标题"（每次启动尝试之前的醒目分隔，便于直接在日志里定位一组记录）。
pub fn launch_log_sep(title: &str) {
    launch_log_line(&format!("==================== {title} ===================="));
}

/// 确保启动日志文件存在（「打开启动日志」按钮用：文件不存在时 explorer 选中会退化打开父目录）。
pub fn ensure_launch_log() {
    let path = launch_log_path();
    if path.exists() {
        return;
    }
    launch_log_line("启动日志已创建：此后每次启动的完整记录都会追加到这里");
}

/// 一次启动尝试的上下文：带同一编号的每一行记录都能直接看出"是谁、在什么条件下启动的"。
#[derive(Clone, Debug)]
pub struct LaunchCtx {
    /// 尝试编号（`启动#N`）
    pub id: u64,
    /// 服务器名
    pub server: String,
    /// 服务器目录
    pub dir: String,
    /// 启动方式说明（run.bat / launch_cmd 模板 / 诊断组合①…）
    pub kind: String,
}

impl LaunchCtx {
    /// 新建一次启动尝试上下文（自动分配编号）。
    pub fn new(server: &str, dir: &Path, kind: &str) -> Self {
        Self {
            id: LAUNCH_SEQ.fetch_add(1, Ordering::Relaxed),
            server: server.to_string(),
            dir: dir.display().to_string(),
            kind: kind.to_string(),
        }
    }

    /// 记录前缀：`启动#3 server="Fabric1.21.11" dir=D:\... 方式=run.bat`
    fn prefix(&self) -> String {
        format!(
            "启动#{} server=\"{}\" dir={} 方式={}",
            self.id, self.server, self.dir, self.kind
        )
    }
}

/// 带上下文写一行记录。
pub fn launch_log_ctx(ctx: &LaunchCtx, msg: &str) {
    launch_log_line(&format!("{} {}", ctx.prefix(), msg));
}

/// 读取环境变量值并截断（取不到写 `<未设置>`）。
fn env_snippet(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) => truncate_chars(&v, ENV_SNIPPET_CHARS),
        Err(_) => "<未设置>".to_string(),
    }
}

/// 按字符（不是字节）截断，避免把多字节字符切坏。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

/// 注入环境变量列表 → `[TEMP=..; TMP=..]`（供 main.rs 在"准备"记录里复用同一格式）
pub fn env_pairs_text(envs: &[(String, String)]) -> String {
    if envs.is_empty() {
        return "[]".to_string();
    }
    let items: Vec<String> = envs.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("[{}]", items.join("; "))
}

/// 本进程完整性级别（复用既有的 current_integrity_rid；Low=0x1000 / Medium=0x2000）。
fn integrity_rid_text() -> String {
    match crate::current_integrity_rid() {
        Some(r) => format!("{r:#x}"),
        None => "未知".to_string(),
    }
}

/// 本进程是否已被归入某个 Job 对象（hJob=NULL 表示"是否在任何 Job 内"；None=查询失败）。
pub fn is_self_in_job() -> Option<bool> {
    unsafe {
        let mut in_job: BOOL = 0;
        let ok = IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job);
        if ok == 0 {
            None
        } else {
            Some(in_job != 0)
        }
    }
}

/// 本进程是否在 Job 内 → 文本（供 launch.log 使用）。
fn self_in_job_text() -> String {
    match is_self_in_job() {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => "未知".to_string(),
    }
}

/// GetExitCodeProcess 的原始 DWORD（能看出 0xC0000142 这类 Windows 级错误码）。
/// 只在 `try_wait` 报告已退出后调用，避免读到 STILL_ACTIVE。
fn raw_exit_code(child: &Child) -> Option<u32> {
    let mut code: DWORD = 0;
    let ok = unsafe { GetExitCodeProcess(child.as_raw_handle() as _, &mut code) };
    if ok == 0 {
        None
    } else {
        Some(code)
    }
}

/// 只在该槽位为空时写入（即记录首行）；纯诊断用途，失败忽略。
fn record_first_line(slot: &Arc<Mutex<Option<String>>>, line: &str) {
    let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
    if s.is_none() {
        *s = Some(line.to_string());
    }
}

/// 槽位取值（空 → `<empty>`，明确写出来而不是留空白）。
fn first_line_text(slot: &Arc<Mutex<Option<String>>>) -> String {
    let s = slot.lock().unwrap_or_else(|e| e.into_inner());
    s.clone().unwrap_or_else(|| "<empty>".to_string())
}

/// 十六进制退出码文本（`0xC0000142`）。
fn hex_code(code: u32) -> String {
    format!("0x{code:08X}")
}

/// 启动后跟踪线程：
/// ① 满 3 秒时把"前 3 秒内"到达的 stdout/stderr 首行写进 launch.log（空则写 `<empty>`）；
/// ② 之后继续轮询等待进程退出，记录退出码与十六进制值（Java 从未启动时通常是 1 或 0xC0000142）。
/// 轮询间隔 1 秒、最长跟踪 12 小时，避免线程无限长眠。
fn spawn_launch_watcher(
    ctx: LaunchCtx,
    child: Arc<Mutex<Child>>,
    first_out: Arc<Mutex<Option<String>>>,
    first_err: Arc<Mutex<Option<String>>>,
) {
    const WINDOW: Duration = Duration::from_secs(3);
    const MAX_TRACK: Duration = Duration::from_secs(12 * 3600);
    thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < WINDOW {
            thread::sleep(Duration::from_millis(100));
        }
        launch_log_ctx(
            &ctx,
            &format!(
                "阶段=前3秒输出 stdout首行=\"{}\" stderr首行=\"{}\"",
                first_line_text(&first_out),
                first_line_text(&first_err)
            ),
        );
        loop {
            let exited = {
                let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
                match c.try_wait() {
                    Ok(Some(st)) => Some((st.code(), raw_exit_code(&c))),
                    _ => None,
                }
            };
            if let Some((code, raw)) = exited {
                let code_text = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let raw_text = raw.map(hex_code).unwrap_or_else(|| "?".to_string());
                launch_log_ctx(
                    &ctx,
                    &format!("阶段=退出 退出码={code_text} 退出码(hex)={raw_text}"),
                );
                break;
            }
            if started.elapsed() > MAX_TRACK {
                launch_log_ctx(&ctx, "阶段=退出 结果=跟踪超时（12 小时仍未退出，停止跟踪）");
                break;
            }
            thread::sleep(Duration::from_millis(1000));
        }
    });
}

/// 进程包装：持有子进程 + 日志通道
pub struct ManagedProcess {
    pub child: Arc<Mutex<Child>>,
    pub log_rx: Receiver<String>,
    /// 是否允许优雅停止（向 stdin 发送 stop）
    pub allow_stdin: bool,
    /// stderr 独立留存的末尾若干行（`STDERR_TAIL_LINES` 上限）。
    /// stdout/stderr 仍然合并进 log_rx（日志栏显示不变），这里额外留一份"只属于 stderr"的
    /// 副本：JVM 启动失败（例如 UnsupportedClassVersionError / 找不到主类）时错误只出现在
    /// stderr，崩溃弹窗需要把它单独列出来，不能靠日志栏去猜。
    pub stderr_tail: Arc<Mutex<VecDeque<String>>>,
    /// 本次启动的完整命令行（诊断信息展示用；cmd /c run.bat 形式则为 cmd /c run.bat）
    pub cmdline: String,
    /// Windows Job Object 句柄。**不再设置 KILL_ON_JOB_CLOSE**：
    /// 目的是让"本工具被崩溃/被任务管理器强杀"时服务器（java）继续运行，避免玩家集体掉线。
    /// 优雅退出（点关闭/菜单退出）由调用方显式结束进程树（见 `kill_tree`）；
    /// 句柄本身只用于把子进程归组，关闭它不会终止任何进程。
    _job: Option<*mut winapi::ctypes::c_void>,
}

// 裸句柄不参与跨线程共享；ManagedProcess 整体在线程间移动时由调用方保证安全。
unsafe impl Send for ManagedProcess {}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        // 仅关闭作业句柄（作业未设置 KILL_ON_JOB_CLOSE，关闭不会终止进程）。
        if let Some(j) = self._job.take() {
            unsafe {
                CloseHandle(j as _);
            }
        }
    }
}

/// 创建不带任何限制标志的 Job Object，仅用于把子进程归组。
///
/// 历史行为（已改）：这里曾设置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，好处是工具崩溃时
/// 服务器一起被回收，代价是"工具闪退 = 玩家全部掉线、世界可能未保存"。
/// 现在改为不设该标志：异常退出时服务器继续运行，下次启动工具会自动识别并提示接管。
fn create_process_job() -> Option<*mut winapi::ctypes::c_void> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        // LimitFlags = 0：不设任何限制，尤其不设 KILL_ON_JOB_CLOSE
        info.BasicLimitInformation.LimitFlags = 0;
        let ret: BOOL = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as LPVOID,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
        );
        if ret == 0 {
            CloseHandle(job);
            return None;
        }
        Some(job as *mut winapi::ctypes::c_void)
    }
}

/// 启动命令，创建隐藏窗口并捕获 stdout/stderr
pub fn spawn_hidden(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    allow_stdin: bool,
) -> std::io::Result<ManagedProcess> {
    spawn_hidden_env(cmd, args, cwd, allow_stdin, &[])
}

/// 同 spawn_hidden，但额外为子进程注入环境变量（如把 TEMP/TMP 指向服务器目录下的独立临时目录）。
/// envs 中的变量覆盖继承来的同名值，且只作用于这一个子进程，不影响本工具自身。
pub fn spawn_hidden_env(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    allow_stdin: bool,
    envs: &[(String, String)],
) -> std::io::Result<ManagedProcess> {
    let ctx = LaunchCtx::new("", cwd, "spawn_hidden_env");
    spawn_hidden_env_ctx(cmd, args, cwd, allow_stdin, envs, &ctx)
}

/// 与 `spawn_hidden_env` 完全相同，但带上本次启动的上下文（服务器名/目录/启动方式），
/// 让写进 `data\launch.log` 的每一行都能直接对上是哪一次启动。
pub fn spawn_hidden_env_ctx(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    allow_stdin: bool,
    envs: &[(String, String)],
    ctx: &LaunchCtx,
) -> std::io::Result<ManagedProcess> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // ---- 诊断记录（一）：把实际要传给 CreateProcess 的一切先落盘 ----
    let args_text = format!("[{}]", args.join(", "));
    let stdin_text = if allow_stdin { "piped" } else { "null" };
    let self_cwd = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    launch_log_ctx(
        ctx,
        &format!(
            "阶段=spawn program={} args={} cwd={} cwd存在={} stdin={} stdout=piped stderr=piped 标志=CREATE_NO_WINDOW(0x08000000) \
             继承环境_TEMP={} 继承环境_TMP={} 继承环境_PATH={} 注入环境={} 本进程完整性rid={} 本进程在Job内={} 本进程cwd={}",
            cmd,
            args_text,
            cwd.display(),
            cwd.exists(),
            stdin_text,
            env_snippet("TEMP"),
            env_snippet("TMP"),
            env_snippet("PATH"),
            env_pairs_text(envs),
            integrity_rid_text(),
            self_in_job_text(),
            self_cwd
        ),
    );

    let mut command = Command::new(cmd);
    command
        .args(args)
        .current_dir(cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        command.env(k, v);
    }
    if allow_stdin {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }

    // ---- 诊断记录（二）：CreateProcess 是否成功 + 子进程 PID ----
    let mut child = match command.spawn() {
        Ok(c) => {
            launch_log_ctx(ctx, &format!("阶段=CreateProcess 结果=成功 pid={}", c.id()));
            c
        }
        Err(e) => {
            let raw = e.raw_os_error();
            let raw_text = raw
                .map(|c| format!("{} ({})", c, hex_code(c as u32)))
                .unwrap_or_else(|| "无".to_string());
            launch_log_ctx(
                ctx,
                &format!("阶段=CreateProcess 结果=失败 错误=\"{e}\" raw_os_error={raw_text}"),
            );
            return Err(e);
        }
    };

    // 把子进程挂入一个不带任何限制的作业（分组用，不再 kill-on-close）。
    // 挂载失败（如子进程已被其它 Job 托管）时静默降级为无作业托管，不影响启动。
    // ---- 诊断记录（三）：Job 分配是否成功及其返回值 ----
    let mut job = create_process_job();
    match job {
        Some(j) => {
            let hproc = child.as_raw_handle();
            let ok: BOOL = unsafe { AssignProcessToJobObject(j as _, hproc as _) };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                launch_log_ctx(
                    ctx,
                    &format!(
                        "阶段=Job分配 结果=失败 返回值=0 GetLastError={} ({})（已降级为无 Job 托管）",
                        err,
                        hex_code(err)
                    ),
                );
                unsafe {
                    CloseHandle(j as _);
                }
                job = None; // 已关闭句柄，避免 drop 时二次关闭
            } else {
                launch_log_ctx(
                    ctx,
                    &format!("阶段=Job分配 结果=成功 返回值={} ({})", ok, hex_code(ok as u32)),
                );
            }
        }
        None => {
            launch_log_ctx(
                ctx,
                "阶段=Job分配 结果=失败（CreateJobObjectW/SetInformationJobObject 返回 0）",
            );
        }
    }

    let stdout = child.stdout.take().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "无法获取 stdout")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "无法获取 stderr")
    })?;

    let (tx, rx) = mpsc::channel::<String>();
    let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    let cmdline = if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, args.join(" "))
    };

    // 前 3 秒内的 stdout/stderr 首行（写 launch.log 用；不改变既有日志通道行为）
    let first_out: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let first_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // stdout 读取线程
    let tx_out = tx.clone();
    let first_out_t = first_out.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    record_first_line(&first_out_t, &l);
                    if tx_out.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // stderr 读取线程：既并入日志通道（保持既有显示），又单独留末尾若干行供崩溃弹窗展示
    let tail_shared = stderr_tail.clone();
    let first_err_t = first_err.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    record_first_line(&first_err_t, &l);
                    {
                        let mut tail = tail_shared.lock().unwrap_or_else(|e| e.into_inner());
                        tail.push_back(l.clone());
                        while tail.len() > STDERR_TAIL_LINES {
                            tail.pop_front();
                        }
                    }
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let child_arc = Arc::new(Mutex::new(child));
    // ---- 诊断记录（四/五）：3 秒输出首行 + 进程退出码（后台线程，best-effort） ----
    spawn_launch_watcher(ctx.clone(), child_arc.clone(), first_out, first_err);

    Ok(ManagedProcess {
        child: child_arc,
        log_rx: rx,
        allow_stdin,
        stderr_tail,
        cmdline,
        _job: job,
    })
}

/// 取该进程 stderr 的末尾 `max` 行（不足则全取；从未产生 stderr 时返回空 Vec，不算错误）。
pub fn stderr_tail_lines(proc: &ManagedProcess, max: usize) -> Vec<String> {
    let tail = proc.stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
    let n = tail.len();
    tail.iter().skip(n.saturating_sub(max)).cloned().collect()
}

/// 向进程 stdin 写一行（用于 send console command 如 stop）
pub fn write_stdin(proc: &ManagedProcess, line: &str) -> std::io::Result<()> {
    if !proc.allow_stdin {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "stdin 未启用",
        ));
    }
    write_stdin_child(&proc.child, line)
}

/// 同 write_stdin，但直接作用于共享的子进程句柄（`ManagedProcess.child` 的克隆）。
///
/// 用途：系统关机/注销时窗口过程要向所有服务器发 stop，但它拿不到 ManagedProcess
/// （那是 App 的字段，窗口过程不能借用 App），因此启动时把 child 句柄登记到全局表里，
/// 关机路径只用这个句柄。不检查 allow_stdin（由调用方判断）。
pub fn write_stdin_child(child: &Arc<Mutex<Child>>, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = child.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(stdin) = child.stdin.as_mut() {
        writeln!(stdin, "{}", line)?;
        stdin.flush()?;
    }
    Ok(())
}

/// 子进程是否仍在运行（按共享句柄查询，不阻塞）。
pub fn child_running(child: &Arc<Mutex<Child>>) -> bool {
    let mut child = child.lock().unwrap_or_else(|e| e.into_inner());
    matches!(child.try_wait(), Ok(None))
}

/// 获取主进程 PID（用于性能采样）
pub fn pid(proc: &ManagedProcess) -> Option<u32> {
    proc.child.lock().ok().map(|c| c.id())
}

/// 尝试优雅停止：写入 stop，最多等待 timeout 秒，超时则强制终止整个进程树
/// 注意：必须在后台线程中调用，避免阻塞 UI
pub fn stop_gracefully(proc: &ManagedProcess, timeout_secs: u64) -> bool {
    let _ = write_stdin(proc, "stop");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        {
            let mut child = proc.child.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(status) = child.try_wait().unwrap_or(None) {
                let _ = status;
                return true;
            }
        }
        if std::time::Instant::now() >= deadline {
            kill_tree(proc);
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// 强制杀死整个进程树（cmd -> java 等子进程一并终止）
/// 仅杀主进程会导致外壳(cmd)退出而子进程(java)继续存活
///
/// 返回值必须真实：taskkill 报告成功、或子进程句柄确认已退出才算成功，
/// 否则返回 false（调用方会据此显示"结束失败"而不是假的"已强制结束"）。
pub fn kill_tree(proc: &ManagedProcess) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let pid = {
        let mut child = proc.child.lock().unwrap_or_else(|e| e.into_inner());
        child.id()
    };
    // taskkill /PID <pid> /T /F：终止进程树，避免残留 java 进程
    // 必须隐藏窗口：taskkill 是控制台程序，直接 spawn 会闪现 cmd 窗口
    // 退出码 0 = 已终止目标（含整树）；128 = 进程已不存在；其余 = 失败（如权限不足）。
    let taskkill_ok = Command::new("taskkill")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|st| st.success())
        .unwrap_or(false);

    // 兜底直杀主进程（taskkill 定位不到或权限不足时它仍可能有效）。
    // 临界区只做非阻塞操作：child 锁还要被 is_running / exit_code / try_wait 使用，
    // 不能在这里长时间持有。
    {
        let mut child = proc.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
    }

    // 有上限地等待子进程真正退出（try_wait 轮询，最长 5 秒）。
    // 不用 child.wait()：它没有超时，遇到卡死的进程会把调用方（UI 线程）永久挂住。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut exited = false;
    loop {
        {
            let mut child = proc.child.lock().unwrap_or_else(|e| e.into_inner());
            match child.try_wait() {
                Ok(Some(_)) => {
                    exited = true;
                    break;
                }
                Ok(None) => {}
                // 查询失败：不再死等，交给下面的返回值判定
                Err(_) => break,
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // 只有真的成功才报成功：taskkill 报告成功，或子进程句柄确认已退出。
    // 两者都不成立（如权限不足且进程卡住）时返回 false，句柄保留在 child 里不丢弃。
    taskkill_ok || exited
}

/// 强制杀死（含整个进程树）
pub fn kill(proc: &ManagedProcess) -> bool {
    kill_tree(proc)
}

/// 检查进程是否还活着
pub fn is_running(proc: &ManagedProcess) -> bool {
    let mut child = proc.child.lock().unwrap_or_else(|e| e.into_inner());
    match child.try_wait() {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(_) => false,
    }
}

/// 同步读取子进程所有剩余输出并返回（阻塞直到退出）——用于批量日志归档
pub fn drain_to_string(proc: &ManagedProcess, max_lines: usize) -> String {
    let mut buf = String::new();
    let mut count = 0;
    while let Ok(line) = proc.log_rx.try_recv() {
        buf.push_str(&line);
        buf.push('\n');
        count += 1;
        if count >= max_lines {
            break;
        }
    }
    buf
}

/// 读取子进程退出码（若已退出）
pub fn exit_code(proc: &ManagedProcess) -> Option<i32> {
    let mut child = proc.child.lock().unwrap_or_else(|e| e.into_inner());
    match child.try_wait() {
        Ok(Some(status)) => status.code(),
        _ => None,
    }
}

/// 供 UI 每帧调用的日志拉取：把通道中所有行追加到 buf，并做行数裁剪。
/// 返回本次实际追加的行数（裁剪前计数），供 UI 判断是否有新日志需要贴底滚动。
pub fn drain_logs(proc: &ManagedProcess, buf: &mut String, max_lines: usize) -> usize {
    let mut added = 0usize;
    while let Ok(line) = proc.log_rx.try_recv() {
        buf.push_str(&line);
        buf.push('\n');
        added += 1;
    }
    trim_lines(buf, max_lines);
    added
}

/// Keep only the newest `max_lines` lines; older lines are dropped.
pub fn trim_lines(buf: &mut String, max_lines: usize) {
    if max_lines == 0 {
        return;
    }
    let lines = buf.lines().count();
    if lines <= max_lines {
        return;
    }
    // Drop the first `drop` lines from the head, keep the trailing max_lines lines.
    let drop = lines - max_lines;
    let bytes = buf.as_bytes();
    let mut idx = 0usize;
    let mut seen = 0usize;
    for &b in bytes {
        if b == b'\n' {
            seen += 1;
            if seen >= drop {
                idx += 1; // 越过第 drop 个换行，下一行开头
                break;
            }
        }
        idx += 1;
    }
    if idx > 0 && idx < buf.len() {
        *buf = buf[idx..].to_string();
    }
}

/// 日志文件尾随读取器：按字节偏移增量读取服务器日志文件（`logs/latest.log`）。
///
/// 背景：通过 `cmd /c run.bat -> java` 启动 MC 服务器时，Java 侧 stdout 在管道
/// 场景下可能被缓冲，日志在启动早期之后停滞、关服时未 flush 的缓冲直接丢失，
/// 导致 UI 日志卡死。而 log4j 的 FileAppender 是实时写盘的，因此改从日志文件
/// 增量读取，彻底绕开 stdout 缓冲链路（MCSManager / PCL2 同款做法）。
///
/// 调用方每帧调用 `tail_logs`；文件尚未出现（服务器启动初期）或已被轮转时会
/// 自动处理；多次尝试仍无文件时返回 `None`，由调用方回退到 stdout 管道。
pub struct LogFileTail {
    path: PathBuf,
    /// 上次已读取到的字节偏移
    offset: u64,
    /// 上一帧未闭合的行尾（文件中最后一段尚无 `\n` 的内容，下一帧拼上）
    pending: String,
    /// 上帧尾部未构成完整 UTF-8 序列的残留字节（正常最多 3 字节），下帧与新字节拼接再解码
    byte_tail: Vec<u8>,
    /// 残留字节连续未能解码成功的帧数（用于"长期压着不输出"的保护阀）
    undecoded_frames: u32,
    /// 连续打开失败次数
    fail_count: u32,
    /// 是否曾成功读到过数据（判定文件通道已生效）
    seen_data: bool,
}

impl LogFileTail {
    /// 为服务器目录创建尾随器（目标固定为 `{dir}/logs/latest.log`）。
    pub fn new(server_dir: &Path) -> Option<Self> {
        Some(Self {
            path: server_dir.join("logs").join("latest.log"),
            offset: 0,
            pending: String::new(),
            byte_tail: Vec::new(),
            undecoded_frames: 0,
            fail_count: 0,
            seen_data: false,
        })
    }

    /// 是否已确认文件通道生效（曾读到过数据）。
    pub fn seen_data(&self) -> bool {
        self.seen_data
    }

    /// 重置尾随状态：清空日志显示时调用，避免下次重读旧内容（偏移/挂起/失效计数全部归零）。
    pub fn reset(&mut self) {
        self.offset = 0;
        self.pending.clear();
        self.byte_tail.clear();
        self.undecoded_frames = 0;
        self.fail_count = 0;
        self.seen_data = false;
    }

    /// 跳过文件当前全部内容：重启/清空后调用，从文件末尾开始只读新增日志，
    /// 避免把上一次启动-关闭的旧日志重新拉进缓冲区（修复"重启不清空/假清空"）。
    pub fn seek_end(&mut self) {
        self.pending.clear();
        self.byte_tail.clear();
        self.undecoded_frames = 0;
        if let Ok(md) = std::fs::metadata(&self.path) {
            self.offset = md.len();
        } else {
            self.offset = u64::MAX; // 文件尚不存在：下一帧打开后 len<offset 会归零重读
        }
        self.fail_count = 0;
        self.seen_data = false;
    }

    /// 读取文件自上次偏移后的新增内容，按行追加到 `buf`（并裁剪到 `max_lines`）。
    ///
    /// 返回值：
    /// - `Some(n)`：文件通道有效，本次新增 `n` 行（可为 0）；
    /// - `None`：文件通道无效（启动后长时间未出现日志文件），调用方应回退 stdout。
    pub fn tail_logs(&mut self, buf: &mut String, max_lines: usize) -> Option<usize> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(_) => {
                self.fail_count += 1;
                // 启动初期文件尚未创建：最多容忍 60 帧（约 1 秒）；文件通道曾生效后
                // 短暂打不开（轮转间隙）：容忍 20 帧，避免误判失效。
                let limit = if self.seen_data { 20 } else { 60 };
                if self.fail_count < limit {
                    return Some(0);
                }
                return None;
            }
        };
        self.fail_count = 0;

        let len = match file.metadata() {
            Ok(m) => m.len(),
            Err(_) => return Some(0),
        };
        if len < self.offset {
            // 文件被轮转/重建：偏移归零，从头读新文件；旧文件残留的不完整字节一并丢弃
            self.offset = 0;
            self.byte_tail.clear();
            self.undecoded_frames = 0;
        }
        if len == self.offset {
            return Some(0);
        }

        // 单帧最多读 1MB，防止一次性读出巨量内容拖慢 UI
        let take = ((len - self.offset) as usize).min(1 << 20);
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.offset)).is_err() {
            return Some(0);
        }
        let mut chunk = Vec::with_capacity(take);
        let mut limited = reader.take(take as u64);
        if Read::read_to_end(&mut limited, &mut chunk).is_err() {
            return Some(0);
        }
        // 偏移只按"真正从文件读走的字节数"推进：解码不完整的尾部字节留在 byte_tail，
        // 不会因为 1MB 上限或写入时机正好切在多字节字符（中文）中间而永久损坏。
        self.offset += chunk.len() as u64;

        // pending 过长保护（如 `\r` 进度条长期不换行）：强制切行
        if self.pending.len() > 65536 {
            buf.push_str(&self.pending);
            buf.push('\n');
            self.pending.clear();
        }

        // 边界安全解码：上帧残留字节与本帧新字节拼接后一起解码，
        // 未构成完整序列的尾部（最多 3 字节）留到下次读取再拼。
        self.byte_tail.extend_from_slice(&chunk);
        let mut text = std::mem::take(&mut self.pending);
        let mut pos = 0usize;
        loop {
            let rest = match self.byte_tail.get(pos..) {
                Some(r) if !r.is_empty() => r,
                _ => break,
            };
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    text.push_str(s);
                    pos += s.len();
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    // valid_up_to 之前的字节由 from_utf8 保证是合法 UTF-8
                    if let Some(s) = self
                        .byte_tail
                        .get(pos..pos + valid)
                        .and_then(|b| std::str::from_utf8(b).ok())
                    {
                        text.push_str(s);
                    }
                    pos += valid;
                    match e.error_len() {
                        // 真正的非法字节（服务端输出了非 UTF-8 内容）：沿用旧行为替换成 � 后消费
                        Some(n) => {
                            text.push('\u{FFFD}');
                            pos += n;
                        }
                        // 文件末尾停在多字节字符中间：留着下次和新字节一起解码
                        None => break,
                    }
                }
            }
        }
        // 只保留真正未消费的残留字节（正常最多 3 字节）
        if pos >= self.byte_tail.len() {
            self.byte_tail.clear();
        } else if pos > 0 {
            let rest = self.byte_tail.split_off(pos);
            self.byte_tail = rest;
        }
        // 保护阀：残留异常膨胀或连续多帧都拼不出完整字符时，强行 lossy 消费并清理，
        // 避免这段字节一直压着不再往后输出。
        if self.byte_tail.is_empty() {
            self.undecoded_frames = 0;
        } else if self.byte_tail.len() > 8192 || self.undecoded_frames >= 120 {
            text.push_str(&String::from_utf8_lossy(&self.byte_tail));
            self.byte_tail.clear();
            self.undecoded_frames = 0;
        } else {
            self.undecoded_frames = self.undecoded_frames.saturating_add(1);
        }

        let mut added = 0usize;
        // 按 `\n` 切行；最后一段无换行则保留到 pending，下一帧拼上
        let mut parts = text.split_inclusive('\n');
        while let Some(part) = parts.next() {
            if part.ends_with('\n') {
                let line = part.trim_end_matches(['\n', '\r']);
                buf.push_str(line);
                buf.push('\n');
                added += 1;
            } else {
                self.pending = part.to_string();
            }
        }
        if added > 0 {
            self.seen_data = true;
            trim_lines(buf, max_lines);
        }
        Some(added)
    }
}

/// 丢弃 stdout 管道中所有待读行（文件通道生效时调用，避免同一日志两处显示）。
pub fn drain_logs_discard(proc: &ManagedProcess) -> usize {
    let mut n = 0usize;
    while proc.log_rx.try_recv().is_ok() {
        n += 1;
    }
    n
}

// ==================== 外部启动实例识别（不在本工具托管内的 java 进程） ====================
//
// 场景：用户在工具外双击 run.bat 启动了服务器。工具只能按文件尾随读到日志，
// 却既不知道"它在跑"，也无法停止/判定崩溃。这里提供识别它所需的三件事：
//   1) 列出全部 java/javaw 进程（PID + 映像名 + 命令行 + 工作集）；
//   2) 读取任意进程的命令行（优先 PEB，拿不到就给 None，由调用方退化判定）；
//   3) 按 PID 结束整棵进程树（外部进程没有 Child 句柄，只能走 taskkill /T /F）。
// 全部使用既有 winapi 依赖，不引入新库。

/// 一个 java/javaw 进程的快照。
#[derive(Clone, Debug, Default)]
pub struct JavaProc {
    /// 进程 ID
    pub pid: u32,
    /// 映像名（java.exe / javaw.exe，小写）
    pub exe: String,
    /// 命令行；读取失败（权限不足 / 32 位系统 / 系统保护进程）时为 None
    pub cmdline: Option<String>,
    /// 工作集（MB）
    pub mem_mb: f32,
}

/// 枚举当前所有 java/javaw 进程。
///
/// 单次快照 + 逐个补命令行与内存，成本约几毫秒，请按秒级节流调用，不要每帧调用。
pub fn list_java_processes() -> Vec<JavaProc> {
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::tlhelp32::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut out: Vec<JavaProc> = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let pid = entry.th32ProcessID;
                let name = wide_to_string(&entry.szExeFile).to_lowercase();
                if pid != 0 && (name == "java.exe" || name == "javaw.exe") {
                    out.push(JavaProc {
                        pid,
                        exe: name,
                        cmdline: None,
                        mem_mb: 0.0,
                    });
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    for p in out.iter_mut() {
        p.cmdline = process_command_line(p.pid);
        p.mem_mb = pid_memory_mb(p.pid);
    }
    out
}

/// 定长 UTF-16 数组 → String（遇到 NUL 截断）。
fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

/// 进程是否存活（OpenProcess 能拿到句柄且未被信号化）。PID 复用窗口极小，够用。
pub fn pid_alive(pid: u32) -> bool {
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winbase::WAIT_OBJECT_0;
    use winapi::um::winnt::{PROCESS_QUERY_LIMITED_INFORMATION, SYNCHRONIZE};
    if pid == 0 {
        return false;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return false;
        }
        let r = winapi::um::synchapi::WaitForSingleObject(h, 0);
        CloseHandle(h);
        r != WAIT_OBJECT_0
    }
}

/// 单个进程的工作集（MB）；取不到返回 0。
pub fn pid_memory_mb(pid: u32) -> f32 {
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
    if pid == 0 {
        return 0.0;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0.0;
        }
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        let ok = GetProcessMemoryInfo(
            h,
            &mut pmc,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        CloseHandle(h);
        if ok == 0 {
            0.0
        } else {
            pmc.WorkingSetSize as f32 / 1_048_576.0
        }
    }
}

/// 某个进程的全部后代 PID（不含自身）。
///
/// 用途：判断"外部启动实例"时必须把本工具托管进程的**整棵进程树**都排除掉。
/// 通过 `cmd /c run.bat` 启动时，本工具直接持有的是 cmd.exe，真正的 java 是它的子进程；
/// 只排除 cmd.exe 的 PID 会把自家托管的 java 误判成别台服务器的"外部实例"。
pub fn pid_descendants(root: u32) -> Vec<u32> {
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::tlhelp32::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let mut entries: Vec<(u32, u32)> = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    let mut out: Vec<u32> = Vec::new();
    let mut frontier: Vec<u32> = vec![root];
    while let Some(pid) = frontier.pop() {
        for &(child, parent) in entries.iter() {
            if parent == pid && child != root && !out.contains(&child) {
                out.push(child);
                frontier.push(child);
            }
        }
    }
    out
}

/// 按 PID 结束整棵进程树（外部实例用；没有 Child 句柄，只能靠 taskkill /T /F）。
/// 返回值必须真实：taskkill 报告成功、或进程已查不到，才算成功。
pub fn kill_pid_tree(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if pid == 0 {
        return false;
    }
    let ok = Command::new("taskkill")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|st| st.success())
        .unwrap_or(false);
    if ok || !pid_alive(pid) {
        return true;
    }
    // 有上限地等一次：taskkill /F 之后进程可能在几百毫秒内才真正退出
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

// ==================== 启动诊断：三种标志组合对比 ====================
//
// 「🧪 启动诊断」依次用同一套 program/args/cwd/环境 启动三次，唯一变量是标志组合：
//   ① 生产组合（CREATE_NO_WINDOW + Job + stdin 管道）—— 复现工具启动失败的那套；
//   ② 去掉 Job；
//   ③ 去掉 CREATE_NO_WINDOW（同时不分配 Job、stdin=null，即 spawn_diagnostic 的行为）。
// 每种都只观察 3 秒就强制结束进程树，结果写进 `data\launch.log` 并回传界面。
// 一次点击即可确定是"哪个标志/Job 导致失败"，还是"三种都成功（那问题在 main.rs 的启动编排）"。

/// 诊断启动的标志组合。
#[derive(Clone, Copy, Debug)]
pub enum DiagMode {
    /// ① 当前生产组合：CREATE_NO_WINDOW + stdout/stderr 管道 + stdin 管道 + 分配 Job
    Production,
    /// ② 去掉 Job：其余与生产组合一致
    NoJob,
    /// ③ 去掉 CREATE_NO_WINDOW：不建隐藏窗口、不分配 Job、stdin=null
    NoNoWindow,
}

impl DiagMode {
    /// 界面与日志里显示的组合说明。
    pub fn label(self) -> &'static str {
        match self {
            DiagMode::Production => "① 当前生产组合（CREATE_NO_WINDOW + Job + stdin 管道）",
            DiagMode::NoJob => "② 去掉 Job（CREATE_NO_WINDOW + stdin 管道）",
            DiagMode::NoNoWindow => "③ 去掉 CREATE_NO_WINDOW（无 Job、stdin=null）",
        }
    }
}

/// 一次诊断启动的观测结果。
pub struct DiagOutcome {
    /// 子进程 PID（cmd.exe 的 PID；java 是它的子进程）
    pub pid: u32,
    /// 前 3 秒内捕获到的 stdout 首行（空则 `<empty>`）
    pub stdout_first: String,
    /// 前 3 秒内捕获到的 stderr 首行（空则 `<empty>`）
    pub stderr_first: String,
    /// 3 秒窗口内**自行退出**时的原始退出码（可看出 0xC0000142 这类 Windows 级错误码）
    pub raw_exit_code: Option<u32>,
    /// 3 秒窗口结束时是否仍在运行（其后已被诊断强制结束）
    pub still_running: bool,
    /// 附注（Job 分配结果、结束确认情况等）
    pub note: String,
}

/// 诊断用直连启动：用与生产路径相同的 program/args/cwd/环境变量启动，但**不**加
/// CREATE_NO_WINDOW、**不**分配 Job、stdin 用 `Stdio::null()`，
/// 并返回 (子进程 PID, stdout 首行, stderr 首行, 自行退出时的退出码)。
///
/// 启动后最多观察 3 秒即强制结束整棵进程树（`kill_pid_tree` + 句柄兜底），
/// 保证诊断本身不会留下孤儿进程或占用端口。
pub fn spawn_diagnostic(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    envs: &[(String, String)],
) -> Result<(u32, String, String, Option<u32>), String> {
    // 调用方未提供服务器名时用目录名兜底，保证这批日志行也能自解释
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let ctx = LaunchCtx::new(&name, cwd, DiagMode::NoNoWindow.label());
    let o = spawn_diagnostic_mode(cmd, args, cwd, envs, DiagMode::NoNoWindow, &ctx)?;
    Ok((o.pid, o.stdout_first, o.stderr_first, o.raw_exit_code))
}

/// 按指定标志组合做一次诊断启动（「🧪 启动诊断」用；`ctx` 决定日志里的服务器与编号）。
pub fn spawn_diagnostic_mode(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    envs: &[(String, String)],
    mode: DiagMode,
    ctx: &LaunchCtx,
) -> Result<DiagOutcome, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let use_no_window = !matches!(mode, DiagMode::NoNoWindow);
    let use_stdin_pipe = !matches!(mode, DiagMode::NoNoWindow);
    let use_job = matches!(mode, DiagMode::Production);

    launch_log_ctx(
        ctx,
        &format!(
            "阶段=诊断启动 组合={} program={} args=[{}] cwd={} cwd存在={} stdin={} stdout=piped stderr=piped 标志={} Job={} \
             继承环境_TEMP={} 继承环境_TMP={} 继承环境_PATH={} 注入环境={} 本进程完整性rid={} 本进程在Job内={} 本进程cwd={}",
            mode.label(),
            cmd,
            args.join(", "),
            cwd.display(),
            cwd.exists(),
            if use_stdin_pipe { "piped" } else { "null" },
            if use_no_window { "CREATE_NO_WINDOW(0x08000000)" } else { "无（会短暂出现控制台窗口）" },
            if use_job { "分配" } else { "不分配" },
            env_snippet("TEMP"),
            env_snippet("TMP"),
            env_snippet("PATH"),
            env_pairs_text(envs),
            integrity_rid_text(),
            self_in_job_text(),
            std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|_| "?".to_string())
        ),
    );

    let mut command = Command::new(cmd);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if use_no_window {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    for (k, v) in envs {
        command.env(k, v);
    }
    if use_stdin_pipe {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }

    let mut child = match command.spawn() {
        Ok(c) => {
            launch_log_ctx(
                ctx,
                &format!("阶段=诊断CreateProcess 结果=成功 pid={}", c.id()),
            );
            c
        }
        Err(e) => {
            let raw = e.raw_os_error();
            let raw_text = raw
                .map(|c| format!("{} ({})", c, hex_code(c as u32)))
                .unwrap_or_else(|| "无".to_string());
            launch_log_ctx(
                ctx,
                &format!(
                    "阶段=诊断CreateProcess 结果=失败 错误=\"{e}\" raw_os_error={raw_text}"
                ),
            );
            return Err(format!("{e}（raw_os_error={raw_text}）"));
        }
    };
    let pid = child.id();

    // Job 只在本组合需要时分配；句柄保持到本次诊断结束再关闭（没有 KILL_ON_JOB_CLOSE，关闭不影响进程）
    let mut job_handle: Option<*mut winapi::ctypes::c_void> = None;
    let mut note = String::new();
    if use_job {
        match create_process_job() {
            Some(j) => {
                let ok: BOOL =
                    unsafe { AssignProcessToJobObject(j as _, child.as_raw_handle() as _) };
                if ok == 0 {
                    let err = unsafe { GetLastError() };
                    note.push_str(&format!(
                        "Job分配失败(GetLastError={err}/{})；",
                        hex_code(err)
                    ));
                    unsafe {
                        CloseHandle(j as _);
                    }
                } else {
                    note.push_str("Job分配成功；");
                    job_handle = Some(j);
                }
            }
            None => note.push_str("Job创建失败；"),
        }
        launch_log_ctx(ctx, &format!("阶段=诊断Job分配 {note}"));
    }

    // stdin 为管道时必须一直持有写端：句柄一被丢弃就等于给子进程 EOF，
    // 而这正是要对比的差异之一，不能让诊断自己把管道提前关掉。
    let _stdin_hold = if use_stdin_pipe { child.stdin.take() } else { None };

    // 只记录首行的两个槽位（读者线程把管道读干，避免子进程写满管道阻塞）
    let first_out: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let first_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    if let Some(out) = child.stdout.take() {
        let slot = first_out.clone();
        thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                match line {
                    Ok(l) => record_first_line(&slot, &l),
                    Err(_) => break,
                }
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let slot = first_err.clone();
        thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines() {
                match line {
                    Ok(l) => record_first_line(&slot, &l),
                    Err(_) => break,
                }
            }
        });
    }

    // 观察窗口：固定 3 秒（进程提前退出也等满，好让读者线程把已到达的输出放进槽位）
    let started = Instant::now();
    let mut raw_exit: Option<u32> = None;
    while started.elapsed() < Duration::from_secs(3) {
        if raw_exit.is_none() {
            if let Ok(Some(_)) = child.try_wait() {
                raw_exit = raw_exit_code(&child);
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(200));
    let stdout_first = first_line_text(&first_out);
    let stderr_first = first_line_text(&first_err);
    let still_running = raw_exit.is_none();

    // 每次尝试结束都必须结束子进程及其整棵树（cmd -> java），避免留下孤儿进程或占用端口
    if still_running {
        let killed = kill_pid_tree(pid);
        let _ = child.kill();
        note.push_str(&format!("已结束进程树(kill_pid_tree={killed})；"));
    } else {
        note.push_str("进程在观察窗口内自行退出；");
    }
    // 有上限地确认退出（taskkill 之后可能还要几百毫秒）
    let mut confirmed = !still_running;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                confirmed = true;
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    if !confirmed {
        note.push_str("⚠ 结束后仍未确认退出，请在任务管理器确认；");
    }

    if let Some(j) = job_handle {
        unsafe {
            CloseHandle(j as _);
        }
    }

    launch_log_ctx(
        ctx,
        &format!(
            "阶段=诊断结束 仍在运行={} 退出码={} 退出码(hex)={} stdout首行=\"{}\" stderr首行=\"{}\" 附注={}",
            still_running,
            raw_exit
                .map(|c| c.to_string())
                .unwrap_or_else(|| "无（未自行退出）".to_string()),
            raw_exit.map(hex_code).unwrap_or_else(|| "无".to_string()),
            stdout_first,
            stderr_first,
            note
        ),
    );

    Ok(DiagOutcome {
        pid,
        stdout_first,
        stderr_first,
        raw_exit_code: raw_exit,
        still_running,
        note,
    })
}

/// 读取任意进程的命令行。
///
/// 手段：OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ) →
/// `NtQueryInformationProcess(ProcessBasicInformation)` 取 PEB →
/// `ReadProcessMemory` 读 PEB.ProcessParameters →
/// 读 RTL_USER_PROCESS_PARAMETERS.CommandLine（UNICODE_STRING）。
/// 64 位下这两个结构的偏移是固定的（0x20 / 0x70），本工具只出 x86_64 产物，
/// 因此按 64 位偏移读取；非 64 位目标直接返回 None，由调用方退化判定。
pub fn process_command_line(pid: u32) -> Option<String> {
    #[cfg(target_arch = "x86_64")]
    {
        read_command_line_x64(pid)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_arch = "x86_64")]
fn read_command_line_x64(pid: u32) -> Option<String> {
    use winapi::ctypes::c_void;
    use winapi::um::memoryapi::ReadProcessMemory;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

    #[repr(C)]
    struct ProcessBasicInformation {
        reserved1: *mut c_void,
        peb_base_address: *mut c_void,
        reserved2: [*mut c_void; 2],
        unique_process_id: usize,
        reserved3: *mut c_void,
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            handle: *mut c_void,
            info_class: u32,
            info: *mut c_void,
            info_len: u32,
            ret_len: *mut u32,
        ) -> i32;
    }

    const PROCESS_BASIC_INFORMATION_CLASS: u32 = 0;
    const PEB_PROCESS_PARAMETERS_OFFSET: usize = 0x20;
    const PARAMS_COMMAND_LINE_OFFSET: usize = 0x70;
    /// 命令行长度上限（UTF-16 字节数）：Windows 上限约 32767 字符，超过必然是读错了
    const MAX_CMDLINE_BYTES: usize = 65536;

    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut pbi: ProcessBasicInformation = std::mem::zeroed();
        let mut got = 0u32;
        let st = NtQueryInformationProcess(
            h,
            PROCESS_BASIC_INFORMATION_CLASS,
            &mut pbi as *mut _ as *mut c_void,
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            &mut got,
        );
        if st < 0 || pbi.peb_base_address.is_null() {
            CloseHandle(h);
            return None;
        }
        let mut read = 0usize;
        let mut params: usize = 0;
        let ok = ReadProcessMemory(
            h,
            (pbi.peb_base_address as usize + PEB_PROCESS_PARAMETERS_OFFSET) as *const c_void,
            &mut params as *mut usize as *mut c_void,
            std::mem::size_of::<usize>(),
            &mut read,
        );
        if ok == 0 || params == 0 {
            CloseHandle(h);
            return None;
        }
        // UNICODE_STRING: Length(u16) MaximumLength(u16) 对齐填充(u32) Buffer(ptr)
        let mut us = [0u8; 16];
        let ok = ReadProcessMemory(
            h,
            (params + PARAMS_COMMAND_LINE_OFFSET) as *const c_void,
            us.as_mut_ptr() as *mut c_void,
            us.len(),
            &mut read,
        );
        if ok == 0 {
            CloseHandle(h);
            return None;
        }
        let len = u16::from_le_bytes([us[0], us[1]]) as usize;
        let buf_ptr = usize::from_le_bytes([us[8], us[9], us[10], us[11], us[12], us[13], us[14], us[15]]);
        if len == 0 || len > MAX_CMDLINE_BYTES || buf_ptr == 0 || len % 2 != 0 {
            CloseHandle(h);
            return None;
        }
        let mut raw = vec![0u8; len];
        let ok = ReadProcessMemory(
            h,
            buf_ptr as *const c_void,
            raw.as_mut_ptr() as *mut c_void,
            len,
            &mut read,
        );
        CloseHandle(h);
        if ok == 0 || read < len {
            return None;
        }
        let wide: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let s = wide_to_string(&wide);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}
