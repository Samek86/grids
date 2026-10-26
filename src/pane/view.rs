use std::sync::Arc;

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::ui::*;
use crate::session::*;
use crate::i18n::*;
use crate::settings;
use windows::Win32::UI::Accessibility::{IAccPropServices, CLSID_AccPropServices, PROPID_ACC_NAME};
use windows::Win32::UI::Controls::{
    CDIS_FOCUS, LIST_VIEW_GROUP_STATE_FLAGS, LVCDI_GROUP, LVGA_HEADER_CENTER, LVGA_HEADER_RIGHT, LVGF_ALIGN, LVGF_DESCRIPTIONBOTTOM, LVGF_DESCRIPTIONTOP, LVGF_EXTENDEDIMAGE, LVGF_FOOTER, LVGF_HEADER,
    LVGF_STATE, LVGF_SUBTITLE, LVGF_TASK, LVGF_TITLEIMAGE, LVGGR_HEADER, LVGROUP, LVGROUP_MASK, LVGS_COLLAPSED, LVGS_COLLAPSIBLE, LVGS_FOCUSED, LVGS_SELECTED, LVGS_SUBSETED, LVM_GETGROUPINFO,
    LVM_GETGROUPRECT, LVM_GETIMAGELIST, LVSIL_GROUPHEADER,
};

// ---------- panes ----------

pub(crate) unsafe fn child(parent: HWND, class: PCWSTR, text: PCWSTR, style: u32, ex: WINDOW_EX_STYLE, id: i32, font: HFONT) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style);
    let menu = HMENU(id as isize as _);
    let h = CreateWindowExW(ex, class, text, style, 0, 0, 0, 0, Some(parent), Some(menu), None, None).unwrap_or_default();
    SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    h
}

pub(crate) unsafe fn create_pane(i: usize) -> Result<Pane> {
    let (main, font) = app(|a| (a.main, a.font));
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PanegridPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    let buttons = [
        (ID_BACK, tr("Back", "뒤로", "戻る"), tr("Back  (Alt+←) · right-click: recent folders", "뒤로  (Alt+←) · 오른쪽 클릭: 최근 폴더", "戻る  (Alt+←) · 右クリック: 最近のフォルダー")),
        (ID_FWD, tr("Forward", "앞으로", "進む"), tr("Forward  (Alt+→) · right-click: recent folders", "앞으로  (Alt+→) · 오른쪽 클릭: 최근 폴더", "進む  (Alt+→) · 右クリック: 最近のフォルダー")),
        (ID_UP, tr("Parent folder", "상위 폴더", "上のフォルダー"), tr("Parent folder  (Alt+↑)", "상위 폴더  (Alt+↑)", "上のフォルダー  (Alt+↑)")),
        (ID_OPEN_IN, tr("Open in another pane", "다른 패널에서 열기", "別のペインで開く"), tr("Open in / copy or move to another pane", "다른 패널에서 열기 / 복사·이동", "別のペインで開く / コピー・移動")),
        (ID_PANE_ADD, tr("New pane tab", "새 패널 탭", "新しいペイン タブ"), tr("New pane tab  (Ctrl+Alt+T)", "새 패널 탭  (Ctrl+Alt+T)", "新しいペイン タブ  (Ctrl+Alt+T)")),
        (ID_FAV, tr("Favourite", "즐겨찾기", "お気に入り"), tr("Add to favourites (filled star: remove)", "즐겨찾기에 추가 (채워진 별: 빼기)", "お気に入りに追加（塗りつぶしの星: 削除）")),
    ];
    for (id, name, tip) in buttons {
        add_tip(child(h, w!("BUTTON"), PCWSTR(HSTRING::from(name).as_ptr()), btn, plain, id, font), tip);
    }
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    let filt = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);
    _ = SetWindowSubclass(filt, Some(filter_hint_proc), 4, 0);
    name_fields(h, i);
    // Folder suggestions as you type, like Explorer's address bar; folders only, since navigate() only browses folders
    _ = SHAutoComplete(edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_AUTOAPPEND_FORCE_OFF);
    add_tip(
        filt,
        tr(
            "Search this folder (Ctrl+F) · icon or Alt+↓: file types",
            "이 폴더 검색 (Ctrl+F) · 아이콘 또는 Alt+↓: 파일 종류",
            "このフォルダーを検索 (Ctrl+F) · アイコンまたは Alt+↓: ファイルの種類",
        ),
    );

    let slot = Arc::new(AtomicUsize::new(i));
    let eb: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
    let site: IServiceProvider = Host(slot.clone()).into();
    IUnknown_SetSite(&eb, &site)?; // before Initialize, so the view picks up our ICommDlgBrowser
    // Details is the starting mode. Columns for a folder with nothing remembered are applied
    // on navigation: Name, Date modified, Size (see folderview::default_details_columns).
    let fs = FOLDERSETTINGS { ViewMode: FVM_DETAILS.0 as u32, fFlags: FWF_NOWEBVIEW.0 as u32 };
    eb.Initialize(h, &RECT::default(), Some(&fs))?;
    eb.SetOptions(EBO_NOBORDER)?;
    let ev: IExplorerBrowserEvents = Events(slot.clone()).into();
    let cookie = eb.Advise(&ev)?;
    let helper = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok(); // drag image over the bar
    let dt: IDropTarget = PaneDrop { slot: slot.clone(), hwnd: h, ok: Cell::new(false), helper }.into();
    _ = RegisterDragDrop(h, &dt);
    let pending = Some(app(|a| a.s.cols[i].clone())); // the session's, for the first folder
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new(), pending, settle: None, view_watch: None, recover: None, icon: -1, ticket: 0, slot })
}

const EM_GETMARGINS: u32 = 0xD4;

/// Screen readers otherwise only say "edit". Redone when the language changes or a pane changes slot.
pub(crate) unsafe fn name_fields(h: HWND, i: usize) {
    let pane = format!("{} {}", tr("Pane", "패널", "ペイン"), i + 1);
    a11y_name(GetDlgItem(Some(h), ID_EDIT).unwrap_or_default(), Some(&format!("{pane} {}", tr("address", "주소", "アドレス"))));
    a11y_name(GetDlgItem(Some(h), ID_FILTER).unwrap_or_default(), Some(&format!("{pane} {}", tr("search", "검색", "検索"))));
}

pub(crate) unsafe fn rename_fields() {
    let panes: Vec<(HWND, usize)> = app(|a| a.panes.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|p| (p.hwnd, i))).collect());
    for (h, i) in panes {
        name_fields(h, i);
        if let Ok(f) = GetDlgItem(Some(h), ID_FILTER) {
            _ = InvalidateRect(Some(f), None, true);
        }
    }
}

unsafe fn a11y_name(h: HWND, name: Option<&str>) {
    if h.0.is_null() {
        return;
    }
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = match name {
        Some(n) => acc.SetHwndPropStr(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_NAME, &HSTRING::from(n)),
        None => acc.ClearHwndProps(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, &[PROPID_ACC_NAME]),
    };
}

/// Empty, unfocused filter: "Name filter · Ctrl+F" in the field's own font. The edit itself stays.
unsafe extern "system" fn filter_hint_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let hint = msg == WM_PAINT && GetWindowTextLengthW(h) == 0 && GetFocus() != h;
    if hint {
        _ = InvalidateRect(Some(h), None, true);
    }
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_SETFOCUS || msg == WM_KILLFOCUS {
        _ = InvalidateRect(Some(h), None, true);
    } else if hint {
        let dc = GetDC(Some(h));
        let mut rc = RECT::default();
        _ = GetClientRect(h, &mut rc);
        rc.left += (SendMessageW(h, EM_GETMARGINS, None, None).0 & 0xffff) as i32;
        let font = SelectObject(dc, HGDIOBJ(SendMessageW(h, WM_GETFONT, None, None).0 as _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, cref(blend(pal().sub, pal().text, 0.2)));
        let mut hint: Vec<u16> = tr("Search · Ctrl+F", "검색 · Ctrl+F", "検索 · Ctrl+F").encode_utf16().collect();
        DrawTextW(dc, &mut hint, &mut rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
        _ = ReleaseDC(Some(h), dc);
    }
    r
}

pub(crate) unsafe fn destroy_pane(i: usize) {
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    if let Some(sv) = view_of::<IShellView>(i) {
        if let Ok(h) = sv.GetWindow() {
            clear_filter_view(h);
        }
    }
    save_view(i);
    let (p, queued) = app(|a| {
        let p = a.panes[i].take();
        if p.as_ref().is_some_and(|p| p.hwnd == a.drop_hover) {
            a.drop_hover = HWND::default();
        }
        if p.as_ref().is_some_and(|p| p.hwnd == a.focus_req) {
            a.focus_req = HWND::default();
        }
        let queued = if a.drop_open.as_ref().is_some_and(|d| d.1 == i) { a.drop_open.take() } else { None };
        (p, queued)
    });
    if let Some(p) = p {
        drop_find(p.hwnd);
        drop_icon_request(p.hwnd);
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p.hwnd), id) {
                a11y_name(c, None);
                if id == ID_FILTER {
                    _ = RemoveWindowSubclass(c, Some(filter_hint_proc), 4);
                }
            }
        }
        _ = RevokeDragDrop(p.hwnd);
        _ = p.eb.Unadvise(p.cookie);
        _ = IUnknown_SetSite(&p.eb, None);
        _ = p.eb.Destroy();
        _ = DestroyWindow(p.hwnd);
    }
    drop(queued);
    if let Ok(mut f) = FILTERS.lock() {
        f[i].clear();
    }
}

// Only the panes visible in the current grid exist; hidden slots keep just their path.
pub(crate) unsafe fn sync_panes() {
    let (main, n) = app(|a| (a.main, a.s.panes()));
    if main.0.is_null() {
        return;
    }
    for i in 0..MAXP {
        let has = app(|a| a.panes[i].is_some());
        if i < n && !has {
            let Ok(p) = create_pane(i) else { continue };
            app(|a| a.panes[i] = Some(p));
            let path = app(|a| a.s.paths[i].clone());
            if path.is_empty() || !navigate(i, &path) {
                let start = app(|a| {
                    if a.s.views[i].0 == 0 {
                        a.s.views[i] = a.s.default_view();
                    }
                    start_folder(&a.s)
                });
                navigate(i, &start);
            }
        } else if i >= n && has {
            destroy_pane(i);
        }
    }
}

// Moves a batch of windows at once (one repaint, no copied bits).
pub(crate) unsafe fn move_all(moves: &[(HWND, RECT)], eb: Option<(&IExplorerBrowser, RECT)>) {
    let Ok(mut dwp) = BeginDeferWindowPos(moves.len() as i32 + 1) else { return };
    for (h, r) in moves.iter().filter(|(h, _)| !h.0.is_null()) {
        match DeferWindowPos(dwp, *h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, MOVE) {
            Ok(d) => dwp = d,
            Err(_) => return,
        }
    }
    if let Some((eb, r)) = eb {
        _ = eb.SetRect(Some(&mut dwp as *mut HDWP), r);
    }
    _ = EndDeferWindowPos(dwp);
}

pub(crate) unsafe fn layout_pane(h: HWND) {
    let Some((eb, dpi, disp)) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| (p.eb.clone(), a.dpi, p.disp.clone()))) else {
        return;
    };
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let s = |v: i32| v * dpi / 96;
    let pg = pane_geo_of(h, rc.right, rc.bottom, dpi);
    // Borderless edit. Horizontal insets stay; the line is centred in the pill from the font.
    let in_pill = |edit: HWND, pill: RECT, l: i32, r: i32| {
        let outer = RECT { left: pill.left + l, top: pill.top, right: (pill.right - r).max(pill.left + l), bottom: pill.bottom };
        (edit, unsafe { edit_window_rect(edit, outer) })
    };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    let mut moves: Vec<(HWND, RECT)> = [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].iter().zip(pg.btn).map(|(&id, r)| (ctl(id), r)).collect();
    // The icon slot is reserved by width, so the edit does not move when the icon arrives.
    let inset = crumb_field(pg.addr, dpi).left - pg.addr.left;
    let add = ctl(ID_PANE_ADD);
    moves.push((add, pg.add));
    moves.push(in_pill(ctl(ID_EDIT), pg.addr, inset, s(6)));
    moves.push(in_pill(ctl(ID_FILTER), pg.filt, s(24), s(6)));
    moves.push((ctl(ID_FAV), pg.fav));
    place_find(h);
    move_all(&moves, Some((&eb, pg.view)));
    // One tab: the add button sits beside ⇄. Two or more: the strip takes over.
    // SW_SHOWNA so a resize does not move focus onto the button.
    _ = ShowWindow(add, if pg.add.right > pg.add.left { SW_SHOWNA } else { SW_HIDE });
    show_path_end(ctl(ID_EDIT), &disp);
    sync_addr(h);
}

pub(crate) unsafe fn relayout() {
    sync_panes();
    let Some((main, moves, label, shown, drive_vis, show_bar, show_side, tree_h, side_h)) = app(|a| {
        a.gap_hot = None; // the splitter may be gone; the window is repainted below
        if a.main.0.is_null() {
            return None;
        }
        if a.zoom.is_some_and(|z| z != a.s.active || z >= a.s.panes()) {
            a.zoom = None;
        }
        let g = geo(a);
        let s = |v: i32| v * a.dpi / 96;
        let sr = side_rect(&g);
        let tree = RECT { left: sr.left + s(2), top: sr.top + s(2), right: sr.right - s(2), bottom: sr.top + g.tree_h };
        let fav = RECT { left: sr.left + s(2), top: sr.top + g.tree_h + g.gap, right: sr.right - s(2), bottom: sr.bottom - s(2) };
        let mut moves = vec![(a.tree_hwnd, tree), (a.side_hwnd, fav)];
        let tb = |id: i32| unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
        let show_bar = a.s.bar;
        let strip_h = s(32);
        let strip_top = 0;
        let btn_top = strip_top + (strip_h - s(24)) / 2;
        // Drive letters, then Layout, save, load, the favourites star and the gear, at the right of the tab row.
        // The last letters hide first so a minimum tab stays clear of the group.
        // The gear stays when the toolbar is off; the other icons and the letters do not.
        let icons = icon_slots(g.w, btn_top, a.dpi, drive_slots(a.s.drive_buttons, a.drives.len()), show_bar);
        for (id, r) in icons.icons {
            moves.push((tb(id), r));
        }
        let mut drive_vis = Vec::new();
        for (i, &(_, _, h)) in a.drives.iter().enumerate() {
            let r = if i < icons.drive_n {
                icons.drives[i]
            } else {
                let x = icons.icons[0].1.left;
                RECT { left: x, top: btn_top, right: x, bottom: btn_top }
            };
            moves.push((h, r));
            drive_vis.push((h, show_bar && i < icons.drive_n));
        }
        let show_side = a.s.tree_on;
        let (tree_h, side_h) = (a.tree_hwnd, a.side_hwnd);
        moves.push((a.tabs_hwnd, RECT { left: 0, top: strip_top, right: g.w, bottom: strip_top + strip_h }));
        let full = RECT { left: g.gx, top: g.top, right: g.w - g.pad, bottom: g.bot };
        let mut shown = Vec::new();
        for (i, p) in a.panes.iter().enumerate().take(a.s.panes()) {
            if let Some(p) = p {
                moves.push((p.hwnd, if a.zoom == Some(i) { full } else { cell_rect(&g, &a.s.heights, i) }));
                shown.push((p.hwnd, a.zoom.is_none_or(|z| z == i)));
            }
        }
        Some((a.main, moves, layout_label(&a.s.heights), shown, drive_vis, show_bar, show_side, tree_h, side_h))
    }) else {
        return;
    };
    move_all(&moves, None);
    for id in [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_FAV, ID_TB_SET] {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            // The gear stays: with the toolbar hidden it is the only mouse path to settings.
            let on = show_bar || id == ID_TB_SET;
            _ = ShowWindow(b, if on { SW_SHOW } else { SW_HIDE });
        }
    }
    for (h, on) in &drive_vis {
        _ = ShowWindow(*h, if *on { SW_SHOW } else { SW_HIDE });
    }
    if !tree_h.0.is_null() {
        _ = ShowWindow(tree_h, if show_side { SW_SHOW } else { SW_HIDE });
    }
    if !side_h.0.is_null() {
        _ = ShowWindow(side_h, if show_side { SW_SHOW } else { SW_HIDE });
    }
    let strip = app(|a| a.tabs_hwnd);
    if !strip.0.is_null() {
        _ = ShowWindow(strip, SW_SHOW);
        // The strip's rect does not change with the toolbar, so a size message never arrives.
        // Invalidate it or the tabs keep the previous width until the window is resized.
        _ = InvalidateRect(Some(strip), None, true);
    }
    // The strip is full width underneath the icons. Raise the visible ones so they take the click.
    for id in [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_FAV, ID_TB_SET] {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            if show_bar || id == ID_TB_SET {
                _ = SetWindowPos(b, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }
    for (h, on) in &drive_vis {
        if *on {
            _ = SetWindowPos(*h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    for (h, on) in &shown {
        _ = ShowWindow(*h, if *on { SW_SHOW } else { SW_HIDE });
    }
    start_reveal(main); // sidebar size may have changed: keep its selection in view
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        if text_of(b) != label {
            _ = SetWindowTextW(b, &HSTRING::from(label));
        }
    }
    _ = InvalidateRect(Some(main), None, true);
    // Paint now: WM_PAINT is only generated once the message queue is empty, which never happens
    // while the mouse keeps moving, so mid-drag the views would keep showing the previous frame.
    _ = RedrawWindow(Some(main), None, None, RDW_UPDATENOW | RDW_ALLCHILDREN);
}

// Splitter drag shows translucent guide bars where the boundaries will land; the panes move once, on release.
pub(crate) unsafe fn show_bars() {
    let Some((main, bars, rects)) = app(|a| {
        let d = a.drag?;
        let g = geo(a);
        let r = gap_line(&g, d, 3 * a.dpi / 96);
        let rects = if matches!(d, Hit::Row(..) | Hit::SideSplit) { [None, r] } else { [r, None] };
        Some((a.main, a.bars, rects))
    }) else {
        return;
    };
    for (b, r) in bars.into_iter().zip(rects) {
        match r {
            Some(r) => {
                let mut p = POINT { x: r.left, y: r.top };
                _ = ClientToScreen(main, &mut p);
                _ = SetWindowPos(b, None, p.x, p.y, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            None => _ = ShowWindow(b, SW_HIDE),
        }
    }
}

pub(crate) extern "system" fn bar_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

pub(crate) unsafe fn end_drag() {
    let Some((bars, kind)) = app(|a| a.drag.take().map(|d| (a.bars, d))) else { return };
    for b in bars {
        _ = ShowWindow(b, SW_HIDE);
    }
    refresh_drives(); // one that came or went while the splitter was held
    relayout();
    redraw_all();
    if matches!(kind, Hit::SideSplit) {
        note_save();
    }
}

// Full repaint of everything once a drag/resize/layout change is over, so nothing stale survives.
pub(crate) unsafe fn redraw_all() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = RedrawWindow(Some(main), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

pub(crate) unsafe fn set_active(i: usize) {
    let (old, hs) = app(|a| {
        let old = std::mem::replace(&mut a.s.active, i);
        if old != i {
            a.focus_req = HWND::default();
        }
        (old, [old, i].map(|k| a.panes[k].as_ref().map(|p| p.hwnd)))
    });
    if old != i {
        if app(|a| a.zoom.take()).is_some() {
            relayout();
            redraw_all();
        }
        for h in hs.into_iter().flatten() {
            repaint_header(h);
        }
        if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
            sync_tree(&item);
        }
        mark_drive();
        tabs_invalidate();
    }
}

pub(crate) unsafe fn repaint_header(pane: HWND) {
    _ = InvalidateRect(Some(pane), None, false);
    for id in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN, ID_PANE_ADD, ID_FAV, ID_EDIT, ID_FILTER] {
        if let Ok(c) = GetDlgItem(Some(pane), id) {
            _ = InvalidateRect(Some(c), None, true);
        }
    }
}

// The sidebar follows the active pane: expand down to its folder and select it.
// (Programmatic selection doesn't navigate: OnSelectionChanged only follows user input.)
pub(crate) unsafe fn sync_tree(item: &IShellItem) {
    let Some((t, main, th)) = app(|a| a.tree.clone().map(|t| (t, a.main, a.tree_hwnd))) else { return };
    // the tree scrolls on its own when it selects; hide that and paint once, after the reveal
    let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).ok();
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    _ = t.EnsureItemVisible(item);
    _ = t.SetItemState(item, NSTCIS_SELECTED.0 as u32, NSTCIS_SELECTED.0 as u32);
    reveal_tree_selection(); // turns redraw back on
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
        _ = InvalidateRect(Some(tv), None, true);
    }
    start_reveal(main); // and again while the tree finishes expanding
}

// Scrolls the tree so the selected folder is on screen: vertically, and sideways when deep nesting
// pushes its name past a narrow sidebar.
pub(crate) unsafe fn reveal_tree_selection() {
    let (th, dpi) = app(|a| (a.tree_hwnd, a.dpi));
    let Ok(tv) = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()) else { return };
    let item = SendMessageW(tv, TVM_GETNEXTITEM, Some(WPARAM(TVGN_CARET)), None).0;
    if item == 0 {
        return;
    }
    let label = || {
        let mut r = RECT::default();
        *(&mut r as *mut RECT as *mut isize) = item; // TVM_GETITEMRECT takes the item in the RECT
        let ok = SendMessageW(tv, TVM_GETITEMRECT, Some(WPARAM(1)), Some(LPARAM(&mut r as *mut _ as isize))).0 != 0;
        ok.then_some(r)
    };
    let mut c = RECT::default();
    _ = GetClientRect(tv, &mut c);
    if c.right == 0 {
        return; // not laid out yet; relayout asks again
    }
    let margin = 16 * dpi / 96;
    // all the scrolling happens unseen, then one repaint if anything moved
    let pos = || (GetScrollPos(tv, SB_HORZ), GetScrollPos(tv, SB_VERT));
    let before = pos();
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    SendMessageW(tv, TVM_ENSUREVISIBLE, None, Some(LPARAM(item)));
    // sideways, a line at a time (avoids guessing the tree's scroll units): the selected folder's name
    // wins over the roots on the left, which may get cut off. One direction only, so a name wider than
    // the sidebar can't bounce between the two rules.
    let left = label().is_some_and(|r| r.left < margin);
    for _ in 0..300 {
        match label() {
            Some(r) if left && r.left < margin && GetScrollPos(tv, SB_HORZ) > 0 => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None), // SB_LINELEFT
            Some(r) if !left && r.right > c.right && r.left > margin => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(1)), None), // SB_LINERIGHT
            _ => break,
        }
    }
    if !left && label().is_some_and(|r| r.left < margin) {
        SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None); // overshot: back one, so the next pass is a no-op
    }
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
    if pos() != before {
        _ = InvalidateRect(Some(tv), None, true);
    }
}

// The tree expands down to a new folder asynchronously, so the reveal runs a few times as it settles.
pub(crate) unsafe fn start_reveal(main: HWND) {
    app(|a| a.reveal_tries = 3);
    SetTimer(Some(main), TIMER_REVEAL, 120, None);
}

pub(crate) unsafe fn set_accent(c: u32) {
    let (bars, panes, drives, peek) = app(|a| {
        if a.s.dark {
            a.s.accent_dark = c;
        } else {
            a.s.accent = c;
        }
        (a.bars, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), a.drives.iter().map(|d| d.2).collect::<Vec<_>>(), a.peek)
    });
    let old = SetClassLongPtrW(bars[0], GCLP_HBRBACKGROUND, CreateSolidBrush(cref(c)).0 as isize);
    if old != 0 {
        _ = DeleteObject(HGDIOBJ(old as _));
    }
    for p in panes {
        repaint_header(p); // all of them: inactive panes show the accent in their filter badge
    }
    for h in drives {
        _ = InvalidateRect(Some(h), None, false); // the active drive's underline
    }
    if !peek.0.is_null() {
        _ = InvalidateRect(Some(peek), None, false); // open quick preview reads the accent as it paints
    }
    drop_group_brushes();
    settings::redraw();
}

// The hosted Explorer list ignores the app-wide dark mode, so its (new, per folder) list view is dyed here.
pub(crate) unsafe fn dark_list(pane: HWND) {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        let class = String::from_utf16_lossy(&c[..n]);
        if class == "SysListView32" || class == "SHELLDLL_DefView" {
            unsafe { (*(out.0 as *mut Vec<(String, HWND)>)).push((class, h)) };
        }
        true.into()
    }
    let mut found: Vec<(String, HWND)> = Vec::new();
    _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut found as *mut _ as isize));
    // the view paints the name column itself (custom draw, light colours); recolour it on the way back
    for (_, v) in found.iter().filter(|(c, _)| c == "SHELLDLL_DefView") {
        _ = SetWindowSubclass(*v, Some(dark_view_proc), 1, 0);
    }
    let lists: Vec<HWND> = found.into_iter().filter(|(c, _)| c == "SysListView32").map(|(_, h)| h).collect();
    let (card, text) = (cref(pal().card).0 as isize, cref(pal().text).0 as isize);
    for lv in lists {
        _ = SetWindowTheme(lv, w!("DarkMode_Explorer"), PCWSTR::null()); // also darkens its scrollbars
        // the sorted-column tint stays white in dark mode
        _ = SetWindowSubclass(lv, Some(dark_list_proc), 2, 0);
        SendMessageW(lv, LVM_SETSELECTEDCOLUMN, Some(WPARAM(usize::MAX)), None);
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            _ = SetWindowTheme(header, w!("DarkMode_ItemsView"), PCWSTR::null());
        }
        // the info tip over a file, and the full name over a truncated one
        let tip = HWND(SendMessageW(lv, LVM_GETTOOLTIPS, None, None).0 as _);
        if !tip.0.is_null() {
            _ = SetWindowTheme(tip, w!("DarkMode_Explorer"), PCWSTR::null());
        }
        _ = InvalidateRect(Some(lv), None, true);
    }
}

/// Puts a list back to the light-theme colours. Subclasses stay, and they do nothing while dark mode is off.
pub(crate) unsafe fn light_list(pane: HWND) {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        if String::from_utf16_lossy(&c[..n]) == "SysListView32" {
            unsafe { (*(out.0 as *mut Vec<HWND>)).push(h) };
        }
        true.into()
    }
    let mut lists: Vec<HWND> = Vec::new();
    _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut lists as *mut _ as isize));
    let (bg, text) = (GetSysColor(COLOR_WINDOW) as isize, GetSysColor(COLOR_WINDOWTEXT) as isize);
    for lv in lists {
        _ = SetWindowTheme(lv, w!("Explorer"), PCWSTR::null());
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(bg)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(bg)));
        SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            _ = SetWindowTheme(header, w!("ItemsView"), PCWSTR::null());
        }
        let tip = HWND(SendMessageW(lv, LVM_GETTOOLTIPS, None, None).0 as _);
        if !tip.0.is_null() {
            _ = SetWindowTheme(tip, PCWSTR::null(), PCWSTR::null());
        }
        _ = InvalidateRect(Some(lv), None, true);
    }
}

/// Windows app mode. None when the value is not there, so a missing key is not treated as light.
pub(crate) unsafe fn system_dark() -> Option<bool> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (mut v, mut n) = (0u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let r = RegGetValueW(HKEY_CURRENT_USER, key, w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut n));
    r.is_ok().then_some(v == 0)
}

/// The dark flag to switch to while following Windows. None when following is off, the setting is unknown, or it already matches.
pub(crate) fn theme_target(auto: bool, sys: Option<bool>, cur: bool) -> Option<bool> {
    sys.filter(|&d| auto && d != cur)
}

// The view re-sets the sorted column through an internal interface (no window message to catch),
// so it is cleared right before each paint instead. The column header is the list's child, so its
// draw notifications come through here too.
pub(crate) unsafe extern "system" fn dark_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let dark = DARK_ON.load(Ordering::Relaxed);
    if msg == WM_PAINT && dark && DefSubclassProc(h, LVM_GETSELECTEDCOLUMN, WPARAM(0), LPARAM(0)).0 as i32 != -1 {
        DefSubclassProc(h, LVM_SETSELECTEDCOLUMN, WPARAM(usize::MAX), LPARAM(0));
    }
    if msg == WM_NOTIFY && dark && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &*(lp.0 as *const NMCUSTOMDRAW);
        if cd.hdr.hwndFrom.0 as isize == DefSubclassProc(h, LVM_GETHEADER, WPARAM(0), LPARAM(0)).0 {
            if cd.dwDrawStage == CDDS_PREPAINT {
                return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
            }
            if cd.dwDrawStage == CDDS_ITEMPREPAINT {
                draw_header_item(cd);
                return LRESULT(CDRF_SKIPDEFAULT as isize);
            }
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// The column header's theme draws its labels in the light theme's dark grey whatever colour it is
// given, so in dark mode each column is drawn here: flat background, label, divider, sort chevron.
pub(crate) unsafe fn draw_header_item(cd: &NMCUSTOMDRAW) {
    let (dc, rc, hdr) = (cd.hdc, cd.rc, cd.hdr.hwndFrom);
    let s = |v: i32| v * GetDpiForWindow(hdr) as i32 / 96;
    let hot = cd.uItemState.0 & (CDIS_HOT.0 | CDIS_SELECTED.0) != 0;
    fill(dc, rc, if hot { blend(pal().card, pal().text, 0.08) } else { pal().card });
    fill(dc, RECT { left: rc.right - 1, ..rc }, pal().line);
    let mut text = [0u16; 260];
    let mut item = HDITEMW { mask: HDI_TEXT | HDI_FORMAT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
    SendMessageW(hdr, HDM_GETITEMW, Some(WPARAM(cd.dwItemSpec)), Some(LPARAM(&mut item as *mut _ as isize)));
    let n = text.iter().position(|&c| c == 0).unwrap_or(0);
    if n > 0 {
        let align = if item.fmt.0 & HDF_RIGHT.0 != 0 {
            DT_RIGHT
        } else if item.fmt.0 & HDF_CENTER.0 != 0 {
            DT_CENTER
        } else {
            DT_LEFT
        };
        let mut r = RECT { left: rc.left + s(7), right: rc.right - s(7), ..rc };
        let font = SelectObject(dc, HGDIOBJ(SendMessageW(hdr, WM_GETFONT, None, None).0 as _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, cref(pal().sub));
        DrawTextW(dc, &mut text[..n], &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
    }
    // sort chevron along the top edge, centred, as Windows 11 draws it
    let up = item.fmt.0 & HDF_SORTUP.0 != 0;
    if up || item.fmt.0 & HDF_SORTDOWN.0 != 0 {
        let (cx, t, arm) = ((rc.left + rc.right) / 2, s(1).max(1), s(4));
        for k in 0..=arm {
            let y = rc.top + s(1) + if up { k } else { arm - k };
            fill(dc, RECT { left: cx - k, top: y, right: cx - k + t, bottom: y + t }, pal().sub);
            fill(dc, RECT { left: cx + k, top: y, right: cx + k + t, bottom: y + t }, pal().sub);
        }
    }
}

pub(crate) unsafe extern "system" fn dark_view_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_NOTIFY && DARK_ON.load(Ordering::Relaxed) && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &mut *(lp.0 as *mut NMLVCUSTOMDRAW);
        let stage = cd.nmcd.dwDrawStage.0;
        // A plain group title is drawn here. Anything richer stays on the list's own drawing.
        if cd.dwItemType == LVCDI_GROUP && (stage == CDDS_PREPAINT.0 || stage == CDDS_ITEMPREPAINT.0) && draw_group_header(cd) {
            return LRESULT(CDRF_SKIPDEFAULT as isize);
        }
        if stage == CDDS_PREPAINT.0 {
            return LRESULT(r.0 | CDRF_NOTIFYITEMDRAW as isize);
        }
        if stage & CDDS_ITEMPREPAINT.0 == CDDS_ITEMPREPAINT.0 {
            (cd.clrText, cd.clrTextBk) = (cref(pal().text), cref(pal().card));
            if stage & CDDS_SUBITEM.0 == 0 {
                return LRESULT(r.0 | CDRF_NOTIFYSUBITEMDRAW as isize);
            }
        }
    }
    r
}

/// Dark-theme group title ("Devices and drives" on This PC): flat card, light text, a hairline.
/// A subtitle, task, image, or a failed lookup returns false and the list draws it.
pub(crate) unsafe fn draw_group_header(cd: &NMLVCUSTOMDRAW) -> bool {
    let (dc, lv, id) = (cd.nmcd.hdc, cd.nmcd.hdr.hwndFrom, cd.nmcd.dwItemSpec);
    let mut head = [0u16; 260];
    let mut more = [[0u16; 64]; 5];
    let [sub, task, top, bottom, foot] = more.each_mut().map(|b| PWSTR(b.as_mut_ptr()));
    let mask = [LVGF_HEADER, LVGF_ALIGN, LVGF_STATE, LVGF_SUBTITLE, LVGF_TASK, LVGF_DESCRIPTIONTOP, LVGF_DESCRIPTIONBOTTOM, LVGF_FOOTER, LVGF_TITLEIMAGE, LVGF_EXTENDEDIMAGE];
    let states = LVGS_COLLAPSIBLE.0 | LVGS_COLLAPSED.0 | LVGS_SELECTED.0 | LVGS_FOCUSED.0 | LVGS_SUBSETED.0;
    let mut g = LVGROUP {
        cbSize: size_of::<LVGROUP>() as u32,
        mask: LVGROUP_MASK(mask.iter().fold(0, |m, f| m | f.0)),
        stateMask: LIST_VIEW_GROUP_STATE_FLAGS(states),
        pszHeader: PWSTR(head.as_mut_ptr()),
        cchHeader: head.len() as i32,
        pszSubtitle: sub,
        cchSubtitle: 64,
        pszTask: task,
        cchTask: 64,
        pszDescriptionTop: top,
        cchDescriptionTop: 64,
        pszDescriptionBottom: bottom,
        cchDescriptionBottom: 64,
        pszFooter: foot,
        cchFooter: 64,
        ..Default::default()
    };
    if SendMessageW(lv, LVM_GETGROUPINFO, Some(WPARAM(id)), Some(LPARAM(&mut g as *mut _ as isize))).0 == -1 {
        return false;
    }
    let st = g.state.0;
    let n = head.iter().position(|&c| c == 0).unwrap_or(0);
    if n == 0 || more.iter().any(|b| b[0] != 0) || st & LVGS_SUBSETED.0 != 0 {
        return false;
    }
    let images = SendMessageW(lv, LVM_GETIMAGELIST, Some(WPARAM(LVSIL_GROUPHEADER as usize)), None).0 != 0;
    if images && (g.iTitleImage >= 0 || g.iExtendedImage >= 0) {
        return false;
    }
    let mut rc = RECT { top: LVGGR_HEADER as i32, ..Default::default() };
    if SendMessageW(lv, LVM_GETGROUPRECT, Some(WPARAM(id)), Some(LPARAM(&mut rc as *mut _ as isize))).0 == 0 || rc.right <= rc.left || rc.bottom <= rc.top {
        return false;
    }
    let s = |v: i32| v * GetDpiForWindow(lv) as i32 / 96;
    let bg = if st & LVGS_SELECTED.0 != 0 {
        blend(pal().card, pal().text, 0.16)
    } else if cd.nmcd.uItemState.0 & CDIS_HOT.0 != 0 {
        blend(pal().card, pal().text, 0.08)
    } else {
        pal().card
    };
    fill(dc, rc, bg);
    let collapsible = st & LVGS_COLLAPSIBLE.0 != 0;
    let bx = if collapsible { rc.bottom - rc.top } else { 0 };
    let r = RECT { left: rc.left + s(8), right: rc.right - bx - s(8), ..rc };
    let align = if g.uAlign.0 & LVGA_HEADER_RIGHT.0 != 0 {
        DT_RIGHT
    } else if g.uAlign.0 & LVGA_HEADER_CENTER.0 != 0 {
        DT_CENTER
    } else {
        DT_LEFT
    };
    let font = HFONT(SendMessageW(lv, WM_GETFONT, None, None).0 as _);
    text(dc, r, &String::from_utf16_lossy(&head[..n]), font, pal().text, align);
    let cy = (rc.top + rc.bottom) / 2;
    if align == DT_LEFT {
        let old = SelectObject(dc, font.into());
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &head[..n], &mut sz);
        SelectObject(dc, old);
        let x = r.left + sz.cx + s(8);
        if x < r.right {
            fill(dc, RECT { left: x, top: cy, right: r.right, bottom: cy + 1 }, pal().line);
        }
    }
    if collapsible {
        let up = st & LVGS_COLLAPSED.0 == 0;
        let (cx, t, arm) = (rc.right - bx / 2, s(1).max(1), s(4));
        for k in 0..=arm {
            let y = cy - arm / 2 + if up { k } else { arm - k };
            fill(dc, RECT { left: cx - k, top: y, right: cx - k + t, bottom: y + t }, pal().sub);
            fill(dc, RECT { left: cx + k, top: y, right: cx + k + t, bottom: y + t }, pal().sub);
        }
    }
    if cd.nmcd.uItemState.0 & CDIS_FOCUS.0 != 0 || st & LVGS_FOCUSED.0 != 0 {
        frame(dc, rc, 1, pal().sub);
    }
    true
}

// Undocumented but long-stable uxtheme exports (Windows 10 1903+): app-wide dark mode, which is what
// makes menus, scrollbars and the hosted Explorer views follow the theme.
pub(crate) unsafe fn uxtheme(ordinal: usize) -> FARPROC {
    let m = LoadLibraryW(w!("uxtheme.dll")).ok()?;
    GetProcAddress(m, PCSTR(ordinal as *const u8))
}

/// AppsUseLightTheme = 0 means Windows is in dark app mode.
pub(crate) unsafe fn windows_dark() -> bool {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD};
    let mut key = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), None, KEY_READ, &mut key).is_err() {
        return false;
    }
    let mut data = [0u8; 4];
    let mut n = 4u32;
    let mut typ = REG_DWORD;
    let ok = RegQueryValueExW(key, w!("AppsUseLightTheme"), None, Some(&mut typ), Some(data.as_mut_ptr()), Some(&mut n));
    _ = RegCloseKey(key);
    ok.is_ok() && u32::from_le_bytes(data) == 0
}

pub(crate) fn resolve_dark(theme: u8) -> bool {
    match theme {
        2 => true,
        1 => false,
        _ => unsafe { windows_dark() },
    }
}

pub(crate) unsafe fn start_folder(s: &Session) -> String {
    match s.start_new {
        1 => {
            let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) else { return THIS_PC.into() };
            let path = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            if path.is_empty() { THIS_PC.into() } else { path }
        }
        2 if !s.start_path.is_empty() => s.start_path.clone(),
        _ => THIS_PC.into(),
    }
}

/// Show or hide extensions for the hosted Explorer views. This is Windows' own HideFileExt value.
pub(crate) unsafe fn apply_ext(show: bool) {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD};
    let mut key = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"), None, KEY_SET_VALUE, &mut key).is_err() {
        return;
    }
    let data = [(!show) as u8, 0, 0, 0];
    _ = RegSetValueExW(key, w!("HideFileExt"), None, REG_DWORD, Some(&data));
    _ = RegCloseKey(key);
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    across_windows(|| {
        for i in 0..MAXP {
            if let Some(sv) = view_of::<IShellView>(i) {
                _ = sv.Refresh();
            }
        }
    });
}

pub(crate) unsafe fn prime_theme(dark: bool) {
    DARK_ON.store(dark, Ordering::Relaxed);
    if let Some(f) = uxtheme(135) {
        let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_preferred_app_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
    }
    if let Some(f) = uxtheme(136) {
        let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
        flush_menu_themes();
    }
}

/// One window: title bar, tree, and that window's Explorer panes.
pub(crate) unsafe fn apply_theme_desk() {
    let (main, dark, tree, accent, tip) = app(|a| (a.main, a.s.dark, a.tree_hwnd, a.s.accent_now(), a.tip));
    if main.0.is_null() {
        return;
    }
    // tooltips: dark on dark, back to the default on light (these windows outlive the theme switch)
    let tip_theme = |t: HWND| {
        if !t.0.is_null() {
            _ = SetWindowTheme(t, if dark { w!("DarkMode_Explorer") } else { PCWSTR::null() }, PCWSTR::null());
        }
    };
    tip_theme(tip);
    tip_theme(app(|a| a.side_hwnd)); // favourites list scrollbar
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(main, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4); // immersive dark title bar
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    if let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) {
        _ = SetWindowTheme(tv, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, PCWSTR::null());
        let colour = |c: u32| LPARAM(if dark { cref(c).0 as isize } else { -1 }); // -1: system default
        SendMessageW(tv, TVM_SETBKCOLOR, None, Some(LPARAM(cref(crate::ui::side_ground()).0 as isize)));
        SendMessageW(tv, TVM_SETTEXTCOLOR, None, Some(colour(pal().text)));
        tip_theme(HWND(SendMessageW(tv, TVM_GETTOOLTIPS, None, None).0 as _));
    }
    set_accent(accent);
    let panes = app(|a| {
        let mut hs: Vec<HWND> = a.panes.iter().flatten().map(|p| p.hwnd).collect();
        for tab in &a.park {
            hs.extend(tab.iter().flatten().map(|p| p.hwnd));
        }
        hs
    });
    for p in panes {
        if DARK_ON.load(Ordering::Relaxed) {
            dark_list(p);
        } else {
            light_list(p);
        }
    }
    redraw_all();
    let popups = app(|a| {
        let match_h = a.matcher.as_ref().map(|m| m.hwnd).unwrap_or_default();
        [a.fav_popup, a.load, a.grid, a.keys_popup, a.peek, match_h]
    });
    for h in popups {
        if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
            _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
        }
    }
}

/// Follow Windows when that mode is on and the setting differs from the panes.
pub(crate) unsafe fn theme_sync() {
    let (auto, cur) = app(|a| (a.s.theme == 0, a.s.dark));
    if let Some(d) = theme_target(auto, system_dark(), cur) {
        app(|a| a.s.dark = d);
        publish_shared();
        prime_theme(d);
        across_windows(|| apply_theme_desk());
        note_save();
    }
}

// Applies the current theme in place. Explorer views stay, so history, filter, selection, scroll and a typed address stay.
pub(crate) unsafe fn apply_theme() {
    let dark = resolve_dark(app(|a| a.s.theme));
    app(|a| a.s.dark = dark);
    publish_shared();
    prime_theme(dark);
    across_windows(|| apply_theme_desk());
    settings::refresh();
    crate::rename::refresh();
    refresh_fav_manager();
}

pub(crate) unsafe fn pick_accent() {
    let (main, cur) = app(|a| (a.main, a.s.accent_now()));
    let mut custom = [COLORREF(0xFFFFFF); 16];
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: main,
        rgbResult: cref(cur),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    if ChooseColorW(&mut cc).as_bool() {
        set_accent(cref(cc.rgbResult.0).0); // COLORREF is BGR; cref swaps back
    }
}

// Bumped on every navigation event and pane teardown, so a click pair spanning a folder change is dropped.
pub(crate) static NAV_GEN: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn gen() -> usize {
    NAV_GEN.load(Ordering::SeqCst)
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Click {
    pub(crate) win: isize,
    pub(crate) gen: usize,
    pub(crate) time: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

thread_local! {
    static LAST_CLICK: Cell<Option<Click>> = const { Cell::new(None) };
}

// Windows 11's list (DirectUIHWND) has no CS_DBLCLKS, so a double-click is two background presses in a row,
// within the system's double-click time and box. The pair is consumed: a third click doesn't go up again.
pub(crate) fn next_click(prev: Option<Click>, now: Click, bg: bool, ms: u32, cx: i32, cy: i32) -> (Option<Click>, bool) {
    if !bg {
        return (None, false);
    }
    match prev {
        Some(p)
            if p.win == now.win
                && p.gen == now.gen
                && now.time.wrapping_sub(p.time) <= ms
                && (now.x - p.x).abs() <= cx / 2
                && (now.y - p.y).abs() <= cy / 2 =>
        {
            (None, true)
        }
        _ => (Some(now), false),
    }
}

// The list itself is background, and so may a row be: a row reports ROLE_SYSTEM_LISTITEM across all its columns,
// yet without full-row select only the name is the item, so its date and size cells are empty space. confirm()
// tells them apart after the click: a press on the name selects or opens it. Group headers, collapse buttons and
// column headers are never background.
// The column header of a details view is a list too, nested in the item list: never background, so a
// double-click on a header divider (auto-fit a column) does not go up a folder.
pub(crate) fn bg_role(role: u32, parent: Option<u32>) -> bool {
    (role == ROLE_SYSTEM_LIST && parent != Some(ROLE_SYSTEM_LIST)) || role == ROLE_SYSTEM_LISTITEM
}

unsafe fn role_of(a: &IAccessible, child: &VARIANT) -> Option<u32> {
    let mut role = a.get_accRole(child).ok()?;
    let r = &role.Anonymous.Anonymous;
    let out = (r.vt == VT_I4).then(|| r.Anonymous.lVal as u32);
    _ = VariantClear(&mut role);
    out
}

// The generation is read before the probe and again after it: a navigation during the probe voids it.
pub(crate) fn bg_probe(gen: impl Fn() -> usize, probe: impl FnOnce() -> bool) -> (usize, bool) {
    let g0 = gen();
    let b = probe();
    (g0, b && gen() == g0)
}

// After dispatch: same view window, nothing selected, and no navigation since the click. The generation is
// read last, after the queries that could run navigation callbacks.
pub(crate) fn confirm(cand: (usize, isize), query: impl FnOnce() -> (Option<isize>, Option<u32>), gen: impl Fn() -> usize) -> bool {
    let (view, sel) = query();
    view == Some(cand.1) && sel == Some(0) && gen() == cand.0
}

// Same-thread window, so WM_GETOBJECT runs inline; call outside app().
pub(crate) unsafe fn on_background(m: &MSG) -> bool {
    if WindowFromPoint(m.pt) != m.hwnd {
        return false;
    }
    let mut acc: Option<IAccessible> = None;
    let mut child = VARIANT::default();
    if AccessibleObjectFromPoint(m.pt, &mut acc, &mut child).is_err() {
        return false;
    }
    let bg = acc.is_some_and(|a| {
        let Some(role) = role_of(&a, &child) else { return false };
        let parent = (role == ROLE_SYSTEM_LIST)
            .then(|| a.accParent().ok().and_then(|p| p.cast::<IAccessible>().ok()))
            .flatten()
            .and_then(|p| role_of(&p, &VARIANT::from(0i32)));
        bg_role(role, parent)
    });
    _ = VariantClear(&mut child);
    bg
}

// A double-click on empty space in a list goes up a folder: (pane, generation, view window) when this press
// completes a background pair.
pub(crate) unsafe fn up_candidate(m: &MSG) -> Option<(usize, usize, isize)> {
    if !matches!(m.message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) || !app(|a| a.s.dblup) {
        return None;
    }
    let i = pane_of(m.hwnd)?;
    let svh = Cell::new(HWND::default());
    let (g0, bg) = bg_probe(gen, || {
        let Some(h) = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()) else {
            return false;
        };
        svh.set(h);
        let mut c = [0u16; 32];
        let n = GetClassNameW(m.hwnd, &mut c) as usize;
        let list = matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32");
        list && (h == m.hwnd || IsChild(h, m.hwnd).as_bool())
            && m.wParam.0 & (MK_CONTROL.0 | MK_SHIFT.0) as usize == 0
            && on_background(m)
    });
    let now = Click { win: m.hwnd.0 as isize, gen: g0, time: m.time, x: m.pt.x, y: m.pt.y };
    let (cx, cy) = (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK));
    let (next, up) = next_click(LAST_CLICK.get(), now, bg, GetDoubleClickTime(), cx, cy);
    LAST_CLICK.set(next);
    up.then(|| (i, g0, svh.get().0 as isize))
}

pub(crate) unsafe fn cycle(d: i32) {
    let (a, n) = app(|a| (a.s.active as i32, a.s.panes() as i32));
    let i = (a + d).rem_euclid(n) as usize;
    set_active(i);
    focus_view(i);
}

/// The slot of the nearest cell wholly to one side of `cur` (dx/dy = -1, 0 or 1) that overlaps it across that axis.
pub(crate) fn neighbour(cells: &[(usize, RECT)], cur: usize, dx: i32, dy: i32) -> Option<usize> {
    let &(_, c) = cells.iter().find(|(s, _)| *s == cur)?;
    let mid = |r: &RECT| if dx != 0 { r.top + r.bottom } else { r.left + r.right };
    cells
        .iter()
        .filter(|(s, _)| *s != cur)
        .filter_map(|&(s, r)| {
            let gap = match (dx, dy) {
                (-1, _) => c.left - r.right,
                (1, _) => r.left - c.right,
                (_, -1) => c.top - r.bottom,
                _ => r.top - c.bottom,
            };
            let overlap = if dx != 0 { r.top.max(c.top) < r.bottom.min(c.bottom) } else { r.left.max(c.left) < r.right.min(c.right) };
            (gap >= 0 && overlap).then_some((gap, (mid(&r) - mid(&c)).abs(), s))
        })
        .min()
        .map(|(_, _, s)| s)
}

// Like Windows Terminal's Alt+arrow or tmux's prefix+arrow: move to the pane on screen in that direction.
pub(crate) unsafe fn move_pane(dx: i32, dy: i32) {
    let (cells, cur) = app(|a| {
        let g = geo(a);
        let cells = (0..a.s.panes()).filter(|&i| a.panes[i].is_some()).map(|i| (i, cell_rect(&g, &a.s.heights, i))).collect::<Vec<_>>();
        (cells, a.s.active)
    });
    if let Some(j) = neighbour(&cells, cur, dx, dy) {
        set_active(j);
        focus_view(j);
    }
}

/// The live pane on screen next to the active one.
fn active_neighbour(dx: i32, dy: i32) -> Option<usize> {
    let (cells, cur) = app(|a| {
        let g = geo(a);
        let cells = (0..a.s.panes()).filter(|&i| a.panes[i].is_some()).map(|i| (i, cell_rect(&g, &a.s.heights, i))).collect::<Vec<_>>();
        (cells, a.s.active)
    });
    neighbour(&cells, cur, dx, dy)
}

/// Ctrl+Alt+Shift+arrow: the active pane trades places with the one in that direction and stays active.
pub(crate) unsafe fn swap_dir(dx: i32, dy: i32) {
    if let Some(j) = active_neighbour(dx, dy) {
        swap_panes(app(|a| a.s.active), j);
    }
}

/// Two panes trade places. Nothing is rebuilt, so each keeps its folder, history, selection, scroll, filter and columns.
pub(crate) unsafe fn swap_panes(i: usize, j: usize) {
    let Some([si, sj]) = app(|a| {
        let live = |k: usize| k < a.s.panes() && a.panes[k].is_some();
        (i != j && live(i) && live(j) && a.drag.is_none()).then(|| [i, j].map(|k| a.panes[k].as_ref().map(|p| p.slot.clone()).unwrap_or_default()))
    }) else {
        return;
    };
    {
        // Filters and slots together, so a view thread sees both before or both after.
        let mut f = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
        f.swap(i, j);
        si.store(j, Ordering::SeqCst);
        sj.store(i, Ordering::SeqCst);
    }
    let panes = app(|a| {
        a.live_filters.swap(i, j);
        a.panes.swap(i, j);
        let mut base = a.base.take();
        swap_in(&mut a.s, base.as_mut(), i, j);
        a.base = base;
        let tab = a.s.tab;
        if let Some(b) = a.tab_base.get_mut(tab) {
            swap_tab_slots(&mut b.grid, i, j);
        }
        a.s.anchor();
        a.zoom = None;
        let to = |k: &mut usize| {
            if *k == i {
                *k = j;
            } else if *k == j {
                *k = i;
            }
        };
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
        a.status_key.clear();
        let _ = pull_dirty(a);
        [i, j].map(|k| a.panes[k].as_ref().map(|p| p.hwnd))
    });
    for k in [i, j] {
        SEL_REV[k].fetch_add(1, Ordering::Relaxed);
    }
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    relayout();
    for (k, h) in panes.into_iter().enumerate() {
        if let Some(h) = h {
            name_fields(h, if k == 0 { i } else { j });
        }
    }
    redraw_all();
    refresh_save_button();
    note_save();
    set_active(j);
    focus_view(j);
}

// Like tmux's pane zoom: the active pane takes the whole grid until toggled back or another pane is picked.
pub(crate) unsafe fn toggle_zoom() {
    if app(|a| a.drag.is_some() || a.s.panes() <= 1) {
        return;
    }
    let i = app(|a| {
        a.zoom = if a.zoom.is_some() { None } else { Some(a.s.active) };
        a.s.active
    });
    relayout();
    redraw_all();
    focus_view(i);
}

/// The left panel (folder tree and favourites) gives its room to the panes. The width is kept.
pub(crate) unsafe fn toggle_tree() {
    if app(|a| a.drag.is_some()) {
        return;
    }
    let f = GetFocus();
    let (had, i) = app(|a| {
        a.s.tree_on ^= true;
        let on = |h: HWND| !h.0.is_null() && (h == f || IsChild(h, f).as_bool());
        (on(a.tree_hwnd) || on(a.side_hwnd), a.s.active)
    });
    relayout();
    redraw_all();
    if had {
        focus_view(i);
    }
    save_session();
    settings::redraw();
}

pub(crate) unsafe fn set_layout(h: Vec<usize>) {
    if h.is_empty() || h.iter().any(|&n| n == 0) {
        return;
    }
    app(|a| a.focus_req = HWND::default());
    for i in 0..MAXP {
        save_view(i);
    }
    let (perm, slots) = app(|a| {
        let (perm, home) = remap(&a.s.heights, &h, &a.home);
        a.home = home;
        (perm, perm.map(|k| a.panes.get(k).and_then(|p| p.as_ref()).map(|p| p.slot.clone())))
    });
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
    let gone = app(|a| {
        let (old_h, old_active) = (a.s.heights.clone(), a.s.active);
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
        a.s.anchor();
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
        let n: usize = h.iter().sum();
        // permute_in already moved `active` with its pane. Off the new grid means that cell is gone.
        let gone = a.s.active >= n;
        if gone {
            let (c, r) = slot(&old_h, old_active);
            let c = c.min(h.len() - 1);
            a.s.active = h[..c].iter().sum::<usize>() + r.min(h[c] - 1);
        }
        a.s.set_heights(h);
        let _ = pull_dirty(a);
        gone
    });
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    relayout();
    let named = app(|a| a.panes.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|p| (p.hwnd, i))).collect::<Vec<_>>());
    for (hwnd, i) in named {
        name_fields(hwnd, i);
    }
    redraw_all();
    refresh_save_button();
    note_save();
    if gone {
        let i = app(|a| a.s.active);
        if view_of::<IShellView>(i).is_some() {
            if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
                sync_tree(&item);
            }
            focus_view(i);
        } else {
            app(|a| a.focus_req = a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd).unwrap_or_default());
        }
        mark_drive();
    }
}

// Same switch as Explorer's "Hidden items" (global shell setting).
pub(crate) unsafe fn hidden() -> bool {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 & 1 != 0
}

pub(crate) unsafe fn toggle_hidden() {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 ^= 1;
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, true);
    for i in 0..MAXP {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
    settings::redraw();
}

