use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;
use crate::settings;

// ---------- favourites: ☆+ in each pane header; Ctrl+B opens the same list ----------

pub(crate) unsafe fn repaint_fav_buttons() {
    for h in app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()) {
        if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
            _ = InvalidateRect(Some(b), None, false);
        }
    }
}

pub(crate) unsafe fn set_favs(paths: &[String], add: bool) {
    let items: Vec<(String, bool)> = paths.iter().cloned().map(|p| (p, false)).collect();
    set_entries(&items, add);
}

/// Add or remove folders and files on the favourites tree (the bottom of the left panel).
/// Files are marked so a double-click opens them. The folder tree is left alone.
pub(crate) unsafe fn set_entries(items: &[(String, bool)], add: bool) {
    if items.is_empty() {
        return;
    }
    app(|a| {
        a.s.ensure_fav_tree();
        for (path, file) in items {
            if add {
                add_item(&mut a.s.tree, path, *file);
            } else {
                remove_path(&mut a.s.tree, path);
            }
        }
        a.s.sync_favs();
        a.fav_hot = None;
    });
    favs_changed();
}

/// The tree changed. Every window, the Ctrl+B list, and the settings page read it.
pub(crate) unsafe fn favs_changed() {
    publish_shared();
    let (pop, dpi, n) = app(|a| (a.fav_popup, a.dpi, visible_rows(&a.s.tree).len()));
    if !pop.0.is_null() && IsWindow(Some(pop)).as_bool() {
        let g = fav_geo(n.max(1), dpi);
        _ = SetWindowPos(pop, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        _ = InvalidateRect(Some(pop), None, false);
    }
    settings::refresh();
    refresh_fav_manager();
    across_windows(|| {
        repaint_fav_buttons();
        clamp_side_scroll();
        let h = app(|a| a.side_hwnd);
        if !h.0.is_null() {
            _ = InvalidateRect(Some(h), None, false);
        }
    });
    note_save();
}

/// The settings editor and the favourites window both write through here.
pub(crate) unsafe fn commit_favs() {
    favs_changed();
    save_session();
}

pub(crate) unsafe fn open_fav_target(path: &str, file: bool) {
    if path.is_empty() {
        return;
    }
    if file {
        if shell_open(path) {
            return;
        }
        if let Some(parent) = parent_of(path) {
            let i = active_pane();
            navigate(i, &parent);
            focus_view(i);
        } else {
            _ = MessageBeep(MB_ICONWARNING);
        }
        return;
    }
    let i = active_pane();
    if !navigate(i, path) {
        _ = MessageBeep(MB_ICONWARNING);
    }
    focus_view(i);
}

fn shell_open(path: &str) -> bool {
    unsafe {
        // A value above 32 is success. Smaller values are the old error codes.
        let h = ShellExecuteW(None, w!("open"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        h.0 as isize > 32
    }
}

/// Parent folder of a file path. `C:\a\b.txt` → `C:\a`, `C:\b.txt` → `C:\`.
pub(crate) fn parent_of(path: &str) -> Option<String> {
    let t = path.trim_end_matches(['\\', '/']);
    let i = t.rfind(['\\', '/'])?;
    let parent = &t[..i];
    if parent.is_empty() {
        return None;
    }
    if parent.ends_with(':') {
        Some(format!("{parent}\\"))
    } else {
        Some(parent.to_string())
    }
}

pub(crate) unsafe fn toggle_fav(i: usize) {
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    set_favs(std::slice::from_ref(&path), !app(|a| a.s.is_fav(&path)));
}

/// Every path already a favourite → the menu removes them; otherwise it adds the rest.
pub(crate) fn fav_removes(paths: &[&str], is_fav: impl Fn(&str) -> bool) -> bool {
    !paths.is_empty() && paths.iter().all(|p| is_fav(p))
}

pub(crate) fn fav_menu_label(paths: &[String]) -> &'static str {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    if fav_removes(&refs, |p| app(|a| a.s.is_fav(p))) {
        tr("Remove from favourites", "즐겨찾기에서 빼기", "お気に入りから削除")
    } else {
        tr("Add to favourites", "즐겨찾기에 추가", "お気に入りに追加")
    }
}

/// Menu index the favourites command sits above: the properties item, or the last item.
pub(crate) fn fav_anchor(properties: Option<u32>, count: i32) -> u32 {
    if count <= 0 {
        0
    } else {
        properties.filter(|&i| (i as i32) < count).unwrap_or(count as u32 - 1)
    }
}

pub(crate) unsafe fn cmd_verb(cm: &IContextMenu, offset: usize) -> String {
    let mut buf = [0u16; 80];
    if cm.GetCommandString(offset, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_err() {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Index of the "properties" verb. Shell command ids are `id_first + offset`; separators and submenus have no id.
pub(crate) unsafe fn fav_anchor_pos(menu: HMENU, cm: &IContextMenu, id_first: u32, id_last: u32, start: u32) -> u32 {
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return 0;
    }
    let mut properties = None;
    for i in start.min(n as u32)..n as u32 {
        if menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == u32::MAX || id < id_first || id >= id_last {
            continue;
        }
        if cmd_verb(cm, (id - id_first) as usize).trim().eq_ignore_ascii_case("properties") {
            properties = Some(i);
            break;
        }
    }
    fav_anchor(properties, n)
}

/// Separators often have a zero id, so GetMenuItemID cannot tell them from a command. MFT_SEPARATOR can.
pub(crate) unsafe fn menu_sep(menu: HMENU, pos: u32) -> bool {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE,
        ..Default::default()
    };
    GetMenuItemInfoW(menu, pos, true, &mut info).is_ok() && info.fType.0 & MFT_SEPARATOR.0 != 0
}

pub(crate) unsafe fn menu_index(menu: HMENU, id: usize) -> Option<u32> {
    let n = GetMenuItemCount(Some(menu)).max(0);
    (0..n).find(|&i| GetMenuItemID(menu, i) as usize == id).map(|i| i as u32)
}

/// Exactly one separator above the favourites command, and none between it and Properties.
/// Run after the shell menu is filled (and again on WM_INITMENUPOPUP, once merged items exist).
pub(crate) unsafe fn tidy_fav_item(menu: HMENU, id: usize) {
    if id == usize::MAX {
        return;
    }
    loop {
        let Some(fav) = menu_index(menu, id) else { return };
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        if fav + 1 >= n || !menu_sep(menu, fav + 1) {
            break;
        }
        if DeleteMenu(menu, fav + 1, MF_BYPOSITION).is_err() {
            break;
        }
    }
    loop {
        let Some(fav) = menu_index(menu, id) else { return };
        if fav < 2 || !menu_sep(menu, fav - 1) || !menu_sep(menu, fav - 2) {
            break;
        }
        if DeleteMenu(menu, fav - 1, MF_BYPOSITION).is_err() {
            break;
        }
    }
    let Some(fav) = menu_index(menu, id) else { return };
    if fav > 0 && !menu_sep(menu, fav - 1) {
        _ = InsertMenuW(menu, fav, MF_SEPARATOR | MF_BYPOSITION, 0, PCWSTR::null());
    }
}

/// Favourites command at `at` (directly above Properties, or the last item). The separator is decided afterwards.
pub(crate) unsafe fn insert_fav(menu: HMENU, at: u32, id: usize, label: &str) {
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let label = HSTRING::from(label);
    if n == 0 {
        _ = AppendMenuW(menu, MF_STRING, id, &label);
        return;
    }
    _ = InsertMenuW(menu, at.min(n - 1), MF_STRING | MF_BYPOSITION, id, &label);
    tidy_fav_item(menu, id);
}

pub(crate) fn wnd_class(h: HWND) -> String {
    let mut c = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut c) } as usize;
    String::from_utf16_lossy(&c[..n])
}

// The real Explorer menu is built by the shell. These are the windows that show it; a header
// or an in-place rename box is left alone.
pub(crate) fn ctx_surface(h: HWND) -> bool {
    matches!(wnd_class(h).as_str(), "SHELLDLL_DefView" | "DirectUIHWND" | "SysListView32")
}

pub(crate) const CTX_SUB: usize = 4;
pub(crate) const ID_CTX_FAV: u32 = 0x8000; // outside the id range handed to IContextMenu::QueryContextMenu
pub(crate) const ID_CTX_PROPS: u32 = 0x8001; // folder Properties, added when the background menu has none (Windows 10)

pub(crate) struct MenuBridge {
    pub(crate) cm3: Option<IContextMenu3>,
    pub(crate) cm2: Option<IContextMenu2>,
}

thread_local! {
    static MENU_BRIDGE: RefCell<Option<MenuBridge>> = const { RefCell::new(None) };
    static CTX_MENU: Cell<isize> = const { Cell::new(0) }; // HMENU we inserted into; submenus are left alone
}

pub(crate) static CTX_OPEN: AtomicBool = AtomicBool::new(false);

pub(crate) struct ClearBridge;
impl Drop for ClearBridge {
    fn drop(&mut self) {
        MENU_BRIDGE.with(|b| *b.borrow_mut() = None);
        CTX_MENU.with(|m| m.set(0));
        CTX_OPEN.store(false, Ordering::Release);
    }
}

pub(crate) unsafe fn hook_ctx_hwnd(h: HWND, i: usize) {
    if !h.0.is_null() && ctx_surface(h) {
        _ = SetWindowSubclass(h, Some(ctx_proc), CTX_SUB, i);
    }
}

pub(crate) unsafe fn hook_ctx(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let Ok(view) = sv.GetWindow() else { return };
    hook_ctx_hwnd(view, i);
    let mut after = None;
    loop {
        let Ok(c) = FindWindowExW(Some(view), after, PCWSTR::null(), PCWSTR::null()) else { break };
        if c.0.is_null() {
            break;
        }
        hook_ctx_hwnd(c, i);
        let mut inner = None;
        loop {
            let Ok(g) = FindWindowExW(Some(c), inner, PCWSTR::null(), PCWSTR::null()) else { break };
            if g.0.is_null() {
                break;
            }
            hook_ctx_hwnd(g, i);
            inner = Some(g);
        }
        after = Some(c);
    }
}

#[repr(C)]
pub(crate) struct LvHit {
    pub(crate) pt: POINT,
    pub(crate) flags: u32,
    pub(crate) item: i32,
    pub(crate) sub: i32,
    pub(crate) group: i32,
}

// Right-click on a row the view hasn't selected yet: select it, unless it is already part of
// a multi-selection (Explorer keeps that selection).
pub(crate) unsafe fn select_clicked(i: usize, screen: POINT) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let lv = view_list(&fv);
    if lv.0.is_null() {
        return;
    }
    let mut pt = screen;
    if !ScreenToClient(lv, &mut pt).as_bool() {
        return;
    }
    let mut hit = LvHit { pt, flags: 0, item: -1, sub: 0, group: 0 };
    let idx = SendMessageW(lv, 0x1012, None, Some(LPARAM(&mut hit as *mut _ as isize))).0 as i32; // LVM_HITTEST
    if idx < 0 {
        return;
    }
    let Ok(pidl) = fv.Item(idx) else { return };
    let selected = fv.GetSelectionState(pidl).unwrap_or(0) & SVSI_SELECT.0 as u32 != 0;
    CoTaskMemFree(Some(pidl as _));
    if !selected {
        _ = fv.SelectItem(idx, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0) as u32);
    }
}

pub(crate) unsafe fn selection_pidls(i: usize) -> Vec<*mut ITEMIDLIST> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(en) = fv.Items::<IEnumIDList>(SVGIO_SELECTION) else { return Vec::new() };
    let mut out = Vec::new();
    loop {
        let mut slot = [std::ptr::null_mut()];
        let mut n = 0u32;
        let hr = en.Next(&mut slot, Some(&mut n));
        if !hr.is_ok() || n == 0 || slot[0].is_null() || out.len() == 2000 {
            break;
        }
        out.push(slot[0]);
    }
    out
}

/// Selected folders and files. The bool is true for a file.
pub(crate) unsafe fn selected_items(i: usize) -> Vec<(String, bool)> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(arr) = fv.GetSelection(false) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(500);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        let file = !it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
        if let Ok(p) = it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
            let p = pwstr(p);
            if !p.is_empty() {
                out.push((p, file));
            }
        }
    }
    out
}

pub(crate) unsafe fn forward_menu(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    MENU_BRIDGE.with(|b| {
        let b = b.borrow();
        let b = b.as_ref()?;
        unsafe {
            if let Some(cm3) = &b.cm3 {
                let mut lr = LRESULT(0);
                if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut lr)).is_ok() {
                    return Some(lr);
                }
            }
            if let Some(cm2) = &b.cm2 {
                if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                    return Some(LRESULT(0));
                }
            }
        }
        None
    })
}

// Shell menu for the pane. Add/Remove favourites sits directly above Properties when the selection has a folder.
// Returns false when the shell menu could not be built, so the view shows its own.
pub(crate) unsafe fn show_ctx(i: usize, pt: POINT) -> bool {
    if CTX_OPEN.swap(true, Ordering::AcqRel) {
        return true;
    }
    let _open = ClearBridge;
    let Some(sv) = view_of::<IShellView>(i) else { return false };
    let owner = sv.GetWindow().unwrap_or_else(|_| app(|a| a.main));
    if owner.0.is_null() {
        return false;
    }
    select_clicked(i, pt);
    let items = selected_items(i);
    let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
    let pidls = selection_pidls(i);
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let background = refs.is_empty();
    let cm = if background {
        sv.GetItemObject::<IContextMenu>(SVGIO_BACKGROUND).ok()
    } else if let Some(folder) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellFolder>().ok()) {
        folder.GetUIObjectOf::<IContextMenu>(owner, &refs, None).ok().or_else(|| sv.GetItemObject::<IContextMenu>(SVGIO_SELECTION).ok())
    } else {
        sv.GetItemObject::<IContextMenu>(SVGIO_SELECTION).ok()
    };
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    let Some(cm) = cm else { return false };
    let props = if background { app(|a| a.s.paths.get(i).cloned().unwrap_or_default()) } else { String::new() };
    popup_ctx(owner, cm, &paths, &items, background, &props, pt)
}

/// Explorer menu for files that are not the pane's current selection (search results, by full path).
/// The favourites item is inserted the same way as a click in the file list.
pub(crate) unsafe fn show_paths_ctx(i: usize, items: &[(String, bool)], pt: POINT) -> bool {
    if items.is_empty() {
        return false;
    }
    if CTX_OPEN.swap(true, Ordering::AcqRel) {
        return true;
    }
    let _open = ClearBridge;
    let Some(sv) = view_of::<IShellView>(i) else { return false };
    let owner = sv.GetWindow().unwrap_or_else(|_| app(|a| a.main));
    if owner.0.is_null() {
        return false;
    }
    let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
    let Some(cm) = menu_for_paths(&paths) else { return false };
    popup_ctx(owner, cm, &paths, items, false, "", pt)
}

unsafe fn menu_for_paths(paths: &[String]) -> Option<IContextMenu> {
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    if pidls.is_empty() {
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let menu = SHCreateShellItemArrayFromIDLists(&refs).ok().and_then(|arr| arr.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject).ok());
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    menu
}

unsafe fn popup_ctx(owner: HWND, cm: IContextMenu, paths: &[String], items: &[(String, bool)], background: bool, props_folder: &str, pt: POINT) -> bool {
    let Ok(menu) = CreatePopupMenu() else { return false };
    let mut flags = CMF_EXPLORE | CMF_CANRENAME;
    if !background {
        flags |= CMF_ITEMMENU;
    }
    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
        flags |= CMF_EXTENDEDVERBS;
    }
    let hr = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags);
    if !hr.is_ok() {
        _ = DestroyMenu(menu);
        return false;
    }
    // The background menu of some views (Windows 10) comes without Properties. Add the folder's own.
    let mut add_props = false;
    if background {
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        let has = (0..n).any(|i| {
            if menu_sep(menu, i) {
                return false;
            }
            let id = GetMenuItemID(menu, i as i32);
            id != u32::MAX && id >= 1 && id < ID_CTX_FAV && cmd_verb(&cm, (id - 1) as usize).trim().eq_ignore_ascii_case("properties")
        });
        if !has && n > 0 {
            if !menu_sep(menu, n - 1) {
                _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            }
            _ = AppendMenuW(menu, MF_STRING, ID_CTX_PROPS as usize, &HSTRING::from(tr("Properties", "속성", "プロパティ")));
            add_props = true;
        }
    }
    if !paths.is_empty() {
        let at = fav_anchor_pos(menu, &cm, 1, ID_CTX_FAV, 0);
        insert_fav(menu, at, ID_CTX_FAV as usize, fav_menu_label(&paths));
    }
    MENU_BRIDGE.with(|b| {
        *b.borrow_mut() = Some(MenuBridge { cm3: cm.cast().ok(), cm2: cm.cast().ok() });
    });
    CTX_MENU.with(|m| m.set(menu.0 as isize));
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, owner, None).0;
    MENU_BRIDGE.with(|b| *b.borrow_mut() = None);
    if cmd == ID_CTX_FAV as i32 {
        let remove = app(|a| paths.iter().all(|p| a.s.is_fav(p)));
        set_entries(items, !remove);
    } else if add_props && cmd == ID_CTX_PROPS as i32 {
        if !props_folder.is_empty() {
            _ = SHObjectProperties(Some(owner), SHOP_FILEPATH, &HSTRING::from(props_folder), None);
        }
    } else if cmd > 0 {
        let mut ici = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: owner,
            lpVerb: PCSTR((cmd as u32 - 1) as usize as *const u8),
            nShow: SW_SHOWNORMAL.0 as i32,
            ..Default::default()
        };
        if GetKeyState(VK_SHIFT.0 as i32) < 0 {
            ici.fMask |= CMIC_MASK_SHIFT_DOWN;
        }
        if GetKeyState(VK_CONTROL.0 as i32) < 0 {
            ici.fMask |= CMIC_MASK_CONTROL_DOWN;
        }
        _ = cm.InvokeCommand(&ici);
    }
    _ = DestroyMenu(menu);
    true
}

pub(crate) unsafe extern "system" fn ctx_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, data: usize) -> LRESULT {
    if msg == WM_INITMENUPOPUP {
        // The shell may insert its own separator while the menu opens. Collapse after that.
        let lr = forward_menu(msg, wp, lp);
        let popup = HMENU(wp.0 as _);
        if CTX_MENU.with(|m| m.get() != 0 && m.get() == popup.0 as isize) {
            tidy_fav_item(popup, ID_CTX_FAV as usize);
        }
        if let Some(lr) = lr {
            return lr;
        }
    } else if matches!(msg, WM_UNINITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        if let Some(lr) = forward_menu(msg, wp, lp) {
            return lr;
        }
    }
    if msg == WM_PARENTNOTIFY && (wp.0 & 0xFFFF) == WM_CREATE as usize {
        hook_ctx_hwnd(HWND(lp.0 as _), data);
    }
    if msg == WM_CONTEXTMENU {
        let pt = if lp.0 == -1 {
            let mut rc = RECT::default();
            let _ = GetWindowRect(h, &mut rc);
            POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 }
        } else {
            POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 }
        };
        let under = if lp.0 == -1 { h } else { WindowFromPoint(pt) };
        if ctx_surface(under) && show_ctx(data, pt) {
            return LRESULT(0);
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// Sidebar menu: the tree builds the shell menu, then asks us for a replacement. Ours forwards
// every shell command and puts the favourites item directly above Properties.
#[implement(IContextMenu, IContextMenu2, IContextMenu3)]
pub(crate) struct FavCtx {
    pub(crate) inner: IContextMenu,
    pub(crate) path: String,
    pub(crate) cmd: Cell<u32>,
    pub(crate) item: Cell<usize>, // menu id of the favourites command
    pub(crate) popup: Cell<isize>, // HMENU that command was inserted into
}

impl IContextMenu_Impl for FavCtx_Impl {
    fn QueryContextMenu(&self, hmenu: HMENU, indexmenu: u32, idcmdfirst: u32, idcmdlast: u32, uflags: u32) -> HRESULT {
        unsafe {
            let hr = self.inner.QueryContextMenu(hmenu, indexmenu, idcmdfirst, idcmdlast, uflags);
            if !hr.is_ok() || uflags & CMF_DEFAULTONLY != 0 {
                return hr;
            }
            let used = (hr.0 as u32) & 0xFFFF;
            // Offset 0 looks like a null verb, so an empty shell menu gets the next slot.
            let offset = if used == 0 { 1 } else { used };
            if idcmdfirst.saturating_add(offset) >= idcmdlast {
                return hr;
            }
            self.cmd.set(offset);
            let item = (idcmdfirst + offset) as usize;
            self.item.set(item);
            self.popup.set(hmenu.0 as isize);
            let at = fav_anchor_pos(hmenu, &self.inner, idcmdfirst, idcmdlast.saturating_add(1), indexmenu);
            insert_fav(hmenu, at, item, fav_menu_label(std::slice::from_ref(&self.path)));
            HRESULT((offset + 1) as i32)
        }
    }
    fn InvokeCommand(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        unsafe {
            if !pici.is_null() && verb_offset(pici) == Some(self.cmd.get()) {
                let remove = app(|a| a.s.is_fav(&self.path));
                set_favs(std::slice::from_ref(&self.path), !remove);
                return Ok(());
            }
            self.inner.InvokeCommand(pici)
        }
    }
    fn GetCommandString(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        if idcmd == self.cmd.get() as usize {
            // No verb or help text for our item. Leave an empty string and say so, rather than Ok with an untouched buffer.
            if !pszname.is_null() && cchmax > 0 {
                unsafe {
                    if utype & GCS_UNICODE != 0 {
                        *(pszname.0 as *mut u16) = 0;
                    } else {
                        *pszname.0 = 0;
                    }
                }
            }
            return Err(E_NOTIMPL.into());
        }
        unsafe { self.inner.GetCommandString(idcmd, utype, Some(preserved), pszname, cchmax) }
    }
}

impl IContextMenu2_Impl for FavCtx_Impl {
    fn HandleMenuMsg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<()> {
        unsafe {
            let result = if let Ok(m) = self.inner.cast::<IContextMenu2>() { m.HandleMenuMsg(umsg, wparam, lparam) } else { Ok(()) };
            if umsg == WM_INITMENUPOPUP {
                tidy_fav_popup(self.popup.get(), self.item.get(), HMENU(wparam.0 as _));
            }
            result
        }
    }
}

impl IContextMenu3_Impl for FavCtx_Impl {
    fn HandleMenuMsg2(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        unsafe {
            let result = if let Ok(m) = self.inner.cast::<IContextMenu3>() {
                m.HandleMenuMsg2(umsg, wparam, lparam, Some(plresult))
            } else if let Ok(m) = self.inner.cast::<IContextMenu2>() {
                m.HandleMenuMsg(umsg, wparam, lparam)
            } else {
                Ok(())
            };
            if umsg == WM_INITMENUPOPUP {
                tidy_fav_popup(self.popup.get(), self.item.get(), HMENU(wparam.0 as _));
            }
            result
        }
    }
}

pub(crate) unsafe fn tidy_fav_popup(popup: isize, item: usize, menu: HMENU) {
    if popup != 0 && popup == menu.0 as isize {
        tidy_fav_item(menu, item);
    }
}

pub(crate) fn verb_offset(ici: *const CMINVOKECOMMANDINFO) -> Option<u32> {
    unsafe {
        let v = (*ici).lpVerb.0 as usize;
        if v > 0 && v < 0x10000 {
            return Some(v as u32);
        }
        if (*ici).cbSize >= size_of::<CMINVOKECOMMANDINFOEX>() as u32 {
            let w = (*(ici as *const CMINVOKECOMMANDINFOEX)).lpVerbW.0 as usize;
            if w > 0 && w < 0x10000 {
                return Some(w as u32);
            }
        }
        None
    }
}

pub(crate) fn shell_folder(it: &IShellItem) -> bool {
    unsafe { it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0) }
}

pub(crate) fn shell_parse(it: &IShellItem) -> String {
    unsafe { it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).map(|p| pwstr(p)).unwrap_or_default() }
}

pub(crate) struct FavGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) rows: Vec<(RECT, RECT)>, // (row, remove ✕)
}

pub(crate) fn fav_geo(n: usize, dpi: i32) -> FavGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, rh, w) = (s(6), s(40), s(380));
    let rows = (0..n as i32)
        .map(|k| {
            let row = RECT { left: pad, top: pad + k * rh, right: w - pad, bottom: pad + (k + 1) * rh };
            (row, RECT { left: row.right - s(32), ..row })
        })
        .collect();
    FavGeo { w, h: 2 * pad + n.max(1) as i32 * rh, rows }
}

fn fav_rows() -> Vec<FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        visible_rows(&a.s.tree)
    })
}

/// Where the favourites list opens: under the gear, or at the top of the active pane when the bar is hidden.
unsafe fn fav_popup_anchor(main: HWND) -> RECT {
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_SET) {
        if IsWindowVisible(b).as_bool() {
            _ = GetWindowRect(b, &mut rc);
            if rc.bottom > rc.top {
                return rc;
            }
        }
    }
    let pane = app(|a| a.panes.get(a.s.active).and_then(|p| p.as_ref()).map(|p| p.hwnd)).unwrap_or_default();
    if !pane.0.is_null() {
        _ = GetWindowRect(pane, &mut rc);
        rc.bottom = rc.top;
    }
    rc
}

pub(crate) unsafe fn open_favs() {
    let (main, dpi, open, closed, n) = app(|a| {
        a.s.ensure_fav_tree();
        (a.main, a.dpi, a.fav_popup, a.fav_closed, visible_rows(&a.s.tree).len())
    });
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let g = fav_geo(n, dpi);
    let rc = fav_popup_anchor(main);
    let prev = GetFocus();
    app(|a| (a.fav_hot, a.fav_prev) = (None, prev));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("PanegridFav"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.fav_popup = h);
}

pub(crate) unsafe fn paint_favs(h: HWND) {
    let (rows, hot, dpi, font, small, icon_font) = app(|a| {
        a.s.ensure_fav_tree();
        (visible_rows(&a.s.tree), a.fav_hot, a.dpi, a.font, a.small_font, a.icon_font)
    });
    let g = fav_geo(rows.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if rows.is_empty() {
            text(dc, rc, tr("No favourites yet · add one with ☆+ in a pane header", "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가", "お気に入りはまだありません · ペインヘッダーの ☆+ で追加"), font, pal().sub, DT_CENTER);
        }
        for (k, (f, (row, remove))) in rows.iter().zip(&g.rows).enumerate() {
            if hot.is_some_and(|(i, _)| i == k) {
                fill(dc, *row, pal().pill);
                let x_on = hot == Some((k, true));
                text(dc, *remove, "\u{E711}", icon_font, if x_on { pal().text } else { pal().sub }, DT_CENTER);
            }
            let indent = s(14) * f.depth.min(6) as i32;
            let icon = RECT { left: row.left + s(6) + indent, right: row.left + s(28) + indent, ..*row };
            let glyph = if f.file { "\u{E8A5}" } else { "\u{E8B7}" };
            text(dc, icon, glyph, icon_font, pal().sub, DT_CENTER);
            let line = |top: i32, hgt: i32| RECT { left: icon.right + s(6), right: remove.left - s(4), top, bottom: top + hgt };
            let label = if f.name.is_empty() { short_name(&f.path) } else { f.name.clone() };
            text(dc, line(row.top + s(3), s(19)), &label, font, pal().text, DT_LEFT);
            let sub = if f.group {
                tr("Group", "그룹", "グループ")
            } else {
                f.path.as_str()
            };
            text(dc, line(row.top + s(21), s(16)), sub, small, pal().sub, DT_LEFT | DT_PATH_ELLIPSIS);
        }
    });
}

pub(crate) fn fav_hit(g: &FavGeo, x: i32, y: i32) -> Option<(usize, bool)> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    g.rows.iter().position(|(row, _)| inside(row)).map(|k| (k, inside(&g.rows[k].1)))
}

pub(crate) extern "system" fn fav_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let g = fav_geo(fav_rows().len(), app(|a| a.dpi));
        match msg {
            WM_PAINT => paint_favs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = fav_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.fav_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP | WM_KEYDOWN => {
                // ✕ / Del: drop it and shrink the list in place
                let remove = |k: usize| {
                    let id = fav_rows().get(k).map(|r| r.id.clone());
                    if let Some(id) = id {
                        app(|a| {
                            delete_at(&mut a.s.tree, &id);
                            a.s.sync_favs();
                            a.fav_hot = None;
                        });
                        favs_changed();
                    }
                };
                let open = |k: usize| {
                    let row = fav_rows().get(k).cloned();
                    let Some(row) = row else { return };
                    if row.group {
                        app(|a| {
                            toggle_open(&mut a.s.tree, &row.id);
                        });
                        favs_changed();
                        return;
                    }
                    _ = DestroyWindow(h);
                    open_fav_target(&row.path, row.file);
                };
                if msg == WM_LBUTTONUP {
                    let (x, y) = xy(lp);
                    match fav_hit(&g, x, y) {
                        Some((k, true)) => remove(k),
                        Some((k, false)) => open(k),
                        None => {}
                    }
                    return LRESULT(0);
                }
                let (n, cur) = (fav_rows().len(), app(|a| a.fav_hot.map(|(k, _)| k)));
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => {
                        let (main, prev, active) = app(|a| (a.main, a.fav_prev, a.s.active));
                        _ = DestroyWindow(h);
                        let gear = GetDlgItem(Some(main), ID_TB_SET).unwrap_or_default();
                        if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && prev != gear {
                            _ = SetFocus(Some(prev));
                        } else {
                            focus_view(active);
                        }
                    }
                    _ if n == 0 => {}
                    k @ (VK_DOWN | VK_UP) => {
                        let next = if k == VK_DOWN { cur.map_or(0, |k| (k + 1) % n) } else { cur.map_or(n - 1, |k| (k + n - 1) % n) };
                        app(|a| a.fav_hot = Some((next, false)));
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    VK_RETURN => if let Some(k) = cur { open(k) },
                    VK_DELETE => if let Some(k) = cur {
                        remove(k);
                        app(|a| a.fav_hot = if n > 1 { Some((k.min(n - 2), false)) } else { None });
                    },
                    _ => {}
                }
            }
            WM_SYSKEYDOWN => {
                let vk = wp.0 as u16;
                let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
                if fav_step(vk, down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT)).is_none() {
                    return DefWindowProcW(h, msg, wp, lp);
                }
                let rows = fav_rows();
                let Some(row) = app(|a| a.fav_hot).and_then(|(k, _)| rows.get(k).cloned()).filter(|r| r.in_tree()) else {
                    return LRESULT(0);
                };
                let moved = app(|a| {
                    let id = nudge_node(&mut a.s.tree, &row.id, vk == VK_DOWN.0);
                    if id.is_some() {
                        a.s.sync_favs();
                    }
                    id
                });
                if let Some(id) = moved {
                    let i = fav_rows().iter().position(|r| r.id == id);
                    app(|a| a.fav_hot = i.map(|k| (k, false)));
                    favs_changed();
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.fav_popup, a.fav_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// F7: the view's own "New folder", so the shell names it, selects it and starts rename.
pub(crate) unsafe fn explorer_new_folder(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let Ok(cm) = sv.GetItemObject::<IContextMenu>(SVGIO_BACKGROUND) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let Ok(menu) = CreatePopupMenu() else { return };
    let verb = b"NewFolder\0";
    if cm.QueryContextMenu(menu, 0, 1, 0x7fff, CMF_NORMAL).is_ok() {
        let ci = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: sv.GetWindow().unwrap_or_default(),
            lpVerb: PCSTR(verb.as_ptr()),
            nShow: SW_SHOWNORMAL.0 as i32,
            ..Default::default()
        };
        if cm.InvokeCommand(&ci).is_err() {
            _ = MessageBeep(MB_ICONWARNING);
        }
    } else {
        _ = MessageBeep(MB_ICONWARNING);
    }
    _ = DestroyMenu(menu);
}

// Ctrl+Shift+N: a new folder in the active pane, then the shell's own rename (F2) once it shows up.
pub(crate) unsafe fn new_folder(i: usize) {
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = tr("New folder", "새 폴더", "新しいフォルダー");
    let name = fresh_folder(base, |n| Path::new(&dir).join(n).exists());
    if std::fs::create_dir(Path::new(&dir).join(&name)).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let full = Path::new(&dir).join(&name);
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_MKDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_NEW.lock() {
        *p = Some(PendingNew { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWF, 40, None);
    }
    focus_view(i);
}

// The view lists the new folder a moment after Refresh. Select it and start the shell rename.
pub(crate) unsafe fn select_new_folder(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // SVSI_EDIT is the shell's rename. If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

#[cfg(test)]
mod tests {
    use super::parent_of;

    #[test]
    fn parent_of_a_file_keeps_the_drive_root() {
        assert_eq!(parent_of(r"C:\a\note.txt").as_deref(), Some(r"C:\a"));
        assert_eq!(parent_of(r"C:\note.txt").as_deref(), Some(r"C:\"));
        assert_eq!(parent_of("note.txt"), None);
    }
}

pub(crate) unsafe fn popup_menu(menu: HMENU, id: i32) {
    let main = app(|a| a.main);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), id) {
        _ = GetWindowRect(b, &mut rc);
    }
    _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom + 4, None, main, None);
}

