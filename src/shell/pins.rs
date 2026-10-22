// Explorer's pinned folders, shown as a virtual group above favourites.
// The list is read from Quick Access on a background thread. It is not part of favorites_tree.

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
    items: Vec<Pinned>,
    /// A read has finished at least once. Paint uses this so it does not poll.
    loaded: bool,
    last: Option<Instant>,
    /// Paths just pinned, kept if a read has not caught up yet.
    fresh: Vec<(String, Instant)>,
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
        .get_or_init(|| Mutex::new(PinState { items: Vec::new(), loaded: false, last: None, fresh: Vec::new(), gone: Vec::new() }))
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
    state().items.clone()
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
/// Collapsed groups are not read unless `force` is set. Never calls the shell on this thread.
pub(crate) fn request_pins(force: bool) {
    if !force && !app(|a| a.s.pins_open) {
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

/// First paint of an expanded group. Later paints do not re-read; expand and WM_ACTIVATE do.
pub(crate) fn pins_on_paint() {
    if !app(|a| a.s.pins_open) || state().loaded {
        return;
    }
    request_pins(false);
}

/// Insert at the front when it is not already listed. False means it was already there.
pub(crate) fn remember_pinned(path: &str, name: &str) -> bool {
    let path = tidy_path(path);
    if path.is_empty() {
        return false;
    }
    let mut s = state();
    if s.items.iter().any(|p| p.path.eq_ignore_ascii_case(&path)) {
        return false;
    }
    let name = {
        let n = name.trim();
        if n.is_empty() { short_name(&path) } else { n.to_string() }
    };
    s.gone.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    s.fresh.push((path.clone(), Instant::now()));
    s.items.insert(0, Pinned { path, name });
    true
}

pub(crate) fn forget_pinned(path: &str) {
    let path = tidy_path(path);
    if path.is_empty() {
        return;
    }
    let mut s = state();
    s.fresh.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    s.gone.push((path.clone(), Instant::now()));
    s.items.retain(|p| !p.path.eq_ignore_ascii_case(&path));
}

/// `pintohome` or `unpinfromhome` on a filesystem folder.
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
        let items = read_pins();
        {
            let now = Instant::now();
            let mut s = state();
            s.fresh.retain(|(_, t)| fresh_enough(*t, now));
            s.gone.retain(|(_, t)| fresh_enough(*t, now));
            let mut items = items;
            items.retain(|p| !s.gone.iter().any(|(g, _)| g.eq_ignore_ascii_case(&p.path)));
            // Drop a grace entry once Quick Access itself lists the folder.
            s.fresh.retain(|(f, _)| !items.iter().any(|i| i.path.eq_ignore_ascii_case(f)));
            let keep: Vec<Pinned> = s.items.iter().filter(|p| s.fresh.iter().any(|(f, _)| f.eq_ignore_ascii_case(&p.path))).cloned().collect();
            for p in keep.into_iter().rev() {
                items.insert(0, p);
            }
            s.items = items;
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

fn read_pins() -> Vec<Pinned> {
    unsafe { pinned_folders(&read_quick_access()) }
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
            let is_folder = child.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
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
