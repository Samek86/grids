use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::shell::*;
use crate::settings;

// ---------- keyboard ----------

// "New Text Document.txt", then "New Text Document (2).txt", … The number sits before the extension.
pub(crate) fn fresh_file(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (base, String::new()),
    };
    let mut n = 2u32;
    loop {
        let name = format!("{stem} ({n}){ext}");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

// File created by Ctrl+Alt+N, waiting for the view to list it so it can be selected and renamed.
pub(crate) struct PendingFile {
    pub(crate) pane: usize,
    pub(crate) name: String,
    pub(crate) left: u8,
}
pub(crate) static PENDING_FILE: Mutex<Option<PendingFile>> = Mutex::new(None);

// Ctrl+Alt+N: an empty text file in the active pane, then the shell's own rename (F2) once it shows up.
pub(crate) unsafe fn new_file(i: usize) {
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = app(|a| if a.s.new_name.trim().is_empty() { default_new_name() } else { a.s.new_name.clone() });
    let name = fresh_file(&base, |n| Path::new(&dir).join(n).exists());
    let full = Path::new(&dir).join(&name);
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&full).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_FILE.lock() {
        *p = Some(PendingFile { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWFILE, 40, None);
    }
    focus_view(i);
}

// The view lists the new file a moment after Refresh. Select it and start the shell rename.
pub(crate) unsafe fn select_new_file(i: usize, name: &str) -> bool {
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
    // SVSI_EDIT is the shell's rename (the same one F2 uses). If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

pub(crate) unsafe fn key(m: &MSG) -> bool {
    let vk = m.wParam.0 as u16;
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    // A conflict dialog is up. Let it see the key.
    if crate::keys::modal() {
        return false;
    }
    // The key that just finished a capture must not also run the command.
    if crate::keys::swallows(vk) {
        return true;
    }
    if crate::keys::capturing() {
        if down(VK_LWIN) || down(VK_RWIN) || m.lParam.0 & (1 << 30) != 0 {
            return true;
        }
        return finish_capture(vk, ctrl, shift, alt);
    }
    // Typing in Settings must not fire pane shortcuts. Esc closes the window.
    if settings::captures(m.hwnd) {
        if !ctrl && !alt && VIRTUAL_KEY(vk) == VK_ESCAPE {
            settings::close();
            return true;
        }
        return false;
    }
    if m.hwnd == app(|a| a.tab_edit) && app(|a| a.tab_renaming.is_some()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => end_rename(true),
            VK_ESCAPE => end_rename(false),
            _ => return false,
        }
        return true;
    }
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = active_pane();
    let addr = pane_ctl(a, ID_EDIT) == Some(m.hwnd);
    let filt = pane_ctl(a, ID_FILTER) == Some(m.hwnd);
    // Address, filter, and tab rename keep every key except Enter and Esc.
    // A rebound letter must not land in the middle of a name.
    if addr || filt {
        match VIRTUAL_KEY(vk) {
            VK_RETURN if addr => go_edit(a),
            VK_RETURN if filt => {
                apply_filter(a);
                focus_view(a);
            }
            VK_ESCAPE if addr => addr_reset(a, m.hwnd),
            VK_ESCAPE if filt => {
                _ = SetWindowTextW(m.hwnd, w!(""));
                apply_filter(a);
                focus_view(a);
            }
            _ => return false,
        }
        return true;
    }
    let shell = || {
        let Some(sv) = view_of::<IShellView>(a) else { return false };
        let svh = sv.GetWindow().unwrap_or_default();
        (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool()) && sv.TranslateAccelerator(m) == S_OK
    };
    let list = view_of::<IFolderView2>(a).is_some_and(|fv| view_list(&fv) == m.hwnd);
    if let Some(i) = crate::keys::lookup(vk, ctrl, shift, alt) {
        return dispatch(crate::keys::id_of(i), vk, a, list, shell);
    }
    // Not an app shortcut. F5, Del, and Ctrl+V still belong to the file list
    // unless the user has bound that chord above.
    match VIRTUAL_KEY(vk) {
        VK_F5 if list => {
            resum_sizes();
            shell()
        }
        VK_DELETE if !ctrl && !alt && list => {
            delete_selection(a, shift);
            true
        }
        VK_V if ctrl && !shift && !alt && list => {
            if !paste_key(m.lParam.0 & (1 << 30) != 0, || try_paste(a)) {
                shell()
            } else {
                true
            }
        }
        _ => shell(),
    }
}

unsafe fn finish_capture(vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    match crate::keys::offer(vk, ctrl, shift, alt) {
        crate::keys::Offer::Ignore => true,
        crate::keys::Offer::Cancel | crate::keys::Offer::Same => {
            crate::keys::swallow(vk);
            settings::redraw();
            true
        }
        crate::keys::Offer::Bind { row, chord, other } => {
            if let Some(other) = other {
                let name = crate::keys::action(other);
                if !settings::confirm_swap(&crate::keys::format_chord(&chord), &name) {
                    crate::keys::swallow(vk);
                    settings::redraw();
                    return true;
                }
            }
            crate::keys::rebind(row, chord);
            crate::keys::swallow(vk);
            save_session();
            settings::refresh();
            true
        }
    }
}

unsafe fn dispatch(id: &str, vk: u16, a: usize, list: bool, shell: impl Fn() -> bool) -> bool {
    match id {
        "tab.new" => new_tab(),
        "tab.close" => close_tab(app(|x| x.s.tab)),
        "tab.next" => switch_rel(1),
        "tab.prev" => switch_rel(-1),
        "tab.reopen" => reopen_closed_at(0),
        "pane.next" => cycle(1),
        "pane.prev" => cycle(-1),
        "pane.goto" => {
            let n = if (0x31..=0x39).contains(&vk) { (vk - 0x31) as usize } else { 0 };
            if n >= app(|x| x.s.panes()) {
                return shell();
            }
            set_active(n);
            focus_view(n);
        }
        "pane.left" => move_pane(-1, 0),
        "pane.right" => move_pane(1, 0),
        "pane.up" => move_pane(0, -1),
        "pane.down" => move_pane(0, 1),
        "nav.back" => browse(a, SBSP_NAVIGATEBACK),
        "nav.fwd" => browse(a, SBSP_NAVIGATEFORWARD),
        "nav.up" => browse(a, SBSP_PARENT),
        "nav.recent" => recent_menu(a),
        "nav.addr" | "nav.addr2" | "nav.addr3" => return focus_ctl(a, ID_EDIT),
        "view.filter" => return focus_ctl(a, ID_FILTER),
        "view.hidden" => toggle_hidden(),
        "view.zoom" => toggle_zoom(),
        "file.path" => {
            if !list {
                return shell();
            }
            copy_paths(a);
        }
        "file.new" => new_file(a),
        "file.dir" => new_folder(a),
        "sess.save" => save_current(),
        "fav.list" => open_favs(),
        "app.settings" => settings::open(app(|x| x.main)),
        "app.help" => open_help(),
        _ => return shell(),
    }
    true
}

pub(crate) unsafe fn pre_translate(m: &MSG) -> bool {
    if m.message == WM_LBUTTONDOWN || m.message == WM_KEYDOWN {
        app(|a| a.tree_input |= a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool());
    }
    match m.message {
        WM_MOUSEMOVE => {
            set_hover(if is_icon_button(m.hwnd) { m.hwnd } else { HWND::default() });
            false
        }
        WM_MOUSELEAVE => {
            if app(|a| a.hover == m.hwnd) {
                set_hover(HWND::default());
            }
            false
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => key(m),
        WM_KEYUP | WM_SYSKEYUP => {
            crate::keys::clear_swallow(m.wParam.0 as u16);
            false
        }
        _ => false,
    }
}

