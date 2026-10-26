use crate::imports::*;
use crate::app::*;
use crate::session::*;
use crate::ui::*;
use super::{focus_view, pick_pane_tab, redraw_all, relayout, rename_fields, save_view, set_active, NAV_GEN, SEL_REV};

// Grab a pane by its header or pane-tab strip and drop it on another pane.
// The file list is not a grab: OLE drag-and-drop from the view is untouched.
// Window-tab reorder is a different window and a different capture.

fn drag_cursor() {
    unsafe {
        if let Ok(c) = LoadCursorW(None, IDC_SIZEALL) {
            SetCursor(Some(c));
        }
    }
}

fn overlay_hwnd(main: HWND) -> HWND {
    let cur = app(|a| a.pane_over);
    if !cur.0.is_null() && unsafe { IsWindow(Some(cur)).as_bool() } {
        return cur;
    }
    let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let h = unsafe { CreateWindowExW(ex, w!("PanegridBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None).unwrap_or_default() };
    if h.0.is_null() {
        return h;
    }
    // Same class brush as the splitter guides (the accent). Half opacity, so the files stay readable.
    unsafe { _ = SetLayeredWindowAttributes(h, COLORREF(0), 128, LWA_ALPHA) };
    app(|a| a.pane_over = h);
    h
}

fn hide_overlay() {
    let h = app(|a| a.pane_over);
    if !h.0.is_null() {
        unsafe { _ = ShowWindow(h, SW_HIDE) };
    }
}

fn show_overlay(main: HWND, rect: Option<RECT>) {
    let Some(r) = rect else {
        hide_overlay();
        return;
    };
    let h = overlay_hwnd(main);
    if h.0.is_null() {
        return;
    }
    let mut p = POINT { x: r.left, y: r.top };
    unsafe {
        _ = ClientToScreen(main, &mut p);
        let w = (r.right - r.left).max(1);
        let hgt = (r.bottom - r.top).max(1);
        _ = SetWindowPos(h, Some(HWND_TOP), p.x, p.y, w, hgt, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

fn hit_drop(a: &App) -> Option<(usize, DropZone, RECT)> {
    let from = a.pane_drag.as_ref()?.from;
    if a.zoom.is_some() {
        return None;
    }
    let mut pt = POINT::default();
    unsafe { _ = GetCursorPos(&mut pt) };
    let mut c = pt;
    unsafe { _ = ScreenToClient(a.main, &mut c) };
    let g = geo(a);
    let n = a.s.panes();
    let heights = &a.s.heights;
    let onto = (0..n).find(|&i| i != from && inside(cell_rect(&g, heights, i), c.x, c.y))?;
    let zone = pane_zone(cell_rect(&g, heights, onto), c.x, c.y)?;
    pane_drop(heights, &a.s.colr, &a.s.rowr, from, onto, zone)?;
    let cells: Vec<RECT> = (0..n).map(|i| cell_rect(&g, heights, i)).collect();
    let preview = drop_preview(heights, &cells, onto, zone)?;
    Some((onto, zone, preview))
}

fn track_pane_drag() {
    let placed = app(|a| {
        if a.pane_drag.is_none() {
            return None;
        }
        let hot = hit_drop(a);
        if let Some(d) = a.pane_drag.as_mut() {
            d.hot = hot.as_ref().map(|&(i, z, _)| (i, z));
        }
        Some((a.main, hot.map(|(_, _, r)| r)))
    });
    let Some((main, rect)) = placed else { return };
    show_overlay(main, rect);
}

/// True when this press is the start of a pane drag (header or strip, not a button or the list).
pub(crate) unsafe fn pane_drag_down(h: HWND, i: usize, x: i32, y: i32) -> bool {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let grab = app(|a| {
        // Splitter drag, zoom, and a window-tab press each already own the mouse.
        if a.drag.is_some() || a.zoom.is_some() || a.s.panes() < 2 || a.tab_drag.is_some() || a.tab_press.is_some() || a.pane_drag.is_some() {
            return None;
        }
        let n = a.s.pane_tab_list(i).len();
        pane_grab(&pane_geo(rc.right, rc.bottom, a.dpi, n > 1), n, x, y, a.dpi)
    });
    let Some(grab) = grab else { return false };
    app(|a| a.pane_press = Some(PanePress { hwnd: h, pane: i, x, y, grab }));
    SetCapture(h);
    true
}

pub(crate) unsafe fn pane_drag_move(h: HWND, x: i32, y: i32) {
    let started = app(|a| {
        let p = a.pane_press.as_ref()?;
        if p.hwnd != h {
            return None;
        }
        let slack = drag_slack(a.dpi);
        if (x - p.x).abs() + (y - p.y).abs() <= slack {
            return None;
        }
        let pane = p.pane;
        a.pane_press = None;
        a.pane_drag = Some(PaneMove { hwnd: h, from: pane, hot: None });
        Some(())
    });
    if started.is_some() {
        drag_cursor();
    }
    if app(|a| a.pane_drag.as_ref().is_some_and(|d| d.hwnd == h)) {
        drag_cursor();
        track_pane_drag();
    }
}

/// Drop, or a click that never crossed the slack. True when this window owned the press.
pub(crate) unsafe fn pane_drag_up(h: HWND) -> bool {
    if app(|a| a.pane_drag.as_ref().is_some_and(|d| d.hwnd == h)) {
        track_pane_drag();
    }
    let (drag, press) = app(|a| {
        let mine = a.pane_drag.as_ref().is_some_and(|d| d.hwnd == h) || a.pane_press.as_ref().is_some_and(|p| p.hwnd == h);
        if !mine {
            return (None, None);
        }
        (a.pane_drag.take(), a.pane_press.take())
    });
    if drag.is_none() && press.is_none() {
        return false;
    }
    hide_overlay();
    _ = ReleaseCapture();
    if let Some(d) = drag {
        if let Some((onto, zone)) = d.hot {
            commit_pane_move(d.from, onto, zone);
        }
        return true;
    }
    if let Some(p) = press {
        // The press already made this pane active (pre_translate). A tab or the empty strip
        // still has to do what a click did before the drag threshold existed.
        match p.grab {
            PaneGrab::Tab(k) => {
                pick_pane_tab(p.pane, k);
                focus_view(p.pane);
            }
            PaneGrab::Strip => focus_view(p.pane),
            PaneGrab::Header => {}
        }
    }
    true
}

/// Esc, or a capture stolen by something else. False when nothing was being dragged.
pub(crate) unsafe fn cancel_pane_drag() -> bool {
    let had = app(|a| {
        let had = a.pane_drag.is_some() || a.pane_press.is_some();
        a.pane_drag = None;
        a.pane_press = None;
        had
    });
    if !had {
        return false;
    }
    hide_overlay();
    _ = ReleaseCapture();
    true
}

/// Move the live panes, then the active tab's grid. Other tabs and other windows stay as they are.
unsafe fn commit_pane_move(from: usize, onto: usize, zone: DropZone) {
    let Some(plan) = app(|a| a.zoom.is_none().then(|| pane_drop(&a.s.heights, &a.s.colr, &a.s.rowr, from, onto, zone)).flatten()) else {
        return;
    };
    app(|a| a.focus_req = HWND::default());
    let n = app(|a| a.s.panes().min(MAXP));
    for i in 0..n {
        save_view(i);
    }
    let perm = plan.perm;
    let slots = app(|a| perm.map(|k| a.panes.get(k).and_then(|p| p.as_ref()).map(|p| p.slot.clone())));
    {
        let mut f = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
        let old = f.clone();
        for (j, &k) in perm.iter().enumerate() {
            f[j] = old[k].clone();
            if let Some(s) = &slots[j] {
                s.store(j, Ordering::SeqCst);
            }
        }
    }
    let revs = SEL_REV.each_ref().map(|r| r.load(Ordering::Relaxed));
    for (j, &k) in perm.iter().enumerate() {
        SEL_REV[j].store(revs[k], Ordering::Relaxed);
    }
    let focus = app(|a| {
        let old_f = a.live_filters.clone();
        for (j, &k) in perm.iter().enumerate() {
            a.live_filters[j] = old_f[k].clone();
        }
        let mut taken: Vec<Option<Pane>> = a.panes.iter_mut().map(Option::take).collect();
        a.panes = perm.iter().map(|&k| taken.get_mut(k).and_then(Option::take)).collect();
        let mut base = a.base.take();
        permute_in(&mut a.s, base.as_mut(), &perm);
        a.base = base;
        let tab = a.s.tab;
        let order: [u8; MAXP] = std::array::from_fn(|i| perm[i] as u8);
        if let Some(b) = a.tab_base.get_mut(tab) {
            reorder_tab_slots(&mut b.grid, &order);
        }
        let to = |k: &mut usize| *k = perm.iter().position(|&p| p == *k).unwrap_or(*k);
        if let Some(d) = a.drop_open.as_mut() {
            to(&mut d.1);
        }
        if let Some(m) = a.matcher.as_mut() {
            to(&mut m.pane);
        }
        if let Some(k) = a.kids.as_mut() {
            to(&mut k.i);
        }
        to(&mut a.peek_pane);
        a.zoom = None;
        a.status_key.clear();
        a.s.heights = plan.heights;
        a.s.colr = plan.colr;
        a.s.rowr = plan.rowr;
        let count = a.s.panes().max(1);
        a.s.active = plan.focus.min(count - 1);
        // The active tab stores this grid. A later switch or save reads it back.
        a.s.anchor();
        let _ = pull_dirty(a);
        a.s.active
    });
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    relayout();
    rename_fields();
    redraw_all();
    refresh_save_button();
    note_save();
    set_active(focus);
    focus_view(focus);
}
