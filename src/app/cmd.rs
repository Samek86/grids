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
    // The preview takes its own keys. Pane shortcuts must not act behind it.
    let peek = app(|a| a.peek);
    if !peek.0.is_null() && m.hwnd == peek {
        return false;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    let win = down(VK_LWIN) || down(VK_RWIN);
    // A conflict dialog is up. Let it see the key.
    if crate::keys::modal() {
        return false;
    }
    // The key that just finished a capture must not also run the command.
    if crate::keys::swallows(vk) {
        return true;
    }
    if crate::keys::capturing() {
        // The Windows key is a modifier, so a chord such as Win+Alt+E can be recorded.
        if matches!(vk, 0x5B | 0x5C) || m.lParam.0 & (1 << 30) != 0 {
            return true;
        }
        return finish_capture(vk, ctrl, shift, alt, win);
    }
    // Typing in Settings must not fire pane shortcuts. Esc closes the window.
    if settings::captures(m.hwnd) {
        if !ctrl && !alt && VIRTUAL_KEY(vk) == VK_ESCAPE {
            settings::close();
            return true;
        }
        return false;
    }
    if app(|a| a.matcher.as_ref().map(|b| b.edit)) == Some(m.hwnd) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => {
                app(|a| a.held = Some(VK_RETURN.0));
                run_match(shift);
            }
            VK_ESCAPE => close_match(true),
            _ => return false,
        }
        return true;
    }
    // The rename box on a session card. Enter and Esc are its own; the rest is typing.
    if !m.hwnd.0.is_null() && m.hwnd == app(|a| a.load_edit) && app(|a| a.load_editing.is_some()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => {
                app(|x| x.held = Some(VK_RETURN.0));
                commit_load_rename();
            }
            VK_ESCAPE => _ = end_load_rename(),
            _ => return false,
        }
        return true;
    }
    if m.hwnd == app(|a| a.tab_edit) && app(|a| a.tab_renaming.is_some()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => end_rename(true),
            VK_ESCAPE => end_rename(false),
            _ => return false,
        }
        return true;
    }
    if m.hwnd == app(|a| a.side_edit) && !app(|a| a.side_edit_at.is_empty()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => end_side_rename(true),
            VK_ESCAPE => end_side_rename(false),
            _ => return false,
        }
        return true;
    }
    // Plain Alt+↑/↓ in the favourites panel or its list reorders that row.
    // It must not send the pane behind it to the parent folder.
    if fav_step(vk, ctrl, alt, shift).is_some() {
        let (side, fav) = app(|a| (a.side_hwnd, a.fav_popup));
        let on = |root: HWND| !root.0.is_null() && (m.hwnd == root || IsChild(root, m.hwnd).as_bool());
        if on(side) || on(fav) {
            return false;
        }
    }
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = active_pane();
    let addr = pane_ctl(a, ID_EDIT) == Some(m.hwnd);
    let filt = pane_ctl(a, ID_FILTER) == Some(m.hwnd);
    let shell = || {
        let Some(sv) = view_of::<IShellView>(a) else { return false };
        let svh = sv.GetWindow().unwrap_or_default();
        (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool()) && sv.TranslateAccelerator(m) == S_OK
    };
    // The file list is a SysListView32 on some systems and a DirectUIHWND on others (Windows 10 has no list
    // window to find), so the focus counts as "in the list" when it is that kind of window inside the view.
    let list = view_of::<IFolderView2>(a).is_some_and(|fv| {
        if view_list(&fv) == m.hwnd {
            return true;
        }
        let svh = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
        let mut c = [0u16; 32];
        let n = GetClassNameW(m.hwnd, &mut c) as usize;
        matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32")
            && !svh.0.is_null()
            && (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool())
    });
    // F6 leaves the address and the filter. Every other chord stays out of a name being typed.
    if let Some(i) = crate::keys::lookup(vk, ctrl, shift, alt, win) {
        let id = crate::keys::id_of(i);
        if id == "app.keys" && m.lParam.0 & (1 << 30) != 0 {
            return true; // holding Ctrl+/ toggles once
        }
        if matches!(id, "focus.next" | "focus.prev" | "app.keys") {
            return dispatch(id, vk, a, list, shell);
        }
    }
    // Address, filter, and tab rename keep every key except Enter and Esc.
    // A rebound letter must not land in the middle of a name.
    if addr || filt {
        match VIRTUAL_KEY(vk) {
            VK_RETURN if addr => go_edit(a),
            VK_RETURN if filt => {
                if !open_find_sel(a) {
                    apply_filter(a);
                    focus_view(a);
                }
            }
            VK_ESCAPE if addr => addr_reset(a, m.hwnd),
            VK_ESCAPE if filt => {
                find_clear(a);
                _ = SetWindowTextW(m.hwnd, w!(""));
                apply_filter(a);
                focus_view(a);
            }
            // Alt+↓ in the filter is the file-kind menu. Recent folders keeps Alt+↓ everywhere else.
            VK_DOWN if filt && alt && !ctrl && !shift => {
                if m.lParam.0 & (1 << 30) == 0 {
                    kind_menu(a);
                }
            }
            VK_DOWN if filt && !alt && !ctrl && !shift => {
                if !nudge_find(a, 1) {
                    return false;
                }
            }
            VK_UP if filt && !alt && !ctrl && !shift => {
                if !nudge_find(a, -1) {
                    return false;
                }
            }
            VK_NEXT if filt && !alt => {
                if !nudge_find(a, 8) {
                    return false;
                }
            }
            VK_PRIOR if filt && !alt => {
                if !nudge_find(a, -8) {
                    return false;
                }
            }
            _ => return false,
        }
        return true;
    }
    if let Some(i) = crate::keys::lookup(vk, ctrl, shift, alt, win) {
        let id = crate::keys::id_of(i);
        if matches!(id, "app.keys" | "view.peek" | "app.front" | "view.gather" | "file.mkdir") && m.lParam.0 & (1 << 30) != 0 {
            return true;
        }
        // Pane-tab chords stay out of the address bar (already returned) and out of every other focus.
        // They also stay quiet while the strips are hidden, so a shared key cannot fight the window tabs.
        if matches!(id, "pane.tab.new" | "pane.tab.close" | "pane.tab.next" | "pane.tab.prev") {
            let in_pane = list || pane_of(m.hwnd).is_some();
            if !app(|x| x.s.pane_tabs_on) || !in_pane {
                return shell();
            }
        }
        return dispatch(id, vk, a, list, shell);
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

unsafe fn finish_capture(vk: u16, ctrl: bool, shift: bool, alt: bool, win: bool) -> bool {
    match crate::keys::offer(vk, ctrl, shift, alt, win) {
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
            apply_hotkey(true);
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
        "pane.swap.left" => swap_dir(-1, 0),
        "pane.swap.right" => swap_dir(1, 0),
        "pane.swap.up" => swap_dir(0, -1),
        "pane.swap.down" => swap_dir(0, 1),
        "nav.back" => browse(a, SBSP_NAVIGATEBACK),
        "nav.fwd" => browse(a, SBSP_NAVIGATEFORWARD),
        "nav.up" => browse(a, SBSP_PARENT),
        "nav.recent" => recent_menu(a),
        "nav.addr" | "nav.addr2" | "nav.addr3" => return focus_ctl(a, ID_EDIT),
        "view.filter" => return focus_ctl(a, ID_FILTER),
        "view.peek" if list => open_peek(a),
        "view.peek" => return shell(),
        "focus.next" => cycle_focus(false),
        "focus.prev" => cycle_focus(true),
        "view.hidden" => toggle_hidden(),
        "view.panel" => toggle_tree(),
        "view.gather" => gather_explorer(),
        "view.zoom" => toggle_zoom(),
        "file.sel" | "file.sel2" if list => {
            app(|a| a.held = Some(vk));
            open_match(a, false);
        }
        "file.desel" if list => {
            app(|a| a.held = Some(vk));
            open_match(a, true);
        }
        "file.invert" if list => {
            app(|a| a.held = Some(vk));
            invert_selection(a);
        }
        "file.sel" | "file.sel2" | "file.desel" | "file.invert" => return shell(),
        "file.path" => {
            if !list {
                return shell();
            }
            copy_paths(a);
        }
        "file.new" => new_file(a),
        "file.dir" => new_folder(a),
        "file.mkdir" if list => explorer_new_folder(a),
        "file.mkdir" => return shell(),
        "pane.tab.new" => {
            new_pane_tab(a);
            focus_view(a);
        }
        "pane.tab.close" => {
            let k = app(|x| {
                let n = x.s.pane_tab_list(a).len();
                if n > 1 { x.s.pane_cur[a].min(n - 1) } else { 0 }
            });
            close_pane_tab(a, k);
        }
        "pane.tab.next" => step_pane_tab(a, 1),
        "pane.tab.prev" => step_pane_tab(a, -1),
        "sess.save" => save_current(),
        "fav.list" => open_favs(),
        "app.settings" => settings::open(app(|x| x.main)),
        "app.help" => open_help(),
        "app.keys" => toggle_keys(),
        "app.front" => hotkey_pressed(),
        "win.new" => new_window(),
        _ => return shell(),
    }
    true
}

pub(crate) unsafe fn pre_translate(m: &MSG) -> bool {
    bind_hwnd(m.hwnd);
    let in_tree = || app(|a| !a.tree_hwnd.0.is_null() && (a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool()));
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
        WM_MOUSEWHEEL => {
            let tabs = app(|a| a.tabs_hwnd);
            if tabs.0.is_null() {
                return false;
            }
            let pt = POINT { x: (m.lParam.0 & 0xffff) as i16 as i32, y: ((m.lParam.0 >> 16) & 0xffff) as i16 as i32 };
            let under = WindowFromPoint(pt);
            if under == tabs || IsChild(tabs, under).as_bool() {
                tab_wheel((m.wParam.0 >> 16) as i16 as i32);
                return true;
            }
            false
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            kids_cancel();
            if m.message == WM_LBUTTONDOWN && in_tree() {
                app(|a| a.tree_input = true);
            }
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            kids_cancel();
            let (eat, next) = held_key(app(|a| a.held), m.wParam.0 as u16, m.lParam.0 & (1 << 30) != 0);
            app(|a| a.held = next);
            if eat {
                return true;
            }
            // Cleared before key() so a selection the tree reports while we handle Ctrl+Tab or F6
            // is not taken for a click in the tree (that would reload the pane and drop Forward).
            let tree = m.message == WM_KEYDOWN && in_tree();
            let prev = app(|a| std::mem::take(&mut a.tree_input));
            let used = key(m);
            app(|a| a.tree_input = tree_input_after(prev, tree, used));
            used
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let vk = m.wParam.0 as u16;
            app(|a| a.held = a.held.filter(|&k| k != vk));
            crate::keys::clear_swallow(vk);
            false
        }
        _ => false,
    }
}

/// F6 ring: 0 file list, 1 address, 2 filter, then the left panel's folder tree and favourites.
/// A hidden panel drops the last two stops.
pub(crate) fn focus_ring(pos: usize, back: bool, panel: bool) -> usize {
    let n = if panel { 5 } else { 3 };
    (pos.min(n - 1) + if back { n - 1 } else { 1 }) % n
}

/// A key Grids handled (Ctrl+Tab, F6, …) must not count as input in the tree.
/// An unhandled key in the tree still does, so the arrows navigate the pane.
pub(crate) fn tree_input_after(prev: bool, in_tree: bool, used: bool) -> bool {
    !used && (prev || in_tree)
}

unsafe fn cycle_focus(back: bool) {
    let a = active_pane();
    let f = GetFocus();
    let (panel, th, side) = app(|x| (x.s.tree_on, x.tree_hwnd, x.side_hwnd));
    let shown = panel && !th.0.is_null() && IsWindowVisible(th).as_bool();
    let addr = pane_ctl(a, ID_EDIT) == Some(f);
    let filt = pane_ctl(a, ID_FILTER) == Some(f);
    let in_tree = !th.0.is_null() && (th == f || IsChild(th, f).as_bool());
    let in_fav = !side.0.is_null() && (side == f || IsChild(side, f).as_bool());
    let pos = if addr { 1 } else if filt { 2 } else if in_tree { 3 } else if in_fav { 4 } else { 0 };
    match focus_ring(pos, back, shown) {
        1 => {
            _ = focus_ctl(a, ID_EDIT);
        }
        2 => {
            _ = focus_ctl(a, ID_FILTER);
        }
        3 => {
            let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).unwrap_or(th);
            _ = SetFocus(Some(tv));
            reveal_tree_selection();
        }
        4 => {
            if app(|x| x.side_sel.is_none()) {
                app(|x| x.side_sel = Some(0));
            }
            _ = SetFocus(Some(side));
            _ = InvalidateRect(Some(side), None, false);
        }
        _ => focus_view(a),
    }
}

#[cfg(test)]
mod focus_tests {
    use super::{focus_ring, tree_input_after};

    #[test]
    fn focus_rings() {
        let walk = |back, panel| (0..5).map(|p| focus_ring(p, back, panel)).collect::<Vec<_>>();
        assert_eq!(walk(false, true), [1, 2, 3, 4, 0]);
        assert_eq!(walk(true, true), [4, 0, 1, 2, 3]);
        assert_eq!(walk(false, false)[..3], [1, 2, 0]);
        assert_eq!(walk(true, false)[..3], [2, 0, 1]);
        // A stop that only exists while the panel is open falls back into the short ring.
        assert_eq!(focus_ring(4, false, false), 0);
        assert_eq!(focus_ring(3, true, false), 1);
    }

    #[test]
    fn tree_input_kept_only_for_tree_keys() {
        assert!(tree_input_after(false, true, false)); // arrow in the tree
        assert!(!tree_input_after(true, true, true)); // Ctrl+Tab / F6 in the tree
        assert!(tree_input_after(true, false, false) && !tree_input_after(false, false, false));
    }
}

