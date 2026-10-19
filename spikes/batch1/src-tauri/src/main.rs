// Disposable feasibility spike for HealthierYou batch 1. Not product code.
// Logs resource use of this app plus every descendant process (the WebView2
// msedgewebview2 tree) every 5 seconds, and UI events reported by the pages.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{Manager, State};

struct Log {
    events: Mutex<File>,
    start: Instant,
}

fn epoch_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn log_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("HY-Spike").join("logs")
}

fn open_append(path: &PathBuf) -> File {
    OpenOptions::new().create(true).append(true).open(path).expect("open log file")
}

#[tauri::command]
fn report(kind: String, data: serde_json::Value, log: State<'_, Log>) {
    if let Ok(mut f) = log.events.lock() {
        let _ = writeln!(f, "{}\t{}\t{}\t{}", epoch_ms(), log.start.elapsed().as_millis(), kind, data);
        let _ = f.flush();
    }
}

// Unattended test mode for the scripted run: HY_AUTOTEST=camera or camera_denied.
#[tauri::command]
fn autotest_mode() -> serde_json::Value {
    serde_json::json!({
        "mode": std::env::var("HY_AUTOTEST").unwrap_or_default(),
        "camera_minutes": std::env::var("HY_CAMERA_MINUTES").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(10.0),
    })
}

fn main() {
    let start = Instant::now();
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    let stamp = epoch_ms();
    let events = open_append(&dir.join(format!("events-{stamp}.tsv")));
    let metrics_path = dir.join(format!("metrics-{stamp}.csv"));
    let info_path = dir.join(format!("info-{stamp}.txt"));

    tauri::Builder::default()
        .manage(Log { events: Mutex::new(events), start })
        .invoke_handler(tauri::generate_handler![report, autotest_mode])
        .setup(move |app| {
            let mut info = open_append(&info_path);
            let _ = writeln!(info, "app_version=0.1.0 batch1 spike");
            let _ = writeln!(info, "exe={:?}", std::env::current_exe().ok());
            let _ = writeln!(info, "webview2_version={:?}", tauri::webview_version());
            let _ = writeln!(info, "build_commit={}", option_env!("HY_BUILD_COMMIT").unwrap_or("unknown"));
            #[cfg(windows)]
            if let Some(pet) = app.get_webview_window("pet") {
                win::no_activate(&pet);
                let _ = writeln!(info, "pet_window_no_activate=applied");
            }
            let pid = std::process::id();
            std::thread::spawn(move || win::sample_loop(pid, metrics_path, info, start));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running spike");
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2};
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, GetSystemTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    pub fn no_activate(w: &tauri::WebviewWindow) {
        if let Ok(h) = w.hwnd() {
            let hwnd = h.0 as HWND;
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize);
            }
        }
    }

    fn ft(t: FILETIME) -> u64 {
        ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
    }

    fn wide_to_string(w: &[u16]) -> String {
        let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..n])
    }

    // pid -> (parent pid, exe name)
    fn snapshot() -> HashMap<u32, (u32, String)> {
        let mut out = HashMap::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap as isize == -1 {
                return out;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut e) != 0 {
                loop {
                    out.insert(e.th32ProcessID, (e.th32ParentProcessID, wide_to_string(&e.szExeFile)));
                    if Process32NextW(snap, &mut e) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
        }
        out
    }

    fn descendants(root: u32, procs: &HashMap<u32, (u32, String)>) -> Vec<u32> {
        let mut set: HashSet<u32> = HashSet::from([root]);
        let mut grew = true;
        while grew {
            grew = false;
            for (&pid, &(ppid, _)) in procs {
                if pid != ppid && set.contains(&ppid) && set.insert(pid) {
                    grew = true;
                }
            }
        }
        set.into_iter().collect()
    }

    struct Sample {
        cpu_100ns: u64,
        working_set: u64,
        private_bytes: u64,
        private_ws: u64,
        path: String,
    }

    fn sample(pid: u32) -> Option<Sample> {
        unsafe {
            let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
            let (mut c, mut x, mut k, mut u) = (z, z, z, z);
            let cpu = if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u) != 0 { ft(k) + ft(u) } else { 0 };
            // EX2 adds PrivateWorkingSetSize (Windows 10 1809+), the figure Task Manager shows as "Memory"
            let mut m: PROCESS_MEMORY_COUNTERS_EX2 = std::mem::zeroed();
            m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
            GetProcessMemoryInfo(h, &mut m as *mut _ as *mut PROCESS_MEMORY_COUNTERS, m.cb);
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let path = if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 {
                String::from_utf16_lossy(&buf[..len as usize])
            } else {
                String::new()
            };
            CloseHandle(h);
            Some(Sample { cpu_100ns: cpu, working_set: m.WorkingSetSize as u64, private_bytes: m.PrivateUsage as u64, private_ws: m.PrivateWorkingSetSize as u64, path })
        }
    }

    fn system_times() -> (u64, u64) {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut i, mut k, mut u) = (z, z, z);
        unsafe { GetSystemTimes(&mut i, &mut k, &mut u) };
        // kernel time includes idle time
        (ft(i), ft(k) + ft(u))
    }

    pub fn sample_loop(root: u32, metrics_path: PathBuf, mut info: File, start: Instant) {
        let ncpu = unsafe {
            let mut si: SYSTEM_INFO = std::mem::zeroed();
            GetSystemInfo(&mut si);
            si.dwNumberOfProcessors.max(1) as u64
        };
        unsafe {
            let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
            ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            GlobalMemoryStatusEx(&mut ms);
            let _ = writeln!(info, "logical_cpus={ncpu} total_ram_mb={}", ms.ullTotalPhys / 1_048_576);
        }
        let mut metrics = open_append(&metrics_path);
        let _ = writeln!(
            metrics,
            "epoch_ms,elapsed_s,process_count,working_set_mb,private_mb,app_cpu_pct_of_machine,machine_cpu_pct,avail_ram_mb,private_ws_mb"
        );
        let mut prev_cpu: HashMap<u32, u64> = HashMap::new();
        let mut prev_sys = system_times();
        let mut prev_t = Instant::now();
        let mut seen_paths: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let procs = snapshot();
            let pids = descendants(root, &procs);
            let (mut ws, mut pb, mut pws, mut cpu_delta) = (0u64, 0u64, 0u64, 0u64);
            let mut cur_cpu = HashMap::new();
            for pid in &pids {
                if let Some(s) = sample(*pid) {
                    ws += s.working_set;
                    pb += s.private_bytes;
                    pws += s.private_ws;
                    cpu_delta += s.cpu_100ns.saturating_sub(*prev_cpu.get(pid).unwrap_or(&s.cpu_100ns));
                    cur_cpu.insert(*pid, s.cpu_100ns);
                    if seen_paths.insert(s.path.clone()) {
                        let name = procs.get(pid).map(|p| p.1.as_str()).unwrap_or("");
                        let _ = writeln!(info, "process pid={pid} name={name} path={}", s.path);
                        let _ = info.flush();
                    }
                }
            }
            prev_cpu = cur_cpu;
            let wall_100ns = prev_t.elapsed().as_nanos() as u64 / 100;
            prev_t = Instant::now();
            let app_pct = 100.0 * cpu_delta as f64 / (wall_100ns.max(1) * ncpu) as f64;
            let sys = system_times();
            let (di, dt) = (sys.0 - prev_sys.0, sys.1 - prev_sys.1);
            prev_sys = sys;
            let machine_pct = if dt > 0 { 100.0 * (dt - di.min(dt)) as f64 / dt as f64 } else { 0.0 };
            let avail = unsafe {
                let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
                ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
                GlobalMemoryStatusEx(&mut ms);
                ms.ullAvailPhys / 1_048_576
            };
            let _ = writeln!(
                metrics,
                "{},{},{},{:.1},{:.1},{:.2},{:.1},{},{:.1}",
                epoch_ms(),
                start.elapsed().as_secs(),
                pids.len(),
                ws as f64 / 1_048_576.0,
                pb as f64 / 1_048_576.0,
                app_pct,
                machine_pct,
                avail,
                pws as f64 / 1_048_576.0
            );
            let _ = metrics.flush();
        }
    }
}

#[cfg(not(windows))]
mod win {
    use super::*;
    pub fn sample_loop(_root: u32, _p: PathBuf, _i: File, _s: Instant) {}
}
