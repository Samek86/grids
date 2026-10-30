//! Mount lifetime for SFTP and FTP, and `WNetAddConnection2` for SMB and WebDAV.
//! rclone runs with `CREATE_NO_WINDOW`. The password is never an argument.

use std::collections::{HashMap, HashSet};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use windows::Win32::NetworkManagement::WNet::{WNetAddConnection2W, WNetCancelConnection2W, NETRESOURCEW, CONNECT_TEMPORARY, RESOURCETYPE_DISK};
use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JobObjectExtendedLimitInformation};

use crate::imports::*;

use super::cred::read_secret;
use super::model::{config_body, covered_by_mount, cred_target, mount_args, mount_key, needs_rclone, share_root, unc_for};
use super::obscure::{obscure, redact};
use super::tools::{find_rclone, install_missing, winfsp_ok};
use crate::session::ServerLink;

pub(crate) const WM_REMOTE: u32 = 0x8000 + 86;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DRIVE_NO_ROOT: u32 = 1;

#[derive(Clone, Debug)]
pub(crate) enum Done {
    Open(String),
    Fail(String),
    NeedTools,
    /// A second click while this server is already connecting.
    Quiet,
}

struct Live {
    letter: char,
    root: String,
    child: Child,
    config: PathBuf,
}

struct Table {
    job: isize,
    mounts: HashMap<String, Live>,
    pending: HashSet<String>,
    reserved: HashSet<char>,
}

static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new(Table { job: 0, mounts: HashMap::new(), pending: HashSet::new(), reserved: HashSet::new() }));
static OUT: Mutex<Vec<Done>> = Mutex::new(Vec::new());
static INSTALLING: AtomicBool = AtomicBool::new(false);

fn table() -> std::sync::MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn mount_roots() -> Vec<String> {
    table().mounts.values().map(|m| m.root.clone()).collect()
}

pub(crate) fn path_on_live_mount(path: &str) -> bool {
    covered_by_mount(path, &mount_roots())
}

pub(crate) fn take_done() -> Vec<Done> {
    let mut q = OUT.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *q)
}

fn push(done: Done) {
    OUT.lock().unwrap_or_else(|e| e.into_inner()).push(done);
}

fn post(owner: isize) {
    if owner == 0 {
        return;
    }
    unsafe {
        let _ = PostMessageW(Some(HWND(owner as *mut _)), WM_REMOTE, WPARAM(0), LPARAM(0));
    }
}

pub(crate) fn begin_connect(owner: isize, link: ServerLink, open: Option<String>) {
    let _ = std::thread::Builder::new().name("panegrid-remote".into()).spawn(move || {
        let done = connect_one(&link, open.as_deref());
        push(done);
        post(owner);
    });
}

pub(crate) fn install_then_connect(owner: isize, link: ServerLink, open: Option<String>) {
    if INSTALLING.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new().name("panegrid-remote-setup".into()).spawn(move || {
        let ok = install_missing();
        INSTALLING.store(false, Ordering::Relaxed);
        let done = if ok { connect_one(&link, open.as_deref()) } else { Done::NeedTools };
        push(done);
        post(owner);
    });
}

pub(crate) fn unmount_all() {
    let taken = std::mem::take(&mut table().mounts);
    for (_, live) in taken {
        stop(live);
    }
}

pub(crate) fn unmount_link(link: &ServerLink) {
    let key = mount_key(link);
    let live = table().mounts.remove(&key);
    if let Some(live) = live {
        stop(live);
    }
}

fn connect_one(link: &ServerLink, open: Option<&str>) -> Done {
    if needs_rclone(&link.kind) {
        mount_rclone(link)
    } else {
        connect_unc(link, open)
    }
}

fn connect_unc(link: &ServerLink, open: Option<&str>) -> Done {
    let unc = unc_for(link);
    let root = share_root(&unc);
    if root.trim_matches('\\').is_empty() {
        return Done::Fail(String::new());
    }
    let dest = open.map(str::trim).filter(|p| !p.is_empty()).unwrap_or(unc.as_str()).to_string();
    let mut secret = read_secret(&cred_target(link)).unwrap_or_default();
    let code = wnet_add(&root, link.user.trim(), &secret);
    wipe(&mut secret);
    if code == 0 || code == 85 {
        Done::Open(dest)
    } else {
        Done::Fail(format!("Windows error {code}"))
    }
}

fn wnet_add(share: &str, user: &str, pass: &str) -> u32 {
    let code = add_once(share, user, pass);
    if code == 1219 {
        cancel_share(share);
        return add_once(share, user, pass);
    }
    code
}

fn add_once(share: &str, user: &str, pass: &str) -> u32 {
    let remote = HSTRING::from(share);
    let user_w = HSTRING::from(user);
    let mut pass_units: Vec<u16> = pass.encode_utf16().chain(std::iter::once(0)).collect();
    let nr = NETRESOURCEW { dwType: RESOURCETYPE_DISK, lpRemoteName: PWSTR(remote.as_ptr() as *mut _), ..Default::default() };
    let code = unsafe {
        if user.is_empty() && pass.is_empty() {
            WNetAddConnection2W(&nr, PCWSTR::null(), PCWSTR::null(), CONNECT_TEMPORARY).0
        } else {
            WNetAddConnection2W(&nr, PCWSTR(pass_units.as_ptr()), PCWSTR(user_w.as_ptr()), CONNECT_TEMPORARY).0
        }
    };
    pass_units.fill(0);
    code
}

fn cancel_share(share: &str) {
    let name = HSTRING::from(share);
    unsafe {
        let _ = WNetCancelConnection2W(&name, CONNECT_TEMPORARY, true);
    }
}

fn mount_rclone(link: &ServerLink) -> Done {
    let key = mount_key(link);
    if let Some(root) = reuse(&key) {
        return Done::Open(root);
    }
    {
        let mut t = table();
        if t.pending.contains(&key) {
            return Done::Quiet;
        }
        t.pending.insert(key.clone());
    }
    let done = mount_new(link, &key);
    table().pending.remove(&key);
    done
}

fn reuse(key: &str) -> Option<String> {
    let root = table().mounts.get(key).map(|m| m.root.clone())?;
    let running = {
        let mut t = table();
        match t.mounts.get_mut(key).map(|m| m.child.try_wait()) {
            Some(Ok(None)) => true,
            _ => false,
        }
    };
    if running && drive_kind(&root) != DRIVE_NO_ROOT {
        return Some(root);
    }
    if let Some(live) = table().mounts.remove(key) {
        stop(live);
    }
    None
}

fn mount_new(link: &ServerLink, key: &str) -> Done {
    if !winfsp_ok() {
        return Done::NeedTools;
    }
    let Some(exe) = find_rclone() else {
        return Done::NeedTools;
    };
    let Some(letter) = free_letter() else {
        return Done::Fail("No free drive letter.".into());
    };
    let mut plain = read_secret(&cred_target(link)).unwrap_or_default();
    let mut token = if plain.is_empty() { None } else { Some(obscure(&plain)) };
    let mut secrets = Vec::new();
    if !plain.is_empty() {
        secrets.push(std::mem::take(&mut plain));
    }
    if let Some(tok) = &token {
        secrets.push(tok.clone());
    }
    let body = config_body(link, token.as_deref());
    wipe(&mut plain);
    let cfg = config_path(key);
    if std::fs::write(&cfg, &body).is_err() {
        wipe_all(&mut secrets);
        let _ = std::fs::remove_file(&cfg);
        table().reserved.remove(&letter);
        return Done::Fail("Could not write the mount config.".into());
    }
    if let Some(tok) = token.as_mut() {
        wipe(tok);
    }
    let args = mount_args(&cfg.display().to_string(), link, letter);
    let mut child = match Command::new(&exe).args(&args).creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            wipe_all(&mut secrets);
            let _ = std::fs::remove_file(&cfg);
            table().reserved.remove(&letter);
            return Done::Fail(e.to_string());
        }
    };
    assign_job(&mut child);
    let stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read_capped(stderr, 8192));
    });
    let root = format!("{letter}:\\");
    let ready = wait_drive(&mut child, &root);
    if ready {
        {
            let mut t = table();
            t.reserved.remove(&letter);
            t.mounts.insert(key.to_string(), Live { letter, root: root.clone(), child, config: cfg });
        }
        wipe_all(&mut secrets);
        return Done::Open(root);
    }
    let _ = child.kill();
    let _ = child.wait();
    let err = rx.recv_timeout(Duration::from_millis(400)).unwrap_or_default();
    let mut text = String::from_utf8_lossy(&err).chars().filter(|c| *c != '\0').take(400).collect::<String>();
    if text.trim().is_empty() {
        text = "rclone did not mount the drive.".into();
    }
    let text = redact(&text, &secrets);
    wipe_all(&mut secrets);
    let _ = std::fs::remove_file(&cfg);
    let _ = net_delete(letter);
    table().reserved.remove(&letter);
    Done::Fail(text)
}

fn assign_job(child: &mut Child) {
    let mut t = table();
    let Some(job) = ensure_job(&mut t) else { return };
    let proc_h = HANDLE(child.as_raw_handle() as *mut _);
    unsafe {
        let _ = AssignProcessToJobObject(job, proc_h);
    }
}

fn ensure_job(t: &mut Table) -> Option<HANDLE> {
    if t.job != 0 {
        return Some(HANDLE(t.job as *mut _));
    }
    let job = unsafe { CreateJobObjectW(None, None).ok()? };
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        let _ = SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32);
    }
    t.job = job.0 as isize;
    Some(job)
}

fn wait_drive(child: &mut Child, root: &str) -> bool {
    for _ in 0..50 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return false;
        }
        if drive_kind(root) != DRIVE_NO_ROOT && drive_kind(root) != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn free_letter() -> Option<char> {
    let mut t = table();
    for c in ('D'..='Z').rev() {
        if t.mounts.values().any(|m| m.letter == c) || t.reserved.contains(&c) {
            continue;
        }
        if drive_kind(&format!("{c}:\\")) == DRIVE_NO_ROOT {
            t.reserved.insert(c);
            return Some(c);
        }
    }
    None
}

fn drive_kind(root: &str) -> u32 {
    let w = HSTRING::from(root);
    unsafe { GetDriveTypeW(&w) }
}

fn stop(mut live: Live) {
    let _ = live.child.kill();
    let _ = live.child.wait();
    let _ = net_delete(live.letter);
    let _ = std::fs::remove_file(&live.config);
}

fn net_delete(letter: char) -> std::io::Result<std::process::ExitStatus> {
    Command::new("net").args(["use", &format!("{letter}:"), "/delete", "/y"]).creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()
}

fn config_path(key: &str) -> PathBuf {
    let mut h: u32 = 2166136261;
    for b in key.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(16777619);
    }
    let dir = std::env::temp_dir().join("panegrid-remote");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{h:08x}.conf"))
}

fn read_capped(stderr: Option<impl std::io::Read>, cap: usize) -> Vec<u8> {
    let Some(mut stderr) = stderr else { return Vec::new() };
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        match stderr.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(buf.len());
                if room > 0 {
                    buf.extend_from_slice(&tmp[..n.min(room)]);
                }
            }
        }
    }
    buf
}

fn wipe(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    bytes.fill(0);
}

fn wipe_all(items: &mut [String]) {
    for s in items {
        wipe(s);
    }
}
