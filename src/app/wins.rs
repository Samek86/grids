// One process, several top-level windows. Each window owns its tabs and its Explorer panes.
// Panes still appear only for the tab on screen. Extra windows are created after the first
// one is visible, so startup stays the cost of a single window.

use serde_json::{Map, Value};

use crate::imports::*;
use crate::pane::*;
use crate::session::*;
use crate::ui::*;
use super::*;

thread_local! {
    static SNAPS: RefCell<Vec<Value>> = RefCell::new(Vec::new());
    static PENDING: RefCell<Vec<Value>> = RefCell::new(Vec::new());
}

/// What a second `grids.exe` does. Single-window mode hands off even when single-instance is off,
/// so a second process cannot open a second window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchPlan {
    NewProcess,
    NewWindow,
    FocusTabs,
}

pub(crate) fn launch_plan(single_instance: bool, one_window: bool) -> LaunchPlan {
    if one_window {
        LaunchPlan::FocusTabs
    } else if single_instance {
        LaunchPlan::NewWindow
    } else {
        LaunchPlan::NewProcess
    }
}

/// What the running process does with a second launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inbound {
    NewWindow,
    Focus,
    Tabs,
}

pub(crate) fn inbound(one_window: bool, has_paths: bool) -> Inbound {
    if !one_window {
        Inbound::NewWindow
    } else if has_paths {
        Inbound::Tabs
    } else {
        Inbound::Focus
    }
}

/// A command that would open a top-level window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spawn {
    Window,
    Tab,
}

pub(crate) fn spawn_kind(one_window: bool) -> Spawn {
    if one_window { Spawn::Tab } else { Spawn::Window }
}

pub(crate) fn allows_another_window(one_window: bool) -> bool {
    spawn_kind(one_window) == Spawn::Window
}

/// Win+Alt+E while this window is already in front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyWhenFront {
    Minimize,
    NewTab,
}

pub(crate) fn hotkey_when_front(one_window: bool) -> HotkeyWhenFront {
    if one_window { HotkeyWhenFront::NewTab } else { HotkeyWhenFront::Minimize }
}

pub(crate) fn set_snaps(v: Vec<Value>) {
    SNAPS.with(|s| *s.borrow_mut() = v);
}

pub(crate) fn take_snaps() -> Vec<Value> {
    SNAPS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

pub(crate) fn stash_windows(v: Vec<Value>) {
    PENDING.with(|s| *s.borrow_mut() = v);
}

pub(crate) fn take_stashed() -> Vec<Value> {
    PENDING.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

fn win_json(w: [i32; 5]) -> Value {
    serde_json::json!({
        "left": w[0],
        "top": w[1],
        "right": w[2],
        "bottom": w[3],
        "maximized": w[4] != 0,
    })
}

fn win_from(v: &Value) -> Option<[i32; 5]> {
    let o = v.as_object()?;
    Some([
        o.get("left")?.as_i64()? as i32,
        o.get("top")?.as_i64()? as i32,
        o.get("right")?.as_i64()? as i32,
        o.get("bottom")?.as_i64()? as i32,
        o.get("maximized").and_then(|n| n.as_bool()).unwrap_or(false) as i32,
    ])
}

/// One window: rect, sidebar, named session, and that window's tabs. Shared prefs stay on the file root.
pub(crate) fn window_value(s: &Session) -> Value {
    let shown = s.view_tabs();
    let at = s.tab.min(shown.len().saturating_sub(1));
    let mut m = Map::new();
    if let Some(w) = s.win {
        m.insert("window".into(), win_json(w));
    }
    m.insert("side".into(), Value::from(s.side));
    m.insert("side_split".into(), Value::from(s.side_split));
    if !s.file.is_empty() {
        m.insert("session".into(), Value::String(s.file.clone()));
    }
    let tabs: Value = serde_json::from_str(&tabs_json(&shown, at)).unwrap_or(Value::Array(Vec::new()));
    m.insert("tabs".into(), tabs);
    Value::Object(m)
}

/// `base` supplies the shared preferences. The value supplies this window's grid.
pub(crate) fn session_from_window(v: &Value, base: &Session) -> Session {
    let mut s = base.clone();
    s.file.clear();
    s.tabs.clear();
    s.tab = 0;
    s.win = None;
    s.heights = vec![2, 2];
    s.colr = even(2);
    s.rowr = vec![even(2); 2];
    s.active = 0;
    s.paths = vec![String::new(); MAXP];
    s.views = vec![(0, 0); MAXP];
    s.cols = vec![String::new(); MAXP];
    let Some(obj) = v.as_object() else {
        s.anchor();
        return s;
    };
    if let Some(w) = obj.get("window").and_then(win_from) {
        s.win = Some(w);
    }
    if let Some(side) = obj.get("side").and_then(|n| n.as_i64()) {
        s.side = side as i32;
    }
    if let Some(split) = obj.get("side_split").and_then(|n| n.as_f64()) {
        s.side_split = sane_split(split);
    }
    if let Some(file) = obj.get("session").and_then(|x| x.as_str()) {
        s.file = file.to_string();
    }
    if let Some(tabs) = obj.get("tabs").and_then(|t| t.as_array()) {
        adopt_tabs(&mut s, tabs);
    } else {
        s.anchor();
    }
    s
}

/// The foreground window first, then the rest. Each snap is that window's own tabs.
pub(crate) unsafe fn snapshot_windows() -> Vec<Value> {
    let n = window_count();
    if n == 0 {
        return Vec::new();
    }
    let focus = GetForegroundWindow();
    let prefer = app(|a| a.desks.iter().position(|d| d.main == focus).unwrap_or(a.cur));
    if prefer < n {
        bind_index(prefer);
    }
    let cur = app(|a| a.cur);
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let i = if k == 0 { cur } else if k <= cur { k - 1 } else { k };
        bind_index(i);
        if app(|a| a.main.0.is_null()) {
            continue;
        }
        remember_tabs();
        let main = app(|a| a.main);
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        if GetWindowPlacement(main, &mut wp).is_ok() {
            let r = wp.rcNormalPosition;
            // Minimized keeps the maximized flag from the last normal or maximized size.
            let minimized = wp.showCmd == SW_SHOWMINIMIZED.0 as u32;
            let max = if minimized {
                app(|a| a.was_max) as i32
            } else {
                let on = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32;
                app(|a| a.was_max = on);
                on as i32
            };
            app(|a| a.s.win = Some([r.left, r.top, r.right, r.bottom, max]));
        }
        out.push(app(|a| window_value(&a.s)));
    }
    bind_index(cur);
    out
}

pub(crate) fn fresh_window_session() -> Session {
    app(|a| {
        let mut n = a.s.clone();
        n.file.clear();
        n.tabs.clear();
        n.tab = 0;
        n.heights = vec![2, 2];
        n.colr = even(2);
        n.rowr = vec![even(2); 2];
        n.active = 0;
        n.paths = vec![String::new(); MAXP];
        n.views = vec![(0, 0); MAXP];
        n.cols = vec![String::new(); MAXP];
        n.pane_folders = vec![Vec::new(); MAXP];
        n.pane_cur = vec![0; MAXP];
        let d = 32 * a.dpi.max(96) / 96;
        if let Some(w) = n.win.as_mut() {
            w[0] += d;
            w[1] += d;
            w[2] += d;
            w[3] += d;
            w[4] = 0;
        }
        n.anchor();
        n
    })
}

/// A new top-level window showing `s`. Only the visible tab's panes are created.
pub(crate) unsafe fn open_window(mut s: Session) -> Result<HWND> {
    let seed = app(|a| a.desks.iter().all(|d| d.main.0.is_null()));
    let pushed = if seed {
        false
    } else {
        let i = app(|a| {
            a.desks.push(blank_desk());
            a.desks.len() - 1
        });
        bind_index(i);
        true
    };
    let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
    let main = match CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("GridsMain"),
        w!("Grids"),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        1400,
        900,
        None,
        None,
        Some(hinst),
        None,
    ) {
        Ok(h) => h,
        Err(e) => {
            if pushed {
                let handles = drop_current_desk();
                release_desk_handles(handles.0, handles.1, handles.2, handles.3, handles.4);
            }
            return Err(e);
        }
    };
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(main, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    let dpi = GetDpiForWindow(main) as i32;
    if s.side <= 0 {
        s.side = 240 * dpi / 96;
    }
    let win = s.win;
    s.anchor();
    let ntabs = s.tabs.len().max(1);
    let view_menu = make_menus().unwrap_or_default();
    let tip_style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02);
    let tip = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, tip_style, 0, 0, 0, 0, Some(main), None, None, None)?;
    app(|a| {
        (a.s, a.main, a.dpi, a.view_menu, a.tip) = (s, main, dpi, view_menu, tip);
        a.park = (0..ntabs).map(|_| no_panes()).collect();
        a.park_lru.clear();
        a.zoom = None;
        a.closed.clear();
    });
    sync_pane_tabs(app(|a| a.s.pane_tabs_on));
    if seed {
        let seeded = app(|a| crate::store::sync_text(&a.s));
        LAST_SAVED.with(|l| *l.borrow_mut() = seeded);
    }
    let tabs_hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsTabs"), w!(""), WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN, 0, 0, 0, 0, Some(main), None, Some(hinst), None)?;
    let tab_edit = child(tabs_hwnd, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), ID_TAB_EDIT, HFONT::default());
    _ = ShowWindow(tab_edit, SW_HIDE);
    app(|a| (a.tabs_hwnd, a.tab_edit) = (tabs_hwnd, tab_edit));
    let bar_ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let mut bars = [HWND::default(); 2];
    for b in &mut bars {
        *b = CreateWindowExW(bar_ex, w!("GridsBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None)?;
        _ = SetLayeredWindowAttributes(*b, COLORREF(0), 170, LWA_ALPHA);
    }
    app(|a| a.bars = bars);
    for (id, label, hint) in toolbar_tips() {
        add_tip(child(main, w!("BUTTON"), PCWSTR(HSTRING::from(label).as_ptr()), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), id, HFONT::default()), hint);
    }
    if let Ok((t, th)) = create_tree(main) {
        app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
    }
    let side = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsSide"), w!(""), WS_CHILD | WS_CLIPSIBLINGS | WS_VSCROLL, 0, 0, 0, 0, Some(main), None, Some(hinst), None).unwrap_or_default();
    app(|a| a.side_hwnd = side);
    register_side_drop(side);
    prime_theme(app(|a| a.s.dark));
    apply_theme_desk();
    apply_font();
    refresh_drives(); // laid out by the relayout below; startup does not lay the window out twice
    match win {
        Some([l, t, r, b, max]) => {
            // Saved position is workspace coordinates. Fit it, and move while still hidden so a
            // different-DPI monitor changes DPI before the final placement.
            let saved = RECT { left: l, top: t, right: r, bottom: b };
            let mut fixed = saved;
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(MonitorFromRect(&saved, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
                fixed = fit_saved(saved, mi.rcMonitor, mi.rcWork);
                let (dx, dy) = (mi.rcWork.left - mi.rcMonitor.left, mi.rcWork.top - mi.rcMonitor.top);
                let (w, h) = (fixed.right - fixed.left, fixed.bottom - fixed.top);
                _ = SetWindowPos(main, None, fixed.left + dx, fixed.top + dy, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            let wp = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                showCmd: if max != 0 { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                rcNormalPosition: fixed,
                ..Default::default()
            };
            _ = SetWindowPlacement(main, &wp);
        }
        None => {
            let mut r = RECT::default();
            _ = GetWindowRect(main, &mut r);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let ok = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool();
            let f = fit(r, mi.rcWork);
            if ok && f != r {
                _ = SetWindowPos(main, None, f.left, f.top, f.right - f.left, f.bottom - f.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            _ = ShowWindow(main, SW_SHOWNORMAL);
        }
    }
    relayout();
    focus_view(app(|a| a.s.active));
    let file = app(|a| a.s.file.clone());
    let saved = std::fs::read_to_string(&file).ok();
    app(|a| {
        let base = match &saved {
            Some(t) => {
                let mut b = Session::parse(t);
                b.reorder(&a.s.order);
                b
            }
            None => a.s.clone(),
        };
        a.base = Some(base);
    });
    refresh_save_button();
    SetTimer(Some(main), TIMER_STATUS, 300, None);
    // Taken by another app: the menu stays unchecked and hotkey stays on, so the next start tries again.
    try_register_hotkey(main);
    if !seed {
        _ = SetForegroundWindow(main);
    }
    Ok(main)
}

pub(crate) unsafe fn new_window() {
    if !allows_another_window(app(|a| a.s.one_window)) {
        new_tab();
        return;
    }
    let s = fresh_window_session();
    if open_window(s).is_ok() {
        note_save();
    }
}

/// Bring `h` forward, restoring it when it is minimized.
pub(crate) unsafe fn reveal_window(h: HWND) {
    if h.0.is_null() {
        return;
    }
    if IsIconic(h).as_bool() {
        _ = ShowWindow(h, SW_RESTORE);
    }
    _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
    bind_hwnd(h);
    _ = SetForegroundWindow(h);
    focus_view(app(|a| a.s.active));
}

/// Paths that would have filled a new window become tabs in this one. A session file keeps its tabs.
pub(crate) unsafe fn open_args_as_tabs(args: Vec<String>) {
    if args.is_empty() || app(|a| a.main.0.is_null()) {
        return;
    }
    let session_file = session_arg(&args).is_some();
    let mut built = fresh_window_session();
    if let Some(f) = session_arg(&args) {
        let Ok(t) = std::fs::read_to_string(f) else {
            _ = MessageBeep(MB_ICONWARNING);
            return;
        };
        built.take_layout(&Session::parse(&t));
    } else {
        apply_args(&mut built, args);
    }
    let mut tabs = built.view_tabs();
    if tabs.is_empty() {
        return;
    }
    if !session_file {
        if let Some(t) = tabs.first_mut() {
            if let Some(p) = t.paths.iter().find(|p| !p.is_empty()) {
                let n = short_name(p);
                if !n.is_empty() {
                    t.name = n;
                }
            }
        }
    }
    let pick = built.tab.min(tabs.len() - 1);
    let base = app(|a| a.s.tabs.len());
    for tab in tabs {
        app(|a| {
            fit_park(a);
            a.s.tabs.push(tab);
            a.park.push(no_panes());
        });
    }
    switch_tab(base + pick);
    note_save();
}

/// A second launch, with or without paths. Empty args are a blank window,
/// unless single-window mode is on: then the existing window comes forward.
pub(crate) unsafe fn open_in_new_window(args: Vec<String>) -> HWND {
    match inbound(app(|a| a.s.one_window), !args.is_empty()) {
        Inbound::Focus => {
            let h = app(|a| a.main);
            reveal_window(h);
            return h;
        }
        Inbound::Tabs => {
            let h = app(|a| a.main);
            reveal_window(h);
            open_args_as_tabs(args);
            return app(|a| a.main);
        }
        Inbound::NewWindow => {}
    }
    let mut s = fresh_window_session();
    if let Some(f) = session_arg(&args) {
        if let Ok(t) = std::fs::read_to_string(f) {
            s.take_layout(&Session::parse(&t));
            s.file = f.to_string();
        }
    } else if !args.is_empty() {
        apply_args(&mut s, args);
    }
    match open_window(s) {
        Ok(h) => {
            _ = SetForegroundWindow(h);
            note_save();
            h
        }
        Err(_) => HWND::default(),
    }
}

/// Saved windows other than the one already on screen. Called from a short timer so the first paint is not waiting on them.
pub(crate) unsafe fn open_stashed() {
    if !allows_another_window(app(|a| a.s.one_window)) {
        take_stashed();
        return;
    }
    let pending = take_stashed();
    if pending.is_empty() {
        return;
    }
    let base = app(|a| a.s.clone());
    let first = app(|a| a.main);
    for v in pending {
        let s = session_from_window(&v, &base);
        if open_window(s).is_err() {
            break;
        }
    }
    if !first.0.is_null() {
        bind_hwnd(first);
        _ = SetForegroundWindow(first);
    }
    note_save();
}

/// Pull tab `i` out of the current window. The last tab is replaced by an empty one so the window can close afterwards.
pub(crate) unsafe fn take_tab(i: usize) -> Option<Tab> {
    end_rename(true);
    let n = app(|a| a.s.tabs.len());
    if i >= n {
        return None;
    }
    if n == 1 {
        remember_tabs();
        let tab = app(|a| a.s.tabs.first().cloned())?;
        destroy_every_pane();
        let blank = fresh_window_session();
        app(|a| {
            let win = a.s.win;
            let side = a.s.side;
            a.s = blank;
            a.s.win = win;
            a.s.side = side;
            a.park = vec![no_panes()];
            a.park_lru.clear();
            a.zoom = None;
            a.panes = no_panes();
        });
        put_filters(&vec![String::new(); MAXP]);
        tabs_invalidate();
        return Some(tab);
    }
    let active = app(|a| a.s.tab);
    if i == active {
        remember_tabs();
        detach_live();
    }
    let (tab, doomed, filters, switched) = app(|a| {
        fit_park(a);
        let tab = a.s.tabs.remove(i);
        let mut parked = a.park.remove(i);
        lru_remove_shift(a, i);
        let was = a.s.tab;
        let switched = i == was;
        if switched {
            let j = i.min(a.s.tabs.len() - 1);
            a.s.tab = j;
            let next = std::mem::replace(&mut a.park[j], no_panes());
            lru_forget(a, j);
            parked = std::mem::replace(&mut a.panes, next);
            a.s.publish();
        } else if i < was {
            a.s.tab = was - 1;
        }
        let filters = a.s.tabs[a.s.tab].filters.clone();
        (tab, parked, filters, switched)
    });
    destroy_set(doomed);
    if switched {
        put_filters(&filters);
        attach_live();
        relayout();
        sync_filter_edits();
        redraw_all();
        focus_view(active_pane());
    }
    tabs_invalidate();
    Some(tab)
}

unsafe fn insert_tab(at: usize, tab: Tab) {
    let j = app(|a| {
        fit_park(a);
        let at = at.min(a.s.tabs.len());
        a.s.tabs.insert(at, tab);
        a.park.insert(at, no_panes());
        lru_insert_shift(a, at);
        if at <= a.s.tab {
            a.s.tab += 1;
        }
        at
    });
    switch_tab(j);
}

unsafe fn undo_take(source: HWND, only: bool, i: usize, tab: Tab) {
    bind_hwnd(source);
    if only {
        app(|a| {
            a.s.tabs = vec![tab];
            a.s.tab = 0;
            a.s.publish();
            a.park = vec![no_panes()];
            a.park_lru.clear();
        });
        relayout();
        tabs_invalidate();
    } else {
        insert_tab(i, tab);
    }
}

pub(crate) unsafe fn move_tab_to_new(i: usize) {
    if !allows_another_window(app(|a| a.s.one_window)) {
        // The new window's content opens as another tab here. The original stays.
        duplicate_tab(i);
        return;
    }
    let source = app(|a| a.main);
    let only = app(|a| a.s.tabs.len() <= 1);
    let Some(tab) = take_tab(i) else { return };
    let mut s = fresh_window_session();
    s.tabs = vec![tab.clone()];
    s.tab = 0;
    s.publish();
    if open_window(s).is_err() {
        undo_take(source, only, i, tab);
        return;
    }
    if only {
        _ = PostMessageW(Some(source), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    note_save();
}

pub(crate) unsafe fn move_tab_to_window(i: usize, dest_tabs: HWND, at: usize) {
    let source_tabs = app(|a| a.tabs_hwnd);
    if dest_tabs == source_tabs {
        return;
    }
    let source = app(|a| a.main);
    let only = app(|a| a.s.tabs.len() <= 1);
    let Some(tab) = take_tab(i) else { return };
    if !bind_hwnd(dest_tabs) {
        undo_take(source, only, i, tab);
        return;
    }
    insert_tab(at, tab);
    if only {
        _ = PostMessageW(Some(source), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    note_save();
}

pub(crate) unsafe fn tab_strip_at(screen: POINT) -> Option<(HWND, usize)> {
    let n = window_count();
    for i in 0..n {
        let (hwnd, dpi, locked) = app(|a| {
            let d = &a.desks[i];
            let locked: Vec<bool> = d.s.tabs.iter().map(|t| t.locked).collect();
            (d.tabs_hwnd, d.dpi, locked)
        });
        if hwnd.0.is_null() {
            continue;
        }
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            continue;
        }
        if screen.x < rc.left || screen.x >= rc.right || screen.y < rc.top || screen.y >= rc.bottom {
            continue;
        }
        let mut client = RECT::default();
        _ = GetClientRect(hwnd, &mut client);
        let mut pt = screen;
        _ = ScreenToClient(hwnd, &mut pt);
        let lay = tab_lay(client.right.max(1), locked.len().max(1), &locked, dpi);
        let at = lay.body.iter().position(|r| pt.x < (r.left + r.right) / 2).unwrap_or(lay.body.len());
        return Some((hwnd, at));
    }
    None
}

pub(crate) unsafe fn finish_tab_drag(strip: HWND, index: usize) {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return;
    }
    if let Some((dest, at)) = tab_strip_at(pt) {
        if dest != strip {
            move_tab_to_window(index, dest, at);
        }
        return;
    }
    let mut rc = RECT::default();
    if GetWindowRect(strip, &mut rc).is_ok() && pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom {
        return;
    }
    move_tab_to_new(index);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_window_decides_launch_spawn_and_hotkey() {
        assert_eq!(launch_plan(false, false), LaunchPlan::NewProcess);
        assert_eq!(launch_plan(true, false), LaunchPlan::NewWindow);
        assert_eq!(launch_plan(false, true), LaunchPlan::FocusTabs);
        assert_eq!(launch_plan(true, true), LaunchPlan::FocusTabs);
        assert_eq!(inbound(false, false), Inbound::NewWindow);
        assert_eq!(inbound(false, true), Inbound::NewWindow);
        assert_eq!(inbound(true, false), Inbound::Focus);
        assert_eq!(inbound(true, true), Inbound::Tabs);
        assert_eq!(spawn_kind(false), Spawn::Window);
        assert_eq!(spawn_kind(true), Spawn::Tab);
        assert!(allows_another_window(false));
        assert!(!allows_another_window(true));
        assert_eq!(hotkey_when_front(false), HotkeyWhenFront::Minimize);
        assert_eq!(hotkey_when_front(true), HotkeyWhenFront::NewTab);
        assert!(!Session::default().one_window);
        let mut on = Session::default();
        on.one_window = true;
        assert!(Session::parse(&on.to_ini()).one_window);
        assert!(!Session::parse("single=0\n").one_window);
    }

    #[test]
    fn window_values_round_trip_each_grids_paths() {
        let mut a = Session::default();
        a.paths[0] = "C:\\one".into();
        a.side = 180;
        a.side_split = 0.25;
        a.win = Some([1, 2, 300, 200, 0]);
        a.accent = 0x112233;
        a.anchor();
        let mut b = Session::default();
        b.paths[0] = "D:\\two".into();
        b.side = 90;
        b.side_split = 0.75;
        b.win = Some([40, 50, 800, 600, 1]);
        b.file = "C:\\s\\b.grids".into();
        b.accent = 0xAABBCC;
        b.anchor();
        let snaps = vec![window_value(&a), window_value(&b)];
        let back_b = session_from_window(&snaps[1], &a);
        assert_eq!(back_b.paths[0], "D:\\two");
        assert_eq!(back_b.win, Some([40, 50, 800, 600, 1]));
        assert_eq!(back_b.side, 90);
        assert_eq!(back_b.side_split, 0.75);
        assert_eq!(back_b.file, "C:\\s\\b.grids");
        assert_eq!(back_b.accent, a.accent);
        assert_ne!(back_b.paths[0], a.paths[0]);
        let back_a = session_from_window(&snaps[0], &a);
        assert_eq!(back_a.paths[0], "C:\\one");
        assert_eq!(back_a.side_split, 0.25);
        assert_eq!(back_a.tabs[0].name, "1");
    }
}
