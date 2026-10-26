//! Startup update check against the latest GitHub release, and the self-replace that installs it.
//! No token and no Actions: one unauthenticated GET to the public releases API, then the zip asset.

use crate::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use windows::Win32::Networking::WinHttp::*;

/// Test builds can point at a local mock or pretend to be older; release builds use the real values.
const API: &str = match option_env!("PANEGRID_UPDATE_API") {
    Some(u) => u,
    None => "https://api.github.com/repos/Samek86/panegrid/releases/latest",
};
pub(crate) const APP_VERSION: &str = match option_env!("PANEGRID_FAKE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
/// Largest panegrid.exe the updater accepts from a zip.
const EXE_MAX: usize = 64 << 20;
const OLD_EXE: &str = "panegrid.old.exe";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Release {
    pub(crate) tag: String,
    pub(crate) url: String,
    pub(crate) size: u64,
}

static FOUND: Mutex<Option<Release>> = Mutex::new(None);
static INSTALLING: AtomicBool = AtomicBool::new(false);

fn version_parts(v: &str) -> Vec<u64> {
    let core = v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("");
    core.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// True when `tag` (v1.2.3) is a higher version than `current`. Missing parts count as 0.
pub(crate) fn is_newer(tag: &str, current: &str) -> bool {
    let (a, b) = (version_parts(tag), version_parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The tag and the first .zip asset of a releases/latest response.
pub(crate) fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v.get("tag_name")?.as_str()?.to_string();
    let asset = v.get("assets")?.as_array()?.iter().find(|a| {
        a.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.to_ascii_lowercase().ends_with(".zip"))
    })?;
    Some(Release {
        tag,
        url: asset.get("browser_download_url")?.as_str()?.to_string(),
        size: asset.get("size")?.as_u64()?,
    })
}

/// Largest body the updater reads (release JSON or zip).
const BODY_MAX: usize = 128 << 20;

/// Closes a WinHTTP handle when dropped.
struct Hnd(*mut core::ffi::c_void);
impl Drop for Hnd {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { _ = WinHttpCloseHandle(self.0); }
        }
    }
}

/// One GET over WinHTTP (no cache layer; redirects followed). Some(body) only on HTTP 200.
/// Blocking: callers run it on the update worker thread.
fn http_get(url: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = url.encode_utf16().collect();
    let mut host = [0u16; 256];
    let mut path = vec![0u16; 4096];
    let mut uc = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        lpszHostName: PWSTR(host.as_mut_ptr()),
        dwHostNameLength: host.len() as u32,
        lpszUrlPath: PWSTR(path.as_mut_ptr()),
        dwUrlPathLength: path.len() as u32,
        dwSchemeLength: 1,
        dwExtraInfoLength: 1,
        ..Default::default()
    };
    unsafe {
        WinHttpCrackUrl(&wide, 0, &mut uc).ok()?;
        // Path and query are cracked separately; the request needs both.
        let mut object: Vec<u16> = path[..uc.dwUrlPathLength as usize].to_vec();
        if !uc.lpszExtraInfo.is_null() {
            object.extend_from_slice(std::slice::from_raw_parts(uc.lpszExtraInfo.0, uc.dwExtraInfoLength as usize));
        }
        object.push(0);
        host[uc.dwHostNameLength as usize] = 0;
        let secure = uc.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
        let ua = HSTRING::from(format!("Panegrid/{APP_VERSION}"));
        let ses = Hnd(WinHttpOpen(&ua, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if ses.0.is_null() {
            return None;
        }
        _ = WinHttpSetTimeouts(ses.0, 10_000, 15_000, 30_000, 60_000);
        let con = Hnd(WinHttpConnect(ses.0, PCWSTR(host.as_ptr()), uc.nPort, 0));
        if con.0.is_null() {
            return None;
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let req = Hnd(WinHttpOpenRequest(con.0, w!("GET"), PCWSTR(object.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if req.0.is_null() {
            return None;
        }
        let hdr: Vec<u16> = "Cache-Control: no-cache\r\nPragma: no-cache\r\n".encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&hdr), None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 << 10];
        loop {
            let mut got = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got).ok()?;
            if got == 0 {
                break;
            }
            body.extend_from_slice(&buf[..got as usize]);
            if body.len() > BODY_MAX {
                return None;
            }
        }
        Some(body)
    }
}

fn download(url: &str, dest: &Path) -> bool {
    _ = std::fs::remove_file(dest);
    http_get(url).is_some_and(|b| std::fs::write(dest, b).is_ok())
}

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Startup: remove the exe a previous update left behind, then ask GitHub when the setting is on.
pub(crate) fn startup(main: HWND, check: bool) {
    let h = main.0 as isize;
    _ = std::thread::Builder::new().name("panegrid-update".into()).spawn(move || {
        if let Some(old) = exe_path().and_then(|e| e.parent().map(|d| d.join(OLD_EXE))) {
            // The previous process may still be closing for a moment.
            for _ in 0..20 {
                if !old.exists() || std::fs::remove_file(&old).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        if !check {
            return;
        }
        let text = http_get(API).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let Some(rel) = parse_release(&text) else { return };
        if is_newer(&rel.tag, APP_VERSION) {
            *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = Some(rel);
            unsafe { _ = PostMessageW(Some(HWND(h as _)), WM_UPDATE, WPARAM(0), LPARAM(0)); }
        }
    });
}

/// Download, verify, extract and swap. On any failure the running exe stays where it was.
fn install(rel: &Release) -> std::result::Result<(), &'static str> {
    let exe = exe_path().ok_or("exe path")?;
    let dir = exe.parent().ok_or("exe folder")?.to_path_buf();
    let zip = dir.join("panegrid.update.zip");
    let new = dir.join("panegrid.new.exe");
    let res = (|| {
        if !download(&rel.url, &zip) {
            return Err("download");
        }
        if std::fs::metadata(&zip).map(|m| m.len()).ok() != Some(rel.size) {
            return Err("size");
        }
        let mut f = std::fs::File::open(&zip).map_err(|_| "open")?;
        let es = zip_entries(&mut f).ok_or("not a zip")?;
        let e = es
            .iter()
            .find(|e| e.name.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("panegrid.exe")))
            .ok_or("no panegrid.exe")?;
        let bytes = zip_read(&mut f, e, EXE_MAX).ok_or("extract")?;
        if bytes.len() as u64 != e.size || !bytes.starts_with(b"MZ") {
            return Err("bad exe");
        }
        std::fs::write(&new, &bytes).map_err(|_| "write")?;
        swap(&exe, &new, &dir.join(OLD_EXE))
    })();
    _ = std::fs::remove_file(&zip);
    _ = std::fs::remove_file(&new);
    res
}

/// Running exe -> panegrid.old.exe, new -> exe. Puts the old one back if the second move fails.
fn swap(exe: &Path, new: &Path, old: &Path) -> std::result::Result<(), &'static str> {
    if old.exists() && std::fs::remove_file(old).is_err() {
        return Err("old exe busy");
    }
    std::fs::rename(exe, old).map_err(|_| "rename running exe")?;
    if std::fs::rename(new, exe).is_err() {
        _ = std::fs::rename(old, exe);
        return Err("move new exe");
    }
    Ok(())
}

fn rollback(exe: &Path) {
    let old = exe.with_file_name(OLD_EXE);
    if old.exists() {
        _ = std::fs::remove_file(exe);
        _ = std::fs::rename(&old, exe);
    }
}

/// WM_UPDATE on the main window.
pub(crate) unsafe fn on_update(h: HWND, stage: usize) {
    match stage {
        0 => {
            let Some(rel) = FOUND.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
            let text = tr(
                "Panegrid {new} is available. You have {cur}.\n\nUpdate now? Panegrid restarts after the download.",
                "Panegrid {new} 버전이 나왔어요. 지금은 {cur} 버전이에요.\n\n지금 업데이트할까요? 다운로드가 끝나면 Panegrid가 다시 시작돼요.",
                "Panegrid {new} が公開されています。現在は {cur} です。\n\n今すぐ更新しますか？ダウンロード後に Panegrid が再起動します。",
            )
            .replace("{new}", &rel.tag)
            .replace("{cur}", &format!("v{APP_VERSION}"));
            let title = tr("Panegrid update", "Panegrid 업데이트", "Panegrid の更新");
            if MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_YESNO | MB_ICONINFORMATION | MB_DEFBUTTON2) != IDYES {
                return;
            }
            if INSTALLING.swap(true, Ordering::SeqCst) {
                return;
            }
            let hv = h.0 as isize;
            let spawned = std::thread::Builder::new().name("panegrid-update-install".into()).spawn(move || {
                let r = install(&rel);
                let stage = if r.is_ok() { 1 } else { 2 };
                _ = PostMessageW(Some(HWND(hv as _)), WM_UPDATE, WPARAM(stage), LPARAM(0));
            });
            if spawned.is_err() {
                INSTALLING.store(false, Ordering::SeqCst);
            }
        }
        1 => {
            let Some(exe) = exe_path() else { return };
            // current_exe still names the original path, which now holds the new build.
            let started = std::process::Command::new(&exe)
                .arg("--updated")
                .arg(std::process::id().to_string())
                .current_dir(exe.parent().unwrap_or(Path::new(".")))
                .spawn()
                .is_ok();
            if !started {
                rollback(&exe);
                INSTALLING.store(false, Ordering::SeqCst);
                failed(h);
                return;
            }
            close_all_windows();
        }
        _ => {
            INSTALLING.store(false, Ordering::SeqCst);
            failed(h);
        }
    }
}

unsafe fn failed(h: HWND) {
    let text = tr(
        "The update could not be installed. Panegrid was left as it was.",
        "업데이트를 설치하지 못했어요. Panegrid는 그대로 두었어요.",
        "更新をインストールできませんでした。Panegrid はそのままです。",
    );
    let title = tr("Panegrid update", "Panegrid 업데이트", "Panegrid の更新");
    MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONWARNING);
}

/// Every Panegrid main window of this process gets WM_CLOSE, so each saves as on a normal exit.
unsafe fn close_all_windows() {
    unsafe extern "system" fn each(h: HWND, _: LPARAM) -> BOOL {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let mut cls = [0u16; 32];
        let n = GetClassNameW(h, &mut cls) as usize;
        if pid == std::process::id() && String::from_utf16_lossy(&cls[..n]) == "PanegridMain" {
            _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        TRUE
    }
    _ = EnumWindows(Some(each), LPARAM(0));
}

/// `--updated <pid>` from the old process: wait for it to save and exit before reading settings.
pub(crate) fn wait_for_previous(args: &mut Vec<String>) {
    if args.first().map(String::as_str) != Some("--updated") {
        return;
    }
    let pid = args.get(1).and_then(|p| p.parse::<u32>().ok());
    args.drain(..args.len().min(2));
    if let Some(pid) = pid {
        unsafe {
            if let Ok(p) = OpenProcess(windows::Win32::System::Threading::PROCESS_SYNCHRONIZE, false, pid) {
                windows::Win32::System::Threading::WaitForSingleObject(p, 20_000);
                _ = CloseHandle(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn release_json_picks_zip_asset() {
        let j = r#"{"tag_name":"v0.2.0","assets":[{"name":"notes.txt","size":3,"browser_download_url":"x"},
            {"name":"panegrid-v0.2.0.zip","size":1234,"browser_download_url":"https://e/p.zip"}]}"#;
        assert_eq!(parse_release(j), Some(Release { tag: "v0.2.0".into(), url: "https://e/p.zip".into(), size: 1234 }));
        assert_eq!(parse_release(r#"{"tag_name":"v1","assets":[]}"#), None);
        assert_eq!(parse_release("not json"), None);
    }

    #[test]
    fn swap_moves_and_rolls_back() {
        let d = std::env::temp_dir().join(format!("pg-swap-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (exe, new, old) = (d.join("panegrid.exe"), d.join("panegrid.new.exe"), d.join(OLD_EXE));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        assert!(swap(&exe, &new, &old).is_ok());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "old");
        rollback(&exe);
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(!old.exists());
        // A missing new file leaves the running exe in place.
        assert!(swap(&exe, &new, &old).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        _ = std::fs::remove_dir_all(&d);
    }
}
