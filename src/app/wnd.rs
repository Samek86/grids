use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::shell::*;
use crate::i18n::*;
use crate::settings;

// ---------- window procs ----------

pub(crate) fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

/// Marks the splitter under the pointer. Only the old and new lines are repainted.
pub(crate) unsafe fn gap_hover(h: HWND, d: Option<Hit>) {
    let (old, lines) = app(|a| {
        let old = std::mem::replace(&mut a.gap_hot, d);
        let g = geo(a);
        (old, [old, d].map(|k| k.and_then(|k| gap_line(&g, k, 2 * a.dpi / 96))))
    });
    if old == d {
        return;
    }
    for r in lines.iter().flatten() {
        _ = InvalidateRect(Some(h), Some(r), false);
    }
    if old.is_none() && d.is_some() {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

pub(crate) unsafe fn cursor_for(h: HWND) -> Option<Hit> {
    let mut pt = POINT::default();
    _ = GetCursorPos(&mut pt);
    _ = ScreenToClient(h, &mut pt);
    app(|a| hit(&geo(a), pt.x, pt.y))
}

pub(crate) extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        if window_count() == 0 {
            return DefWindowProcW(h, msg, wp, lp);
        }
        match msg {
            // While the user drags the window edge, lay the panes out once at the end (same cost reason as splitters).
            WM_SIZE => {
                let kind = wp.0 as u32;
                let sizing = app(|a| {
                    if kind != SIZE_MINIMIZED {
                        a.was_max = kind == SIZE_MAXIMIZED;
                    }
                    a.sizing
                });
                if !sizing {
                    relayout();
                    redraw_all(); // maximize / restore
                }
                // Back from minimized: WM_ACTIVATE arrived while minimized and left focus alone.
                let f = GetFocus();
                if kind != SIZE_MINIMIZED && !app(|a| a.focus_wait) && GetActiveWindow() == h && (f.is_invalid() || f == h) {
                    restore_focus();
                }
            }
            WM_ENTERSIZEMOVE => {
                app(|a| a.sizing = true);
                kids_cancel(); // the '›' is about to move
            }
            WM_EXITSIZEMOVE => {
                app(|a| a.sizing = false);
                refresh_drives(); // one that came or went mid-resize
                relayout();
                redraw_all();
                note_save();
            }
            // A drive came or went. Only queued: no disk call while the system waits on the reply,
            // and TRUE grants a removal query.
            WM_DEVICECHANGE => {
                _ = PostMessageW(Some(h), WM_DRIVES, WPARAM(0), LPARAM(0));
                return LRESULT(1);
            }
            // A network drive or subst sends no device message. Look again when Grids comes forward.
            WM_ACTIVATEAPP => {
                if wp.0 != 0 {
                    _ = PostMessageW(Some(h), WM_DRIVES, WPARAM(0), LPARAM(0));
                }
            }
            WM_DRIVES => {
                if !app(|a| a.drag.is_some() || a.sizing) && refresh_drives() {
                    relayout();
                    redraw_all();
                }
            }
            WM_PAINT => paint_main(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_main covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_TIMER if wp.0 == TIMER_REVEAL => {
                _ = KillTimer(Some(h), TIMER_REVEAL);
                reveal_tree_selection();
                let left = app(|a| {
                    a.reveal_tries = a.reveal_tries.saturating_sub(1);
                    a.reveal_tries
                });
                if left > 0 {
                    SetTimer(Some(h), TIMER_REVEAL, if left == 2 { 330 } else { 550 }, None);
                }
            }
            WM_TIMER if wp.0 == TIMER_STATUS => {
                update_status();
                check_dirty();
                retry_save();
            }
            WM_TIMER if wp.0 == TIMER_SAVE => {
                _ = KillTimer(Some(h), TIMER_SAVE);
                save_session();
            }
            WM_TIMER if wp.0 == TIMER_WINS => {
                _ = KillTimer(Some(h), TIMER_WINS);
                open_stashed();
            }
            WM_SETCURSOR if wp.0 == h.0 as usize && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let Some(d) = app(|a| a.drag).or_else(|| cursor_for(h)) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                let c = match d {
                    Hit::Side | Hit::Col(_) => IDC_SIZEWE,
                    Hit::Row(..) | Hit::SideSplit => IDC_SIZENS,
                };
                SetCursor(LoadCursorW(None, c).ok());
                return LRESULT(1);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| hit(&geo(a), x, y)) {
                    gap_hover(h, None);
                    app(|a| a.drag = Some(d));
                    SetCapture(h);
                }
            }
            WM_MOUSELEAVE => gap_hover(h, None),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if app(|a| a.drag.is_none()) {
                    gap_hover(h, app(|a| hit(&geo(a), x, y)));
                }
                let moved = app(|a| {
                    let d = a.drag?;
                    let g = geo(a);
                    let min = 60 * a.dpi / 96;
                    match d {
                        Hit::Side => a.s.side = x - g.gap / 2,
                        Hit::SideSplit => a.s.side_split = split_ratio_at(y - g.gap / 2, g.top, g.bot - g.top, g.gap, side_min(a.dpi)),
                        Hit::Col(k) => drag_ratio(&mut a.s.colr, k, x - g.gap / 2, g.gx, g.w - g.pad - g.gx, g.gap, min),
                        Hit::Row(c, k) => drag_ratio(&mut a.s.rowr[c], k, y - g.gap / 2, g.top, g.bot - g.top, g.gap, min),
                    }
                    a.s.side = kept_side(a.s.tree_on, a.s.side, geo(a).side);
                    Some(())
                });
                if moved.is_some() {
                    show_bars(); // panes stay put until release: relaying out 16 Explorer views per mouse move is ~120ms
                }
            }
            WM_LBUTTONUP => {
                _ = ReleaseCapture();
                end_drag();
            }
            WM_CAPTURECHANGED => end_drag(),
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                let d = app(|a| hit(&geo(a), x, y));
                app(|a| match d {
                    Some(Hit::Side) => a.s.side = 240 * a.dpi / 96,
                    Some(Hit::SideSplit) => a.s.side_split = 0.5,
                    Some(Hit::Col(_)) => a.s.colr = even(a.s.heights.len()),
                    Some(Hit::Row(c, _)) => a.s.rowr[c] = even(a.s.heights[c]),
                    None => {}
                });
                if matches!(d, Some(Hit::SideSplit)) {
                    note_save();
                }
                relayout();
                redraw_all();
            }
            WM_COMMAND => match wp.0 & 0xffff {
                id if id == ID_TB_LAYOUT as usize => open_grid(),
                id if id == ID_TB_SAVE as usize => save_current(),
                id if id == ID_TB_LOAD as usize => open_load(),
                id if id == ID_TB_FAV as usize => open_favs(),
                id if id == ID_TB_THEME as usize => {
                    let dark = app(|a| a.s.dark);
                    app(|a| a.s.theme = if dark { 1 } else { 2 });
                    apply_theme();
                    save_session();
                }
                ID_NEWFILE => create_text_file(),
                id if id == ID_TB_SET as usize => settings::open(h),
                id if id == ID_TB_VIEW as usize => popup_menu(app(|a| a.view_menu), ID_TB_VIEW),
                id if (ID_DRIVE as usize..ID_DRIVE as usize + 26).contains(&id) => {
                    let i = app(|a| a.s.active);
                    if navigate(i, &format!("{}:\\", (b'A' + (id - ID_DRIVE as usize) as u8) as char)) {
                        focus_view(i);
                    } else {
                        _ = MessageBeep(MB_ICONWARNING); // no medium, or gone since the button was drawn
                    }
                }
                ID_HIDDEN => toggle_hidden(),
                ID_PANEL => toggle_tree(),
                ID_GATHER => gather_explorer(),
                ID_HOTKEY => toggle_hotkey(),
                ID_FOLLOW => {
                    let follow = app(|a| {
                        if a.s.theme == 0 {
                            a.s.theme = if a.s.dark { 2 } else { 1 };
                            false
                        } else {
                            a.s.theme = 0;
                            true
                        }
                    });
                    publish_shared();
                    if follow {
                        _ = PostMessageW(Some(h), WM_THEME_SYNC, WPARAM(0), LPARAM(0));
                    }
                    save_session();
                    settings::redraw();
                }
                ID_EXT => {
                    let on = app(|a| {
                        a.s.ext ^= true;
                        a.s.ext
                    });
                    apply_ext(on);
                    save_session();
                    settings::redraw();
                }
                ID_DBLUP => {
                    app(|a| a.s.dblup ^= true);
                    settings::redraw();
                }
                ID_PANE_TABS => toggle_pane_tabs(),
                ID_SETTINGS => settings::open(h),
                ID_LANG_AUTO => set_language(Lang::Auto),
                ID_LANG_EN => set_language(Lang::En),
                ID_LANG_KO => set_language(Lang::Ko),
                ID_LANG_JA => set_language(Lang::Ja),
                ID_HELP => open_help(),
                ID_ABOUT => about(h),
                id @ ID_ACCENT..=1405 => set_accent(pal().accents[id - ID_ACCENT].1),
                ID_ACCENT_PICK => pick_accent(),
                ID_KEYS => toggle_keys(),
                _ => {}
            },
            WM_INITMENUPOPUP => {
                let (menu, dblup, pane_tabs, accent, lang, panel, ext, hot) = app(|a| (a.view_menu, a.s.dblup, a.s.pane_tabs_on, a.s.accent_now(), a.s.lang, a.s.tree_on, a.s.ext, a.hot_on));
                let check = |id: usize, on: bool| CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
                check(ID_HIDDEN, hidden());
                check(ID_PANEL, panel);
                check(ID_EXT, ext);
                check(ID_FOLLOW, app(|a| a.s.theme == 0));
                check(ID_HOTKEY, hot);
                if let Some(i) = crate::keys::index("app.front") {
                    let label = format!("{}\t{}", tr("Bring Grids to front from anywhere", "어디서나 Grids 불러오기", "どこからでも Grids を前面に"), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_HOTKEY as u32, MF_BYCOMMAND | MF_STRING, ID_HOTKEY as usize, &HSTRING::from(label));
                }
                if let Some(i) = crate::keys::index("view.panel") {
                    let label = format!("{}\t{}", tr("Left panel", "왼쪽 패널", "左のパネル"), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_PANEL as u32, MF_BYCOMMAND | MF_STRING, ID_PANEL as usize, &HSTRING::from(label));
                }
                if let Some(i) = crate::keys::index("view.gather") {
                    let label = format!(
                        "{}\t{}",
                        tr("Open folders from Explorer windows", "탐색기 창의 폴더 가져오기", "エクスプローラーのウィンドウからフォルダーを開く"),
                        crate::keys::chord_text(i)
                    );
                    _ = ModifyMenuW(menu, ID_GATHER as u32, MF_BYCOMMAND | MF_STRING, ID_GATHER as usize, &HSTRING::from(label));
                }
                check(ID_DBLUP, dblup);
                check(ID_PANE_TABS, pane_tabs);
                check(ID_LANG_AUTO, lang == Lang::Auto);
                check(ID_LANG_EN, lang == Lang::En);
                check(ID_LANG_KO, lang == Lang::Ko);
                check(ID_LANG_JA, lang == Lang::Ja);
                for (k, (_, c)) in pal().accents.iter().enumerate() {
                    check(ID_ACCENT + k, *c == accent);
                }
            }
            WM_COPYDATA => {
                let cd = &*(lp.0 as *const CopyData);
                if cd.dw != COPY_ID {
                    return LRESULT(0);
                }
                // copy first: the sender's buffer is only valid during this call
                let args = if cd.cb == 0 || cd.lp.is_null() {
                    Vec::new()
                } else {
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(cd.lp as *const u16, cd.cb as usize / 2));
                    text.split('\n').filter(|s| !s.is_empty()).map(String::from).collect()
                };
                open_in_new_window(args);
                return LRESULT(1);
            }
            WM_ACTIVATE => {
                // Back in front: refresh Explorer pins and favourite files if either group is open.
                // Debounced in request_pins. One read fills both lists.
                let (state, min) = ((wp.0 & 0xffff) as u32, (wp.0 >> 16) & 0xffff != 0);
                if state != WA_INACTIVE {
                    app(|a| a.last_front = a.cur);
                    request_pins(false);
                }
                // Keyboard activation restores focus at once. A click waits until that click is handled.
                if state == WA_INACTIVE {
                    keep_focus(h);
                } else if state == WA_ACTIVE && !min {
                    restore_focus();
                    return LRESULT(0);
                } else if state == WA_CLICKACTIVE && !min {
                    app(|a| a.focus_wait = true);
                    SetTimer(Some(h), TIMER_FOCUS, 10, None);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_MOUSEACTIVATE => {
                app(|a| a.click_nc = (lp.0 & 0xffff) as u32 != HTCLIENT);
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_SYSCOMMAND => {
                if (wp.0 as u32) & 0xfff0 == SC_MINIMIZE {
                    keep_focus(h);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_TIMER if wp.0 == TIMER_FOCUS => {
                if !GetCapture().is_invalid() || GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 {
                    return LRESULT(0);
                }
                _ = KillTimer(Some(h), TIMER_FOCUS);
                let nc = app(|a| {
                    a.focus_wait = false;
                    std::mem::take(&mut a.click_nc)
                });
                let f = GetFocus();
                if GetActiveWindow() == h && (f.is_invalid() || f == h) {
                    if nc {
                        restore_focus();
                    } else {
                        focus_view(app(|a| a.s.active));
                    }
                }
            }
            WM_DPICHANGED => {
                kids_cancel();
                let dpi = (wp.0 & 0xffff) as i32;
                app(|a| {
                    a.s.side = a.s.side * dpi / a.dpi;
                    a.dpi = dpi;
                });
                apply_font();
                fit_load();
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            // Restart or sign-out does not send WM_CLOSE. Saving here keeps the last folders.
            WM_ENDSESSION if wp.0 != 0 => save_session(),
            // Windows light/dark changed. Queued so nothing is drawn inside the broadcast.
            WM_SETTINGCHANGE => {
                if lp.0 != 0 {
                    let wide = PCWSTR(lp.0 as *const u16);
                    if wide.to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        _ = PostMessageW(Some(h), WM_THEME_SYNC, WPARAM(0), LPARAM(0));
                    }
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_THEME_SYNC => theme_sync(),
            WM_HOTKEY if wp.0 == HOTKEY_ID as usize => hotkey_pressed(),
            WM_CLOSE => {
                _ = KillTimer(Some(h), TIMER_SAVE);
                _ = KillTimer(Some(h), TIMER_WINS);
                if app(|a| a.s.autosave && !a.s.file.is_empty()) {
                    let path = app(|a| a.s.file.clone());
                    write_session(Path::new(&path));
                }
                publish_shared();
                let last = window_count() <= 1;
                if last {
                    // The last window stays in the file, so the next launch restores every window that was open.
                    save_session();
                    _ = DestroyWindow(h);
                } else {
                    destroy_every_pane();
                    app(|a| a.tree = None);
                    let handles = drop_current_desk();
                    save_session();
                    _ = DestroyWindow(h);
                    release_desk_handles(handles.0, handles.1, handles.2, handles.3, handles.4);
                }
            }
            WM_DESTROY => {
                release_hotkey_hwnd(h);
                _ = KillTimer(Some(h), TIMER_STATUS);
                _ = KillTimer(Some(h), TIMER_SAVE);
                _ = KillTimer(Some(h), TIMER_WINS);
                // A window that was not the last one already left the desk list in WM_CLOSE.
                if bind_hwnd(h) {
                    destroy_every_pane();
                    app(|a| a.tree = None);
                    let handles = drop_current_desk();
                    release_desk_handles(handles.0, handles.1, handles.2, handles.3, handles.4);
                }
                if window_count() == 0 {
                    PostQuitMessage(0);
                } else {
                    try_register_hotkey(app(|a| a.main));
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

pub(crate) extern "system" fn pane_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let nav = |flags: u32| {
            if let Some(i) = pane_of(h) {
                set_active(i);
                browse(i, flags);
                focus_view(i);
            }
        };
        match msg {
            WM_TIMER if wp.0 == TIMER_SETTLE => settle_cols(h),
            WM_TIMER if wp.0 == TIMER_NEWFILE => {
                let job = PENDING_FILE.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_file(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_FILE.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWFILE);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWFILE);
                }
            }
            WM_TIMER if wp.0 == TIMER_NEWF => {
                let job = PENDING_NEW.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_folder(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_NEW.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWF);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWF);
                }
            }
            WM_SIZE => layout_pane(h),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let at = pane_of(h).and_then(|i| crumb_at(h, i, x, y)).and_then(|hit| hit.map(|(k, ..)| k));
                crumb_hover(h, at);
            }
            WM_MOUSELEAVE => crumb_hover(h, None),
            WM_KIDS => kids_show(h, wp.0),
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(i) = pane_of(h) {
                    if pane_tab_mouse(h, i, x, y, false) {
                        return LRESULT(0);
                    }
                }
                let on_kind = {
                    let dpi = app(|a| a.dpi);
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    filter_icon_rect(pane_geo(rc.right, rc.bottom, dpi).filt, 24 * dpi / 96)
                        .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                };
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    if on_kind {
                        kind_menu(i);
                    } else {
                        crumb_click(h, i, x, y);
                    }
                }
            }
            WM_MBUTTONUP => {
                let (x, y) = xy(lp);
                if let Some(i) = pane_of(h) {
                    if pane_tab_mouse(h, i, x, y, true) {
                        return LRESULT(0);
                    }
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                if let Some(i) = pane_of(h) {
                    if on_pane_strip(h, x, y) {
                        let mut rc = RECT::default();
                        _ = GetClientRect(h, &mut rc);
                        let dpi = app(|a| a.dpi);
                        let n = app(|a| a.s.pane_tab_list(i).len());
                        // Empty strip: a new pane tab. On a tab this is just the second click.
                        if matches!(pane_tab_hit(&pane_geo(rc.right, rc.bottom, dpi), n, x, y, dpi), Some(PaneTabHit::Strip)) {
                            new_pane_tab(i);
                            focus_view(i);
                        } else {
                            pane_tab_mouse(h, i, x, y, false);
                        }
                        return LRESULT(0);
                    }
                }
                let dpi = app(|a| a.dpi);
                let addr = addr_rect(h, dpi);
                let on_addr = x >= addr.left && x < addr.right && y >= addr.top && y < addr.bottom;
                let on_kind = {
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    filter_icon_rect(pane_geo(rc.right, rc.bottom, dpi).filt, 24 * dpi / 96)
                        .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                };
                if !on_addr && !on_kind {
                    if let Some(i) = pane_of(h) {
                        set_active(i);
                        toggle_zoom();
                    }
                }
            }
            WM_PAINT => paint_pane(h),
            // The icon's square only. The edit and the crumbs stay where they are.
            WM_PANE_ICON => {
                let (changed, dpi) = app(|a| {
                    let p = a.panes.iter_mut().flatten().find(|p| p.hwnd == h);
                    let new = p.and_then(|p| take_icon(p.ticket, wp.0 as u32, lp.0 as i32).filter(|&n| n != p.icon).map(|n| p.icon = n));
                    (new.is_some(), a.dpi)
                });
                if let Some(r) = addr_icon(addr_rect(h, dpi), dpi).filter(|_| changed) {
                    _ = InvalidateRect(Some(h), Some(&r), false);
                }
            }
            // folder dropped on the top bar: still this pane's? then open it here, as ⇄ does
            WM_DROP_OPEN => {
                let queued = app(|a| {
                    let mine = a.drop_open.as_ref().is_some_and(|(w, i, _)| *w == h && a.panes[*i].as_ref().is_some_and(|p| p.hwnd == h));
                    if mine {
                        a.drop_open.take()
                    } else {
                        None
                    }
                });
                if let Some((_, i, item)) = queued {
                    browse_item(i, &item);
                    set_active(i);
                    focus_view(i);
                }
            }
            WM_ERASEBKGND => return LRESULT(1), // paint_pane covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            // borderless edits sit on the field colour
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().pill));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().pill));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                // pill gets an accent outline while its edit has focus
                (ID_EDIT, EN_SETFOCUS | EN_KILLFOCUS) => sync_addr(h),
                (ID_FILTER, EN_SETFOCUS) => {
                    _ = InvalidateRect(Some(h), None, false);
                    _ = KillTimer(Some(h), TIMER_FIND);
                    if let Some(i) = pane_of(h) {
                        find_retarget(i);
                    }
                }
                (ID_FILTER, EN_KILLFOCUS) => {
                    _ = InvalidateRect(Some(h), None, false);
                    SetTimer(Some(h), TIMER_FIND, 120, None);
                }
                (ID_BACK, _) => nav(SBSP_NAVIGATEBACK),
                (ID_FWD, _) => nav(SBSP_NAVIGATEFORWARD),
                (ID_UP, _) => nav(SBSP_PARENT),
                (ID_OPEN_IN, _) => {
                    if let Some(i) = pane_of(h) {
                        open_in_menu(h, i);
                    }
                }
                (ID_FAV, _) => {
                    if let Some(i) = pane_of(h) {
                        toggle_fav(i);
                    }
                }
                // typing in the filter box applies after a short pause
                (ID_FILTER, EN_CHANGE) => {
                    if let Some(i) = pane_of(h) {
                        find_cancel(i);
                    }
                    _ = SetTimer(Some(h), TIMER_FILTER, 250, None);
                }
                _ => {}
            },
            WM_TIMER if wp.0 == TIMER_FILTER => {
                _ = KillTimer(Some(h), TIMER_FILTER);
                if let Some(i) = pane_of(h) {
                    apply_filter(i);
                    refresh_find(i);
                }
            }
            WM_TIMER if wp.0 == TIMER_FIND => {
                _ = KillTimer(Some(h), TIMER_FIND);
                let focus = GetFocus();
                let list = GetDlgItem(Some(h), ID_FILTER).ok();
                let on_filter = list.is_some_and(|e| e == focus);
                let on_list = focus_is_find(h, focus);
                if !on_filter && !on_list {
                    if let Some(i) = pane_of(h) {
                        hide_find(i);
                    }
                }
            }
            WM_FIND => on_find_msg(h),
            // right-click on ← / →: recent folders; anywhere else as before
            WM_CONTEXTMENU if [ID_BACK, ID_FWD].iter().any(|&id| GetDlgItem(Some(h), id).is_ok_and(|b| b.0 as usize == wp.0)) => {
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    recent_menu(i);
                }
            }
            // Right-click on the header itself (not its buttons or fields): colour label.
            WM_CONTEXTMENU if wp.0 == h.0 as usize => {
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: (lp.0 >> 16 & 0xffff) as i16 as i32 };
                let screen = p;
                _ = ScreenToClient(h, &mut p);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let pg = pane_geo(rc.right, rc.bottom, app(|a| a.dpi));
                // The pane-tab strip is not the header: a right-click there does not open the colour menu.
                let header_bottom = if pg.tabs.bottom > pg.tabs.top { pg.tabs.top } else { pg.view.top };
                match pane_of(h) {
                    Some(i) if lp.0 != -1 && p.y < header_bottom => tint_menu(h, i, screen),
                    _ => return DefWindowProcW(h, msg, wp, lp),
                }
            }
            // mouse side buttons
            WM_APPCOMMAND => match (lp.0 >> 16) & 0x0fff {
                1 => nav(SBSP_NAVIGATEBACK),
                2 => nav(SBSP_NAVIGATEFORWARD),
                _ => return DefWindowProcW(h, msg, wp, lp),
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- Win+Alt+E, or whatever chord the keymap now has ----------

pub(crate) const HOTKEY_ID: i32 = 1;

fn hotkey_busy(chord: &str) -> String {
    match ui() {
        1 => format!("{chord}는 다른 앱이 이미 쓰고 있음"),
        2 => format!("{chord} は他のアプリが使用中です"),
        _ => format!("{chord} is already used by another app"),
    }
}

/// Registers or drops the global hotkey. A bare key is refused: it would steal typing in every app.
pub(crate) unsafe fn set_hotkey(main: HWND, on: bool) -> bool {
    if main.0.is_null() {
        return false;
    }
    if !on {
        _ = UnregisterHotKey(Some(main), HOTKEY_ID);
        return false;
    }
    let Some(chord) = crate::keys::hot_chord() else { return false };
    if chord.digits || (!chord.ctrl && !chord.alt && !chord.shift && !chord.win) {
        return false;
    }
    let mut mods = MOD_NOREPEAT;
    if chord.ctrl {
        mods |= MOD_CONTROL;
    }
    if chord.alt {
        mods |= MOD_ALT;
    }
    if chord.shift {
        mods |= MOD_SHIFT;
    }
    if chord.win {
        mods |= MOD_WIN;
    }
    RegisterHotKey(Some(main), HOTKEY_ID, mods, chord.vk as u32).is_ok()
}

/// Startup and a window taking over after the owner closes. A failure leaves the wish on.
pub(crate) unsafe fn try_register_hotkey(main: HWND) {
    if main.0.is_null() || app(|a| !a.s.hotkey || a.hot_on) {
        return;
    }
    let on = set_hotkey(main, true);
    app(|a| {
        a.hot_on = on;
        a.hot_hwnd = if on { main } else { HWND::default() };
    });
}

/// After a rebind or a reset. The wish stays on when the new chord cannot be registered.
pub(crate) unsafe fn apply_hotkey(notify: bool) {
    let (want, old, main) = app(|a| (a.s.hotkey, a.hot_hwnd, a.main));
    if !old.0.is_null() {
        _ = UnregisterHotKey(Some(old), HOTKEY_ID);
    }
    let target = if old.0.is_null() { main } else { old };
    let on = want && set_hotkey(target, true);
    app(|a| {
        a.hot_on = on;
        a.hot_hwnd = if on { target } else { HWND::default() };
    });
    if notify && want && !on {
        let chord = crate::keys::index("app.front").map(crate::keys::chord_text).unwrap_or_default();
        app(|a| {
            a.note = Some((hotkey_busy(&chord), Instant::now() + std::time::Duration::from_secs(3)));
            a.status_key.clear();
        });
    }
}

pub(crate) unsafe fn release_hotkey_hwnd(h: HWND) {
    if app(|a| a.hot_hwnd == h) {
        _ = UnregisterHotKey(Some(h), HOTKEY_ID);
        app(|a| {
            a.hot_on = false;
            a.hot_hwnd = HWND::default();
        });
    }
}

/// The check shows what is really registered, so a failed start is unchecked and one click retries.
pub(crate) unsafe fn toggle_hotkey() {
    let (was, target) = app(|a| (a.hot_on, if a.hot_hwnd.0.is_null() { a.main } else { a.hot_hwnd }));
    let now = crate::keys::hotkey_flip(was, |on| set_hotkey(target, on));
    app(|a| {
        a.hot_on = now;
        a.hot_hwnd = if now { target } else { HWND::default() };
        a.s.hotkey = now;
        if !was && !now {
            let chord = crate::keys::index("app.front").map(crate::keys::chord_text).unwrap_or_default();
            a.note = Some((hotkey_busy(&chord), Instant::now() + std::time::Duration::from_secs(3)));
            a.status_key.clear();
        }
    });
    publish_shared();
    save_session();
}

/// Front, with the file list ready. Pressed again while Grids is in front: minimize.
/// A dialog that has disabled its owner comes forward instead.
pub(crate) unsafe fn hotkey_pressed() {
    let mains = app(|a| a.desks.iter().map(|d| d.main).filter(|h| !h.0.is_null()).collect::<Vec<_>>());
    let disabled = mains.into_iter().find(|h| !IsWindowEnabled(*h).as_bool());
    if let Some(main) = disabled {
        if IsIconic(main).as_bool() {
            _ = ShowWindow(main, SW_RESTORE);
        }
        let p = GetLastActivePopup(main);
        if !p.0.is_null() && p != main {
            _ = SetForegroundWindow(p);
        }
        return;
    }
    let fg = GetForegroundWindow();
    let root = GetAncestor(fg, GA_ROOTOWNER);
    let ours = app(|a| a.desks.iter().find(|d| d.main == root || d.main == fg).map(|d| d.main));
    if let Some(main) = ours {
        if !IsIconic(main).as_bool() {
            if hotkey_when_front(app(|a| a.s.one_window)) == HotkeyWhenFront::NewTab {
                bind_hwnd(main);
                new_tab();
            } else {
                _ = ShowWindow(main, SW_MINIMIZE);
            }
            return;
        }
    }
    let main = app(|a| a.desks.get(a.last_front).map(|d| d.main).filter(|h| !h.0.is_null()).unwrap_or(a.hot_hwnd));
    if main.0.is_null() {
        return;
    }
    if IsIconic(main).as_bool() {
        _ = ShowWindow(main, SW_RESTORE);
    }
    if SetForegroundWindow(main).as_bool() && GetForegroundWindow() == main {
        bind_hwnd(main);
        app(|a| a.last_front = a.cur);
        focus_view(app(|a| a.s.active));
    }
}

// The README on GitHub, in the UI language.
pub(crate) unsafe fn open_help() {
    let url = format!("{HOME}/blob/main/{}", tr("README.md", "README.ko.md", "README.md"));
    ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
}

pub(crate) unsafe fn about(owner: HWND) {
    let text = format!(
        "Grids {}\n{}\n\n{}\n{HOME}\n\n{}",
        env!("CARGO_PKG_VERSION"),
        tr("Fast multi-pane file manager for Windows", "Windows용 빠른 다중 패널 파일 관리자", "Windows 向けの高速マルチペイン ファイル マネージャー"),
        tr("MIT License · © 2026 efww", "MIT 라이선스 · © 2026 efww", "MIT ライセンス · © 2026 efww"),
        tr("Open the project page?", "프로젝트 페이지를 열까요?", "プロジェクト ページを開きますか?"),
    );
    if MessageBoxW(Some(owner), &HSTRING::from(text), &HSTRING::from(tr("About Grids", "Grids 정보", "Grids のバージョン情報")), MB_YESNO | MB_ICONINFORMATION) == IDYES {
        ShellExecuteW(None, w!("open"), &HSTRING::from(HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

// Toolbar buttons: id, accessible name, tooltip.
pub(crate) fn toolbar_tips() -> [(i32, &'static str, &'static str); 7] {
    [
        (ID_TB_LAYOUT, "", tr("Layout: click = even grid, drag = rows per column", "레이아웃: 클릭 = 균등, 끌기 = 열마다 칸 수", "レイアウト: クリック = 均等、ドラッグ = 列ごとの行数")),
        (ID_TB_SAVE, tr("Save session", "세션 저장", "セッションを保存"), ""), // tooltip set by refresh_save_button
        (ID_TB_LOAD, tr("Load", "불러오기", "読み込み"), tr("Load a session", "세션 불러오기", "セッションを読み込む")),
        (ID_TB_VIEW, tr("View", "보기", "表示"), tr("View: hidden files · colours · language · settings · help", "보기: 숨김 파일 · 선택 색 · 언어 · 설정 · 도움말", "表示: 隠しファイル · 色 · 言語 · 設定 · ヘルプ")),
        (ID_TB_FAV, tr("Favourites", "즐겨찾기", "お気に入り"), tr("Favourite folders (Ctrl+B; open in the active pane)", "즐겨찾기 폴더 (Ctrl+B, 선택한 패널에서 열기)", "お気に入りフォルダー (Ctrl+B、アクティブなペインで開く)")),
        (ID_TB_THEME, tr("Theme", "테마", "テーマ"), tr("Light / dark theme", "라이트 / 다크 테마", "ライト / ダーク テーマ")),
        (ID_TB_SET, tr("Settings", "설정", "設定"), tr("Settings", "설정", "設定")),
    ]
}

// Switches the UI language now: menus, fonts, toolbar tooltips, panes, and an open settings window.
pub(crate) unsafe fn set_language(lang: Lang) {
    UI.store(lang.resolve(), Ordering::Relaxed);
    app(|a| a.s.lang = lang);
    publish_shared();
    across_windows(|| {
        let (main, old) = app(|a| (a.main, a.view_menu));
        if main.0.is_null() {
            return;
        }
        if let Ok(m) = make_menus() {
            app(|a| a.view_menu = m);
            if !old.0.is_null() {
                _ = DestroyMenu(old);
            }
        }
        for (id, _, hint) in toolbar_tips() {
            if let Ok(b) = GetDlgItem(Some(main), id) {
                tip_text(b, hint, TTM_UPDATETIPTEXTW);
            }
        }
        refresh_drive_tips();
        refresh_save_button();
        rename_fields();
        app(|a| a.status_key.clear());
    });
    apply_font_all();
    apply_theme();
    save_session();
}

// The view popup opened from the top toolbar (no classic menu bar).
pub(crate) unsafe fn make_menus() -> Result<HMENU> {
    let view = CreatePopupMenu()?;
    let item = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| AppendMenuW(menu, flags, id, &HSTRING::from(text));
    item(view, MF_STRING, ID_HIDDEN, tr("Show hidden files\tCtrl+H", "숨김 파일 표시\tCtrl+H", "隠しファイルを表示\tCtrl+H"))?;
    item(view, MF_STRING, ID_PANEL, tr("Left panel\tCtrl+Shift+E", "왼쪽 패널\tCtrl+Shift+E", "左のパネル\tCtrl+Shift+E"))?;
    item(
        view,
        MF_STRING,
        ID_GATHER,
        tr(
            "Open folders from Explorer windows\tCtrl+Shift+O",
            "탐색기 창의 폴더 가져오기\tCtrl+Shift+O",
            "エクスプローラーのウィンドウからフォルダーを開く\tCtrl+Shift+O",
        ),
    )?;
    item(view, MF_STRING, ID_EXT, tr("Show file extensions", "파일 확장명 표시", "ファイルの拡張子を表示"))?;
    item(view, MF_STRING, ID_FOLLOW, tr("Follow Windows light / dark", "Windows 라이트·다크 설정 따르기", "Windows のライト / ダークに合わせる"))?;
    item(view, MF_STRING, ID_HOTKEY, &format!("{}\t{}", tr("Bring Grids to front from anywhere", "어디서나 Grids 불러오기", "どこからでも Grids を前面に"), crate::keys::chord_text(crate::keys::index("app.front").unwrap_or(0))))?;
    item(view, MF_STRING, ID_NEWFILE, tr("New text file", "새 텍스트 파일", "新しいテキスト ファイル"))?;
    item(view, MF_STRING, ID_DBLUP, tr("Double-click empty space: parent folder", "빈 곳 더블클릭: 위 폴더로", "空白をダブルクリック: 上のフォルダー"))?;
    item(view, MF_STRING, ID_PANE_TABS, tr("Tabs in panes", "패널 안의 탭", "ペイン内のタブ"))?;
    let colors = CreatePopupMenu()?;
    for (k, (name, _)) in LIGHT.accents.iter().enumerate() {
        item(colors, MF_STRING, ID_ACCENT + k, tr(ACCENT_EN[k], name, ACCENT_JA[k]))?;
    }
    AppendMenuW(colors, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(colors, MF_STRING, ID_ACCENT_PICK, tr("Custom...", "직접 선택...", "カスタム..."))?;
    item(view, MF_POPUP, colors.0 as usize, tr("Selection colour", "선택 색", "選択色"))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_SETTINGS, tr("Settings...", "설정...", "設定..."))?;
    let langs = CreatePopupMenu()?;
    item(langs, MF_STRING, ID_LANG_AUTO, tr("Follow Windows", "Windows 설정 따르기", "Windows の設定に従う"))?;
    item(langs, MF_STRING, ID_LANG_EN, "English")?;
    item(langs, MF_STRING, ID_LANG_KO, "한국어")?;
    item(langs, MF_STRING, ID_LANG_JA, "日本語")?;
    item(view, MF_POPUP, langs.0 as usize, tr("Language", "언어", "言語"))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_KEYS, tr("Keyboard shortcuts...", "단축키...", "ショートカット キー..."))?;
    item(view, MF_STRING, ID_HELP, tr("Help (online)\tF1", "도움말 (온라인)\tF1", "ヘルプ (オンライン)\tF1"))?;
    item(view, MF_STRING, ID_ABOUT, tr("About Grids", "Grids 정보", "Grids のバージョン情報"))?;
    Ok(view)
}

