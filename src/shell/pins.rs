// Explorer's pinned folders and favourite files, shown as virtual groups above favourites.
// One Quick Access read on a background thread returns both. Neither list is part of favorites_tree.

use std::time::{Duration, Instant};

use crate::app::*;
use crate::imports::*;
use crate::session::*;
use crate::ui::*;

/// Quick Access / Home shell folder. Pinned and frequent items both live here.
const QUICK_ACCESS: &str = "shell:::{679f85cb-0220-4080-b29b-5540cc05aab6}";

/// System.Home.IsPinned — {635E9051-50A7-4BBE-A265-7784719C26C9}, 2.
const PKEY_HOME_ISPINNED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x635E9051_50A7_4BBE_A265_7784719C26C9),
    pid: 2,
};

pub(crate) const PIN_WAIT: Duration = Duration::from_secs(2);
pub(crate) const WM_PINS: u32 = WM_APP + 44;
const PIN_CAP: usize = 256;

struct PinState {
    folders: Vec<Pinned>,
    files: Vec<Pinned>,
    /// A read has finished at least once. Paint uses this so it does not poll.
    loaded: bool,
    last: Option<Instant>,
    /// Folder paths just pinned, kept if a read has not caught up yet.
    fresh_dir: Vec<(String, Instant)>,
    /// File paths just pinned, kept if a read has not caught up yet.
    fresh_file: Vec<(String, Instant)>,
    /// Paths just unpinned, hidden if a read still lists them.
    gone: Vec<(String, Instant)>,
}

const HOLD: Duration = Duration::from_secs(3);

fn fresh_enough(t: Instant, now: Instant) -> bool {
    now.saturating_duration_since(t) < HOLD
}

fn state() -> std::sync::MutexGuard<'static, PinState> {
    static STATE: std::sync::OnceLock<Mutex<PinState>> = std::sync::OnceLock::new();
    STATE
        .get_or_init(|| {
            Mutex::new(PinState {
                folders: Vec::new(),
                files: Vec::new(),
                loaded: false,
                last: None,
                fresh_dir: Vec::new(),
                fresh_file: Vec::new(),
                gone: Vec::new(),
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

static GATE: Mutex<()> = Mutex::new(());
static WAKE: Condvar = Condvar::new();
static WANT: AtomicBool = AtomicBool::new(false);
static FORCE: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
static INFLIGHT: AtomicBool = AtomicBool::new(false);
static SIDES: Mutex<Vec<isize>> = Mutex::new(Vec::new());

pub(crate) fn pin_list() -> Vec<Pinned> {
    state().folders.clone()
}

pub(crate) fn pin_files() -> Vec<Pinned> {
    state().files.clone()
}

pub(crate) fn note_pins_hwnd(h: HWND) {
    let id = h.0 as isize;
    if id == 0 {
        return;
    }
    let mut v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
    if !v.contains(&id) {
        v.push(id);
    }
}

pub(crate) fn drop_pins_hwnd(h: HWND) {
    let id = h.0 as isize;
    SIDES.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| *x != id);
}

fn post_pins() {
    let mut v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|id| {
        let h = HWND(*id as *mut core::ffi::c_void);
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                _ = PostMessageW(Some(h), WM_PINS, WPARAM(0), LPARAM(0));
                true
            } else {
                false
            }
        }
    });
}

/// Start a Quick Access read. `force` skips the 2s gap (a pin or unpin we just did).
/// A collapsed group is not read unless the other group is open or `force` is set.
/// Never calls the shell on this thread.
pub(crate) fn request_pins(force: bool) {
    if !force && !app(|a| a.s.pins_open || a.s.files_open) {
        return;
    }
    if !force {
        let last = state().last;
        if !pins_due(last, Instant::now(), PIN_WAIT) {
            return;
        }
    }
    if force {
        FORCE.store(true, Ordering::Relaxed);
    }
    if INFLIGHT.load(Ordering::Relaxed) {
        if force {
            AGAIN.store(true, Ordering::Relaxed);
        }
        return;
    }
    WANT.store(true, Ordering::Relaxed);
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().name("explorer-pins".into()).spawn(pin_worker));
    WAKE.notify_one();
}

/// First paint while either group is expanded and no read has finished.
/// Later paints do not re-read; expand and WM_ACTIVATE do.
pub(crate) fn pins_on_paint() {
    if !app(|a| a.s.pins_open || a.s.files_open) || state().loaded {
        return;
    }
    request_pins(false);
}

/// Insert at the front of the folder or file list when it is not already there.
/// False means that list already had it. `file` chooses the favourite-files list.
pub(crate) fn remember_pinned(path: &str, name: &str, file: bool) -> bool {
    let path = tidy_path(path);
    if path.is_empty() {
        return false;
    }
    let mut s = state();
    let listed = if file { &s.files } else { &s.folders };
    if listed.iter().any(|p| p.path.eq_ignore_ascii_case(&path)) {
        return false;
    }
    let name = {
        let n = name.trim();
        if n.is_empty() { short_name(&path) } else { n.to_string() }
    };
    s.gone.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    if file {
        s.fresh_dir.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
        s.folders.retain(|p| !p.path.eq_ignore_ascii_case(&path));
        s.fresh_file.push((path.clone(), Instant::now()));
        s.files.insert(0, Pinned { path, name });
    } else {
        s.fresh_file.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
        s.files.retain(|p| !p.path.eq_ignore_ascii_case(&path));
        s.fresh_dir.push((path.clone(), Instant::now()));
        s.folders.insert(0, Pinned { path, name });
    }
    true
}

pub(crate) fn forget_pinned(path: &str) {
    let path = tidy_path(path);
    if path.is_empty() {
        return;
    }
    let mut s = state();
    s.fresh_dir.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    s.fresh_file.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    s.gone.push((path.clone(), Instant::now()));
    s.folders.retain(|p| !p.path.eq_ignore_ascii_case(&path));
    s.files.retain(|p| !p.path.eq_ignore_ascii_case(&path));
}

/// True for a real directory. A file that Explorer can also open as a folder (a zip) is not one.
pub(crate) fn shell_is_folder(flags: SFGAO_FLAGS) -> bool {
    flags.contains(SFGAO_FOLDER) && !flags.contains(SFGAO_STREAM)
}

/// `pintohome` or `unpinfromhome` on a filesystem folder or file.
pub(crate) fn shell_home_verb(path: &str, pin: bool) -> bool {
    let path = tidy_path(path);
    if path.is_empty() {
        return false;
    }
    let verb = if pin { "pintohome" } else { "unpinfromhome" };
    unsafe { invoke_verb(&path, verb) }
}

fn pin_worker() {
    unsafe { _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    loop {
        let force = {
            let mut gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if WANT.swap(false, Ordering::Relaxed) {
                    break FORCE.swap(false, Ordering::Relaxed);
                }
                gate = WAKE.wait(gate).unwrap_or_else(|e| e.into_inner());
            }
        };
        if !force {
            let last = state().last;
            if !pins_due(last, Instant::now(), PIN_WAIT) {
                continue;
            }
        }
        INFLIGHT.store(true, Ordering::Relaxed);
        let split = read_pins();
        {
            let now = Instant::now();
            let mut s = state();
            s.fresh_dir.retain(|(_, t)| fresh_enough(*t, now));
            s.fresh_file.retain(|(_, t)| fresh_enough(*t, now));
            s.gone.retain(|(_, t)| fresh_enough(*t, now));
            let gone = s.gone.clone();
            let cur_dir = std::mem::take(&mut s.folders);
            let mut fresh_dir = std::mem::take(&mut s.fresh_dir);
            let folders = merge_read(split.folders, &cur_dir, &mut fresh_dir, &gone);
            let cur_file = std::mem::take(&mut s.files);
            let mut fresh_file = std::mem::take(&mut s.fresh_file);
            let files = merge_read(split.files, &cur_file, &mut fresh_file, &gone);
            s.folders = folders;
            s.fresh_dir = fresh_dir;
            s.files = files;
            s.fresh_file = fresh_file;
            s.loaded = true;
            s.last = Some(now);
        }
        INFLIGHT.store(false, Ordering::Relaxed);
        post_pins();
        if AGAIN.swap(false, Ordering::Relaxed) {
            FORCE.store(true, Ordering::Relaxed);
            WANT.store(true, Ordering::Relaxed);
        }
    }
}

fn merge_read(read: Vec<Pinned>, current: &[Pinned], fresh: &mut Vec<(String, Instant)>, gone: &[(String, Instant)]) -> Vec<Pinned> {
    let mut items = read;
    items.retain(|p| !gone.iter().any(|(g, _)| g.eq_ignore_ascii_case(&p.path)));
    // Drop a grace entry once Quick Access itself lists the item.
    fresh.retain(|(f, _)| !items.iter().any(|i| i.path.eq_ignore_ascii_case(f)));
    let keep: Vec<Pinned> = current.iter().filter(|p| fresh.iter().any(|(f, _)| f.eq_ignore_ascii_case(&p.path))).cloned().collect();
    for p in keep.into_iter().rev() {
        items.insert(0, p);
    }
    items
}

fn read_pins() -> PinSplit {
    unsafe { split_pins(&read_quick_access()) }
}

unsafe fn read_quick_access() -> Vec<PinCand> {
    let Ok(root) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS), None) else {
        return Vec::new();
    };
    let Ok(folder) = root.BindToHandler::<_, IShellFolder>(None, &BHID_SFObject) else {
        return Vec::new();
    };
    let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0) as u32;
    let mut enumerator: Option<IEnumIDList> = None;
    if folder.EnumObjects(HWND::default(), flags, &mut enumerator) != S_OK {
        return Vec::new();
    }
    let Some(enumerator) = enumerator else { return Vec::new() };
    let mut out = Vec::new();
    let key = PKEY_HOME_ISPINNED;
    while out.len() < PIN_CAP {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        let mut fetched = 0u32;
        let hr = enumerator.Next(std::slice::from_mut(&mut pidl), Some(&mut fetched));
        if hr != S_OK || fetched == 0 || pidl.is_null() {
            if !pidl.is_null() {
                ILFree(Some(pidl as *const ITEMIDLIST));
            }
            break;
        }
        if let Ok(child) = SHCreateItemWithParent::<_, IShellItem2>(None, &folder, pidl) {
            let pinned = child.GetBool(&key).is_ok_and(|b| b.as_bool());
            // A zip is both a folder and a stream. That is a file, not a directory.
            let is_folder = child.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).is_ok_and(shell_is_folder);
            let path = child.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).unwrap_or_default();
            let name = child.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
            out.push(PinCand { path, name, pinned, folder: is_folder });
        }
        ILFree(Some(pidl as *const ITEMIDLIST));
    }
    out
}

unsafe fn invoke_verb(path: &str, verb: &str) -> bool {
    let file = HSTRING::from(path);
    let verb_w = HSTRING::from(verb);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb_w.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if ShellExecuteExW(&mut sei).is_ok() {
        return true;
    }
    let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&file, None) else { return false };
    let Ok(cm) = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) else { return false };
    let ansi = format!("{verb}\0");
    let ici = CMINVOKECOMMANDINFO {
        cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
        lpVerb: PCSTR(ansi.as_ptr()),
        nShow: SW_HIDE.0 as i32,
        ..Default::default()
    };
    cm.InvokeCommand(&ici).is_ok()
}

#[cfg(test)]
mod tests {
    use super::shell_is_folder;
    use crate::imports::*;

    #[test]
    fn a_zip_is_a_file_and_a_directory_is_a_folder() {
        assert!(shell_is_folder(SFGAO_FOLDER));
        assert!(!shell_is_folder(SFGAO_FOLDER | SFGAO_STREAM));
        assert!(!shell_is_folder(SFGAO_STREAM));
        assert!(!shell_is_folder(SFGAO_FLAGS::default()));
    }
}
