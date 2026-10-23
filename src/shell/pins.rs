// Explorer's pinned folders and favourite files, shown as virtual groups above favourites.
// One Quick Access read on a background thread returns both. Neither list is part of favorites_tree.

use std::time::{Duration, Instant};

use crate::app::*;
use crate::imports::*;
use crate::session::*;
use crate::ui::*;
use windows::Win32::System::Variant::{
    VT_BOOL, VT_EMPTY, VT_ERROR, VT_I1, VT_I2, VT_I8, VT_INT, VT_NULL, VT_UI1, VT_UI2, VT_UI4, VT_UI8, VT_UINT,
};

// PROPVARIANT without the StructuredStorage feature: that module is large enough
// that a release build of it is killed on a small machine. Layout is the COM one
// (8-byte header plus a 16-byte value) on x64.
#[repr(C)]
struct PropVar {
    vt: u16,
    w1: u16,
    w2: u16,
    w3: u16,
    data: PropData,
}

#[repr(C)]
union PropData {
    bytes: [u8; 16],
    _align: u64,
}

link!("ole32.dll" "system" fn PropVariantClear(pvar: *mut PropVar) -> HRESULT);

/// IUnknown (3) + IShellItem (5) + the five IShellItem2 methods before GetProperty.
const GET_PROPERTY: usize = 13;

/// Quick Access / Home shell folder. Pinned and frequent items both live here.
const QUICK_ACCESS: &str = "shell:::{679f85cb-0220-4080-b29b-5540cc05aab6}";

/// System.Home.IsPinned — {30C8EEF4-A832-41E2-AB32-E3C3CA28FD29}, 4 (from PSGetPropertyKeyFromName).
const PKEY_HOME_ISPINNED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x30C8EEF4_A832_41E2_AB32_E3C3CA28FD29),
    pid: 4,
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
/// Set when a read finished before any favourites window existed, so the next one invalidates.
static DIRTY: AtomicBool = AtomicBool::new(false);
static TRIES: AtomicUsize = AtomicUsize::new(0);
static SIDES: Mutex<Vec<isize>> = Mutex::new(Vec::new());

const PIN_TRIES: usize = 6;

pub(crate) fn pin_list() -> Vec<Pinned> {
    state().folders.clone()
}

pub(crate) fn pin_files() -> Vec<Pinned> {
    state().files.clone()
}

pub(crate) fn pins_loaded() -> bool {
    state().loaded
}

pub(crate) fn note_pins_hwnd(h: HWND) {
    let id = h.0 as isize;
    if id == 0 {
        return;
    }
    {
        let mut v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
        if !v.contains(&id) {
            v.push(id);
        }
    }
    // The read can finish before this window exists. The rows are already stored;
    // invalidate so the first paint is not the last one.
    if DIRTY.swap(false, Ordering::AcqRel) {
        unsafe {
            _ = InvalidateRect(Some(h), None, false);
            if !pins_loaded() {
                _ = SetTimer(Some(h), TIMER_PINS, 700, None);
            }
        }
    }
}

pub(crate) fn drop_pins_hwnd(h: HWND) {
    let id = h.0 as isize;
    SIDES.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| *x != id);
}

fn post_pins() {
    let mut posted = 0usize;
    {
        let mut v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
        v.retain(|id| {
            let h = HWND(*id as *mut core::ffi::c_void);
            unsafe {
                if IsWindow(Some(h)).as_bool() {
                    _ = PostMessageW(Some(h), WM_PINS, WPARAM(0), LPARAM(0));
                    posted += 1;
                    true
                } else {
                    false
                }
            }
        });
    }
    DIRTY.store(posted == 0, Ordering::Release);
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
    let start = {
        let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
        if INFLIGHT.load(Ordering::Relaxed) {
            if force {
                AGAIN.store(true, Ordering::Relaxed);
            }
            false
        } else {
            if force {
                FORCE.store(true, Ordering::Relaxed);
            }
            WANT.store(true, Ordering::Relaxed);
            true
        }
    };
    if !start {
        return;
    }
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
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            pin_log(&format!("CoInitializeEx {:#010x}", hr.0 as u32));
        }
    }
    loop {
        let force = wait_request();
        if !force {
            let last = state().last;
            if !pins_due(last, Instant::now(), PIN_WAIT) {
                end_request();
                continue;
            }
        }
        let read = unsafe { read_quick_access() };
        apply_read(&read);
        end_request();
        post_pins();
    }
}

/// Take a queued read. `INFLIGHT` is set before the gate is released so a force
/// that arrives during the read is remembered instead of lost.
fn wait_request() -> bool {
    let mut gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if WANT.swap(false, Ordering::Relaxed) {
            INFLIGHT.store(true, Ordering::Relaxed);
            return FORCE.swap(false, Ordering::Relaxed);
        }
        gate = WAKE.wait(gate).unwrap_or_else(|e| e.into_inner());
    }
}

fn end_request() {
    let again = {
        let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
        INFLIGHT.store(false, Ordering::Relaxed);
        let again = AGAIN.swap(false, Ordering::Relaxed);
        if again {
            FORCE.store(true, Ordering::Relaxed);
            WANT.store(true, Ordering::Relaxed);
        }
        again
    };
    if again {
        WAKE.notify_one();
    }
}

fn apply_read(read: &HomeRead) {
    let split = if read.ok { split_pins(&read.items) } else { PinSplit { folders: Vec::new(), files: Vec::new() } };
    let pinned = read.items.iter().filter(|i| i.pinned).count();
    let empty_path = read.items.iter().filter(|i| i.path.is_empty()).count();
    let paths = read
        .items
        .iter()
        .filter(|i| i.pinned && !i.path.is_empty())
        .take(12)
        .map(|i| i.path.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    pin_log(&format!(
        "pins ok={} err={} seen={} pinned={} folders={} files={} empty_path={} vts={} paths={paths}",
        u8::from(read.ok),
        read.err,
        read.items.len(),
        pinned,
        split.folders.len(),
        split.files.len(),
        empty_path,
        read.vts,
    ));
    let now = Instant::now();
    let mut s = state();
    if read.ok {
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
        TRIES.store(0, Ordering::Relaxed);
    } else if TRIES.fetch_add(1, Ordering::Relaxed) + 1 >= PIN_TRIES {
        s.loaded = true;
    }
    s.last = Some(now);
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

struct HomeRead {
    ok: bool,
    items: Vec<PinCand>,
    vts: String,
    err: String,
}

fn home_fail(err: String) -> HomeRead {
    HomeRead { ok: false, items: Vec::new(), vts: String::new(), err }
}

/// Nonzero `VT_BOOL` / integer means pinned. Empty, null, and error are unknown so the caller can fall back.
fn flag_from_variant(vt: u16, bool_val: i16, signed: i64, unsigned: u64) -> Option<bool> {
    if vt == VT_EMPTY.0 || vt == VT_NULL.0 || vt == VT_ERROR.0 {
        return None;
    }
    if vt == VT_BOOL.0 {
        return Some(bool_val != 0);
    }
    if vt == VT_I1.0 || vt == VT_I2.0 || vt == VT_I4.0 || vt == VT_I8.0 || vt == VT_INT.0 {
        return Some(signed != 0);
    }
    if vt == VT_UI1.0 || vt == VT_UI2.0 || vt == VT_UI4.0 || vt == VT_UI8.0 || vt == VT_UINT.0 {
        return Some(unsigned != 0);
    }
    None
}

fn note_vt(vts: &mut String, vt: u16, pinned: bool) {
    if vts.len() > 180 {
        return;
    }
    if !vts.is_empty() {
        vts.push(',');
    }
    vts.push_str(&format!("{vt}:{}", u8::from(pinned)));
}

#[cfg(debug_assertions)]
fn pin_log(msg: &str) {
    let path = std::env::temp_dir().join("grids-pins.log");
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
    use std::io::Write;
    let _ = writeln!(file, "{msg}");
}

#[cfg(not(debug_assertions))]
fn pin_log(_: &str) {}

unsafe fn read_quick_access() -> HomeRead {
    let root = match SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS), None) {
        Ok(root) => root,
        Err(e) => return home_fail(format!("parse {:#010x}", e.code().0 as u32)),
    };
    let enumerator = match root.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) {
        Ok(enumerator) => enumerator,
        Err(e) => return home_fail(format!("enum {:#010x}", e.code().0 as u32)),
    };
    let folder = root.BindToHandler::<_, IShellFolder2>(None, &BHID_SFObject).ok();
    let mut out = Vec::new();
    let mut vts = String::new();
    while out.len() < PIN_CAP {
        let mut slot = [None];
        // celt is 1, so a null fetched-count is allowed. The item pointer is the signal.
        let hr = enumerator.Next(&mut slot, None);
        let Some(child) = slot[0].take() else {
            if hr.is_err() && out.is_empty() {
                return home_fail(format!("next {:#010x}", hr.unwrap_err().code().0 as u32));
            }
            break;
        };
        let (pinned, vt) = item_is_pinned(&child, folder.as_ref());
        // A zip is both a folder and a stream. That is a file, not a directory.
        let is_folder = child.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).is_ok_and(shell_is_folder);
        let path = child.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).unwrap_or_default();
        let name = child.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
        note_vt(&mut vts, vt, pinned);
        out.push(PinCand { path, name, pinned, folder: is_folder });
    }
    HomeRead { ok: true, items: out, vts, err: String::new() }
}

/// `System.Home.IsPinned` is `VT_BOOL` on some Windows builds and `VT_UI4` (or another integer) on others.
/// `GetBool` rejects the integer form, which made every Quick Access child look unpinned.
unsafe fn item_is_pinned(child: &IShellItem, folder: Option<&IShellFolder2>) -> (bool, u16) {
    if let Ok(item) = child.cast::<IShellItem2>() {
        let (flag, vt) = property_flag(&item);
        if let Some(flag) = flag {
            return (flag, vt);
        }
        if let Some(folder) = folder {
            if let Some((flag, vt2)) = details_flag(folder, child) {
                return (flag, vt2);
            }
        }
        return (false, vt);
    }
    if let Some(folder) = folder {
        if let Some((flag, vt)) = details_flag(folder, child) {
            return (flag, vt);
        }
    }
    (false, 0)
}

unsafe fn property_flag(item: &IShellItem2) -> (Option<bool>, u16) {
    let mut pv = PropVar { vt: 0, w1: 0, w2: 0, w3: 0, data: PropData { bytes: [0; 16] } };
    if get_property(item, &mut pv).is_err() {
        return (None, 0);
    }
    let (vt, bool_val, signed, unsigned) = prop_numbers(&pv);
    let flag = flag_from_variant(vt, bool_val, signed, unsigned);
    _ = PropVariantClear(&mut pv);
    (flag, vt)
}

unsafe fn get_property(item: &IShellItem2, pv: *mut PropVar) -> HRESULT {
    type GetProperty = unsafe extern "system" fn(*mut core::ffi::c_void, *const PROPERTYKEY, *mut PropVar) -> HRESULT;
    let this = windows::core::Interface::as_raw(item);
    let vtbl = *(this as *const *const usize);
    let get_property: GetProperty = std::mem::transmute(*vtbl.add(GET_PROPERTY));
    get_property(this, &PKEY_HOME_ISPINNED, pv)
}

unsafe fn prop_numbers(pv: &PropVar) -> (u16, i16, i64, u64) {
    let d = pv.data.bytes;
    let bool_val = i16::from_le_bytes([d[0], d[1]]);
    let i8_val = d[0] as i8 as i64;
    let u8_val = d[0] as u64;
    let i16_val = i16::from_le_bytes([d[0], d[1]]) as i64;
    let u16_val = u16::from_le_bytes([d[0], d[1]]) as u64;
    let i32_val = i32::from_le_bytes([d[0], d[1], d[2], d[3]]) as i64;
    let u32_val = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as u64;
    let i64_val = i64::from_le_bytes(d[0..8].try_into().unwrap());
    let u64_val = u64::from_le_bytes(d[0..8].try_into().unwrap());
    let (signed, unsigned) = match pv.vt {
        v if v == VT_I1.0 => (i8_val, 0),
        v if v == VT_UI1.0 => (0, u8_val),
        v if v == VT_I2.0 => (i16_val, 0),
        v if v == VT_UI2.0 => (0, u16_val),
        v if v == VT_I4.0 || v == VT_INT.0 => (i32_val, 0),
        v if v == VT_UI4.0 || v == VT_UINT.0 => (0, u32_val),
        v if v == VT_I8.0 => (i64_val, 0),
        v if v == VT_UI8.0 => (0, u64_val),
        _ => (0, 0),
    };
    let bool_val = if pv.vt == VT_BOOL.0 { bool_val } else { 0 };
    (pv.vt, bool_val, signed, unsigned)
}

unsafe fn details_flag(folder: &IShellFolder2, child: &IShellItem) -> Option<(bool, u16)> {
    let rel = child_rel_pidl(child)?;
    let got = folder.GetDetailsEx(rel.ptr, &PKEY_HOME_ISPINNED);
    drop(rel);
    let mut v = got.ok()?;
    let (vt, bool_val, signed, unsigned) = variant_numbers(&v);
    let flag = flag_from_variant(vt, bool_val, signed, unsigned).unwrap_or(false);
    _ = VariantClear(&mut v);
    Some((flag, vt))
}

unsafe fn variant_numbers(v: &VARIANT) -> (u16, i16, i64, u64) {
    let head = &v.Anonymous.Anonymous;
    let vt = head.vt;
    let raw = &head.Anonymous;
    let (bool_val, signed, unsigned) = match vt {
        VT_BOOL => (raw.boolVal.0, 0, 0),
        VT_I1 => (0, raw.cVal as i64, 0),
        VT_UI1 => (0, 0, raw.bVal as u64),
        VT_I2 => (0, raw.iVal as i64, 0),
        VT_UI2 => (0, 0, raw.uiVal as u64),
        VT_I4 => (0, raw.lVal as i64, 0),
        VT_INT => (0, raw.intVal as i64, 0),
        VT_UI4 => (0, 0, raw.ulVal as u64),
        VT_UINT => (0, 0, raw.uintVal as u64),
        VT_I8 => (0, raw.llVal, 0),
        VT_UI8 => (0, 0, raw.ullVal),
        _ => (0, 0, 0),
    };
    (vt.0, bool_val, signed, unsigned)
}

struct RelPidl {
    owned: *mut ITEMIDLIST,
    ptr: *const ITEMIDLIST,
}

impl Drop for RelPidl {
    fn drop(&mut self) {
        if !self.owned.is_null() {
            unsafe { ILFree(Some(self.owned as *const ITEMIDLIST)) };
        }
    }
}

unsafe fn child_rel_pidl(child: &IShellItem) -> Option<RelPidl> {
    if let Ok(pai) = child.cast::<IParentAndItem>() {
        let mut rel: *mut ITEMIDLIST = std::ptr::null_mut();
        let ok = pai.GetParentAndItem(None, None, Some(std::ptr::addr_of_mut!(rel))).is_ok();
        if ok && !rel.is_null() {
            return Some(RelPidl { owned: rel, ptr: rel });
        }
        if !rel.is_null() {
            ILFree(Some(rel as *const ITEMIDLIST));
        }
    }
    let abs = SHGetIDListFromObject(child).ok()?;
    if abs.is_null() {
        return None;
    }
    let rel = ILFindLastID(abs);
    if rel.is_null() {
        ILFree(Some(abs as *const ITEMIDLIST));
        return None;
    }
    Some(RelPidl { owned: abs, ptr: rel })
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
    use super::{flag_from_variant, shell_is_folder};
    use crate::imports::*;
    use windows::Win32::System::Variant::{VT_BOOL, VT_EMPTY, VT_NULL, VT_UI4};

    #[test]
    fn a_zip_is_a_file_and_a_directory_is_a_folder() {
        assert!(shell_is_folder(SFGAO_FOLDER));
        assert!(!shell_is_folder(SFGAO_FOLDER | SFGAO_STREAM));
        assert!(!shell_is_folder(SFGAO_STREAM));
        assert!(!shell_is_folder(SFGAO_FLAGS::default()));
    }

    #[test]
    fn propvar_is_the_com_layout() {
        assert_eq!(std::mem::size_of::<super::PropVar>(), 24);
        assert_eq!(std::mem::align_of::<super::PropVar>(), 8);
    }

    #[test]
    fn home_ispinned_accepts_bool_and_any_nonzero_integer() {
        assert_eq!(VT_BOOL.0, 11);
        assert_eq!(VT_UI4.0, 19);
        assert_eq!(flag_from_variant(VT_BOOL.0, -1, 0, 0), Some(true));
        assert_eq!(flag_from_variant(VT_BOOL.0, 0, 0, 0), Some(false));
        assert_eq!(flag_from_variant(VT_UI4.0, 0, 0, 1), Some(true));
        assert_eq!(flag_from_variant(VT_UI4.0, 0, 0, 0), Some(false));
        assert_eq!(flag_from_variant(VT_I4.0, 0, 0, 0), Some(false));
        assert_eq!(flag_from_variant(VT_I4.0, 0, 3, 0), Some(true));
        assert_eq!(flag_from_variant(VT_I4.0, 0, -2, 0), Some(true));
        assert_eq!(flag_from_variant(VT_EMPTY.0, -1, 1, 1), None);
        assert_eq!(flag_from_variant(VT_NULL.0, -1, 1, 1), None);
    }
}
