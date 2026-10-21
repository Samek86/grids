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
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    // Typing in Settings must not fire pane shortcuts. Esc closes the window.
    if settings::captures(m.hwnd) {
        if !ctrl && !alt && VIRTUAL_KEY(m.wParam.0 as u16) == VK_ESCAPE {
            settings::close();
            return true;
        }
        return false;
    }
    if m.hwnd == app(|a| a.tab_edit) && app(|a| a.tab_renaming.is_some()) {
        match VIRTUAL_KEY(m.wParam.0 as u16) {
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
    // F2, Del, Ctrl+C/V ... are the shell view's accelerators: hand them over.
    let shell = || {
        let Some(sv) = view_of::<IShellView>(a) else { return false };
        let svh = sv.GetWindow().unwrap_or_default();
        (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool()) && sv.TranslateAccelerator(m) == S_OK
    };
    let list = || view_of::<IFolderView2>(a).is_some_and(|fv| view_list(&fv) == m.hwnd);
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        // Ctrl+Tab used to cycle panes. It now moves between tabs; panes use Ctrl+PageDown / Ctrl+PageUp.
        VK_T if ctrl && shift && !alt => reopen_closed_at(0),
        VK_T if ctrl && !shift && !alt => new_tab(),
        VK_W if ctrl && !shift && !alt => close_tab(app(|x| x.s.tab)),
        VK_TAB if ctrl && !alt => switch_rel(if shift { -1 } else { 1 }),
        VK_NEXT if ctrl && !alt => cycle(1),
        VK_PRIOR if ctrl && !alt => cycle(-1),
        k if ctrl && !shift && !alt && (VK_1.0..=VK_9.0).contains(&k.0) && ((k.0 - VK_1.0) as usize) < app(|x| x.s.panes()) => {
            let i = (k.0 - VK_1.0) as usize;
            set_active(i);
            focus_view(i);
        }
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && shift && !ctrl => match k {
            VK_LEFT => move_pane(-1, 0),
            VK_RIGHT => move_pane(1, 0),
            VK_UP => move_pane(0, -1),
            _ => move_pane(0, 1),
        },
        VK_LEFT if alt => browse(a, SBSP_NAVIGATEBACK),
        VK_RIGHT if alt => browse(a, SBSP_NAVIGATEFORWARD),
        VK_UP if alt => browse(a, SBSP_PARENT),
        VK_L if ctrl => return focus_ctl(a, ID_EDIT),
        VK_D if alt => return focus_ctl(a, ID_EDIT),
        VK_F4 if !alt => return focus_ctl(a, ID_EDIT),
        VK_F if ctrl => return focus_ctl(a, ID_FILTER),
        VK_H if ctrl => toggle_hidden(),
        VK_OEM_COMMA if ctrl && !shift && !alt => settings::open(app(|a| a.main)),
        VK_S if ctrl => save_current(),
        VK_B if ctrl && !shift && !alt => open_favs(),
        VK_M if ctrl && shift && !alt => toggle_zoom(),
        VK_N if ctrl && alt && !shift && !addr && !filt => new_file(a),
        VK_N if ctrl && shift && !alt && !addr && !filt => new_folder(a),
        VK_F1 => open_help(),
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
        _ if addr || filt => return false,
        VK_C if ctrl && shift && !alt && list() => copy_paths(a),
        VK_DOWN if alt && !ctrl && !shift => recent_menu(a),
        VK_F5 if list() => {
            resum_sizes();
            return shell();
        }
        VK_DELETE if !ctrl && !alt && list() => delete_selection(a, shift),
        VK_V if ctrl && !shift && !alt && list() => {
            if !paste_key(m.lParam.0 & (1 << 30) != 0, || try_paste(a)) {
                return shell();
            }
        }
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
        _ => false,
    }
}

