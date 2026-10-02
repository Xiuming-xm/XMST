use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use winapi::shared::minwindef::{BOOL, DWORD, LPVOID};
use winapi::um::handleapi::CloseHandle;
use winapi::um::jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject};
use winapi::um::winnt::{
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JobObjectExtendedLimitInformation,
};

/// 进程包装：持有子进程 + 日志通道
pub struct ManagedProcess {
    pub child: Arc<Mutex<Child>>,
    pub log_rx: Receiver<String>,
    /// 是否允许优雅停止（向 stdin 发送 stop）
    pub allow_stdin: bool,
    /// Windows Job Object（KILL_ON_JOB_CLOSE）：句柄关闭时整树终止。
    /// 主进程（本工具）崩溃 / 退出或 ManagedProcess 被 drop 时，MC 子进程不会成孤儿。
    _job: Option<*mut winapi::ctypes::c_void>,
}

// 裸句柄不参与跨线程共享；ManagedProcess 整体在线程间移动时由调用方保证安全。
unsafe impl Send for ManagedProcess {}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        // 关闭 Job 句柄：KILL_ON_JOB_CLOSE 会终止仍存活在作业内的全部进程（含孙子进程）。
        if let Some(j) = self._job.take() {
            unsafe {
                CloseHandle(j as _);
            }
        }
    }
}

/// 创建带 KILL_ON_JOB_CLOSE 的 Job Object；失败返回 None（调用方静默降级为无作业托管）。
fn create_kill_on_close_job() -> Option<*mut winapi::ctypes::c_void> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
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
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = Command::new(cmd);
    command
        .args(args)
        .current_dir(cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if allow_stdin {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }

    let mut child = command.spawn()?;

    // kill-on-drop：spawn 后立刻把子进程挂入 KILL_ON_JOB_CLOSE 作业。
    // 主进程崩溃 / 退出 / 托管对象被 drop 时整树自动终止，杜绝 MC 子进程成孤儿。
    // 挂载失败（如子进程已被其它 Job 托管）时静默降级为无作业托管，不影响启动。
    let mut job = create_kill_on_close_job();
    if let Some(j) = job {
        let hproc = child.as_raw_handle();
        unsafe {
            let ok: BOOL = AssignProcessToJobObject(j as _, hproc as _);
            if ok == 0 {
                CloseHandle(j as _);
                job = None; // 已关闭句柄，避免 drop 时二次关闭
            }
        }
    }

    let stdout = child.stdout.take().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "无法获取 stdout")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "无法获取 stderr")
    })?;

    let (tx, rx) = mpsc::channel::<String>();

    // stdout 读取线程
    let tx_out = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx_out.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // stderr 读取线程
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(ManagedProcess {
        child: Arc::new(Mutex::new(child)),
        log_rx: rx,
        allow_stdin,
        _job: job,
    })
}

/// 向进程 stdin 写一行（用于 send console command 如 stop）
pub fn write_stdin(proc: &ManagedProcess, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if !proc.allow_stdin {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "stdin 未启用",
        ));
    }
    let mut child = proc.child.lock().unwrap();
    if let Some(stdin) = child.stdin.as_mut() {
        writeln!(stdin, "{}", line)?;
        stdin.flush()?;
    }
    Ok(())
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
            let mut child = proc.child.lock().unwrap();
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
pub fn kill_tree(proc: &ManagedProcess) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let pid = {
        let mut child = proc.child.lock().unwrap();
        child.id()
    };
    // taskkill /PID <pid> /T /F：终止进程树，避免残留 java 进程
    // 必须隐藏窗口：taskkill 是控制台程序，直接 spawn 会闪现 cmd 窗口
    let _ = Command::new("taskkill")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // reap 主进程（可能已被 taskkill 结束）
    let mut child = proc.child.lock().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    true
}

/// 强制杀死（含整个进程树）
pub fn kill(proc: &ManagedProcess) -> bool {
    kill_tree(proc)
}

/// 检查进程是否还活着
pub fn is_running(proc: &ManagedProcess) -> bool {
    let mut child = proc.child.lock().unwrap();
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
    let mut child = proc.child.lock().unwrap();
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
        self.fail_count = 0;
        self.seen_data = false;
    }

    /// 跳过文件当前全部内容：重启/清空后调用，从文件末尾开始只读新增日志，
    /// 避免把上一次启动-关闭的旧日志重新拉进缓冲区（修复"重启不清空/假清空"）。
    pub fn seek_end(&mut self) {
        self.pending.clear();
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
            // 文件被轮转/重建：偏移归零，从头读新文件
            self.offset = 0;
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
        self.offset += chunk.len() as u64;

        // pending 过长保护（如 `\r` 进度条长期不换行）：强制切行
        if self.pending.len() > 65536 {
            buf.push_str(&self.pending);
            buf.push('\n');
            self.pending.clear();
        }

        let mut text = self.pending.clone();
        text.push_str(&String::from_utf8_lossy(&chunk));
        self.pending.clear();

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
