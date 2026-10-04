//! 桌面捕获式窗口背景（唯一在本机环境可用的「半透明 / 毛玻璃 / 亚克力」实现）。
//!
//! 背景：本机（Win10 19045 + NVIDIA + 虚拟/远程显示会话）实测**任何系统级透明机制都无效**：
//!   * `DwmEnableBlurBehindWindow`（空区域/整窗区域）—— 返回 S_OK，但窗口 alpha 不被合成；
//!   * `SetWindowCompositionAttribute` 亚克力 accent —— 返回 TRUE，但完全不可见；
//!   * `DwmExtendFrameIntoClientArea(-1)` —— 返回 S_OK，无效果；
//!   * `WS_EX_LAYERED` + `SetLayeredWindowAttributes` —— 返回 TRUE，GL 内容完全不淡化
//!     （分层 alpha 只作用于 GDI 重定向表面，GPU 呈现的 swapchain 走另一条路）。
//! 根因是 OpenGL 的呈现路径（现代驱动 / DXGI flip 模型）本身不透明，宿主程序无法改变。
//!
//! 因此改为「把窗口背后的桌面抓进窗口当背景」：
//!   1. `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` —— 抓屏时排除本窗口，避免自我递归；
//!   2. GDI `BitBlt` 抓取窗口所在屏幕区域 → 降采样（降采样+线性放大 = 廉价模糊）；
//!   3. 作为 egui 纹理画在 background 层，UI 面板按不透明度叠在上面。
//! 效果与 DWM 亚克力在观感上等价（毛玻璃=强降采样模糊，半透明=原图/轻模糊），
//! 且在**任何**系统/驱动/远程会话下都成立。
//!
//! 代价（需在文档中告知用户）：开启效果期间，本窗口会被系统排除在截屏/录屏/共享屏幕之外。

use std::sync::{Arc, Mutex};
use std::time::Instant;


/// `WDA_EXCLUDEFROMCAPTURE`（Win10 2004+；winapi 0.3 未提供该常量，自行定义）。
const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;
/// `WDA_NONE`
const WDA_NONE: u32 = 0x0000_0000;

/// 一个可复用的抓屏缓冲 + egui 纹理。
///
/// 抓屏在**工作线程**上完成（`worker`）：单次抓屏在原图档（ds=1）下可达 ~60-100ms，
/// 放在 UI 线程上会直接表现为掉帧/闪动（拖动时尤其明显）。UI 线程只做两件事：
/// 提交参数、把结果上传成纹理。
pub struct BackdropCapture {
    tex: Option<egui::TextureHandle>,
    last: Option<Instant>,
    /// 当前是否已对本窗口设置「排除抓屏」。
    affinity_set: bool,
    /// 纹理对应的像素尺寸，尺寸变化时需要重建纹理。
    size: (usize, usize),
    /// 诊断：最近一次抓取到的画面统计（均值 + 采样点），写入 bg_debug.log
    last_mean: (u8, u8, u8),
    last_samples: Vec<(u8, u8, u8)>,
    captures: u64,
    /// 最近一次抓取的窗口矩形（用于几何变化时强制重抓）
    last_rect: (i32, i32, i32, i32),
    /// 最近一次抓取使用的降采样倍率（UV 偏移换算是按它算的）
    last_ds: i32,
    /// 最近一次抓取耗时（毫秒，含工作线程里的全部处理）
    last_ms: f32,
    /// 历史最大耗时（毫秒）
    max_ms: f32,
    /// 请求序号：每次提交参数 +1，用于识别结果是否已被应用
    seq: u64,
    applied_seq: u64,
    /// 是否有一次抓取正在工作线程上执行（避免排队堆积）
    pending: bool,
    shared: Arc<Mutex<Shared>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

/// UI 线程 ↔ 工作线程之间传递的任务/结果。
#[derive(Default)]
struct Shared {
    job: Option<Job>,
    done: Option<Done>,
    stop: bool,
}

#[derive(Clone, Copy)]
struct Job {
    seq: u64,
    rect: (i32, i32, i32, i32),
    ds: i32,
    blur: i32,
}

struct Done {
    seq: u64,
    rect: (i32, i32, i32, i32),
    ds: i32,
    size: (usize, usize),
    px: Vec<egui::Color32>,
    mean: (u8, u8, u8),
    samples: Vec<(u8, u8, u8)>,
    ms: f32,
}

impl Drop for BackdropCapture {
    fn drop(&mut self) {
        // 锁中毒也要写入停止标志，否则工作线程永不退出、join 会一直等
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
        if let Some(h) = self.worker.take() {
            wait_worker(h);
        }
    }
}

/// 有界等待工作线程退出：线程已结束立即返回，超时后放弃等待（句柄分离）。
///
/// 抓屏走 GDI/DWM，极少数情况下可能卡在系统调用里；托盘态/退出流程不能被它无界拖住。
fn wait_worker(h: std::thread::JoinHandle<()>) {
    /// 最长等待时间
    const WAIT_MAX: std::time::Duration = std::time::Duration::from_millis(1500);
    /// 轮询间隔
    const POLL: std::time::Duration = std::time::Duration::from_millis(2);
    let deadline = Instant::now() + WAIT_MAX;
    while !h.is_finished() {
        if Instant::now() >= deadline {
            // 超时：不再 join（detach），避免无界等待
            return;
        }
        std::thread::sleep(POLL);
    }
    let _ = h.join();
}

impl Default for BackdropCapture {
    fn default() -> Self {
        Self {
            tex: None,
            last: None,
            affinity_set: false,
            size: (0, 0),
            last_mean: (0, 0, 0),
            last_samples: Vec::new(),
            captures: 0,
            last_rect: (0, 0, 0, 0),
            last_ds: 1,
            last_ms: 0.0,
            max_ms: 0.0,
            seq: 0,
            applied_seq: 0,
            pending: false,
            shared: Arc::new(Mutex::new(Shared::default())),
            worker: None,
        }
    }
}

impl BackdropCapture {
    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.tex.as_ref()
    }

    /// 最近一次抓取使用的窗口矩形。
    pub fn last_rect(&self) -> (i32, i32, i32, i32) {
        self.last_rect
    }

    /// 最近一次抓取使用的降采样倍率。
    pub fn last_downscale(&self) -> i32 {
        self.last_ds.max(1)
    }

    /// 丢弃纹理句柄。
    ///
    /// 托盘态会清空 egui 缓存并重置字体（把工作集压到 ~2MB），此后旧的纹理句柄可能已失效；
    /// 恢复时必须**重新创建纹理**而不是往失效句柄 `set()`。
    pub fn forget_texture(&mut self) {
        self.tex = None;
        self.size = (0, 0);
    }

    /// 诊断信息：(均值 RGB, 采样点, 抓取次数, 纹理尺寸, 最近耗时 ms, 最大耗时 ms)
    pub fn stats(&self) -> ((u8, u8, u8), &[(u8, u8, u8)], u64, (usize, usize), f32, f32) {
        (
            self.last_mean,
            &self.last_samples,
            self.captures,
            self.size,
            self.last_ms,
            self.max_ms,
        )
    }

    /// 释放复用的 GDI 对象（旧同步实现遗留；现由工作线程自持 GDI 对象）。
    #[cfg(windows)]
    fn release_gdi(&mut self) {}

    #[cfg(not(windows))]
    fn release_gdi(&mut self) {}

    /// 打开/关闭「本窗口排除抓屏」。关闭时恢复 `WDA_NONE`。
    #[cfg(windows)]
    pub fn set_capture_exclusion(hwnd: isize, excluded: bool) -> bool {
        use winapi::um::winuser::SetWindowDisplayAffinity;
        if hwnd == 0 {
            return false;
        }
        let flag = if excluded {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        unsafe { SetWindowDisplayAffinity(hwnd as _, flag) != 0 }
    }

    #[cfg(not(windows))]
    pub fn set_capture_exclusion(_hwnd: isize, _excluded: bool) -> bool {
        false
    }

    /// 确保抓屏排除状态与 `wanted` 一致（只在变化时调用系统 API）。
    pub fn ensure_exclusion(&mut self, hwnd: isize, wanted: bool) -> bool {
        if wanted == self.affinity_set {
            return true;
        }
        let ok = Self::set_capture_exclusion(hwnd, wanted);
        if ok || !wanted {
            self.affinity_set = wanted;
        }
        ok
    }

    /// 按需刷新背景纹理（非阻塞）。
    ///
    /// * `rect_px`：窗口在屏幕上的物理矩形 (x, y, w, h)；
    /// * `downscale`：降采样倍率（1 = 原图，最清晰；越大越糊）；
    /// * `blur_px`：缩小后小图上的盒式模糊半径（0 = 不模糊，用于「半透明」）；
    /// * `min_interval`：本次允许的最小抓取间隔（0 = 立即）。
    ///
    /// 返回 `true` 表示**本帧上传了新纹理**（抓取本身在工作线程上进行，不阻塞 UI）。
    #[cfg(windows)]
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        rect_px: (i32, i32, i32, i32),
        downscale: i32,
        blur_px: i32,
        min_interval: std::time::Duration,
    ) -> bool {
        let (x, y, w, h) = rect_px;
        if w <= 0 || h <= 0 {
            return false;
        }
        self.ensure_worker();

        // 1) 提交任务（上一单还没做完就不排新单，避免堆积）
        if !self.pending {
            let due = self
                .last
                .map(|t| t.elapsed() >= min_interval)
                .unwrap_or(true);
            if due {
                self.seq += 1;
                self.last = Some(Instant::now());
                self.last_rect = rect_px;
                self.pending = true;
                let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
                s.job = Some(Job {
                    seq: self.seq,
                    rect: (x, y, w, h),
                    ds: downscale.clamp(1, 32),
                    blur: blur_px,
                });
            }
        }

        // 2) 收取结果并上传纹理
        let done = {
            let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            s.done.take()
        };
        let Some(d) = done else {
            return false;
        };
        self.pending = false;
        if d.seq <= self.applied_seq {
            return false;
        }
        self.applied_seq = d.seq;
        self.last_rect = d.rect;
        self.last_ds = d.ds;
        self.last_ms = d.ms;
        if d.ms > self.max_ms {
            self.max_ms = d.ms;
        }
        self.last_mean = d.mean;
        self.last_samples = d.samples;
        self.captures += 1;
        self.size = d.size;
        let color = egui::ColorImage {
            size: [d.size.0, d.size.1],
            pixels: d.px,
        };
        match &mut self.tex {
            Some(t) => t.set(color, egui::TextureOptions::LINEAR),
            None => {
                self.tex =
                    Some(ctx.load_texture("xmst_backdrop", color, egui::TextureOptions::LINEAR))
            }
        }
        true
    }

    /// 启动抓屏工作线程（只启动一次）。
    #[cfg(windows)]
    fn ensure_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let shared = self.shared.clone();
        self.worker = Some(std::thread::spawn(move || {
            // GDI 对象只在本线程内创建/复用，不跨线程共享
            let mut gdi = GdiBuf::default();
            loop {
                let job = {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if s.stop {
                        return;
                    }
                    s.job.take()
                };
                let Some(j) = job else {
                    std::thread::sleep(std::time::Duration::from_millis(4));
                    continue;
                };
                let t0 = Instant::now();
                let (_, _, w, h) = j.rect;
                let tw = ((w + j.ds - 1) / j.ds).max(1) as usize;
                let th = ((h + j.ds - 1) / j.ds).max(1) as usize;
                let Some(mut px) = grab_scaled_cached(
                    j.rect.0,
                    j.rect.1,
                    w,
                    h,
                    tw,
                    th,
                    &mut gdi,
                ) else {
                    continue;
                };
                if j.blur > 0 {
                    box_blur(&mut px, tw, th, j.blur);
                }
                let n = px.len().max(1) as u64;
                let (mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64);
                for c in &px {
                    sr += c.r() as u64;
                    sg += c.g() as u64;
                    sb += c.b() as u64;
                }
                let mean = ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8);
                let mut samples = Vec::with_capacity(4);
                for fx in [0.10f32, 0.30, 0.62, 0.90] {
                    let idx = ((tw as f32 * fx) as usize).min(tw.saturating_sub(1))
                        + (th / 2).min(th.saturating_sub(1)) * tw;
                    if let Some(c) = px.get(idx) {
                        samples.push((c.r(), c.g(), c.b()));
                    }
                }
                let ms = t0.elapsed().as_secs_f32() * 1000.0;
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                s.done = Some(Done {
                    seq: j.seq,
                    rect: j.rect,
                    ds: j.ds,
                    size: (tw, th),
                    px,
                    mean,
                    samples,
                    ms,
                });
            }
        }));
    }

    #[cfg(not(windows))]
    pub fn update(
        &mut self,
        _ctx: &egui::Context,
        _rect_px: (i32, i32, i32, i32),
        _downscale: i32,
        _blur_px: i32,
        _min_interval: std::time::Duration,
    ) -> bool {
        false
    }
}

/// 工作线程内复用的 GDI 内存 DC + 位图（每次抓取都新建是主要的固定开销）。
#[cfg(windows)]
#[derive(Default)]
struct GdiBuf {
    dc: isize,
    bmp: isize,
    size: (i32, i32),
}

#[cfg(windows)]
impl GdiBuf {
    /// 确保缓冲尺寸正确（尺寸变化时重建），返回 (dc, bmp)。
    fn ensure(&mut self, screen: winapi::shared::windef::HDC, tw: i32, th: i32) -> bool {
        use winapi::um::wingdi::{CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, SelectObject};
        if self.dc != 0 && self.size == (tw, th) {
            return true;
        }
        unsafe {
            if self.bmp != 0 {
                DeleteObject(self.bmp as _);
                self.bmp = 0;
            }
            if self.dc != 0 {
                DeleteDC(self.dc as _);
                self.dc = 0;
            }
            let mem = CreateCompatibleDC(screen);
            if mem.is_null() {
                return false;
            }
            let b = CreateCompatibleBitmap(screen, tw, th);
            if b.is_null() {
                DeleteDC(mem);
                return false;
            }
            SelectObject(mem, b as _);
            self.dc = mem as isize;
            self.bmp = b as isize;
            self.size = (tw, th);
        }
        true
    }
}

#[cfg(windows)]
impl Drop for GdiBuf {
    fn drop(&mut self) {
        use winapi::um::wingdi::{DeleteDC, DeleteObject};
        unsafe {
            if self.bmp != 0 {
                DeleteObject(self.bmp as _);
            }
            if self.dc != 0 {
                DeleteDC(self.dc as _);
            }
        }
    }
}

/// 对缩小后的图像做一次可分离盒式模糊（O(w·h·r)），把小图上的点采样锯齿抹平成
/// 「毛玻璃」质感。只在小图上调用（几十×几十像素），开销可忽略。
fn box_blur(px: &mut [egui::Color32], w: usize, h: usize, r: i32) {
    if r <= 0 || w == 0 || h == 0 || px.len() < w * h {
        return;
    }
    let r = r as isize;
    let mut tmp = px.to_vec();
    // 水平
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dx in -r..=r {
                let xx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
                let c = tmp[row + xx];
                sr += c.r() as u32;
                sg += c.g() as u32;
                sb += c.b() as u32;
                n += 1;
            }
            px[row + x] = egui::Color32::from_rgb((sr / n) as u8, (sg / n) as u8, (sb / n) as u8);
        }
    }
    // 垂直
    tmp.copy_from_slice(px);
    for y in 0..h {
        for x in 0..w {
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in -r..=r {
                let yy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                let c = tmp[yy * w + x];
                sr += c.r() as u32;
                sg += c.g() as u32;
                sb += c.b() as u32;
                n += 1;
            }
            px[y * w + x] = egui::Color32::from_rgb((sr / n) as u8, (sg / n) as u8, (sb / n) as u8);
        }
    }
}

/// 用 GDI 抓取屏幕区域并**直接缩放到目标尺寸**（工作线程内调用，复用 `GdiBuf`）。
#[cfg(windows)]
fn grab_scaled_cached(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    tw: usize,
    th: usize,
    gdi: &mut GdiBuf,
) -> Option<Vec<egui::Color32>> {
    use winapi::um::wingdi::{
        GetDIBits, SelectObject, SetStretchBltMode, StretchBlt, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, COLORONCOLOR, SRCCOPY,
    };
    use winapi::um::winuser::{GetDC, ReleaseDC};

    if tw == 0 || th == 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        if !gdi.ensure(screen, tw as i32, th as i32) {
            ReleaseDC(std::ptr::null_mut(), screen);
            return None;
        }
        let mem = gdi.dc as _;
        let b = gdi.bmp as _;

        SetStretchBltMode(mem, COLORONCOLOR);
        let ok = StretchBlt(
            mem,
            0,
            0,
            tw as i32,
            th as i32,
            screen,
            x,
            y,
            w,
            h,
            SRCCOPY,
        ) != 0;

        let mut out: Option<Vec<egui::Color32>> = None;
        if ok {
            // GetDIBits 要求位图未被选入 DC：临时取消选择，取完再选回
            let deselected = SelectObject(mem, std::ptr::null_mut());
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = tw as i32;
            bi.bmiHeader.biHeight = -(th as i32);
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut buf = vec![0u8; tw * th * 4];
            let lines = GetDIBits(
                mem,
                b,
                0,
                th as u32,
                buf.as_mut_ptr() as *mut _,
                &mut bi,
                DIB_RGB_COLORS,
            );
            if !deselected.is_null() {
                SelectObject(mem, b as _);
            }
            if lines == th as i32 {
                let mut px = Vec::with_capacity(tw * th);
                for i in (0..buf.len()).step_by(4) {
                    px.push(egui::Color32::from_rgb(buf[i + 2], buf[i + 1], buf[i]));
                }
                out = Some(px);
            }
        }
        ReleaseDC(std::ptr::null_mut(), screen);
        out
    }
}
