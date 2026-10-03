//! Lightweight process-tree performance sampling for running servers.
//! CPU% is relative to total machine capacity (0-100%), memory is working set in MB.
//! Sampling enumerates the process snapshot once and aggregates the root PID tree,
//! which is cheap enough at 1 sample/sec and only runs while the monitoring UI is visible.

use std::collections::{HashSet, VecDeque};
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub cpu_pct: f32,
    pub mem_mb: f32,
}

pub struct PerfMonitor {
    pub samples: VecDeque<Sample>,
    prev: Option<(Instant, u64)>,
}

impl PerfMonitor {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(240),
            prev: None,
        }
    }

    /// Push one sample for the process tree rooted at `root_pid`.
    /// First call only records a baseline; subsequent calls produce cpu_pct values.
    pub fn sample(&mut self, root_pid: u32) {
        let now = Instant::now();
        let (mem_mb, ticks) = proc_tree_stats(root_pid);
        match self.prev {
            Some((t, prev_ticks)) => {
                let wall = now.duration_since(t).as_secs_f64();
                let dt = ticks.saturating_sub(prev_ticks);
                let ncpu = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
                    .max(1) as f64;
                let cpu = if wall > 0.0 {
                    (dt as f64 / 10_000_000.0 / wall / ncpu * 100.0).min(100.0) as f32
                } else {
                    0.0
                };
                self.samples.push_back(Sample {
                    cpu_pct: cpu,
                    mem_mb,
                });
                while self.samples.len() > 240 {
                    self.samples.pop_front();
                }
                self.prev = Some((now, ticks));
            }
            None => {
                self.prev = Some((now, ticks));
            }
        }
    }

    pub fn last(&self) -> Option<(f32, f32)> {
        self.samples.back().map(|s| (s.cpu_pct, s.mem_mb))
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.prev = None;
    }
}

/// System-wide memory load percentage (0-100) via GlobalMemoryStatusEx.
pub fn system_mem_load_pct() -> u8 {
    unsafe {
        let mut ms: winapi::um::sysinfoapi::MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of::<winapi::um::sysinfoapi::MEMORYSTATUSEX>() as u32;
        if winapi::um::sysinfoapi::GlobalMemoryStatusEx(&mut ms) != 0 {
            ms.dwMemoryLoad as u8
        } else {
            0
        }
    }
}

/// System-wide CPU usage percentage (0-100) via GetSystemTimes.
/// Keeps an internal previous-snapshot; first call returns 0 (baseline only).
pub fn system_cpu_usage_pct() -> f32 {
    use std::sync::Mutex;
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::processthreadsapi::GetSystemTimes;
    static PREV: Mutex<Option<(u64, u64, u64)>> = Mutex::new(None);

    let ft = |f: &FILETIME| -> u64 { ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64 };
    let (mut idle, mut kernel, mut user): (FILETIME, FILETIME, FILETIME) = unsafe {
        let (mut i, mut k, mut u) = (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed());
        if GetSystemTimes(&mut i, &mut k, &mut u) == 0 {
            return 0.0;
        }
        (i, k, u)
    };
    let (idle_t, kernel_t, user_t) = (ft(&idle), ft(&kernel), ft(&user));
    let mut guard = PREV.lock().unwrap_or_else(|e| e.into_inner());
    match *guard {
        Some((p_idle, p_kernel, p_user)) => {
            let total = (kernel_t + user_t).saturating_sub(p_kernel + p_user);
            let idle_d = idle_t.saturating_sub(p_idle);
            if total == 0 {
                return 0.0;
            }
            *guard = Some((idle_t, kernel_t, user_t));
            ((1.0 - idle_d as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
        }
        None => {
            *guard = Some((idle_t, kernel_t, user_t));
            0.0
        }
    }
}

/// Aggregate working set (MB) and CPU ticks for root PID + all descendants.
fn proc_tree_stats(root_pid: u32) -> (f32, u64) {
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::processthreadsapi::{GetProcessTimes, OpenProcess};
    use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use winapi::um::tlhelp32::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    // Snapshot all PIDs once.
    let mut entries: Vec<(u32, u32)> = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return (0.0, 0);
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

    // BFS over descendants of root_pid (root may already be gone -> children remain).
    let mut targets: HashSet<u32> = HashSet::new();
    let mut frontier: Vec<u32> = vec![root_pid];
    while let Some(pid) = frontier.pop() {
        if !targets.insert(pid) {
            continue;
        }
        for &(pid2, ppid) in &entries {
            if ppid == pid && !targets.contains(&pid2) {
                frontier.push(pid2);
            }
        }
    }

    let mut total_mem: u64 = 0;
    let mut total_ticks: u64 = 0;
    for pid in targets {
        unsafe {
            let h = OpenProcess(winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION | winapi::um::winnt::PROCESS_VM_READ, 0, pid);
            if h.is_null() {
                continue;
            }
            let mut ct: FILETIME = std::mem::zeroed();
            let mut et: FILETIME = std::mem::zeroed();
            let mut kt: FILETIME = std::mem::zeroed();
            let mut ut: FILETIME = std::mem::zeroed();
            if GetProcessTimes(h, &mut ct, &mut et, &mut kt, &mut ut) != 0 {
                let ft = |f: &FILETIME| -> u64 {
                    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
                };
                total_ticks += ft(&kt) + ft(&ut);
            }
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            if GetProcessMemoryInfo(
                h,
                &mut pmc,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ) != 0
            {
                total_mem += pmc.WorkingSetSize as u64;
            }
            CloseHandle(h);
        }
    }
    (total_mem as f32 / 1_048_576.0, total_ticks)
}
