// Third-party context menu entries, and the Panegrid-only filter.
//
// The registry is read, never written. Enablement lives in panegrid.json (`ctx_menu`),
// not in HKCR and not in a .grids file. Explorer is unchanged.
//
// Building the menu (only when something that applies to this click is off):
// `SHCreateDefaultContextMenu` gets the shell's own verbs (Open, Copy, Delete, Properties, …)
// plus any association key that has no disabled third-party handler for this click.
// A key that does is left out, and each built-in or still-enabled handler on it is
// CoCreated on its own, so a disabled DLL is never loaded. Static verbs on those keys
// are appended here. `aKeys` cannot name a single handler — the key is the whole class —
// so this split is what actually skips the load.
//
// If that object cannot be created, the shell menu is built as usual and matching items
// are removed afterwards. That still loads the handler. App items (favourites, batch
// rename, WinMerge) are inserted by `popup_ctx` either way, and a click with nothing
// disabled never comes through here.
//
// Set `PANEGRID_CTX_TIMING=1` to log each phase with OutputDebugString (DebugView).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER};

use crate::imports::*;
use crate::app::*;
use crate::i18n::tr;

/// `CMINVOKECOMMANDINFOEX.lpVerbW` is a Unicode verb. The windows crate no longer exports the flag.
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;

pub(crate) const FILE: u8 = 1;
pub(crate) const FOLDER: u8 = 2;
pub(crate) const BACKGROUND: u8 = 4;
pub(crate) const DRIVE: u8 = 8;
const TARGET_ALL: u8 = FILE | FOLDER | BACKGROUND | DRIVE;

const FIXED: &[(&str, u8)] = &[
    ("*", FILE),
    ("AllFilesystemObjects", FILE | FOLDER | DRIVE),
    ("Directory", FOLDER | DRIVE),
    ("Directory\\Background", BACKGROUND),
    ("Folder", FOLDER | DRIVE),
    ("Folder\\Background", BACKGROUND),
    ("Drive", DRIVE),
];

const BASE_END: u32 = 0x3000;
const SLOT: u32 = 0x100;
const VERB_BASE: u32 = 0x7000;
const VERB_END: u32 = 0x7F00;
const KEY_CAP: usize = 16;

link!("shell32.dll" "system" fn SHCreateDefaultContextMenu(pdcm: *const DefCtx, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> HRESULT);
link!("kernel32.dll" "system" fn ExpandEnvironmentStringsW(src: PCWSTR, dst: PWSTR, n: u32) -> u32);
link!("shlwapi.dll" "system" fn SHLoadIndirectString(src: PCWSTR, dst: PWSTR, cch: u32, reserved: *mut core::ffi::c_void) -> HRESULT);
link!("version.dll" "system" fn GetFileVersionInfoSizeW(file: PCWSTR, handle: *mut u32) -> u32);
link!("version.dll" "system" fn GetFileVersionInfoW(file: PCWSTR, handle: u32, len: u32, data: *mut core::ffi::c_void) -> BOOL);
link!("version.dll" "system" fn VerQueryValueW(block: *const core::ffi::c_void, sub: PCWSTR, buf: *mut *mut core::ffi::c_void, len: *mut u32) -> BOOL);

#[repr(C)]
struct DefCtx {
    hwnd: HWND,
    pcmcb: *mut core::ffi::c_void,
    pidl_folder: *const ITEMIDLIST,
    psf: *mut core::ffi::c_void,
    cidl: u32,
    _pad0: u32,
    apidl: *const *const ITEMIDLIST,
    punk: *mut core::ffi::c_void,
    ckeys: u32,
    _pad1: u32,
    akeys: *const HKEY,
}

#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    targets: u8,
    builtin: bool,
    clsid: Option<GUID>,
    verb: String,
    command: String,
    rank: u8,
    extended: bool,
    single: bool,
    position: i8,
    classes: Vec<String>,
    /// Class passed to IShellExtInit. The most specific registration.
    anchor: String,
}

struct Part {
    first: u32,
    cm: IContextMenu,
}

struct VerbSlot {
    id: u32,
    command: String,
}

#[implement(IContextMenu, IContextMenu2, IContextMenu3)]
struct CtxHost {
    folder: IShellFolder,
    folder_pidl: *mut ITEMIDLIST,
    children: Vec<*mut ITEMIDLIST>,
    data: Option<IDataObject>,
    owner: HWND,
    targets: u8,
    paths: Vec<String>,
    classes: Vec<String>,
    base: RefCell<Option<IContextMenu>>,
    parts: RefCell<Vec<Part>>,
    verbs: RefCell<Vec<VerbSlot>>,
    keys: RefCell<Vec<HKEY>>,
}

impl Drop for CtxHost {
    fn drop(&mut self) {
        unsafe {
            if !self.folder_pidl.is_null() {
                CoTaskMemFree(Some(self.folder_pidl as _));
            }
            for p in &self.children {
                if !p.is_null() {
                    CoTaskMemFree(Some(*p as _));
                }
            }
            for k in self.keys.get_mut().iter().copied() {
                _ = RegCloseKey(k);
            }
        }
    }
}

pub(crate) struct UiRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) targets: u8,
    pub(crate) off: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[derive(Debug)]
pub(crate) enum Tick {
    Off,
    On,
    Mixed,
}

pub(crate) fn target_label(bit: u8) -> &'static str {
    match bit {
        FILE => tr("File", "파일", "ファイル"),
        FOLDER => tr("Folder", "폴더", "フォルダー"),
        BACKGROUND => tr("Empty space", "빈 곳", "空白"),
        DRIVE => tr("Drive", "드라이브", "ドライブ"),
        _ => "",
    }
}

pub(crate) fn target_bits() -> [u8; 4] {
    [FILE, FOLDER, BACKGROUND, DRIVE]
}

/// A drive root (`C:` or `C:\`) is a drive. Anything else is a file or a folder.
pub(crate) fn path_kind(path: &str, is_file: bool) -> u8 {
    let t = path.trim().trim_end_matches(['\\', '/']);
    let b = t.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        DRIVE
    } else if is_file {
        FILE
    } else {
        FOLDER
    }
}

pub(crate) fn click_targets(background: bool, items: &[(String, bool)]) -> u8 {
    if background {
        return BACKGROUND;
    }
    items.iter().fold(0, |a, (p, file)| a | path_kind(p, *file))
}

pub(crate) fn needs_filter(targets: u8) -> bool {
    if targets == 0 {
        return false;
    }
    app(|a| a.s.ctx_off.values().any(|v| bits_of(v) & targets != 0))
}

pub(crate) fn tick_of(off: u8, attached: u8) -> Tick {
    let off = off & attached;
    if attached == 0 || off == 0 {
        Tick::On
    } else if off == attached {
        Tick::Off
    } else {
        Tick::Mixed
    }
}

pub(crate) fn target_enabled(off: u8, bit: u8) -> bool {
    off & bit == 0
}

/// Master checkbox. On means every attached target is enabled; a click then turns them all off.
/// Anything else (off or mixed) turns them all on.
pub(crate) fn toggle_master(map: &mut BTreeMap<String, Vec<String>>, id: &str, attached: u8) {
    let cur = bits_of(map.get(id).map(Vec::as_slice).unwrap_or(&[]));
    let next = if cur & attached == 0 { attached } else { cur & !attached };
    write_bits(map, id, next & TARGET_ALL);
}

pub(crate) fn toggle_target(map: &mut BTreeMap<String, Vec<String>>, id: &str, bit: u8, attached: u8) {
    let cur = bits_of(map.get(id).map(Vec::as_slice).unwrap_or(&[]));
    let next = (cur ^ bit) & attached & TARGET_ALL;
    write_bits(map, id, next);
}

pub(crate) fn clear_off(map: &mut BTreeMap<String, Vec<String>>) {
    map.clear();
}

pub(crate) fn ui_rows(off: &BTreeMap<String, Vec<String>>, q: &str) -> Vec<UiRow> {
    let q = q.trim().to_lowercase();
    catalogue()
        .into_iter()
        .filter(|r| !r.builtin && r.targets != 0)
        .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q) || target_blob(r.targets).contains(&q))
        .map(|r| UiRow {
            off: bits_of(off.get(&r.id).map(Vec::as_slice).unwrap_or(&[])),
            id: r.id,
            name: r.name,
            targets: r.targets,
        })
        .collect()
}

/// Search hit on a handler name. A one-character query does not walk the registry.
pub(crate) fn name_hit(q: &str) -> bool {
    let q = q.trim();
    if q.chars().count() < 2 {
        return false;
    }
    let q = q.to_lowercase();
    catalogue().iter().any(|r| !r.builtin && r.name.to_lowercase().contains(&q))
}

pub(crate) fn invalidate() {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
}

thread_local! {
    static STRIP: Cell<u8> = const { Cell::new(0) };
}

pub(crate) fn arm_strip(targets: u8) {
    STRIP.with(|s| s.set(targets));
}

pub(crate) fn take_strip() -> u8 {
    STRIP.with(|s| s.replace(0))
}

pub(crate) fn log_phase(phase: &str, t0: Instant) {
    if !timing() {
        return;
    }
    let ms = t0.elapsed().as_millis();
    let text = format!("[panegrid] ctx {phase} {ms} ms\0");
    let w: Vec<u16> = text.encode_utf16().collect();
    unsafe { OutputDebugStringW(PCWSTR(w.as_ptr())) };
}

fn timing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("PANEGRID_CTX_TIMING") {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => false,
    })
}

pub(crate) unsafe fn compose(owner: HWND, folder: &IShellFolder, children: &[*const ITEMIDLIST], targets: u8, paths: &[String]) -> Option<IContextMenu> {
    let t0 = Instant::now();
    let folder_pidl = SHGetIDListFromObject(folder).unwrap_or(std::ptr::null_mut());
    let mut kids = Vec::new();
    for c in children {
        if c.is_null() {
            continue;
        }
        let d = ILClone(*c as *mut ITEMIDLIST);
        if !d.is_null() {
            kids.push(d);
        }
    }
    let refs: Vec<*const ITEMIDLIST> = kids.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let data = if refs.is_empty() { None } else { folder.GetUIObjectOf::<IDataObject>(owner, &refs, None).ok() };
    let classes = classes_for(targets, paths);
    let host = CtxHost {
        folder: folder.clone(),
        folder_pidl,
        children: kids,
        data,
        owner,
        targets,
        paths: paths.to_vec(),
        classes,
        base: RefCell::new(None),
        parts: RefCell::new(Vec::new()),
        verbs: RefCell::new(Vec::new()),
        keys: RefCell::new(Vec::new()),
    };
    log_phase("compose", t0);
    Some(host.into())
}

/// Same parent folder: one filtered menu. Mixed parents fall back to the shell menu.
pub(crate) unsafe fn compose_paths(owner: HWND, items: &[(String, bool)], targets: u8) -> Option<IContextMenu> {
    let parent = crate::parent_of(&items.first()?.0)?;
    if !items.iter().all(|(p, _)| crate::parent_of(p).is_some_and(|q| q.eq_ignore_ascii_case(&parent))) {
        return None;
    }
    let mut folder_pidl = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(&parent), None, &mut folder_pidl, 0, None).ok()?;
    if folder_pidl.is_null() {
        return None;
    }
    let desktop = SHGetDesktopFolder().ok();
    let folder: Option<IShellFolder> = desktop.as_ref().and_then(|d| d.BindToObject(folder_pidl, None).ok());
    CoTaskMemFree(Some(folder_pidl as _));
    let folder = folder?;
    let mut kids: Vec<*mut ITEMIDLIST> = Vec::new();
    for (path, _) in items {
        let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path);
        if name.is_empty() {
            continue;
        }
        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let wide = HSTRING::from(name);
        if folder.ParseDisplayName(HWND::default(), None, &wide, None, &mut child, std::ptr::null_mut()).is_ok() && !child.is_null() {
            kids.push(child);
        }
    }
    if kids.len() != items.len() {
        for p in &kids {
            CoTaskMemFree(Some(*p as _));
        }
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = kids.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
    let menu = compose(owner, &folder, &refs, targets, &paths);
    for p in kids {
        CoTaskMemFree(Some(p as _));
    }
    menu
}

pub(crate) unsafe fn strip_disabled(menu: HMENU, cm: &IContextMenu, targets: u8) {
    let off = app(|a| a.s.ctx_off.clone());
    let cat = catalogue();
    let mut verbs = Vec::new();
    let mut names = Vec::new();
    for r in &cat {
        if !blocked(r, targets, &off) {
            continue;
        }
        if !r.verb.is_empty() {
            verbs.push(r.verb.to_ascii_lowercase());
        }
        if !r.name.is_empty() {
            names.push(fold_menu(&r.name));
        }
    }
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return;
    }
    for i in (0..n as u32).rev() {
        if super::menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        let verb = if id != u32::MAX && id >= 1 && id < 0x8000 { super::cmd_verb(cm, (id - 1) as usize) } else { String::new() };
        let verb_l = verb.trim().to_ascii_lowercase();
        if !verb_l.is_empty() && (builtin_verb(&verb_l) || shell_owned(&verb_l)) {
            continue;
        }
        let text = fold_menu(&item_text(menu, i));
        let drop_verb = !verb_l.is_empty() && verbs.iter().any(|v| v == &verb_l);
        let drop_name = !text.is_empty() && names.iter().any(|n| n == &text);
        if drop_verb || drop_name {
            _ = DeleteMenu(menu, i, MF_BYPOSITION);
        }
    }
}

fn item_text(menu: HMENU, pos: u32) -> String {
    let mut buf = [0u16; 256];
    let mut info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: PWSTR(buf.as_mut_ptr()),
        cch: buf.len() as u32,
        ..Default::default()
    };
    unsafe {
        if GetMenuItemInfoW(menu, pos, true, &mut info).is_err() {
            return String::new();
        }
    }
    let n = info.cch as usize;
    let n = n.min(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string()
}

fn fold_menu(s: &str) -> String {
    s.chars().filter(|c| *c != '&').collect::<String>().trim().to_ascii_lowercase()
}

impl IContextMenu_Impl for CtxHost_Impl {
    fn QueryContextMenu(&self, hmenu: HMENU, index_menu: u32, id_first: u32, id_last: u32, flags: u32) -> HRESULT {
        unsafe { self.fill_menu(hmenu, index_menu, id_first, id_last, flags) }
    }
    fn InvokeCommand(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        unsafe { self.invoke(pici) }
    }
    fn GetCommandString(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        unsafe { self.command_string(idcmd, utype, preserved, pszname, cchmax) }
    }
}

impl IContextMenu2_Impl for CtxHost_Impl {
    fn HandleMenuMsg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<()> {
        unsafe { self.forward_msg(umsg, wparam, lparam, std::ptr::null_mut()) }
    }
}

impl IContextMenu3_Impl for CtxHost_Impl {
    fn HandleMenuMsg2(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        unsafe { self.forward_msg(umsg, wparam, lparam, plresult) }
    }
}

impl CtxHost {
    unsafe fn fill_menu(&self, menu: HMENU, index_menu: u32, id_first: u32, id_last: u32, flags: u32) -> HRESULT {
        self.reset_parts();
        let off = app(|a| a.s.ctx_off.clone());
        let cat = catalogue();
        let (clean, dirty) = split_classes(&self.classes, self.targets, &cat, &off);
        let mut keys = Vec::new();
        let mut opened = Vec::new();
        for class in clean.iter().take(KEY_CAP) {
            if let Some(k) = open_hkcr(class) {
                keys.push(k);
                opened.push(class.clone());
            }
        }
        let refs: Vec<*const ITEMIDLIST> = self.children.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
        let dcm = DefCtx {
            hwnd: self.owner,
            pcmcb: std::ptr::null_mut(),
            pidl_folder: self.folder_pidl,
            psf: Interface::as_raw(&self.folder),
            cidl: refs.len() as u32,
            _pad0: 0,
            apidl: if refs.is_empty() { std::ptr::null() } else { refs.as_ptr() },
            punk: std::ptr::null_mut(),
            ckeys: keys.len() as u32,
            _pad1: 0,
            akeys: if keys.is_empty() { std::ptr::null() } else { keys.as_ptr() },
        };
        let t_base = Instant::now();
        let mut ppv = std::ptr::null_mut();
        let hr = SHCreateDefaultContextMenu(&dcm, &IContextMenu::IID, &mut ppv);
        if hr.is_err() || ppv.is_null() {
            for k in keys {
                _ = RegCloseKey(k);
            }
            log_phase("base-fail", t_base);
            return E_FAIL;
        }
        let base: IContextMenu = IContextMenu::from_raw(ppv);
        let base_last = id_first.saturating_add(BASE_END).min(id_last);
        let qhr = base.QueryContextMenu(menu, index_menu, id_first, base_last, flags);
        log_phase("base", t_base);
        if qhr.is_err() {
            for k in keys {
                _ = RegCloseKey(k);
            }
            return qhr;
        }
        *self.base.borrow_mut() = Some(base);
        *self.keys.borrow_mut() = keys;

        let shift = flags & CMF_EXTENDEDVERBS != 0;
        let manual = manual_rows(&dirty, &opened, self.targets, &cat, &off);
        let t_ext = Instant::now();
        let mut next = id_first.saturating_add(BASE_END);
        let mut at = GetMenuItemCount(Some(menu)).max(0) as u32;
        for row in &manual {
            if row.verb.is_empty() {
                // COM handler
            } else if row.command.is_empty() && row.clsid.is_some() {
                // DelegateExecute with no command string: load it like a handler.
            } else {
                continue;
            }
            let Some(guid) = row.clsid else { continue };
            if next.saturating_add(SLOT) > id_last.min(VERB_BASE) {
                break;
            }
            let Some(cm) = cocreate(&guid, &row.anchor, self.folder_pidl, self.data.as_ref()) else { continue };
            let hr = cm.QueryContextMenu(menu, at, next, next + SLOT, flags);
            if hr.is_ok() {
                self.parts.borrow_mut().push(Part { first: next, cm });
            }
            next = next.saturating_add(SLOT);
            at = GetMenuItemCount(Some(menu)).max(0) as u32;
        }
        log_phase("handlers", t_ext);

        let t_verb = Instant::now();
        let mut verb_id = VERB_BASE.max(next);
        let mut top_at = index_menu;
        for row in &manual {
            if row.command.is_empty() || row.verb.is_empty() {
                continue;
            }
            if shell_owned(&row.verb) {
                continue;
            }
            if row.extended && !shift {
                continue;
            }
            if row.single && self.paths.len() != 1 {
                continue;
            }
            if verb_id >= VERB_END || verb_id >= id_last {
                break;
            }
            let label = HSTRING::from(&row.name);
            let pos = if row.position < 0 { top_at } else { at };
            if InsertMenuW(menu, pos, MF_STRING | MF_BYPOSITION, verb_id as usize, &label).is_ok() {
                self.verbs.borrow_mut().push(VerbSlot { id: verb_id, command: row.command.clone() });
                if row.position < 0 {
                    top_at = top_at.saturating_add(1);
                }
                at = GetMenuItemCount(Some(menu)).max(0) as u32;
            }
            verb_id += 1;
        }
        log_phase("verbs", t_verb);
        HRESULT(0x200)
    }

    unsafe fn invoke(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        if pici.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let Some(offset) = crate::verb_offset(pici) else {
            if let Some(base) = self.base.borrow().as_ref() {
                return base.InvokeCommand(pici);
            }
            return Err(E_NOTIMPL.into());
        };
        // popup_ctx queries with id 1, then passes menu_id - 1.
        let menu_id = offset.saturating_add(1);
        if let Some(slot) = self.verbs.borrow().iter().find(|v| v.id == menu_id) {
            run_command((*pici).hwnd, &slot.command, &self.paths);
            return Ok(());
        }
        if let Some(part) = self.parts.borrow().iter().find(|p| menu_id >= p.first && menu_id < p.first + SLOT) {
            let local = menu_id - part.first;
            return invoke_part(&part.cm, local, &*pici);
        }
        if menu_id < BASE_END {
            if let Some(base) = self.base.borrow().as_ref() {
                return base.InvokeCommand(pici);
            }
        }
        Err(E_NOTIMPL.into())
    }

    unsafe fn command_string(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        let menu_id = idcmd.saturating_add(1) as u32;
        if let Some(slot) = self.verbs.borrow().iter().find(|v| v.id == menu_id) {
            write_verb(pszname, cchmax, utype, "");
            let _ = slot;
            return Ok(());
        }
        if let Some(part) = self.parts.borrow().iter().find(|p| menu_id >= p.first && menu_id < p.first + SLOT) {
            let local = (menu_id - part.first) as usize;
            return part.cm.GetCommandString(local, utype, Some(preserved), pszname, cchmax);
        }
        if menu_id < BASE_END {
            if let Some(base) = self.base.borrow().as_ref() {
                return base.GetCommandString(idcmd, utype, Some(preserved), pszname, cchmax);
            }
        }
        Err(E_NOTIMPL.into())
    }

    unsafe fn forward_msg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        let mut result = Ok(());
        if let Some(base) = self.base.borrow().as_ref() {
            result = forward_one(base, umsg, wparam, lparam, plresult);
        }
        for part in self.parts.borrow().iter() {
            let r = forward_one(&part.cm, umsg, wparam, lparam, plresult);
            if r.is_ok() {
                result = r;
            }
        }
        result
    }

    fn reset_parts(&self) {
        self.base.borrow_mut().take();
        self.parts.borrow_mut().clear();
        self.verbs.borrow_mut().clear();
        let old = std::mem::take(&mut *self.keys.borrow_mut());
        unsafe {
            for k in old {
                _ = RegCloseKey(k);
            }
        }
    }
}

unsafe fn forward_one(cm: &IContextMenu, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
    if let Ok(m) = cm.cast::<IContextMenu3>() {
        return m.HandleMenuMsg2(umsg, wparam, lparam, if plresult.is_null() { None } else { Some(plresult) });
    }
    if let Ok(m) = cm.cast::<IContextMenu2>() {
        return m.HandleMenuMsg(umsg, wparam, lparam);
    }
    Ok(())
}

unsafe fn invoke_part(cm: &IContextMenu, local: u32, ici: &CMINVOKECOMMANDINFO) -> Result<()> {
    let verb = part_verb(cm, local as usize);
    if !verb.is_empty() {
        let mut wide: Vec<u16> = verb.encode_utf16().chain(std::iter::once(0)).collect();
        let mut ansi: Vec<u8> = verb.bytes().chain(std::iter::once(0)).collect();
        let mut ex = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: ici.fMask | CMIC_MASK_UNICODE,
            hwnd: ici.hwnd,
            lpVerb: PCSTR(ansi.as_mut_ptr()),
            lpParameters: ici.lpParameters,
            lpDirectory: ici.lpDirectory,
            nShow: ici.nShow,
            dwHotKey: ici.dwHotKey,
            lpVerbW: PCWSTR(wide.as_mut_ptr()),
            ..Default::default()
        };
        if cm.InvokeCommand(&ex as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO).is_ok() {
            return Ok(());
        }
        let _ = (&mut ex, &mut wide, &mut ansi);
    }
    let mut c = *ici;
    c.lpVerb = PCSTR(local as usize as *const u8);
    cm.InvokeCommand(&c)
}

unsafe fn part_verb(cm: &IContextMenu, offset: usize) -> String {
    let mut buf = [0u16; 80];
    if cm.GetCommandString(offset, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_err() {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

unsafe fn write_verb(pszname: PSTR, cchmax: u32, utype: u32, text: &str) {
    if pszname.is_null() || cchmax == 0 {
        return;
    }
    if utype & GCS_UNICODE != 0 {
        let w = pszname.0 as *mut u16;
        let mut n = 0usize;
        for c in text.encode_utf16().take(cchmax as usize - 1) {
            *w.add(n) = c;
            n += 1;
        }
        *w.add(n) = 0;
    } else {
        let mut n = 0usize;
        for b in text.bytes().take(cchmax as usize - 1) {
            *pszname.0.add(n) = b;
            n += 1;
        }
        *pszname.0.add(n) = 0;
    }
}

fn run_command(hwnd: HWND, command: &str, paths: &[String]) {
    for group in invocations(command, paths) {
        let line = expand_command(command, &group);
        let (exe, args) = split_cmd(&line);
        if exe.is_empty() {
            continue;
        }
        unsafe {
            let file = HSTRING::from(&exe);
            let params = HSTRING::from(&args);
            let _ = ShellExecuteW(Some(hwnd), w!("open"), &file, &params, PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

fn invocations(command: &str, paths: &[String]) -> Vec<Vec<String>> {
    let low = command.to_ascii_lowercase();
    let has_star = command.contains("%*");
    let has_one = command.contains("%1") || low.contains("%l") || low.contains("%v") || low.contains("%w");
    if has_star || !has_one || paths.len() <= 1 {
        vec![if paths.is_empty() { Vec::new() } else { paths.to_vec() }]
    } else {
        paths.iter().take(64).map(|p| vec![p.clone()]).collect()
    }
}

fn expand_command(template: &str, paths: &[String]) -> String {
    substitute(&expand_env(template), paths)
}

fn substitute(template: &str, paths: &[String]) -> String {
    let one = paths.first().map(String::as_str).unwrap_or("");
    let all = paths.iter().map(|p| quote_path(p)).collect::<Vec<_>>().join(" ");
    let mut s = template.replace("%*", &all);
    for token in ["%1", "%L", "%l", "%V", "%v", "%W", "%w"] {
        s = s.replace(token, one);
    }
    s
}

fn quote_path(p: &str) -> String {
    if p.contains(' ') || p.contains('\t') { format!("\"{p}\"") } else { p.to_string() }
}

fn split_cmd(line: &str) -> (String, String) {
    let c = line.trim();
    if let Some(rest) = c.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
    }
    match c.split_once(char::is_whitespace) {
        Some((exe, args)) => (exe.to_string(), args.trim().to_string()),
        None => (c.to_string(), String::new()),
    }
}

fn expand_env(s: &str) -> String {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = vec![0u16; 4096];
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), PWSTR(buf.as_mut_ptr()), buf.len() as u32) };
    if n == 0 || n as usize > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..n as usize - 1])
}

unsafe fn cocreate(guid: &GUID, class: &str, folder_pidl: *const ITEMIDLIST, data: Option<&IDataObject>) -> Option<IContextMenu> {
    let cm: IContextMenu = CoCreateInstance(guid, None, CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER).ok()?;
    if let Ok(init) = cm.cast::<IShellExtInit>() {
        let key = open_hkcr(class);
        let hk = key.unwrap_or_default();
        let _ = init.Initialize(Some(folder_pidl), data, Some(hk));
        if let Some(k) = key {
            _ = RegCloseKey(k);
        }
    }
    Some(cm)
}

fn bits_of(names: &[String]) -> u8 {
    names.iter().fold(0, |a, n| {
        a | match n.as_str() {
            "file" => FILE,
            "folder" => FOLDER,
            "background" => BACKGROUND,
            "drive" => DRIVE,
            _ => 0,
        }
    })
}

fn write_bits(map: &mut BTreeMap<String, Vec<String>>, id: &str, bits: u8) {
    if id.is_empty() || bits == 0 {
        map.remove(id);
        return;
    }
    let mut v = Vec::new();
    for (bit, name) in [(FILE, "file"), (FOLDER, "folder"), (BACKGROUND, "background"), (DRIVE, "drive")] {
        if bits & bit != 0 {
            v.push(name.to_string());
        }
    }
    map.insert(id.to_string(), v);
}

fn blocked(row: &Row, click: u8, off: &BTreeMap<String, Vec<String>>) -> bool {
    if row.builtin {
        return false;
    }
    let relevant = row.targets & click;
    relevant != 0 && bits_of(off.get(&row.id).map(Vec::as_slice).unwrap_or(&[])) & relevant != 0
}

fn split_classes(classes: &[String], click: u8, cat: &[Row], off: &BTreeMap<String, Vec<String>>) -> (Vec<String>, Vec<String>) {
    let mut clean = Vec::new();
    let mut dirty = Vec::new();
    for c in classes {
        if class_dirty(c, click, cat, off) {
            dirty.push(c.clone());
        } else {
            clean.push(c.clone());
        }
    }
    (clean, dirty)
}

fn class_dirty(class: &str, click: u8, cat: &[Row], off: &BTreeMap<String, Vec<String>>) -> bool {
    cat.iter().any(|r| r.has_class(class) && blocked(r, click, off))
}

fn manual_rows<'a>(dirty: &[String], clean: &[String], click: u8, cat: &'a [Row], off: &BTreeMap<String, Vec<String>>) -> Vec<&'a Row> {
    let mut out = Vec::new();
    for row in cat {
        if row.targets & click == 0 || blocked(row, click, off) {
            continue;
        }
        let on_dirty = dirty.iter().any(|c| row.has_class(c));
        let on_clean = clean.iter().any(|c| row.has_class(c));
        if on_dirty && !on_clean && !out.iter().any(|r: &&Row| r.id == row.id) {
            out.push(row);
        }
    }
    out
}

impl Row {
    fn has_class(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c.eq_ignore_ascii_case(class))
    }
}

fn target_blob(bits: u8) -> String {
    target_bits().into_iter().filter(|b| bits & b != 0).map(target_label).collect::<Vec<_>>().join(" ").to_lowercase()
}

struct Cache {
    rows: Vec<Row>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn catalogue() -> Vec<Row> {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = guard.as_ref() {
        return c.rows.clone();
    }
    let t0 = Instant::now();
    let rows = unsafe { scan_all() };
    log_phase("scan", t0);
    guard.replace(Cache { rows: rows.clone() });
    rows
}

unsafe fn scan_all() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut dlls: HashMap<String, (String, String)> = HashMap::new();
    let mut seen_clsid: HashMap<String, (String, String, String)> = HashMap::new();
    let approved = read_approved();
    for (root, prefix) in roots() {
        for &(rel, bits) in FIXED {
            scan_class(root, prefix, rel, bits, &mut rows, &mut dlls, &mut seen_clsid, &approved);
        }
        scan_extensions(root, prefix, &mut rows, &mut dlls, &mut seen_clsid, &approved);
    }
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    rows
}

fn roots() -> [(HKEY, &'static str); 5] {
    [
        (HKEY_CLASSES_ROOT, ""),
        (HKEY_CURRENT_USER, r"Software\Classes\"),
        (HKEY_LOCAL_MACHINE, r"Software\Classes\"),
        (HKEY_LOCAL_MACHINE, r"Software\Wow6432Node\Classes\"),
        (HKEY_CLASSES_ROOT, r"Wow6432Node\"),
    ]
}

unsafe fn scan_extensions(root: HKEY, prefix: &str, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String)>, clsids: &mut HashMap<String, (String, String, String)>, approved: &HashMap<String, String>) {
    let base = prefix.trim_end_matches('\\');
    let Some(key) = open_at(root, base) else { return };
    let mut i = 0u32;
    while i < 8000 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        if name.starts_with('.') {
            scan_class(root, prefix, &name, FILE, rows, dlls, clsids, approved);
            let full = join_reg(prefix, &name);
            if let Some(k) = open_at(root, &full) {
                let prog = key_sz(k, PCWSTR::null());
                let perceived = key_sz_named(k, "PerceivedType");
                _ = RegCloseKey(k);
                if progid_like(&prog) {
                    scan_class(root, prefix, &prog, FILE, rows, dlls, clsids, approved);
                }
                if !perceived.is_empty() && !perceived.contains(['\\', '/']) {
                    scan_class(root, prefix, &format!(r"SystemFileAssociations\{perceived}"), FILE, rows, dlls, clsids, approved);
                }
            }
            scan_class(root, prefix, &format!(r"SystemFileAssociations\{name}"), FILE, rows, dlls, clsids, approved);
        } else if name.eq_ignore_ascii_case("SystemFileAssociations") {
            let assoc = join_reg(prefix, &name);
            let Some(ak) = open_at(root, &assoc) else { continue };
            let mut j = 0u32;
            while j < 4000 {
                let Some(child) = enum_key(ak, j) else { break };
                j += 1;
                scan_class(root, prefix, &format!(r"SystemFileAssociations\{child}"), FILE, rows, dlls, clsids, approved);
            }
            _ = RegCloseKey(ak);
        }
    }
    _ = RegCloseKey(key);
}

fn progid_like(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.len() < 128 && !t.starts_with('.') && !t.contains(['\\', '/']) && parse_guid(t).is_none()
}

unsafe fn scan_class(root: HKEY, prefix: &str, rel: &str, targets: u8, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String)>, clsids: &mut HashMap<String, (String, String, String)>, approved: &HashMap<String, String>) {
    let full = join_reg(prefix, rel);
    scan_handlers(root, &full, rel, targets, rows, dlls, clsids, approved);
    scan_verbs(root, &full, rel, targets, rows, dlls, clsids);
}

unsafe fn scan_handlers(root: HKEY, full: &str, rel: &str, targets: u8, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String)>, clsids: &mut HashMap<String, (String, String, String)>, approved: &HashMap<String, String>) {
    let path = format!(r"{full}\shellex\ContextMenuHandlers");
    let Some(key) = open_at(root, &path) else { return };
    let mut i = 0u32;
    while i < 512 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        let sub = format!(r"{path}\{name}");
        let value = open_at(root, &sub).map(|k| {
            let v = key_sz(k, PCWSTR::null());
            unsafe { _ = RegCloseKey(k) };
            v
        }).unwrap_or_default();
        let guid = parse_guid(&value).or_else(|| parse_guid(&name));
        let Some(guid) = guid else { continue };
        let (title, dll, company) = clsid_meta(&guid, approved, dlls, clsids);
        let builtin = com_builtin(&name, &guid, &dll, &company);
        let mut friendly = if !title.is_empty() { title } else if parse_guid(&name).is_none() { name.clone() } else { String::new() };
        if friendly.is_empty() {
            friendly = guid_text(&guid);
        }
        upsert(rows, Row {
            id: format!("clsid:{}", guid_text(&guid)),
            name: friendly,
            targets,
            builtin,
            clsid: Some(guid),
            verb: String::new(),
            command: String::new(),
            rank: specificity(rel),
            extended: false,
            single: false,
            position: 0,
            classes: vec![rel.to_string()],
            anchor: rel.to_string(),
        });
    }
    _ = RegCloseKey(key);
}

unsafe fn scan_verbs(root: HKEY, full: &str, rel: &str, targets: u8, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String)>, clsids: &mut HashMap<String, (String, String, String)>) {
    let path = format!(r"{full}\shell");
    let Some(key) = open_at(root, &path) else { return };
    let mut i = 0u32;
    while i < 512 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        if matches!(name.to_ascii_lowercase().as_str(), "command" | "ddeexec" | "droptarget" | "shellex") {
            continue;
        }
        let sub = format!(r"{path}\{name}");
        let Some(vk) = open_at(root, &sub) else { continue };
        if value_exists(vk, "LegacyDisable") || value_exists(vk, "ProgrammaticAccessOnly") {
            _ = RegCloseKey(vk);
            continue;
        }
        let display = key_sz(vk, PCWSTR::null());
        let extended = value_exists(vk, "Extended");
        let position = match key_sz_named(vk, "Position").to_ascii_lowercase().as_str() {
            "top" => -1,
            "bottom" => 1,
            _ => 0,
        };
        let single = key_sz_named(vk, "MultiSelectModel").eq_ignore_ascii_case("single");
        let command = open_at(root, &format!(r"{sub}\command")).map(|ck| {
            let c = key_sz(ck, PCWSTR::null());
            unsafe { _ = RegCloseKey(ck) };
            c
        }).unwrap_or_default();
        let delegate = key_sz_named(vk, "DelegateExecute");
        _ = RegCloseKey(vk);
        let delegate_guid = parse_guid(&delegate);
        let delegate_dll = delegate_guid.as_ref().map(|g| clsid_meta(g, &HashMap::new(), dlls, clsids).1).unwrap_or_default();
        if command.is_empty() && delegate_guid.is_none() {
            continue;
        }
        let builtin = verb_builtin(&name, &command, &delegate_dll);
        let friendly = {
            let shown = if display.is_empty() { name.clone() } else { indirect(&display) };
            if shown.is_empty() { name.clone() } else { shown }
        };
        upsert(rows, Row {
            id: format!("verb:{}", name.to_ascii_lowercase()),
            name: friendly,
            targets,
            builtin,
            clsid: delegate_guid,
            verb: name,
            command: command.clone(),
            rank: specificity(rel),
            extended,
            single,
            position,
            classes: vec![rel.to_string()],
            anchor: rel.to_string(),
        });
    }
    _ = RegCloseKey(key);
}

fn upsert(rows: &mut Vec<Row>, incoming: Row) {
    if let Some(have) = rows.iter_mut().find(|r| r.id == incoming.id) {
        have.targets |= incoming.targets;
        have.builtin |= incoming.builtin;
        have.extended = have.extended && incoming.extended;
        have.single = have.single && incoming.single;
        if incoming.position < 0 || have.position < 0 {
            have.position = -1;
        } else if incoming.position > 0 || have.position > 0 {
            have.position = 1;
        }
        if incoming.rank >= have.rank && !incoming.command.is_empty() {
            have.command = incoming.command.clone();
            have.rank = incoming.rank;
            have.anchor = incoming.anchor.clone();
        }
        if have.clsid.is_none() {
            have.clsid = incoming.clsid;
        }
        if have.name.is_empty() || parse_guid(&have.name).is_some() {
            if !incoming.name.is_empty() {
                have.name = incoming.name;
            }
        }
        if have.verb.is_empty() {
            have.verb = incoming.verb;
        }
        for c in incoming.classes {
            if !have.classes.iter().any(|e| e.eq_ignore_ascii_case(&c)) {
                have.classes.push(c);
            }
        }
        return;
    }
    rows.push(incoming);
}

fn specificity(class: &str) -> u8 {
    let c = class.to_ascii_lowercase();
    if c.starts_with('.') || c.starts_with("systemfileassociations\\") {
        5
    } else if c == "drive" || c.ends_with("\\background") {
        4
    } else if c == "directory" || c == "folder" {
        3
    } else if c == "allfilesystemobjects" {
        2
    } else if c == "*" {
        1
    } else {
        4
    }
}

fn com_builtin(key_name: &str, guid: &GUID, dll: &str, company: &str) -> bool {
    if builtin_clsid(guid) || builtin_handler_key(key_name) {
        return true;
    }
    inbox_dll(dll, company)
}

fn verb_builtin(key: &str, command: &str, delegate_dll: &str) -> bool {
    if builtin_verb(key) {
        return true;
    }
    if inbox_dll(delegate_dll, "") {
        return true;
    }
    !command.is_empty() && command_inbox(command)
}

fn inbox_dll(dll: &str, company: &str) -> bool {
    if dll.is_empty() || !under_windows(dll) {
        return false;
    }
    if !company.is_empty() && !company.to_ascii_lowercase().contains("microsoft") {
        return false;
    }
    true
}

fn command_inbox(cmd: &str) -> bool {
    if foreign_path(&cmd.to_ascii_lowercase()) {
        return false;
    }
    let exe = split_cmd(cmd).0.to_ascii_lowercase();
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(&exe);
    under_windows(&exe)
        || matches!(file, "cmd.exe" | "powershell.exe" | "pwsh.exe" | "wt.exe" | "wscript.exe" | "cscript.exe" | "rundll32.exe" | "explorer.exe" | "cmd" | "powershell" | "wt")
}

fn foreign_path(low: &str) -> bool {
    let b = low.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i].is_ascii_alphabetic() && b[i + 1] == b':' && (b[i + 2] == b'\\' || b[i + 2] == b'/') {
            let rest = &low[i..];
            let end = rest.find(['"', ' ', '\t']).unwrap_or(rest.len());
            if !under_windows(&rest[..end]) {
                return true;
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn under_windows(path: &str) -> bool {
    let p = path.trim().trim_matches('"').replace('/', "\\").to_ascii_lowercase();
    let p = p.trim_start_matches("\\\\?\\").to_string();
    p.contains("\\windows\\system32\\")
        || p.contains("\\windows\\syswow64\\")
        || p.contains("\\windows\\systemapps\\")
        || p.contains("\\windows\\winsxs\\")
        || p.contains("\\windows\\systemresources\\")
        || p.contains("\\windows\\explorer.exe")
}

pub(crate) fn builtin_verb(name: &str) -> bool {
    let n = name.trim().trim_matches('&').to_ascii_lowercase();
    if n.starts_with("windows.") {
        return true;
    }
    matches!(
        n.as_str(),
        "open"
            | "opennew"
            | "openas"
            | "openwith"
            | "openwithprompt"
            | "opencontaining"
            | "opennewprocess"
            | "opennewwindow"
            | "explore"
            | "find"
            | "edit"
            | "print"
            | "printto"
            | "preview"
            | "runas"
            | "runasuser"
            | "cut"
            | "copy"
            | "paste"
            | "delete"
            | "link"
            | "rename"
            | "properties"
            | "pintohome"
            | "unpinfromhome"
            | "pintostartscreen"
            | "pintotaskbar"
            | "unpinfromtaskbar"
            | "cmd"
            | "powershell"
            | "pwsh"
            | "wsl"
            | "bash"
            | "sharing"
            | "share"
            | "previousversions"
            | "copyaspath"
            | "format"
            | "eject"
            | "disconnect"
            | "connect"
            | "manage"
            | "modernshare"
            | "updateencryptionsettings"
            | "encryptionmenu"
            | "windowsdefender"
            | "playto"
            | "casttodevice"
            | "giveaccessto"
            | "includeinlibrary"
            | "new"
    )
}

fn shell_owned(verb: &str) -> bool {
    matches!(verb.to_ascii_lowercase().as_str(), "open" | "opennew" | "explore" | "find" | "delete" | "properties" | "link" | "rename" | "cut" | "copy" | "paste")
}

fn builtin_handler_key(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    if parse_guid(&n).is_some_and(|g| builtin_clsid(&g)) {
        return true;
    }
    matches!(
        n.as_str(),
        "open with"
            | "openwith"
            | "send to"
            | "sendto"
            | "sharing"
            | "share"
            | "previous versions"
            | "previousversions"
            | "offline files"
            | "offlinefiles"
            | "compatibility"
            | "encryption"
            | "encrypt"
            | "new"
            | "work folders"
            | "workfolders"
            | "windows defender"
            | "windowsdefender"
            | "include in library"
            | "library location"
            | "play to"
            | "playto"
            | "cast to device"
            | "pin to start"
            | "pintostart"
            | "pin to taskbar"
            | "pintotaskbar"
            | "unpin from taskbar"
            | "give access to"
            | "restore previous versions"
            | "extract all"
            | "compressedfolder"
            | "briefcasemenu"
            | "modern sharing"
            | "modernsharing"
            | "efs"
            | "offline files menu"
    ) || builtin_verb(&n.replace(' ', ""))
}

fn builtin_clsid(g: &GUID) -> bool {
    const IDS: &[&str] = &[
        "7ba4c740-9e81-11cf-99d3-00aa004ae837", // Send to
        "09799afb-ad67-11d1-abcd-00c04fc30936", // Open with
        "d969a300-e7ff-11d0-a93b-00a0c90f2719", // New
        "f81e9010-6ea4-11ce-a7ff-00aa003ca9f6", // Sharing
        "596ab062-b4d2-4215-9f74-e9109b0a8153", // Previous versions
        "e88dcce0-b7b3-11d1-a9f0-00aa0060fa31", // Compressed folder
        "b8cdcb65-b1bf-4b42-9428-1dfdb7ee92af", // Extract all
        "a2a9545d-a0c2-42b4-9708-a0b2badd77c8", // Pin to Start
        "90aa3a4e-1cba-4233-b8bb-535773d48449", // Pin to taskbar
        "470c0ebd-5d73-4d58-9ced-e91e22e23282", // Pin to Start screen
        "3dad6c5d-2167-4cae-9914-f99e41c12cfa", // Include in library
        "09a47860-11b0-4da5-afa5-26d86198a780", // Windows Defender
        "4a7ded0a-ad25-11d0-98a8-0800361b1103", // Briefcase
        "e2bf9676-5f8f-435c-9bda-ad7b2c0c3c9a", // Modern share
        "474c98ee-cf3d-41f5-80e3-4aab0ab04301", // Offline files
        "a470f8cf-a1e8-4f65-8335-227475aa5c46", // Encryption
        "7ad84985-87b4-4a16-be58-8b72a5b390f7", // Play to / Cast
        "b41db860-64e4-11d2-9906-00004b3083ff", // Send to (older)
        "888dca60-fc0a-11cf-8f0f-00c04fd7d062", // Send mail
    ];
    let text = guid_text(g);
    IDS.contains(&text.as_str())
}

fn clsid_meta(guid: &GUID, approved: &HashMap<String, String>, dlls: &mut HashMap<String, (String, String)>, clsids: &mut HashMap<String, (String, String, String)>) -> (String, String, String) {
    let key = guid_text(guid);
    if let Some((name, dll, company)) = clsids.get(&key) {
        return (name.clone(), dll.clone(), company.clone());
    }
    let (mut name, dll) = unsafe { read_clsid(&key) };
    if name.is_empty() {
        if let Some(n) = approved.get(&key) {
            name = n.clone();
        }
    }
    let (desc, company) = if dll.is_empty() { (String::new(), String::new()) } else { file_version(&dll, dlls) };
    if name.is_empty() {
        name = desc.clone();
    }
    if name.is_empty() {
        name = dll.rsplit(['\\', '/']).next().unwrap_or("").to_string();
    }
    clsids.insert(key, (name.clone(), dll.clone(), company.clone()));
    (name, dll, company)
}

unsafe fn read_clsid(guid: &str) -> (String, String) {
    let braced = format!("{{{guid}}}");
    let paths = [
        format!(r"CLSID\{braced}"),
        format!(r"Software\Classes\CLSID\{braced}"),
        format!(r"Software\Classes\Wow6432Node\CLSID\{braced}"),
        format!(r"Software\Wow6432Node\Classes\CLSID\{braced}"),
        format!(r"Wow6432Node\CLSID\{braced}"),
    ];
    let roots = [HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_LOCAL_MACHINE, HKEY_CLASSES_ROOT];
    let mut name = String::new();
    let mut dll = String::new();
    for (root, path) in roots.iter().copied().zip(paths.iter()) {
        let Some(k) = open_at(root, path) else { continue };
        if name.is_empty() {
            let v = key_sz(k, PCWSTR::null());
            if parse_guid(&v).is_none() {
                name = v;
            }
        }
        if dll.is_empty() {
            if let Some(s) = open_at(root, &format!(r"{path}\InprocServer32")) {
                dll = key_sz(s, PCWSTR::null());
                _ = RegCloseKey(s);
            }
            if dll.is_empty() {
                if let Some(s) = open_at(root, &format!(r"{path}\LocalServer32")) {
                    dll = key_sz(s, PCWSTR::null());
                    _ = RegCloseKey(s);
                }
            }
        }
        _ = RegCloseKey(k);
        if !name.is_empty() && !dll.is_empty() {
            break;
        }
    }
    (name, dll)
}

unsafe fn read_approved() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let paths = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved"),
    ];
    for (root, path) in paths {
        let Some(k) = open_at(root, path) else { continue };
        let mut i = 0u32;
        while i < 4000 {
            let mut name = [0u16; 128];
            let mut nlen = name.len() as u32;
            let mut data = [0u16; 256];
            let mut dlen = (data.len() * 2) as u32;
            let mut kind = 0u32;
            let err = RegEnumValueW(k, i, Some(PWSTR(name.as_mut_ptr())), &mut nlen, None, Some(&mut kind), Some(data.as_mut_ptr() as _), Some(&mut dlen));
            if err != ERROR_SUCCESS {
                break;
            }
            i += 1;
            let id = String::from_utf16_lossy(&name[..nlen as usize]);
            let Some(g) = parse_guid(&id) else { continue };
            let chars = (dlen as usize / 2).saturating_sub(1).min(data.len());
            let label = String::from_utf16_lossy(&data[..chars]).trim_end_matches('\0').trim().to_string();
            if !label.is_empty() {
                out.entry(guid_text(&g)).or_insert(label);
            }
        }
        _ = RegCloseKey(k);
    }
    out
}

fn file_version(path: &str, cache: &mut HashMap<String, (String, String)>) -> (String, String) {
    let key = path.to_ascii_lowercase();
    if let Some(v) = cache.get(&key) {
        return v.clone();
    }
    let got = unsafe { read_version(path) };
    cache.insert(key, got.clone());
    got
}

unsafe fn read_version(path: &str) -> (String, String) {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle = 0u32;
    let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), &mut handle);
    if size == 0 || size > 8 * 1024 * 1024 {
        return (String::new(), String::new());
    }
    let mut block = vec![0u8; size as usize];
    if !GetFileVersionInfoW(PCWSTR(wide.as_ptr()), 0, size, block.as_mut_ptr() as _).as_bool() {
        return (String::new(), String::new());
    }
    let mut trans = std::ptr::null_mut();
    let mut tlen = 0u32;
    let query = |sub: &str| -> String {
        let w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
        let mut p = std::ptr::null_mut();
        let mut n = 0u32;
        if !VerQueryValueW(block.as_ptr() as _, PCWSTR(w.as_ptr()), &mut p, &mut n).as_bool() || p.is_null() || n < 2 {
            return String::new();
        }
        let words = (n as usize).min(512);
        let slice = std::slice::from_raw_parts(p as *const u16, words);
        let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
        String::from_utf16_lossy(&slice[..end]).trim().to_string()
    };
    let _ = query;
    if !VerQueryValueW(block.as_ptr() as _, w!("\\VarFileInfo\\Translation"), &mut trans, &mut tlen).as_bool() || trans.is_null() || tlen < 4 {
        return (String::new(), String::new());
    }
    let b = std::slice::from_raw_parts(trans as *const u8, 4);
    let lang = u16::from_le_bytes([b[0], b[1]]);
    let cp = u16::from_le_bytes([b[2], b[3]]);
    let stem = format!("\\StringFileInfo\\{lang:04x}{cp:04x}");
    (query(&format!("{stem}\\FileDescription")), query(&format!("{stem}\\CompanyName")))
}

fn indirect(s: &str) -> String {
    if !s.starts_with('@') {
        return s.to_string();
    }
    let src: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 512];
    let hr = unsafe { SHLoadIndirectString(PCWSTR(src.as_ptr()), PWSTR(buf.as_mut_ptr()), buf.len() as u32, std::ptr::null_mut()) };
    if hr.is_err() {
        return s.trim_start_matches('@').to_string();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let t = String::from_utf16_lossy(&buf[..n]);
    if t.is_empty() { s.to_string() } else { t }
}

pub(crate) fn classes_for(targets: u8, paths: &[String]) -> Vec<String> {
    let mut v = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !v.iter().any(|e: &String| e.eq_ignore_ascii_case(&s)) {
            v.push(s);
        }
    };
    if targets & FILE != 0 {
        for p in paths {
            if path_kind(p, true) != FILE {
                continue;
            }
            let Some(ext) = ext_of(p) else { continue };
            push(format!(".{ext}"));
            push(format!(r"SystemFileAssociations\.{ext}"));
            if let Some(prog) = unsafe { prog_of(&ext) } {
                push(prog);
            }
            if let Some(per) = unsafe { perceived_of(&ext) } {
                push(format!(r"SystemFileAssociations\{per}"));
            }
        }
        push("*".into());
        push("AllFilesystemObjects".into());
    }
    if targets & BACKGROUND != 0 {
        push(r"Directory\Background".into());
        push(r"Folder\Background".into());
    }
    if targets & DRIVE != 0 {
        push("Drive".into());
        push("Directory".into());
        push("Folder".into());
        push("AllFilesystemObjects".into());
    }
    if targets & FOLDER != 0 {
        push("Directory".into());
        push("Folder".into());
        push("AllFilesystemObjects".into());
    }
    v
}

fn ext_of(path: &str) -> Option<String> {
    let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 40 || ext.contains([' ', '\\', '/']) {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

unsafe fn prog_of(ext: &str) -> Option<String> {
    let k = open_hkcr(&format!(".{ext}"))?;
    let v = key_sz(k, PCWSTR::null());
    _ = RegCloseKey(k);
    progid_like(&v).then_some(v)
}

unsafe fn perceived_of(ext: &str) -> Option<String> {
    let k = open_hkcr(&format!(".{ext}"))?;
    let v = key_sz_named(k, "PerceivedType");
    _ = RegCloseKey(k);
    (!v.is_empty() && !v.contains(['\\', '/'])).then_some(v)
}

fn open_hkcr(rel: &str) -> Option<HKEY> {
    unsafe { open_at(HKEY_CLASSES_ROOT, rel) }
}

unsafe fn open_at(root: HKEY, path: &str) -> Option<HKEY> {
    if path.is_empty() {
        return None;
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    let err = RegOpenKeyExW(root, PCWSTR(wide.as_ptr()), None, KEY_READ, &mut key);
    (err == ERROR_SUCCESS && !key.is_invalid()).then_some(key)
}

fn join_reg(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else if prefix.ends_with('\\') {
        format!("{prefix}{rel}")
    } else {
        format!(r"{prefix}\{rel}")
    }
}

unsafe fn enum_key(key: HKEY, index: u32) -> Option<String> {
    let mut name = [0u16; 256];
    let mut n = name.len() as u32;
    let err = RegEnumKeyExW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None);
    if err != ERROR_SUCCESS || n == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&name[..n as usize]))
}

unsafe fn key_sz(key: HKEY, value: PCWSTR) -> String {
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let err = RegGetValueW(key, PCWSTR::null(), value, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n));
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').trim().to_string()
}

unsafe fn key_sz_named(key: HKEY, name: &str) -> String {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    key_sz(key, PCWSTR(wide.as_ptr()))
}

unsafe fn value_exists(key: HKEY, name: &str) -> bool {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut n = 0u32;
    RegGetValueW(key, PCWSTR::null(), PCWSTR(wide.as_ptr()), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_RT_REG_MULTI_SZ, None, None, Some(&mut n)) == ERROR_SUCCESS
}

fn parse_guid(s: &str) -> Option<GUID> {
    let t = s.trim().trim_start_matches('{').trim_end_matches('}');
    let hex: String = t.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u128::from_str_radix(&hex, 16).ok()?;
    let text = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    // from_u128 matches the textual GUID, so the dashed form must round-trip.
    let g = GUID::from_u128(n);
    (guid_text(&g) == text.to_ascii_lowercase()).then_some(g)
}

fn guid_text(g: &GUID) -> String {
    format!("{g:?}").trim_start_matches('{').trim_end_matches('}').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(bits: u8) -> Vec<String> {
        let mut m = BTreeMap::new();
        write_bits(&mut m, "x", bits);
        m.remove("x").unwrap_or_default()
    }

    #[test]
    fn kinds_follow_the_path() {
        assert_eq!(path_kind(r"C:\", false), DRIVE);
        assert_eq!(path_kind(r"C:", false), DRIVE);
        assert_eq!(path_kind(r"D:\Work", false), FOLDER);
        assert_eq!(path_kind(r"D:\Work\a.txt", true), FILE);
        assert_eq!(path_kind(r"\\server\share", false), FOLDER);
        assert_eq!(click_targets(true, &[(r"C:\Windows".into(), false)]), BACKGROUND);
        assert_eq!(click_targets(false, &[(r"C:\a.txt".into(), true), (r"C:\b".into(), false)]), FILE | FOLDER);
        assert_eq!(click_targets(false, &[(r"E:\".into(), false)]), DRIVE);
    }

    #[test]
    fn new_handlers_start_enabled_and_reset_clears_them() {
        let mut m = BTreeMap::new();
        assert_eq!(bits_of(m.get("clsid:abc").map(Vec::as_slice).unwrap_or(&[])), 0);
        assert_eq!(tick_of(0, FILE | FOLDER), Tick::On);
        toggle_master(&mut m, "clsid:abc", FILE | FOLDER);
        assert_eq!(tick_of(bits_of(&m["clsid:abc"]), FILE | FOLDER), Tick::Off);
        toggle_target(&mut m, "clsid:abc", FILE, FILE | FOLDER);
        assert_eq!(tick_of(bits_of(&m["clsid:abc"]), FILE | FOLDER), Tick::Mixed);
        assert!(target_enabled(bits_of(&m["clsid:abc"]), FILE));
        assert!(!target_enabled(bits_of(&m["clsid:abc"]), FOLDER));
        toggle_master(&mut m, "clsid:abc", FILE | FOLDER);
        assert!(!m.contains_key("clsid:abc"));
        toggle_master(&mut m, "verb:7-zip", BACKGROUND);
        clear_off(&mut m);
        assert!(m.is_empty());
        assert_eq!(names(FILE | DRIVE), vec!["drive".to_string(), "file".to_string()]);
    }

    #[test]
    fn builtins_cannot_be_blocked_and_a_clean_class_stays_with_the_shell() {
        let mut off = BTreeMap::new();
        off.insert("clsid:third".into(), vec!["file".into()]);
        off.insert("clsid:sendto".into(), vec!["file".into(), "folder".into()]);
        let third = Row {
            id: "clsid:third".into(),
            name: "Third".into(),
            targets: FILE | FOLDER,
            builtin: false,
            clsid: None,
            verb: String::new(),
            command: String::new(),
            rank: 2,
            extended: false,
            single: false,
            position: 0,
            classes: vec!["AllFilesystemObjects".into()],
            anchor: "AllFilesystemObjects".into(),
        };
        let send = Row {
            id: "clsid:sendto".into(),
            name: "Send to".into(),
            targets: FILE | FOLDER,
            builtin: true,
            clsid: None,
            verb: String::new(),
            command: String::new(),
            rank: 2,
            extended: false,
            single: false,
            position: 0,
            classes: vec!["AllFilesystemObjects".into()],
            anchor: "AllFilesystemObjects".into(),
        };
        let cat = vec![third, send];
        assert!(blocked(&cat[0], FILE, &off));
        assert!(!blocked(&cat[0], FOLDER, &off));
        assert!(!blocked(&cat[1], FILE, &off), "a built-in handler stays even if the map names it");
        assert!(class_dirty("AllFilesystemObjects", FILE, &cat, &off));
        assert!(!class_dirty("AllFilesystemObjects", FOLDER, &cat, &off));
        assert!(!class_dirty("*", FILE, &cat, &off));
        let (clean, dirty) = split_classes(&["*".into(), "AllFilesystemObjects".into()], FILE, &cat, &off);
        assert_eq!(clean, vec!["*".to_string()]);
        assert_eq!(dirty, vec!["AllFilesystemObjects".to_string()]);
        let manual = manual_rows(&dirty, &clean, FILE, &cat, &off);
        assert!(manual.iter().any(|r| r.id == "clsid:sendto"));
        assert!(manual.iter().all(|r| r.id != "clsid:third"));
    }

    #[test]
    fn inbox_verbs_and_windows_binaries_are_not_third_party() {
        assert!(builtin_verb("Open"));
        assert!(builtin_verb("windows.copyaspath"));
        assert!(builtin_verb("cmd"));
        assert!(!builtin_verb("7-Zip"));
        assert!(!builtin_verb("OpenWithNotepad"));
        assert!(under_windows(r"C:\Windows\System32\shell32.dll"));
        assert!(under_windows(r"\\?\C:\Windows\SysWOW64\zipfldr.dll"));
        assert!(!under_windows(r"C:\Program Files\7-Zip\7-zip.dll"));
        assert!(command_inbox(r#"cmd.exe /s /k pushd "%V""#));
        assert!(!command_inbox(r#""C:\Program Files\7-Zip\7zFM.exe" "%1""#));
        assert!(shell_owned("properties"));
        assert!(!shell_owned("SevenZip"));
    }

    #[test]
    fn command_tokens_expand_without_touching_the_registry() {
        assert_eq!(substitute(r#""C:\App\a.exe" "%1""#, &[r"C:\a b.txt".into()]), r#""C:\App\a.exe" "C:\a b.txt""#);
        assert_eq!(substitute(r"tool.exe %*", &[r"C:\a.txt".into(), r"C:\b c.txt".into()]), r#"tool.exe C:\a.txt "C:\b c.txt""#);
        assert_eq!(split_cmd(r#""C:\Program Files\7-Zip\7zFM.exe" "%1""#), (r"C:\Program Files\7-Zip\7zFM.exe".into(), r#""%1""#.into()));
        let multi = invocations(r#""C:\App\a.exe" "%1""#, &[r"C:\a.txt".into(), r"C:\b.txt".into()]);
        assert_eq!(multi.len(), 2);
        let once = invocations(r"tool.exe %*", &[r"C:\a.txt".into(), r"C:\b.txt".into()]);
        assert_eq!(once.len(), 1);
    }

    #[test]
    fn file_classes_include_the_extension_and_the_shared_keys() {
        let c = classes_for(FILE, &[r"C:\work\note.txt".into()]);
        assert!(c.iter().any(|s| s == ".txt"));
        assert!(c.iter().any(|s| s == "*"));
        assert!(c.iter().any(|s| s.eq_ignore_ascii_case("AllFilesystemObjects")));
        let d = classes_for(DRIVE | BACKGROUND, &[r"C:\".into()]);
        assert!(d.iter().any(|s| s == "Drive"));
        assert!(d.iter().any(|s| s == r"Directory\Background"));
        assert!(d.iter().all(|s| s != "*"));
    }

    #[test]
    fn guid_text_round_trips_the_send_to_clsid() {
        let g = parse_guid("{7BA4C740-9E81-11CF-99D3-00AA004AE837}").unwrap();
        assert_eq!(guid_text(&g), "7ba4c740-9e81-11cf-99d3-00aa004ae837");
        assert!(builtin_clsid(&g));
        assert!(parse_guid("not-a-guid").is_none());
    }
}
