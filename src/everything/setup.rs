// Install state, Start and Install. Called from the settings window, never from startup.
// Registry, winget and CreateProcess run on a background thread. The UI is told with WM_EV.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};

use crate::imports::*;
use crate::WM_EV;

use super::locate::*;
use super::run;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Unknown,
    Ready(Presence),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Activity {
    Idle,
    Checking,
    Starting,
    Installing,
    Winget(String),
    Started,
    Installed,
    StartFailed,
    InstallFailed(i32),
    OpenedPage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvAction {
    Start,
    Install,
}

struct Cache {
    shown: Shown,
    exe: Option<String>,
    activity: Activity,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { shown: Shown::Unknown, exe: None, activity: Activity::Idle });
static SETTINGS: AtomicIsize = AtomicIsize::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn bind_settings(hwnd: isize) {
    SETTINGS.store(hwnd, Ordering::Relaxed);
}

pub(crate) fn shown() -> Shown {
    cache().shown.clone()
}

pub(crate) fn activity() -> Activity {
    cache().activity.clone()
}

pub(crate) fn busy() -> bool {
    matches!(activity(), Activity::Checking | Activity::Starting | Activity::Installing | Activity::Winget(_))
}

pub(crate) fn action() -> Option<EvAction> {
    match shown() {
        Shown::Ready(Presence::InstalledStopped) => Some(EvAction::Start),
        Shown::Ready(Presence::NotInstalled) => Some(EvAction::Install),
        _ => None,
    }
}

fn ping() {
    let h = SETTINGS.load(Ordering::Relaxed);
    if h != 0 {
        unsafe { _ = PostMessageW(Some(HWND(h as _)), WM_EV, WPARAM(0), LPARAM(0)); }
    }
}

fn set_activity(activity: Activity) {
    cache().activity = activity;
    ping();
}

pub(crate) fn probe_async() {
    if matches!(activity(), Activity::Starting | Activity::Installing | Activity::Winget(_)) {
        return;
    }
    set_activity(Activity::Checking);
    let _ = std::thread::Builder::new().name("grids-ev".into()).spawn(|| {
        let (shown, exe) = detect();
        {
            let mut c = cache();
            c.shown = shown;
            c.exe = exe;
            if matches!(c.activity, Activity::Checking) {
                c.activity = Activity::Idle;
            }
        }
        ping();
    });
}

pub(crate) fn start_async() {
    if busy() {
        return;
    }
    set_activity(Activity::Starting);
    let _ = std::thread::Builder::new().name("grids-ev-start".into()).spawn(|| {
        let exe = cache().exe.clone().or_else(find_exe);
        let Some(exe) = exe else {
            set_activity(Activity::StartFailed);
            return;
        };
        cache().exe = Some(exe.clone());
        if !launch(&exe) {
            set_activity(Activity::StartFailed);
            return;
        }
        let mut up = false;
        for _ in 0..24 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if run::running() {
                up = true;
                break;
            }
        }
        let (shown, found) = detect();
        {
            let mut c = cache();
            c.shown = shown;
            if found.is_some() {
                c.exe = found;
            }
            c.activity = if up { Activity::Started } else { Activity::StartFailed };
        }
        ping();
    });
}

pub(crate) fn install_async() {
    if busy() {
        return;
    }
    let _ = std::thread::Builder::new().name("grids-ev-install".into()).spawn(|| {
        if install_plan(winget_available()) == InstallPlan::Browser {
            open_downloads();
            set_activity(Activity::OpenedPage);
            return;
        }
        set_activity(Activity::Installing);
        match run_winget() {
            Ok(0) => {
                let (shown, exe) = detect();
                {
                    let mut c = cache();
                    c.shown = shown;
                    c.exe = exe;
                    c.activity = Activity::Installed;
                }
                ping();
            }
            Ok(code) => set_activity(Activity::InstallFailed(code)),
            Err(_) => {
                open_downloads();
                set_activity(Activity::OpenedPage);
            }
        }
    });
}

fn detect() -> (Shown, Option<String>) {
    let hwnd = run::ipc_hwnd();
    let running = hwnd.is_some();
    let version = hwnd.and_then(run::version);
    let exe = find_exe();
    (Shown::Ready(classify(exe.is_some() || running, running, version)), exe)
}

fn find_exe() -> Option<String> {
    let mut extra = Vec::new();
    if let Some(p) = app_path() {
        extra.push(p);
    }
    extra.extend(uninstall_exes());
    let pf = std::env::var("ProgramFiles").unwrap_or_default();
    let pfx = std::env::var("ProgramFiles(x86)").unwrap_or_default();
    let path = std::env::var("PATH").unwrap_or_default();
    let cands = install_candidates(&pf, &pfx, &path, &extra);
    first_present(&cands, |p| std::path::Path::new(p).is_file())
}

fn app_path() -> Option<String> {
    for sub in [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Everything.exe",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\Everything.exe",
    ] {
        let v = reg_sz(HKEY_LOCAL_MACHINE, sub, PCWSTR::null()).trim().trim_matches('"').to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

fn uninstall_exes() -> Vec<String> {
    let mut out = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for sub in [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ] {
            collect_uninstall(root, sub, &mut out);
        }
    }
    out
}

fn collect_uninstall(root: HKEY, sub: &str, out: &mut Vec<String>) {
    let wide: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(root, PCWSTR(wide.as_ptr()), None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return;
    }
    let mut i = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut n = name.len() as u32;
        let err = unsafe { RegEnumKeyExW(key, i, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None) };
        if err == ERROR_NO_MORE_ITEMS {
            break;
        }
        i += 1;
        if err != ERROR_SUCCESS || n == 0 {
            continue;
        }
        let child = String::from_utf16_lossy(&name[..n as usize]);
        let full = format!(r"{sub}\{child}");
        if let Some(exe) = exe_from_uninstall(
            &reg_sz(root, &full, w!("DisplayName")),
            &reg_sz(root, &full, w!("Publisher")),
            &reg_sz(root, &full, w!("InstallLocation")),
            &reg_sz(root, &full, w!("DisplayIcon")),
            &reg_sz(root, &full, w!("UninstallString")),
        ) {
            out.push(exe);
        }
        if i > 4096 {
            break;
        }
    }
    unsafe { _ = RegCloseKey(key); }
}

fn reg_sz(key: HKEY, sub: &str, value: PCWSTR) -> String {
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let wide: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    let err = unsafe { RegGetValueW(key, PCWSTR(wide.as_ptr()), value, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)) };
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').to_string()
}

fn launch(exe: &str) -> bool {
    unsafe {
        let file = HSTRING::from(exe);
        let run = |params: PCWSTR| {
            let r = ShellExecuteW(None, w!("open"), &file, params, PCWSTR::null(), SW_SHOWNORMAL);
            r.0 as isize > 32
        };
        run(w!("-startup")) || run(PCWSTR::null())
    }
}

fn open_downloads() {
    let url = HSTRING::from(DOWNLOADS);
    unsafe { ShellExecuteW(None, w!("open"), &url, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL); }
}

fn winget_available() -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    if path.split(';').any(|dir| {
        let dir = dir.trim().trim_matches('"');
        !dir.is_empty() && std::path::Path::new(dir).join("winget.exe").is_file()
    }) {
        return true;
    }
    std::env::var("LOCALAPPDATA").ok().is_some_and(|local| std::path::Path::new(&local).join("Microsoft").join("WindowsApps").join("winget.exe").is_file())
}

fn run_winget() -> std::io::Result<i32> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new("winget")
        .args(["install", "--id", "voidtools.Everything", "-e", "--disable-interactivity", "--accept-package-agreements", "--accept-source-agreements"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let t1 = std::thread::spawn(|| { if let Some(r) = stdout { pump(r); } });
    let t2 = std::thread::spawn(|| { if let Some(r) = stderr { pump(r); } });
    let status = child.wait()?;
    let _ = t1.join();
    let _ = t2.join();
    Ok(status.code().unwrap_or(1))
}

fn pump(mut reader: impl std::io::Read) {
    let mut buf = [0u8; 512];
    let mut acc = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                while let Some(i) = acc.iter().position(|b| *b == b'\n' || *b == b'\r') {
                    let line = String::from_utf8_lossy(&acc[..i]).trim().to_string();
                    acc.drain(..=i);
                    if line.is_empty() {
                        continue;
                    }
                    let line = if line.chars().count() > 180 { line.chars().take(180).collect() } else { line };
                    set_activity(Activity::Winget(line));
                }
            }
        }
    }
}
