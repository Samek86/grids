//! rclone and WinFsp. The install question is decided in `model::should_ask_tools`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, HKEY_LOCAL_MACHINE, KEY_READ};

use super::model::{winget_args, WINGET_RCLONE, WINGET_WINFSP};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) fn find_rclone() -> Option<PathBuf> {
    if let Some(p) = which("rclone.exe") {
        return Some(p);
    }
    let mut candidates = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(PathBuf::from(&local).join(r"Microsoft\WinGet\Links\rclone.exe"));
        candidates.push(PathBuf::from(&local).join(r"Programs\rclone\rclone.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join(r"rclone\rclone.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub(crate) fn rclone_ok() -> bool {
    find_rclone().is_some()
}

pub(crate) fn winfsp_ok() -> bool {
    let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    if Path::new(&pf86).join(r"WinFsp\bin\winfsp-x64.dll").is_file() || Path::new(&pf).join(r"WinFsp\bin\winfsp-x64.dll").is_file() {
        return true;
    }
    unsafe {
        let mut key = Default::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, windows::core::w!("SOFTWARE\\WinFsp"), None, KEY_READ, &mut key).is_ok() {
            let _ = RegCloseKey(key);
            return true;
        }
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, windows::core::w!("SOFTWARE\\WOW6432Node\\WinFsp"), None, KEY_READ, &mut key).is_ok() {
            let _ = RegCloseKey(key);
            return true;
        }
    }
    false
}

/// Install whichever of rclone and WinFsp is missing. Hidden. Returns true when both are present afterwards.
pub(crate) fn install_missing() -> bool {
    if !rclone_ok() {
        let _ = run_winget(WINGET_RCLONE);
    }
    if !winfsp_ok() {
        let _ = run_winget(WINGET_WINFSP);
    }
    rclone_ok() && winfsp_ok()
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(';') {
        let dir = dir.trim().trim_matches('"');
        if dir.is_empty() {
            continue;
        }
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn run_winget(id: &str) -> bool {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(which("winget.exe").unwrap_or_else(|| PathBuf::from("winget")));
    cmd.args(winget_args(id)).creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.status().is_ok_and(|s| s.success())
}
