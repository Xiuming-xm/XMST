//! 统一下载基础设施：reqwest(rustls-tls) 分片并发下载 + 进度回调 + 完成唤醒。
//!
//! 设计要点：
//! - 全部走阻塞客户端（blocking），实际网络 IO 由调用方放入独立线程，
//!   UI 线程只轮询 `DlProgress`（Arc<Mutex<..>>）快照，不做任何阻塞请求。
//! - rustls-tls 避免 OpenSSL / gnu 工具链问题；不启用默认 features（去 native-tls / 多余能力）。
//! - 分片策略：支持 HTTP Range 且文件 >= 8MB 时并发分片（最多 4 片），否则单流退化；
//!   分片期进度按已落盘字节轮询回调（约 800ms 节流），合并阶段标记 phase="merging"。
//! - 完成信号：`DlProgress.done=true`；托盘态调用方不轮询进度，下载线程结束前
//!   直接 `ctx.request_repaint()` 唤醒一次，满足"托盘态不轮询、完成才唤醒"。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 进度快照（UI 每帧读取，线程写入；字段均为轻量值）
#[derive(Debug, Clone, Default)]
pub struct DlProgress {
    /// 已下载字节数
    pub downloaded: u64,
    /// 总字节数（未知为 None）
    pub total: Option<u64>,
    /// 阶段：connecting / downloading / merging / done / error
    pub phase: &'static str,
    /// 错误信息（phase=error 时非空）
    pub error: Option<String>,
    /// 是否已完成（成功或失败）
    pub done: bool,
}

impl DlProgress {
    /// 进度比例 0.0..=1.0（总字节未知时为 0）
    pub fn fraction(&self) -> f32 {
        match self.total {
            Some(t) if t > 0 => (self.downloaded as f64 / t as f64).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }

    /// 进度百分比 0.0..=100.0
    pub fn percent(&self) -> f32 {
        self.fraction() * 100.0
    }
}

/// 进度回调节流间隔（约 800ms）
const PROGRESS_THROTTLE: Duration = Duration::from_millis(800);
/// 默认最大并发分片数（调用方未指定时使用）
pub const DEFAULT_MAX_PARTS: u64 = 32;
/// 触发分片的最小文件大小
const PART_MIN_SIZE: u64 = 8 * 1024 * 1024;
/// 单流下载读取缓冲
const BUF_SIZE: usize = 64 * 1024;

/// 创建统一的阻塞 HTTP 客户端（rustls-tls）
pub fn new_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("XMST/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// GET 并解析 JSON（供版本列表 / Modrinth API 使用）
pub fn fetch_json(client: &reqwest::blocking::Client, url: &str) -> Result<serde_json::Value, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json().map_err(|e| format!("JSON 解析失败: {e}"))
}

/// 下载单个文件（分片并发 + 进度回调），进度回调按 ~800ms 节流。
/// 返回 Ok(())；失败返回 Err(描述)。进度回调最后一个调用保证 phase=done/error。
/// 分片并发下载：支持 HTTP Range 且文件 >= 8MB 时按 `max_parts` 并发分片，
/// 否则单流退化。`max_parts` 为 0 时使用 `DEFAULT_MAX_PARTS`。
pub fn download_file_parallel(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    max_parts: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>, &'static str),
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let file_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "download.bin".to_string());
    let part_dir = dest.parent().unwrap_or(Path::new("."));
    let max_parts = if max_parts == 0 { DEFAULT_MAX_PARTS } else { max_parts };

    // ---- 探测 Range 支持与总大小 ----
    let resp = client
        .get(url)
        .header("Range", "bytes=0-0")
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    let range_ok = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let total = if range_ok {
        resp.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.trim().parse::<u64>().ok())
    } else {
        resp.content_length()
    };
    drop(resp);

    // 空文件（416 / 0 长度）直接落空文件
    if total == Some(0) {
        File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
        on_progress(0, Some(0), "done");
        return Ok(());
    }

    // 分片模式：支持 Range 且 >= 8MB
    if range_ok {
        if let Some(total) = total {
            if total >= PART_MIN_SIZE {
                let parts = (total / PART_MIN_SIZE + 1).min(max_parts);
                let part_size = total / parts + 1;
                let done_bytes = Arc::new(AtomicU64::new(0));
                let done_parts = Arc::new(AtomicUsize::new(0));
                let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                let part_paths: Vec<_> = (0..parts)
                    .map(|i| part_dir.join(format!(".{file_name}.part{i}")))
                    .collect();

                std::thread::scope(|s| {
                    for i in 0..parts {
                        let client = client.clone();
                        let url = url.to_string();
                        let part = part_paths[i as usize].clone();
                        let done_bytes = Arc::clone(&done_bytes);
                        let done_parts = Arc::clone(&done_parts);
                        let err = Arc::clone(&err);
                        let start = i * part_size;
                        let end = ((i + 1) * part_size - 1).min(total - 1);
                        s.spawn(move || {
                            let r = download_range(&client, &url, start, end, &part);
                            match r {
                                Ok(len) => {
                                    done_bytes.fetch_add(len, Ordering::Relaxed);
                                }
                                Err(e) => {
                                    *err.lock().unwrap() = Some(e);
                                }
                            }
                            done_parts.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                    // 主线程轮询进度（节流回调），直到全部片完成或出错
                    let mut last = Instant::now() - PROGRESS_THROTTLE;
                    loop {
                        let finished = done_parts.load(Ordering::Relaxed);
                        let has_err = err.lock().unwrap().is_some();
                        if finished >= parts as usize || has_err {
                            break;
                        }
                        if last.elapsed() >= PROGRESS_THROTTLE {
                            on_progress(done_bytes.load(Ordering::Relaxed), Some(total), "downloading");
                            last = Instant::now();
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                });
                if let Some(e) = err.lock().unwrap().clone() {
                    for p in &part_paths {
                        let _ = std::fs::remove_file(p);
                    }
                    return Err(e);
                }
                // 合并分片
                on_progress(total, Some(total), "merging");
                {
                    let mut out = File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
                    for p in &part_paths {
                        let mut f = File::open(p).map_err(|e| format!("打开分片失败: {e}"))?;
                        let mut buf = vec![0u8; BUF_SIZE];
                        loop {
                            let n = f.read(&mut buf).map_err(|e| format!("读分片失败: {e}"))?;
                            if n == 0 {
                                break;
                            }
                            out.write_all(&buf[..n])
                                .map_err(|e| format!("写文件失败: {e}"))?;
                        }
                    }
                }
                for p in &part_paths {
                    let _ = std::fs::remove_file(p);
                }
                on_progress(total, Some(total), "done");
                return Ok(());
            }
        }
    }

    // ---- 单流下载（小文件 / 不支持 Range） ----
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    on_progress(0, total, "downloading");
    let mut out = File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut downloaded: u64 = 0;
    let mut last = Instant::now() - PROGRESS_THROTTLE;
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("读取响应失败: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("写文件失败: {e}"))?;
        downloaded += n as u64;
        if last.elapsed() >= PROGRESS_THROTTLE {
            on_progress(downloaded, total, "downloading");
            last = Instant::now();
        }
    }
    out.flush().map_err(|e| format!("刷新文件失败: {e}"))?;
    on_progress(downloaded, total, "done");
    Ok(())
}

/// 下载单个 Range 分片到 part 路径（返回实际写入字节数）
fn download_range(
    client: &reqwest::blocking::Client,
    url: &str,
    start: u64,
    end: u64,
    part: &Path,
) -> Result<u64, String> {
    let mut resp = client
        .get(url)
        .header("Range", format!("bytes={start}-{end}"))
        .send()
        .map_err(|e| format!("分片请求失败: {e}"))?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("分片响应异常: HTTP {}", resp.status()));
    }
    let mut out = File::create(part).map_err(|e| format!("创建分片失败: {e}"))?;
    let mut buf = vec![0u8; BUF_SIZE];
    let mut written: u64 = 0;
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("读分片失败: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("写分片失败: {e}"))?;
        written += n as u64;
    }
    out.flush().map_err(|e| format!("刷新分片失败: {e}"))?;
    Ok(written)
}
