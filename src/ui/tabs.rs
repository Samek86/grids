use crate::imports::*;
use super::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;

// Segoe Fluent Icons / MDL2 Assets code points
pub(crate) const ICON_SEARCH: &str = "\u{E721}";
pub(crate) const ICON_FILTER: &str = "\u{E71C}";
pub(crate) const ICON_LOCK: &str = "\u{E72E}";
pub(crate) const ICON_CLOSE: &str = "\u{E711}";
pub(crate) const ICON_PLUS: &str = "\u{E710}";
/// Tab stripe colours, 0xRRGGBB. Chosen so a thin bar and a light tint read on both themes.
pub(crate) const TAB_COLORS: [(u32, &str, &str, &str); 8] = [
    (0xE15A4A, "Red", "빨강", "赤"),
    (0xE8943A, "Orange", "주황", "オレンジ"),
    (0xC49212, "Yellow", "노랑", "黄"),
    (0x3EAE6E, "Green", "초록", "緑"),
    (0x2A9D9A, "Teal", "청록", "ティール"),
    (0x3D7EE0, "Blue", "파랑", "青"),
    (0x8A5CD0, "Purple", "보라", "紫"),
    (0xD45A8C, "Pink", "분홍", "ピンク"),
];
pub(crate) const CLOSED_KEEP: usize = 10;
pub(crate) const ID_TAB_EDIT: i32 = 208;

pub(crate) fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_TB_LAYOUT => "\u{E8A9}",
        ID_TB_SAVE => "\u{E74E}",
        ID_TB_LOAD => "\u{E838}",
        ID_TB_FAV => "\u{E734}",
        ID_TB_SET => "\u{E713}",
        ID_TB_THEME if DARK_ON.load(Ordering::Relaxed) => "\u{E706}", // sun: back to light
        ID_TB_THEME => "\u{E708}",                                     // moon: go dark
        _ => "\u{E890}",
    }
}

// ---------- tabs ----------
// The strip is its own child so splitter hit-testing stays below it. Every hidden tab keeps its
// grid on the session. Only the most recently hidden tab also keeps its Explorer windows.

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TabHit {
    Tab(usize),
    Close(usize),
    Plus,
    Empty,
}

pub(crate) struct TabLay {
    pub(crate) body: Vec<RECT>,
    pub(crate) close: Vec<Option<RECT>>,
    pub(crate) plus: RECT,
}

pub(crate) fn tab_lay(width: i32, n: usize, locked: &[bool], dpi: i32) -> TabLay {
    let s = |v: i32| v * dpi / 96;
    let n = n.max(1);
    let (pad, gap, plus_w, max_w, min_w) = (s(6), s(2), s(28), s(90), s(28));
    let mut w = max_w;
    let used = |w: i32| n as i32 * w + gap * (n as i32 - 1) + plus_w + pad * 2;
    if used(w) > width {
        w = ((width - pad * 2 - plus_w - gap * (n as i32 - 1)) / n as i32).max(min_w);
    }
    let y = s(4);
    let h = s(26);
    let mut x = pad;
    let mut body = Vec::new();
    let mut close = Vec::new();
    for i in 0..n {
        let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
        let cw = s(16);
        let c = if locked.get(i).copied().unwrap_or(false) {
            None
        } else {
            Some(RECT {
                left: r.right - s(4) - cw,
                top: r.top + (h - cw) / 2,
                right: r.right - s(4),
                bottom: r.top + (h - cw) / 2 + cw,
            })
        };
        body.push(r);
        close.push(c);
        x += w + gap;
    }
    TabLay { body, close, plus: RECT { left: x, top: y, right: x + plus_w, bottom: y + h } }
}

pub(crate) fn in_rect(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub(crate) fn tab_hit(lay: &TabLay, x: i32, y: i32) -> TabHit {
    if in_rect(lay.plus, x, y) {
        return TabHit::Plus;
    }
    for (i, r) in lay.body.iter().enumerate() {
        if in_rect(*r, x, y) {
            if lay.close[i].is_some_and(|c| in_rect(c, x, y)) {
                return TabHit::Close(i);
            }
            return TabHit::Tab(i);
        }
    }
    TabHit::Empty
}

pub(crate) fn put_filters(f: &[String]) {
    if let Ok(mut g) = FILTERS.lock() {
        for i in 0..MAXP {
            g[i] = f.get(i).cloned().unwrap_or_default();
        }
    }
    unsafe { sync_filter_hwnds() };
}

pub(crate) fn fit_park(a: &mut App) {
    let n = a.s.tabs.len();
    if a.park.len() < n {
        a.park.resize_with(n, no_panes);
    } else if a.park.len() > n {
        a.park.truncate(n);
    }
    let npark = a.park.len();
    a.park_lru.retain(|&i| i < npark);
}

pub(crate) fn lru_note(a: &mut App, i: usize) {
    a.park_lru.retain(|&x| x != i);
    a.park_lru.push(i);
}

pub(crate) fn lru_forget(a: &mut App, i: usize) {
    a.park_lru.retain(|&x| x != i);
}

pub(crate) fn lru_insert_shift(a: &mut App, at: usize) {
    for x in &mut a.park_lru {
        if *x >= at {
            *x += 1;
        }
    }
}

pub(crate) fn lru_remove_shift(a: &mut App, at: usize) {
    a.park_lru.retain(|&x| x != at);
    for x in &mut a.park_lru {
        if *x > at {
            *x -= 1;
        }
    }
}

pub(crate) fn park_has_browsers(set: &[Option<Pane>]) -> bool {
    set.iter().any(|p| p.is_some())
}

/// Drop hidden Explorer windows past PARK_CACHE. Their folders, columns and filters stay on the tab.
pub(crate) unsafe fn evict_park() {
    loop {
        let victim = app(|a| {
            let live = a.park.iter().filter(|s| park_has_browsers(s)).count();
            if live <= PARK_CACHE {
                return None;
            }
            a.park_lru.iter().copied().find(|&i| a.park.get(i).is_some_and(|s| park_has_browsers(s))).or_else(|| {
                a.park.iter().position(|s| park_has_browsers(s))
            })
        });
        let Some(i) = victim else { break };
        let set = app(|a| {
            lru_forget(a, i);
            if i < a.park.len() {
                std::mem::replace(&mut a.park[i], no_panes())
            } else {
                no_panes()
            }
        });
        destroy_set(set);
    }
}

pub(crate) unsafe fn pull_filters() -> Vec<String> {
    let (n, stored) = app(|a| {
        let stored = a.s.tabs.get(a.s.tab).map(|t| t.filters.clone()).unwrap_or_default();
        (a.s.panes(), stored)
    });
    let mut f = pad_filters(stored);
    for i in 0..n {
        if let Some(e) = pane_ctl(i, ID_FILTER) {
            f[i] = filter_spec(&text_of(e));
        }
    }
    put_filters(&f);
    f
}

/// Fold the live grid and filters into the active tab so a switch or a save keeps them.
pub(crate) unsafe fn remember_tabs() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        save_view(i);
    }
    let filters = pull_filters();
    app(|a| {
        let tab = a.s.tab;
        if tab < a.s.tabs.len() {
            a.s.tabs[tab].filters = pad_filters(filters);
        }
        a.s.anchor();
    });
}

pub(crate) unsafe fn detach_live() {
    for i in 0..MAXP {
        let got = app(|a| {
            a.panes[i].as_mut().map(|p| {
                let eb = p.eb.clone();
                let cookie = std::mem::take(&mut p.cookie);
                (eb, cookie, p.hwnd)
            })
        });
        if let Some((eb, cookie, hwnd)) = got {
            if cookie != 0 {
                _ = eb.Unadvise(cookie);
            }
            _ = IUnknown_SetSite(&eb, None);
            _ = KillTimer(Some(hwnd), TIMER_SETTLE);
            _ = KillTimer(Some(hwnd), TIMER_FILTER);
            _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub(crate) unsafe fn attach_live() {
    for i in 0..MAXP {
        let eb = app(|a| a.panes[i].as_ref().map(|p| p.eb.clone()));
        let Some(eb) = eb else { continue };
        let slot = app(|a| a.panes[i].as_ref().map(|p| p.slot.clone()));
        let Some(slot) = slot else { continue };
        slot.store(i, Ordering::SeqCst);
        let site: IServiceProvider = Host(slot.clone()).into();
        _ = IUnknown_SetSite(&eb, &site);
        let ev: IExplorerBrowserEvents = Events(slot).into();
        if let Ok(c) = eb.Advise(&ev) {
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.cookie = c;
                }
            });
        }
    }
}

pub(crate) unsafe fn sync_filter_edits() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        let spec = filter_of(i);
        if let Some(e) = pane_ctl(i, ID_FILTER) {
            if text_of(e) != spec {
                _ = SetWindowTextW(e, &HSTRING::from(spec.as_str()));
            }
        }
    }
}

pub(crate) unsafe fn destroy_set(set: Vec<Option<Pane>>) {
    if set.iter().all(|p| p.is_none()) {
        return;
    }
    let (back, keep) = app(|a| {
        let keep = FILTERS.lock().map(|f| f.clone()).unwrap_or_else(|_| [const { String::new() }; MAXP]);
        (std::mem::replace(&mut a.panes, set), keep)
    });
    // These browsers belong to a tab that is not on screen. Their grid is already on that tab.
    KEEP_VIEWS.with(|c| c.set(true));
    for i in 0..MAXP {
        destroy_pane(i);
    }
    KEEP_VIEWS.with(|c| c.set(false));
    app(|a| a.panes = back);
    put_filters(&keep);
}

pub(crate) unsafe fn destroy_every_pane() {
    for i in 0..MAXP {
        destroy_pane(i);
    }
    let sets = app(|a| a.park.drain(..).collect::<Vec<_>>());
    for set in sets {
        destroy_set(set);
    }
}

pub(crate) fn tabs_invalidate() {
    let h = app(|a| a.tabs_hwnd);
    if !h.0.is_null() {
        unsafe { _ = InvalidateRect(Some(h), None, false) };
    }
}

pub(crate) unsafe fn switch_tab(j: usize) {
    let (cur, n) = app(|a| (a.s.tab, a.s.tabs.len()));
    if j >= n || j == cur {
        return;
    }
    end_rename(true);
    remember_tabs();
    detach_live();
    let filters = app(|a| {
        fit_park(a);
        let live = std::mem::replace(&mut a.panes, no_panes());
        let tab = a.s.tab;
        a.park[tab] = live;
        lru_note(a, tab);
        a.panes = std::mem::take(&mut a.park[j]);
        if a.panes.len() != MAXP {
            a.panes = no_panes();
        }
        lru_forget(a, j);
        a.s.tab = j;
        a.s.publish();
        a.zoom = None;
        a.s.tabs[j].filters.clone()
    });
    put_filters(&filters);
    attach_live();
    relayout();
    sync_filter_edits();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    evict_park();
    note_save();
}

pub(crate) unsafe fn switch_rel(d: i32) {
    let (i, n) = app(|a| (a.s.tab as i32, a.s.tabs.len() as i32));
    if n <= 0 {
        return;
    }
    switch_tab((i + d).rem_euclid(n) as usize);
}

pub(crate) unsafe fn new_tab() {
    end_rename(true);
    remember_tabs();
    detach_live();
    app(|a| {
        fit_park(a);
        let live = std::mem::replace(&mut a.panes, no_panes());
        let tab = a.s.tab;
        a.park[tab] = live;
        lru_note(a, tab);
        let fresh = Session {
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            cols: vec![String::new(); MAXP],
            ..a.s.clone()
        };
        let name = next_tab_name(&a.s.tabs);
        a.s.tabs.push(Tab::from_live(&fresh, name, false, vec![String::new(); MAXP]));
        a.park.push(no_panes());
        a.s.tab = a.s.tabs.len() - 1;
        a.s.publish();
        a.zoom = None;
    });
    put_filters(&vec![String::new(); MAXP]);
    attach_live();
    relayout();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    evict_park();
    note_save();
}

pub(crate) unsafe fn duplicate_tab(i: usize) {
    end_rename(true);
    remember_tabs();
    let j = app(|a| {
        if i >= a.s.tabs.len() {
            return None;
        }
        fit_park(a);
        let mut t = a.s.tabs[i].clone();
        t.name = duplicate_name(&a.s.tabs, &t.name);
        t.locked = false;
        let j = (i + 1).min(a.s.tabs.len());
        a.s.tabs.insert(j, t);
        a.park.insert(j, no_panes());
        lru_insert_shift(a, j);
        if j <= a.s.tab {
            a.s.tab += 1;
        }
        Some(j)
    });
    if let Some(j) = j {
        switch_tab(j);
    }
}

pub(crate) unsafe fn close_tab(i: usize) {
    end_rename(true);
    if !app(|a| can_close_tab(&a.s.tabs, i)) {
        return;
    }
    let active = app(|a| a.s.tab);
    if i == active {
        remember_tabs();
        detach_live();
    }
    let (doomed, filters, switched) = app(|a| {
        fit_park(a);
        let tab = a.s.tabs.remove(i);
        let mut parked = a.park.remove(i);
        lru_remove_shift(a, i);
        a.closed.insert(0, tab);
        a.closed.truncate(CLOSED_KEEP);
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
        (parked, filters, switched)
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
    note_save();
}

pub(crate) unsafe fn close_other_tabs(keep: usize) {
    let victims = app(|a| close_other_indexes(&a.s.tabs, keep));
    for i in victims.into_iter().rev() {
        close_tab(i);
    }
}

pub(crate) unsafe fn reopen_closed_at(k: usize) {
    end_rename(true);
    let j = app(|a| {
        if k >= a.closed.len() {
            return None;
        }
        let t = a.closed.remove(k);
        fit_park(a);
        let j = (a.s.tab + 1).min(a.s.tabs.len());
        a.s.tabs.insert(j, t);
        a.park.insert(j, no_panes());
        lru_insert_shift(a, j);
        Some(j)
    });
    if let Some(j) = j {
        switch_tab(j);
    }
}

pub(crate) unsafe fn reorder_tab(from: usize, to: usize) {
    if from == to {
        return;
    }
    app(|a| {
        if from >= a.s.tabs.len() || to >= a.s.tabs.len() {
            return;
        }
        fit_park(a);
        reorder_vec(&mut a.s.tabs, from, to);
        reorder_vec(&mut a.park, from, to);
        a.park_lru = a.park_lru.iter().copied().map(|i| moved_active(i, from, to)).collect();
        a.s.tab = moved_active(a.s.tab, from, to);
    });
    tabs_invalidate();
    note_save();
}

pub(crate) unsafe fn end_rename(save: bool) {
    let (edit, which) = app(|a| {
        a.tab_rename_skip = !save;
        (a.tab_edit, a.tab_renaming.take())
    });
    if edit.0.is_null() || which.is_none() {
        return;
    }
    if save {
        if let Some(i) = which {
            let name = text_of(edit);
            app(|a| {
                if let Some(t) = a.s.tabs.get_mut(i) {
                    let name = name.trim();
                    if !name.is_empty() {
                        t.name = name.chars().take(80).collect();
                    }
                }
            });
        }
    }
    _ = ShowWindow(edit, SW_HIDE);
    tabs_invalidate();
    if save {
        note_save();
    }
}

pub(crate) unsafe fn begin_rename(i: usize) {
    let (edit, font, dpi, name, lay_w) = app(|a| {
        let mut rc = RECT::default();
        _ = GetClientRect(a.tabs_hwnd, &mut rc);
        let name = a.s.tabs.get(i).map(|t| t.name.clone()).unwrap_or_default();
        (a.tab_edit, a.font, a.dpi, name, rc.right)
    });
    if edit.0.is_null() || app(|a| i >= a.s.tabs.len()) {
        return;
    }
    let locked: Vec<bool> = app(|a| a.s.tabs.iter().map(|t| t.locked).collect());
    let lay = tab_lay(lay_w, locked.len(), &locked, dpi);
    let Some(r) = lay.body.get(i).copied() else { return };
    let s = |v: i32| v * dpi / 96;
    app(|a| a.tab_renaming = Some(i));
    _ = SetWindowTextW(edit, &HSTRING::from(name.as_str()));
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    _ = SetWindowPos(edit, None, r.left + s(6), r.top + s(3), (r.right - r.left - s(28)).max(s(24)), r.bottom - r.top - s(6), SWP_NOZORDER | SWP_SHOWWINDOW);
    _ = SetFocus(Some(edit));
    let len = GetWindowTextLengthW(edit) as usize;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(len as isize)));
}

pub(crate) unsafe fn set_tab_color(i: usize, color: Option<u32>) {
    app(|a| {
        if let Some(t) = a.s.tabs.get_mut(i) {
            t.color = color;
        }
    });
    tabs_invalidate();
    note_save();
}

pub(crate) unsafe fn tab_menu(i: usize, x: i32, y: i32) {
    let (hwnd, locked, others, color) = app(|a| {
        let tab = a.s.tabs.get(i);
        let locked = tab.is_some_and(|t| t.locked);
        let others = !close_other_indexes(&a.s.tabs, i).is_empty();
        (a.tabs_hwnd, locked, others, tab.and_then(|t| t.color))
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, text: &str, on: bool| {
        let flags = if on { MF_STRING } else { MF_STRING | MF_GRAYED };
        _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
    };
    add(1, tr("Rename tab", "이름 변경", "タブ名の変更"), true);
    add(2, if locked { tr("Unlock tab", "탭 잠금 해제", "タブのロックを解除") } else { tr("Lock tab", "탭 잠금", "タブをロック") }, true);
    add(3, tr("Duplicate tab", "탭 복제", "タブを複製"), true);
    add(6, tr("Move to new window", "새 창으로 이동", "新しいウィンドウへ移動"), true);
    add(4, tr("Close other tabs", "다른 탭 모두 닫기", "他のタブをすべて閉じる"), others);
    add(5, tr("Save this tab as session", "이 탭을 세션으로 저장", "このタブをセッションとして保存"), true);
    let Ok(colors) = CreatePopupMenu() else {
        _ = DestroyMenu(menu);
        return;
    };
    let mark = |menu: HMENU, id: usize, on: bool| {
        if on {
            _ = CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | MF_CHECKED).0);
        }
    };
    _ = AppendMenuW(colors, MF_STRING, 19, &HSTRING::from(tr("None", "없음", "なし")));
    mark(colors, 19, color.is_none());
    for (k, &(c, en, ko, ja)) in TAB_COLORS.iter().enumerate() {
        let id = 20 + k;
        _ = AppendMenuW(colors, MF_STRING, id, &HSTRING::from(tr(en, ko, ja)));
        mark(colors, id, color == Some(c));
    }
    _ = AppendMenuW(menu, MF_POPUP, colors.0 as usize, &HSTRING::from(tr("Tab color", "탭 색 지정", "タブの色")));
    let mut pt = POINT { x, y };
    _ = ClientToScreen(hwnd, &mut pt);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, hwnd, None).0 as usize;
    _ = DestroyMenu(menu);
    match pick {
        1 => begin_rename(i),
        2 => {
            app(|a| {
                if let Some(t) = a.s.tabs.get_mut(i) {
                    t.locked ^= true;
                }
            });
            tabs_invalidate();
            note_save();
        }
        3 => duplicate_tab(i),
        6 => move_tab_to_new(i),
        4 => close_other_tabs(i),
        5 => save_tab_as_session(i),
        19 => set_tab_color(i, None),
        n if (20..20 + TAB_COLORS.len()).contains(&n) => set_tab_color(i, Some(TAB_COLORS[n - 20].0)),
        _ => {}
    }
}

pub(crate) unsafe fn install_grids(s: Session) {
    destroy_every_pane();
    let filters = s.tabs.get(s.tab.min(s.tabs.len().saturating_sub(1))).map(|t| t.filters.clone()).unwrap_or_default();
    let n = s.tabs.len().max(1);
    put_filters(&filters);
    app(|a| {
        a.s = s;
        a.park = (0..n).map(|_| no_panes()).collect();
        a.park_lru.clear();
        a.zoom = None;
    });
}

pub(crate) fn current_tab_lay() -> TabLay {
    app(|a| {
        let mut rc = RECT::default();
        unsafe { _ = GetClientRect(a.tabs_hwnd, &mut rc) };
        let locked: Vec<bool> = a.s.tabs.iter().map(|t| t.locked).collect();
        tab_lay(rc.right.max(1), locked.len().max(1), &locked, a.dpi)
    })
}

pub(crate) unsafe fn paint_tabs(h: HWND) {
    let (active, names, locked, colors, hot, hot_close, hot_plus, dpi, font, icon, accent) = app(|a| {
        (
            a.s.tab,
            a.s.tabs.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            a.s.tabs.iter().map(|t| t.locked).collect::<Vec<_>>(),
            a.s.tabs.iter().map(|t| t.color).collect::<Vec<_>>(),
            a.tab_hot,
            a.tab_hot_close,
            a.tab_hot_plus,
            a.dpi,
            a.font,
            a.icon_font,
            a.s.accent_now(),
        )
    });
    let lay = current_tab_lay();
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        fill(dc, rc, pal().bg);
        fill(dc, RECT { top: rc.bottom - 1, ..rc }, pal().line);
        for (i, r) in lay.body.iter().enumerate() {
            let on = i == active;
            let over = hot == Some(i);
            let col = colors.get(i).copied().flatten();
            let bg = match (on, over, col) {
                (true, _, Some(c)) => blend(pal().card, c, 0.18),
                (true, _, None) => pal().card,
                (_, true, Some(c)) => blend(blend(pal().card, pal().bg, 0.45), c, 0.16),
                (_, true, None) => blend(pal().card, pal().bg, 0.45),
                (_, _, Some(c)) => blend(pal().bg, c, 0.18),
                _ => pal().bg,
            };
            fill(dc, *r, bg);
            if !on {
                frame(dc, *r, 1, pal().line);
            }
            if on {
                fill(dc, RECT { bottom: r.top + s(2), ..*r }, col.unwrap_or(accent));
                fill(dc, RECT { top: r.bottom - 1, bottom: r.bottom, ..*r }, pal().card);
            }
            if let Some(c) = col {
                let inset = if on { 0 } else { 1 };
                fill(dc, RECT { left: r.left + inset, top: r.top + inset, right: r.left + inset + s(4), bottom: r.bottom - inset }, c);
            }
            let lock = locked.get(i).copied().unwrap_or(false);
            let nudge = if col.is_some() { s(5) } else { 0 };
            let mut label = *r;
            if lock {
                let ir = RECT { left: r.left + nudge + s(4), top: r.top, right: r.left + nudge + s(20), bottom: r.bottom };
                text(dc, ir, ICON_LOCK, icon, if on { pal().text } else { pal().sub }, DT_CENTER);
                label.left = ir.right;
            } else {
                label.left += s(8) + nudge;
            }
            if let Some(c) = lay.close[i] {
                label.right = c.left;
                let ink = if hot_close && hot == Some(i) { accent } else { pal().sub };
                text(dc, c, ICON_CLOSE, icon, ink, DT_CENTER);
            } else {
                label.right -= s(8);
            }
            text(dc, label, names.get(i).map(|n| n.as_str()).unwrap_or(""), font, if on { pal().text } else { pal().sub }, DT_LEFT);
        }
        let plus_bg = if hot_plus { blend(pal().card, pal().bg, 0.45) } else { pal().bg };
        fill(dc, lay.plus, plus_bg);
        text(dc, lay.plus, ICON_PLUS, icon, pal().text, DT_CENTER);
    });
}

pub(crate) unsafe fn tab_mouse(x: i32, y: i32) -> TabHit {
    tab_hit(&current_tab_lay(), x, y)
}

pub(crate) extern "system" fn tabs_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_PAINT => paint_tabs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hit = tab_mouse(x, y);
                let (hot, close, plus) = match hit {
                    TabHit::Tab(i) => (Some(i), false, false),
                    TabHit::Close(i) => (Some(i), true, false),
                    TabHit::Plus => (None, false, true),
                    TabHit::Empty => (None, false, false),
                };
                let (from, paint) = app(|a| {
                    if let Some((pi, px, py, _)) = a.tab_press {
                        if a.tab_drag.is_none() && (x - px).abs() + (y - py).abs() > (4 * a.dpi / 96).max(4) {
                            a.tab_drag = Some(pi);
                        }
                    }
                    let from = a.tab_drag;
                    let same = a.tab_hot == hot && a.tab_hot_close == close && a.tab_hot_plus == plus;
                    if !same {
                        a.tab_hot = hot;
                        a.tab_hot_close = close;
                        a.tab_hot_plus = plus;
                    }
                    (from, !same)
                });
                if let Some(from) = from {
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    // Reorder only while the pointer stays in this strip. Outside, mouse-up moves the tab.
                    if y >= 0 && y < rc.bottom {
                        let lay = current_tab_lay();
                        let to = lay.body.iter().position(|r| x < (r.left + r.right) / 2).unwrap_or(lay.body.len().saturating_sub(1));
                        if to != from && to < app(|a| a.s.tabs.len()) {
                            reorder_tab(from, to);
                            app(|a| a.tab_drag = Some(to));
                        }
                    }
                }
                if paint || from.is_some() {
                    tabs_invalidate();
                }
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut t);
            }
            WM_MOUSELEAVE => {
                app(|a| {
                    a.tab_hot = None;
                    a.tab_hot_close = false;
                    a.tab_hot_plus = false;
                });
                tabs_invalidate();
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                end_rename(true);
                match tab_mouse(x, y) {
                    TabHit::Tab(i) | TabHit::Close(i) => {
                        let on_close = matches!(tab_mouse(x, y), TabHit::Close(_));
                        app(|a| a.tab_press = Some((i, x, y, on_close)));
                        SetCapture(h);
                        if !on_close {
                            switch_tab(i);
                        }
                    }
                    TabHit::Plus => new_tab(),
                    TabHit::Empty => {}
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let press = app(|a| a.tab_press.take());
                let dragged = app(|a| a.tab_drag.take());
                _ = ReleaseCapture();
                if let Some(from) = dragged {
                    finish_tab_drag(h, from);
                } else if let Some((i, _, _, on_close)) = press {
                    if on_close {
                        if let TabHit::Close(j) = tab_mouse(x, y) {
                            if j == i {
                                close_tab(i);
                            }
                        }
                    }
                }
            }
            WM_LBUTTONDBLCLK => {
                if let TabHit::Tab(i) = tab_mouse(xy(lp).0, xy(lp).1) {
                    begin_rename(i);
                }
            }
            WM_MBUTTONUP => {
                if let TabHit::Tab(i) | TabHit::Close(i) = tab_mouse(xy(lp).0, xy(lp).1) {
                    close_tab(i);
                }
            }
            WM_RBUTTONUP => {
                let (x, y) = xy(lp);
                let i = match tab_mouse(x, y) {
                    TabHit::Tab(i) | TabHit::Close(i) => i,
                    _ => app(|a| a.s.tab),
                };
                tab_menu(i, x, y);
            }
            WM_CAPTURECHANGED => {
                app(|a| {
                    a.tab_press = None;
                    a.tab_drag = None;
                });
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().card));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                (ID_TAB_EDIT, EN_KILLFOCUS) => {
                    let skip = app(|a| a.tab_rename_skip);
                    if !skip {
                        end_rename(true);
                    } else {
                        app(|a| a.tab_rename_skip = false);
                    }
                }
                _ => {}
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

