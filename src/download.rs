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
//! - **超时分层**：连接超时 / 首字节（响应头）超时 / 空闲超时分别定义，见下方常量。
//!   大文件下载**不设整体超时**，只用"连续 N 秒没有读到任何数据"的空闲超时；
//!   需要该语义时请用 `new_download_client()` 建客户端（API/JSON 小请求继续用 `new_client()`）。
//! - **先下到 `.part` 再改名**：所有落盘都是 `<目标名>.part`，校验字节数（已知远端大小时
//!   必须相等）与可选哈希后才 `rename` 到最终名；失败/中断删除 `.part`（无续传逻辑）。
//!   远端文件名一律经 `sanitize_remote_filename()` 消毒（防 zip-slip），`.part` 路径同源。

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
/// 计算摘要时的读取缓冲
const HASH_BUF_SIZE: usize = 256 * 1024;

// ---------- 超时常量（集中定义，便于统一调整） ----------

/// 连接（TCP/TLS 握手）超时
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 首字节 / 响应头超时：只用于体积很小的探测请求，避免连不上时长时间干等。
/// 真正的大文件传输不套这个上限（否则大文件必被掐断）。
pub const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(30);
/// 空闲超时：连续这么久**没有读到任何数据**才判定失败（读到数据即刷新窗口）。
/// 只有它管大文件，所以下载耗时再长也不会失败，只有真的卡死才会失败。
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// API / JSON 等小请求的整体上限（保持既有行为）
pub const API_TIMEOUT: Duration = Duration::from_secs(120);

/// 落盘临时文件后缀：先写 `<目标名>.part`，校验通过后改名
pub const PART_SUFFIX: &str = ".part";

/// 文件名是否可安全落盘：非空、不是 `.` / `..`、不含路径分隔符 / 盘符(`:`) / 控制字符。
fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.chars().any(|ch| ch.is_control())
}

/// 取远端文件名的**最后一段**并去掉危险成分：拒绝空/`.`/`..`/包含路径分隔符/盘符/控制字符；
/// 非法时回退到调用方给的默认名（如 "download.bin"）。
///
/// 远端返回的文件名（Content-Disposition / 版本 JSON）完全由对方控制，
/// 直接 `join` 到本地目录时 `..\..\x` 之类的名字会把文件写到目标目录之外（zip-slip）。
pub fn sanitize_remote_filename(raw: &str, fallback: &str) -> String {
    // 只取最后一段：同时兼容 `/` 与 `\`（远端可能返回 Windows 风格路径）
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let name = last.trim();
    if is_safe_file_name(name) {
        return name.to_string();
    }
    let fb = fallback.trim();
    if is_safe_file_name(fb) {
        return fb.to_string();
    }
    "download.bin".to_string()
}

/// 把调用方给的目标路径解析成 (最终路径, `.part` 临时路径)。
/// 名字经 `sanitize_remote_filename()` 消毒：两个路径必定仍在该文件的父目录内。
fn resolve_paths(dest: &Path) -> (PathBuf, PathBuf) {
    let raw_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_name = sanitize_remote_filename(&raw_name, "download.bin");
    let dir = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let final_path = dir.join(&file_name);
    let part_path = dir.join(format!("{file_name}{PART_SUFFIX}"));
    (final_path, part_path)
}

/// 创建统一的阻塞 HTTP 客户端（rustls-tls）：用于 **API / JSON 等小请求**。
/// 这里的 120s 是"等待响应头 + 每一次 read"的上限（blocking 语义），不是下载总时长上限。
pub fn new_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("XMST/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(API_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// 大文件下载专用客户端：**不设整体超时**，只设连接超时 + 空闲超时。
///
/// blocking reqwest 的 client 级 `timeout` 作用于两处：(1) 等待响应头；
/// (2) `Response::read()` 的**每一次**读取。因此它天然是"空闲超时"语义 ——
/// 只要还有数据到达就不断刷新读窗口，只有连续 `IDLE_TIMEOUT` 一个字节都没读到才失败；
/// 下载一个 2 小时的大文件不会被总时长掐断。
pub fn new_download_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("XMST/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(IDLE_TIMEOUT)
        // 保活探测：对端消失（拔网线/内核崩溃）时能更早让读操作报错
        .tcp_keepalive(Duration::from_secs(30))
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

// ---------- 摘要（不引入额外依赖） ----------

/// 流式 SHA-1（RFC 3174）
struct Sha1 {
    h: [u32; 5],
    buf: Vec<u8>,
    len: u64,
}

impl Sha1 {
    fn new() -> Self {
        Sha1 {
            h: [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }

    fn compress(&mut self, block: &[u8]) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks_exact(4).take(16).enumerate() {
            if let Some(slot) = w.get_mut(i) {
                *slot = u32::from_be_bytes([
                    c.first().copied().unwrap_or(0),
                    c.get(1).copied().unwrap_or(0),
                    c.get(2).copied().unwrap_or(0),
                    c.get(3).copied().unwrap_or(0),
                ]);
            }
        }
        for i in 16..80 {
            let x = w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16];
            w[i] = x.rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (self.h[0], self.h[1], self.h[2], self.h[3], self.h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
    }

    /// 追加数据（缓冲未满 64 字节的尾块，`buf.len() < 64` 恒成立）
    fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if !self.buf.is_empty() {
            let need = 64usize.saturating_sub(self.buf.len());
            let take = need.min(data.len());
            let (head, tail) = data.split_at(take);
            self.buf.extend_from_slice(head);
            data = tail;
            if self.buf.len() >= 64 {
                let block = std::mem::take(&mut self.buf);
                self.compress(&block);
                self.buf = Vec::with_capacity(64);
            }
        }
        let mut it = data.chunks_exact(64);
        for c in &mut it {
            self.compress(c);
        }
        let rest = it.remainder();
        if !rest.is_empty() {
            self.buf.extend_from_slice(rest);
        }
    }

    fn finish_hex(mut self) -> String {
        let bit_len = self.len.wrapping_mul(8);
        let mut tail: Vec<u8> = Vec::with_capacity(72);
        tail.push(0x80);
        while (self.buf.len() + tail.len()) % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bit_len.to_be_bytes());
        let mut all = std::mem::take(&mut self.buf);
        all.extend_from_slice(&tail);
        for c in all.chunks_exact(64) {
            self.compress(c);
        }
        let mut out = String::with_capacity(40);
        for v in self.h {
            out.push_str(&format!("{v:08x}"));
        }
        out
    }
}

/// 流式 SHA-256（FIPS 180-4）
struct Sha256 {
    h: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

const SHA256_K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4,
    0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe,
    0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f,
    0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da, 0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
    0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc,
    0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
    0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070, 0x19a4_c116,
    0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7,
    0xc671_78f2,
];

impl Sha256 {
    fn new() -> Self {
        Sha256 {
            h: [
                0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c,
                0x1f83_d9ab, 0x5be0_cd19,
            ],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }

    fn compress(&mut self, block: &[u8]) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).take(16).enumerate() {
            if let Some(slot) = w.get_mut(i) {
                *slot = u32::from_be_bytes([
                    c.first().copied().unwrap_or(0),
                    c.get(1).copied().unwrap_or(0),
                    c.get(2).copied().unwrap_or(0),
                    c.get(3).copied().unwrap_or(0),
                ]);
            }
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = self.h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for (dst, src) in self.h.iter_mut().zip(v.iter()) {
            *dst = dst.wrapping_add(*src);
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if !self.buf.is_empty() {
            let need = 64usize.saturating_sub(self.buf.len());
            let take = need.min(data.len());
            let (head, tail) = data.split_at(take);
            self.buf.extend_from_slice(head);
            data = tail;
            if self.buf.len() >= 64 {
                let block = std::mem::take(&mut self.buf);
                self.compress(&block);
                self.buf = Vec::with_capacity(64);
            }
        }
        let mut it = data.chunks_exact(64);
        for c in &mut it {
            self.compress(c);
        }
        let rest = it.remainder();
        if !rest.is_empty() {
            self.buf.extend_from_slice(rest);
        }
    }

    fn finish_hex(mut self) -> String {
        let bit_len = self.len.wrapping_mul(8);
        let mut tail: Vec<u8> = Vec::with_capacity(72);
        tail.push(0x80);
        while (self.buf.len() + tail.len()) % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bit_len.to_be_bytes());
        let mut all = std::mem::take(&mut self.buf);
        all.extend_from_slice(&tail);
        for c in all.chunks_exact(64) {
            self.compress(c);
        }
        let mut out = String::with_capacity(64);
        for v in self.h {
            out.push_str(&format!("{v:08x}"));
        }
        out
    }
}

/// 计算文件的十六进制摘要（流式，不整份读入内存）。`sha256=false` 时用 SHA-1。
pub fn file_hash_hex(path: &Path, sha256: bool) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| format!("打开文件失败: {e}"))?;
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    if sha256 {
        let mut h = Sha256::new();
        loop {
            let n = f.read(&mut buf).map_err(|e| format!("读取文件失败: {e}"))?;
            if n == 0 {
                break;
            }
            h.update(buf.get(..n).unwrap_or(&[]));
        }
        Ok(h.finish_hex())
    } else {
        let mut h = Sha1::new();
        loop {
            let n = f.read(&mut buf).map_err(|e| format!("读取文件失败: {e}"))?;
            if n == 0 {
                break;
            }
            h.update(buf.get(..n).unwrap_or(&[]));
        }
        Ok(h.finish_hex())
    }
}

/// 校验已下载文件：大小（若给了期望值）与可选哈希（sha1/sha256 十六进制，大小写不敏感）。
/// 第二个参数布尔值 = true 表示 sha256，false 表示 sha1。
/// 通过时返回**实际字节数**，便于调用方记录/展示。
pub fn verify_downloaded_file(
    path: &Path,
    expected_size: Option<u64>,
    expected_hash: Option<(&str, bool)>,
) -> Result<u64, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取文件信息失败: {e}"))?;
    if !meta.is_file() {
        return Err("校验目标不是文件".to_string());
    }
    let size = meta.len();
    if let Some(exp) = expected_size {
        if exp != size {
            return Err(format!(
                "文件大小校验失败：实际 {size} 字节，预期 {exp} 字节（可能下载不完整）"
            ));
        }
    }
    if let Some((want, is_sha256)) = expected_hash {
        let want = want.trim();
        if !want.is_empty() {
            let algo = if is_sha256 { "SHA-256" } else { "SHA-1" };
            let got = file_hash_hex(path, is_sha256)?;
            if !got.eq_ignore_ascii_case(want) {
                return Err(format!(
                    "{algo} 校验失败：实际 {got}，预期 {}（文件可能已损坏或被替换）",
                    want.to_ascii_lowercase()
                ));
            }
        }
    }
    Ok(size)
}

/// 校验通过后把 `.part` 改名成最终文件。
/// Windows 上 `rename` 一般可直接覆盖同名文件；仅当报"已存在/无权限"时才先删旧文件重试。
fn replace_file(part: &Path, dest: &Path) -> Result<(), String> {
    match std::fs::rename(part, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            if dest.exists() {
                let _ = std::fs::remove_file(dest);
                if std::fs::rename(part, dest).is_ok() {
                    return Ok(());
                }
            }
            Err(format!("重命名到最终文件失败: {e}"))
        }
    }
}

/// 下载单个文件（分片并发 + 进度回调），进度回调按 ~800ms 节流。
/// 返回 Ok(())；失败返回 Err(描述)。进度回调最后一个调用保证 phase=done。
/// 分片并发下载：支持 HTTP Range 且文件 >= 8MB 时按 `max_parts` 并发分片，
/// 否则单流退化。`max_parts` 为 0 时使用 `DEFAULT_MAX_PARTS`。
///
/// 落盘流程：写入 `<目标名>.part` → 校验字节数（远端大小已知时必须相等）→ `rename`。
/// **大文件请改用 `new_download_client()` 建客户端**（有硬性哈希/大小要求时用
/// `download_file_parallel_verified()`）。
pub fn download_file_parallel(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    max_parts: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>, &'static str),
) -> Result<(), String> {
    download_file_parallel_verified(client, url, dest, max_parts, None, None, on_progress)
}

/// 同 `download_file_parallel`，但可额外做**下载前大小核对 + 下载后哈希校验**
/// （既有公开函数签名保持不变，本函数为新增的可选校验入口）。
///
/// - `expected_size`：期望字节数；与远端大小不符时立即失败，下载后必须精确相等。
/// - `expected_hash`：`(十六进制摘要, 是否 SHA-256)`，大小写不敏感。
///
/// 校验在 `.part` 上完成后才改名到最终文件，因此**最终文件一定是校验过的**；
/// 失败时删除 `.part`（保留原同名文件不动）。
pub fn download_file_parallel_verified(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    max_parts: u64,
    expected_size: Option<u64>,
    expected_hash: Option<(&str, bool)>,
    on_progress: &mut dyn FnMut(u64, Option<u64>, &'static str),
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let (final_path, part_path) = resolve_paths(dest);
    let max_parts = if max_parts == 0 { DEFAULT_MAX_PARTS } else { max_parts };

    let r = download_to_part(
        client,
        url,
        &final_path,
        &part_path,
        max_parts,
        expected_size,
        expected_hash,
        on_progress,
    );
    if r.is_err() {
        // 失败/中断：删除残留的 .part（这里没有续传逻辑，不做保留）
        let _ = std::fs::remove_file(&part_path);
    }
    r
}

/// 实际下载流程：探测 → 写 `.part` → 校验 → 改名。
#[allow(clippy::too_many_arguments)]
fn download_to_part(
    client: &reqwest::blocking::Client,
    url: &str,
    final_path: &Path,
    part_path: &Path,
    max_parts: u64,
    expected_size: Option<u64>,
    expected_hash: Option<(&str, bool)>,
    on_progress: &mut dyn FnMut(u64, Option<u64>, &'static str),
) -> Result<(), String> {
    // 落盘名已由 resolve_paths() 消毒；分片文件也从这个已消毒名派生
    let file_name = final_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "download.bin".to_string());
    let part_dir = final_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    // ---- 探测 Range 支持与总大小（小请求：套首字节超时） ----
    let resp = client
        .get(url)
        .header("Range", "bytes=0-0")
        .timeout(FIRST_BYTE_TIMEOUT)
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

    // 下载前核对：远端大小已知且与期望不符就没必要下了（省流量也省时间）
    if let (Some(exp), Some(remote)) = (expected_size, total) {
        if exp != remote {
            return Err(format!(
                "远端文件大小与预期不符：远端 {remote} 字节，预期 {exp} 字节"
            ));
        }
    }
    // 校验用的期望大小：优先调用方给的，其次远端声明的
    let want_size = expected_size.or(total);

    // 空文件（416 / 0 长度）直接落空文件
    if total == Some(0) {
        File::create(part_path).map_err(|e| format!("创建文件失败: {e}"))?;
        verify_downloaded_file(part_path, want_size, expected_hash)?;
        replace_file(part_path, final_path)?;
        on_progress(0, Some(0), "done");
        return Ok(());
    }

    // ---- 分片模式：支持 Range 且 >= 8MB ----
    if range_ok {
        if let Some(total) = total {
            if total >= PART_MIN_SIZE {
                let parts = (total / PART_MIN_SIZE + 1).min(max_parts);
                let part_size = total / parts + 1;
                let done_bytes = Arc::new(AtomicU64::new(0));
                let done_parts = Arc::new(AtomicUsize::new(0));
                let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                // 分片片段：名字从已消毒的 file_name 派生，且以 `.` 开头（不会被当成正式文件）
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
                                    *err.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                                }
                            }
                            done_parts.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                    // 主线程轮询进度（节流回调），直到全部片完成或出错
                    let mut last = Instant::now() - PROGRESS_THROTTLE;
                    loop {
                        let finished = done_parts.load(Ordering::Relaxed);
                        let has_err = err.lock().unwrap_or_else(|e| e.into_inner()).is_some();
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
                if let Some(e) = err.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                    for p in &part_paths {
                        let _ = std::fs::remove_file(p);
                    }
                    return Err(e);
                }
                // 合并分片到 `.part`（合并完才校验、才改名）
                on_progress(total, Some(total), "merging");
                {
                    let mut out = File::create(part_path).map_err(|e| format!("创建文件失败: {e}"))?;
                    for p in &part_paths {
                        let mut f = File::open(p).map_err(|e| format!("打开分片失败: {e}"))?;
                        let mut buf = vec![0u8; BUF_SIZE];
                        loop {
                            let n = f.read(&mut buf).map_err(|e| format!("读分片失败: {e}"))?;
                            if n == 0 {
                                break;
                            }
                            out.write_all(buf.get(..n).unwrap_or(&[]))
                                .map_err(|e| format!("写文件失败: {e}"))?;
                        }
                    }
                    out.flush().map_err(|e| format!("刷新文件失败: {e}"))?;
                }
                for p in &part_paths {
                    let _ = std::fs::remove_file(p);
                }
                verify_downloaded_file(part_path, want_size, expected_hash)?;
                replace_file(part_path, final_path)?;
                on_progress(total, Some(total), "done");
                return Ok(());
            }
        }
    }

    // ---- 单流下载（小文件 / 不支持 Range） ----
    // 这里**不设**每请求整体超时：大文件靠客户端的空闲超时兜底（见 new_download_client）
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    if let (Some(exp), Some(remote)) = (expected_size, total) {
        if exp != remote {
            return Err(format!(
                "远端文件大小与预期不符：远端 {remote} 字节，预期 {exp} 字节"
            ));
        }
    }
    let want_size = expected_size.or(total);
    on_progress(0, total, "downloading");
    let mut out = File::create(part_path).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut downloaded: u64 = 0;
    let mut last = Instant::now() - PROGRESS_THROTTLE;
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("读取响应失败: {e}（连续 {} 秒无数据即中断，可重试）", IDLE_TIMEOUT.as_secs()))?;
        if n == 0 {
            break;
        }
        out.write_all(buf.get(..n).unwrap_or(&[]))
            .map_err(|e| format!("写文件失败: {e}"))?;
        downloaded += n as u64;
        if last.elapsed() >= PROGRESS_THROTTLE {
            on_progress(downloaded, total, "downloading");
            last = Instant::now();
        }
    }
    out.flush().map_err(|e| format!("刷新文件失败: {e}"))?;
    // 已知大小必须精确相等；再按需校验哈希；都过了才改名
    verify_downloaded_file(part_path, want_size, expected_hash)?;
    replace_file(part_path, final_path)?;
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
        out.write_all(buf.get(..n).unwrap_or(&[]))
            .map_err(|e| format!("写分片失败: {e}"))?;
        written += n as u64;
    }
    out.flush().map_err(|e| format!("刷新分片失败: {e}"))?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vectors() {
        let mut h = Sha1::new();
        h.update(b"abc");
        assert_eq!(h.finish_hex(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        let mut h = Sha1::new();
        h.update(b"");
        assert_eq!(h.finish_hex(), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        // 分多次喂数据（含跨 64 字节块边界）必须与一次喂完结果一致
        let one = {
            let mut h = Sha1::new();
            h.update(&[b'a'; 200]);
            h.finish_hex()
        };
        let split = {
            let mut h = Sha1::new();
            h.update(&[b'a'; 1]);
            h.update(&[b'a'; 63]);
            h.update(&[b'a'; 64]);
            h.update(&[b'a'; 72]);
            h.finish_hex()
        };
        assert_eq!(one, split);
        assert_eq!(one.len(), 40);
    }

    #[test]
    fn sha256_known_vectors() {
        let mut h = Sha256::new();
        h.update(b"abc");
        assert_eq!(
            h.finish_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut h = Sha256::new();
        h.update(b"");
        assert_eq!(
            h.finish_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // 分块喂数据与一次喂完结果一致（覆盖缓冲与多块路径）
        let one = {
            let mut h = Sha256::new();
            h.update(&[b'x'; 300]);
            h.finish_hex()
        };
        let split = {
            let mut h = Sha256::new();
            for _ in 0..10 {
                h.update(&[b'x'; 30]);
            }
            h.finish_hex()
        };
        assert_eq!(one, split);
        assert_eq!(one.len(), 64);
    }

    #[test]
    fn part_path_is_sanitized() {
        let (final_path, part_path) = resolve_paths(Path::new("C:\\srv\\..\\..\\evil.jar"));
        assert_eq!(final_path.file_name().and_then(|s| s.to_str()), Some("evil.jar"));
        assert_eq!(part_path.file_name().and_then(|s| s.to_str()), Some("evil.jar.part"));
        assert_eq!(part_path.parent(), final_path.parent());
    }

    #[test]
    fn hex_hash_compare_is_case_insensitive() {
        let dir = std::env::temp_dir().join("xmst_dl_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("hash_probe.bin");
        if std::fs::write(&p, b"abc").is_ok() {
            let ok = verify_downloaded_file(
                &p,
                Some(3),
                Some(("A9993E364706816ABA3E25717850C26C9CD0D89D", false)),
            );
            assert_eq!(ok, Ok(3));
            assert!(verify_downloaded_file(&p, Some(4), None).is_err());
            assert!(verify_downloaded_file(&p, None, Some(("00", false))).is_err());
        }
        let _ = std::fs::remove_file(&p);
    }
}
