// Favourites, one half of the left panel. One owner-drawn window: groups, files, and
// folders from the same tree the Ctrl+B list uses, plus two virtual Explorer groups on top.
// Pinned folders come first. Favourite files sit directly under them, and only when Explorer has any.

use crate::imports::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;
use crate::shell::*;
use super::*;
use windows::Win32::UI::Controls::{TTF_ABSOLUTE, TTF_TRACK, TTM_SETMAXTIPWIDTH, TTM_TRACKACTIVATE, TTM_TRACKPOSITION};

const ID_SIDE_EDIT: i32 = 209;
/// Tracking tool for a row in the favourites panel. Lock is 7, tab paths 8, address 9.
const FAV_TIP_ID: usize = 10;
/// Tracking tool for a row in the favourites window.
const FAV_POP_TIP_ID: usize = 11;

/// Favourites row height in px. `tree` is the folder tree's `TVM_GETITEMHEIGHT`.
/// When that control is not up yet, `font_px` (the UI font's line height, already scaled
/// for DPI) plus the padding a tree view adds.
pub(crate) fn fav_row_height(tree: i32, font_px: i32) -> i32 {
    if tree > 0 {
        tree
    } else {
        (font_px + 4).max(1)
    }
}

unsafe fn sys_tree() -> HWND {
    let host = app(|a| a.tree_hwnd);
    if host.0.is_null() {
        HWND::default()
    } else {
        FindWindowExW(Some(host), None, w!("SysTreeView32"), PCWSTR::null()).unwrap_or_default()
    }
}

/// The folder tree's current item height, or 0 when the control is not created yet.
unsafe fn tree_item_height() -> i32 {
    let tv = sys_tree();
    if tv.0.is_null() {
        return 0;
    }
    let h = SendMessageW(tv, TVM_GETITEMHEIGHT, None, None).0 as i32;
    if h > 0 { h } else { 0 }
}

fn row_h() -> i32 {
    unsafe {
        let tree = tree_item_height();
        let (fh, dpi) = app(|a| (a.fh, a.dpi));
        fav_row_height(tree, if fh > 0 { fh } else { 16 * dpi.max(1) / 96 })
    }
}

/// Give the folder tree the UI font (so its item height follows DPI)
/// and keep the favourites scroll inside the new row size.
pub(crate) unsafe fn sync_fav_metrics() {
    let (font, fh, dpi) = app(|a| (a.font, a.fh, a.dpi));
    let tv = sys_tree();
    if !tv.0.is_null() && !font.is_invalid() {
        SendMessageW(tv, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
    _ = (fh, dpi);
    clamp_side_scroll();
}

fn rows() -> Vec<FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        let pins = if a.s.pins_open { pin_list() } else { Vec::new() };
        let files = pin_files();
        let pins_label = pins_section_label();
        let files_label = files_section_label();
        compose_side(&a.s.tree, a.s.pins_open, &pins, pins_label, a.s.files_open, &files, files_label)
    })
}

fn side_view_h() -> i32 {
    let h = app(|a| a.side_hwnd);
    let mut rc = RECT::default();
    if !h.0.is_null() {
        unsafe { _ = GetClientRect(h, &mut rc) };
    }
    rc.bottom
}

/// Height of the removal error, or 0 when there is nothing to say.
fn note_band() -> i32 {
    if pin_file_note().is_empty() { 0 } else { row_h().saturating_mul(3) }
}

/// The list's visible height. The error band sits under it and does not scroll away.
fn list_page() -> i32 {
    (side_view_h() - note_band()).max(0)
}

unsafe fn paint_pin_note(dc: HDC, rc: RECT, font: HFONT, dpi: i32) {
    let note = pin_file_note();
    if note.is_empty() {
        return;
    }
    let s = |v: i32| v * dpi / 96;
    let top = (rc.bottom - note_band()).max(rc.top);
    fill(dc, RECT { left: 0, top, right: rc.right, bottom: rc.bottom }, side_ground());
    let ink = if DARK_ON.load(Ordering::Relaxed) { 0xFFB4AB } else { 0x9F1C14 };
    fill(dc, RECT { left: 0, top, right: rc.right, bottom: top + s(2).max(1) }, ink);
    wrap_text(dc, RECT { left: s(8), top: top + s(4), right: rc.right - s(8), bottom: rc.bottom - s(2) }, &note, font, ink, false);
}

/// Largest vertical scroll offset: total row height minus what the panel shows.
fn max_side_scroll() -> i32 {
    let total = rows().len() as i32 * row_h();
    (total - list_page()).max(0)
}

/// Mirror the scroll state into the native vertical scrollbar (hidden while everything fits).
pub(crate) fn sync_side_scrollbar() {
    let h = app(|a| a.side_hwnd);
    if h.0.is_null() {
        return;
    }
    let total = rows().len() as i32 * row_h();
    let (page, pos) = (list_page(), app(|a| a.side_scroll));
    let si = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (total - 1).max(0),
        nPage: page.max(0) as u32,
        nPos: pos,
        nTrackPos: 0,
    };
    unsafe { windows::Win32::UI::Controls::SetScrollInfo(h, SB_VERT, &si, true) };
}

pub(crate) fn clamp_side_scroll() {
    let max = max_side_scroll();
    let changed = app(|a| {
        let next = a.side_scroll.min(max);
        let changed = next != a.side_scroll;
        a.side_scroll = next;
        changed
    });
    sync_side_scrollbar();
    if changed {
        unsafe { dismiss_side_fav_tip() };
    }
}

fn mix_str(h: u64, s: &str) -> u64 {
    let mut h = h.wrapping_mul(31).wrapping_add(s.chars().count() as u64);
    for c in s.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u64);
    }
    h
}

/// Changes when a row is added, removed, renamed, or its path changes. Character-wise, so a
/// non-ASCII path is part of the value and is never cut.
fn fav_rows_mark() -> u64 {
    mark_rows(&rows())
}

fn mark_rows(list: &[FavRow]) -> u64 {
    let mut h = 0x9E37_79B9u64.wrapping_add(list.len() as u64);
    for r in list {
        h = mix_str(h, &r.path);
        h = mix_str(h, &r.name);
        h = h.wrapping_mul(31).wrapping_add(r.id.len() as u64);
        for id in &r.id {
            h = h.wrapping_mul(31).wrapping_add(*id as u64);
        }
        h = h.wrapping_mul(31).wrapping_add(r.depth as u64);
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.group));
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.open));
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.file));
        h = h.wrapping_mul(31).wrapping_add(match r.kind {
            RowKind::Tree => 1,
            RowKind::Pins => 2,
            RowKind::Files => 3,
        });
    }
    h
}

fn drag_of(r: &FavRow) -> SideDrag {
    SideDrag { id: r.id.clone(), kind: r.kind, group: r.group, file: r.file, path: r.path.clone(), name: r.name.clone() }
}

fn tree_pos(list: &[FavRow], id: &[usize]) -> Option<usize> {
    list.iter().position(|r| r.in_tree() && r.id == id)
}

/// Location tooltip for one favourites row.
/// A filesystem path is the absolute path. A shell place has no file path, so the tip is its
/// display name, or the parsing name when that name is empty — the address-bar rule.
/// Every leaf is included, whether or not the label on screen is cut off. A group has no location.
/// A missing leaf keeps the location and adds a short note. Nothing is cut on a byte index.
pub(crate) fn fav_tip_text(path: &str, name: &str, group: bool, missing: bool) -> String {
    if group {
        return String::new();
    }
    let loc = addr_tip_text(path, name);
    if loc.is_empty() {
        return String::new();
    }
    if !missing {
        return loc;
    }
    let note = fav_missing_note();
    let mut out = String::with_capacity(loc.len() + 2 + note.len());
    out.push_str(&loc);
    out.push('\r');
    out.push('\n');
    out.push_str(note);
    out
}

fn fav_row_tip(row: &FavRow) -> String {
    fav_tip_text(&row.path, &row.name, row.group, !row.group && known_missing(&row.path))
}

fn fav_tip_info(owner: HWND, id: usize, label: &HSTRING) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: id,
        lpszText: PWSTR(label.as_ptr() as *mut u16),
        ..Default::default()
    }
}

/// Show `text` on the window tooltip, or hide that tool when `text` is empty.
/// The tool stays installed. It sits below the pointer so a click still hits the row.
unsafe fn place_fav_tip(owner: HWND, id: usize, installed: bool, text: &str) -> bool {
    let tip = app(|a| a.tip);
    if tip.0.is_null() || owner.0.is_null() {
        return installed;
    }
    if text.is_empty() {
        if installed {
            let label = HSTRING::from("");
            let ti = fav_tip_info(owner, id, &label);
            SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&ti as *const _ as isize)));
        }
        return installed;
    }
    let label = HSTRING::from(text);
    let ti = fav_tip_info(owner, id, &label);
    let lp = LPARAM(&ti as *const _ as isize);
    // Wide enough that a normal path stays one line. A positive width is what lets the missing note break.
    SendMessageW(tip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(720)));
    if !installed {
        SendMessageW(tip, TTM_ADDTOOLW, None, Some(lp));
    }
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(lp));
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return true;
    }
    let below = pt.y + 22 * app(|a| a.dpi) / 96;
    let at = LPARAM((pt.x as u16 as usize as isize) | ((below as u16 as usize as isize) << 16));
    SendMessageW(tip, TTM_TRACKPOSITION, None, Some(at));
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(lp));
    true
}

unsafe fn cursor_in_client(h: HWND) -> Option<(i32, i32)> {
    if h.0.is_null() {
        return None;
    }
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() || !ScreenToClient(h, &mut pt).as_bool() {
        return None;
    }
    let mut rc = RECT::default();
    if GetClientRect(h, &mut rc).is_err() {
        return None;
    }
    (pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom).then_some((pt.x, pt.y))
}

/// Hide the panel tip and let the next hover show it.
pub(crate) unsafe fn hide_side_fav_tip() {
    let (owner, on) = app(|a| (a.side_hwnd, a.fav_tip));
    app(|a| a.fav_tip_hold = None);
    if on {
        // place_fav_tip borrows the app state itself, so it runs outside app().
        let now = place_fav_tip(owner, FAV_TIP_ID, true, "");
        app(|a| a.fav_tip = now);
    }
}

/// Hide the panel tip after a scroll or a change of the rows. The pointer has to move before it returns.
pub(crate) unsafe fn dismiss_side_fav_tip() {
    let h = app(|a| a.side_hwnd);
    let on = app(|a| a.fav_tip);
    if on {
        let now = place_fav_tip(h, FAV_TIP_ID, true, "");
        app(|a| a.fav_tip = now);
    }
    app(|a| a.fav_tip_hold = cursor_in_client(h));
}

/// `x` and `y` are client coordinates. A drag, or the point that just scrolled away, shows nothing.
pub(crate) unsafe fn show_side_fav_tip(x: i32, y: i32, row: Option<&FavRow>) {
    if app(|a| a.side_drag.is_some() || a.side_external) {
        hide_side_fav_tip();
        return;
    }
    if app(|a| a.fav_tip_hold) == Some((x, y)) {
        return;
    }
    app(|a| a.fav_tip_hold = None);
    let owner = app(|a| a.side_hwnd);
    let text = row.map(fav_row_tip).unwrap_or_default();
    let installed = app(|a| a.fav_tip);
    let now = place_fav_tip(owner, FAV_TIP_ID, installed, &text);
    app(|a| a.fav_tip = now);
}

/// The favourites window uses the same tooltip. `owner` is that window; the desk is already current.
pub(crate) unsafe fn show_pop_fav_tip(owner: HWND, row: Option<&FavRow>) {
    let text = row.map(fav_row_tip).unwrap_or_default();
    let installed = app(|a| a.fav_pop_tip);
    let now = place_fav_tip(owner, FAV_POP_TIP_ID, installed, &text);
    app(|a| a.fav_pop_tip = now);
}

pub(crate) unsafe fn hide_pop_fav_tip(owner: HWND) {
    let on = app(|a| a.fav_pop_tip);
    if on {
        let now = place_fav_tip(owner, FAV_POP_TIP_ID, true, "");
        app(|a| a.fav_pop_tip = now);
    }
}

/// The window is going away, so the tool has to come off the tooltip. The next window adds its own.
pub(crate) unsafe fn drop_pop_fav_tip(owner: HWND) {
    let (tip, on) = app(|a| (a.tip, a.fav_pop_tip));
    app(|a| a.fav_pop_tip = false);
    if !on || tip.0.is_null() || owner.0.is_null() {
        return;
    }
    let label = HSTRING::from("");
    let ti = fav_tip_info(owner, FAV_POP_TIP_ID, &label);
    let lp = LPARAM(&ti as *const _ as isize);
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
    SendMessageW(tip, TTM_DELTOOLW, None, Some(lp));
}

fn row_at(y: i32) -> Option<usize> {
    let n = rows().len();
    let scroll = app(|a| a.side_scroll);
    if y < 0 || y >= list_page() || n == 0 {
        return None;
    }
    let i = (y + scroll) / row_h();
    (i >= 0 && (i as usize) < n).then_some(i as usize)
}

fn chevron_hit(x: i32, row: &FavRow, dpi: i32) -> bool {
    if !row.group {
        return false;
    }
    let s = |v: i32| v * dpi / 96;
    x < s(4) + s(14) * row.depth.min(8) as i32 + s(18)
}

/// Throw away every window's group wash. The next paint builds a brush for the accent in effect then.
pub(crate) unsafe fn drop_group_brushes() {
    let old = app(|a| {
        a.desks.iter_mut().map(|d| {
            d.group_brush_rgb = 0;
            std::mem::replace(&mut d.group_brush, HBRUSH::default())
        }).collect::<Vec<_>>()
    });
    for b in old {
        if !b.is_invalid() {
            _ = DeleteObject(b.into());
        }
    }
}

unsafe fn group_wash_brush(rgb: u32) -> HBRUSH {
    let cached = app(|a| (!a.group_brush.is_invalid() && a.group_brush_rgb == rgb).then_some(a.group_brush));
    if let Some(b) = cached {
        return b;
    }
    let brush = CreateSolidBrush(cref(rgb));
    if brush.is_invalid() {
        return brush;
    }
    let old = app(|a| {
        a.group_brush_rgb = rgb;
        std::mem::replace(&mut a.group_brush, brush)
    });
    if !old.is_invalid() && old != brush {
        _ = DeleteObject(old.into());
    }
    brush
}

pub(crate) unsafe fn paint_side(h: HWND) {
    pins_on_paint();
    let list = rows();
    app(|a| a.side_rows_mark = Some(mark_rows(&list)));
    let (dpi, font, icon, hot, sel, scroll, drop, accent) = app(|a| {
        (a.dpi, a.font, a.icon_font, a.side_hot, a.side_sel, a.side_scroll, a.side_drop, a.s.accent_now())
    });
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let rh = row_h();
        let list_bottom = rc.bottom - note_band();
        fill(dc, rc, side_ground());
        if list.is_empty() {
            text(dc, RECT { left: s(12), top: s(8), right: rc.right - s(8), bottom: s(48) }, tr("No favourites yet", "즐겨찾기가 없어요", "お気に入りはまだありません"), font, pal().sub, DT_LEFT);
            text(dc, RECT { left: s(12), top: s(28), right: rc.right - s(8), bottom: s(72) }, tr("Right-click to add a group", "우클릭해서 그룹을 만드세요", "右クリックでグループを作成"), font, pal().sub, DT_LEFT);
            paint_pin_note(dc, rc, font, dpi);
            return;
        }
        let any_group = list.iter().any(|f| f.group);
        let bold = if any_group { bold_ui_font() } else { font };
        let wash = if any_group {
            group_wash_brush(group_tint(accent, side_ground(), DARK_ON.load(Ordering::Relaxed)))
        } else {
            HBRUSH::default()
        };
        for (i, f) in list.iter().enumerate() {
            let y = i as i32 * rh - scroll;
            if y + rh < 0 || y >= list_bottom {
                continue;
            }
            let r = RECT { left: 0, top: y, right: rc.right, bottom: y + rh };
            let into = drop.is_some_and(|(row, zone)| row == i && zone == 1);
            if f.group && !wash.is_invalid() {
                FillRect(dc, &r, wash);
            }
            // Selection and hover replace the wash. The selected fill is mostly accent, so it wins.
            if sel == Some(i) {
                fill(dc, r, blend(accent, side_ground(), 0.16));
            } else if hot == Some(i) {
                fill(dc, r, blend(pal().bg, pal().card, if DARK_ON.load(Ordering::Relaxed) { 0.5 } else { 0.7 }));
            }
            if into {
                fill(dc, r, blend(accent, side_ground(), 0.28));
            }
            let indent = s(14) * f.depth.min(8) as i32;
            let mut x = s(4) + indent;
            let label_left;
            if f.group {
                let chev = RECT { left: x, top: y, right: x + s(16), bottom: y + rh };
                text(dc, chev, if f.open { "\u{E70D}" } else { "\u{E76C}" }, icon, pal().sub, DT_CENTER);
                x += s(16);
                let ir = RECT { left: x, top: y, right: x + s(18), bottom: y + rh };
                let glyph_ink = if sel == Some(i) || into { on_color(accent) } else { accent };
                text(dc, ir, "\u{E8B7}", icon, glyph_ink, DT_CENTER);
                label_left = ir.right + s(4);
            } else {
                let slot = (16 * dpi / 96).max(1);
                let iy = y + (rh - slot).max(0) / 2;
                if let Some((list, index, dim)) = shell_icon(&f.path, f.file) {
                    draw_fav_icon(dc, list, index, x, iy, slot, dim);
                }
                label_left = x + slot + s(4);
            }
            let label = if f.name.is_empty() { short_name(&f.path) } else { f.name.clone() };
            let label_font = if f.group { bold } else { font };
            // A path the background check (or a click) has found gone is drawn in the secondary colour.
            let ink = if !f.group && known_missing(&f.path) { pal().sub } else { pal().text };
            text(dc, RECT { left: label_left, right: rc.right - s(6), ..r }, &label, label_font, ink, DT_LEFT);
        }
        if !list.iter().any(|f| f.in_tree()) {
            let y = list.len() as i32 * rh - scroll + s(8);
            text(dc, RECT { left: s(12), top: y, right: rc.right - s(8), bottom: y + s(22) }, tr("No favourites yet", "즐겨찾기가 없어요", "お気に入りはまだありません"), font, pal().sub, DT_LEFT);
            text(dc, RECT { left: s(12), top: y + s(20), right: rc.right - s(8), bottom: y + s(44) }, tr("Right-click to add a group", "우클릭해서 그룹을 만드세요", "右クリックでグループを作成"), font, pal().sub, DT_LEFT);
        }
        if let Some((row, zone)) = drop {
            if zone != 1 {
                if let Some(f) = list.get(row) {
                    let at = if zone == 0 { row } else { after_row(&list, row) };
                    let line_y = if zone == 0 { row as i32 * rh - scroll } else { at as i32 * rh - scroll + rh - s(2) };
                    // A line under the Explorer groups is a new root favourite, not a child of that row.
                    let indent = if f.in_tree() { s(14) * f.depth.min(8) as i32 } else { 0 };
                    if line_y + s(2) >= 0 && line_y < list_bottom {
                        fill(dc, RECT { left: s(8) + indent, top: line_y, right: rc.right - s(8), bottom: line_y + s(2) }, accent);
                    }
                }
            }
        }
        paint_pin_note(dc, rc, font, dpi);
    });
    if list.iter().any(|f| !f.group) {
        arm_icon_prewarm(h);
    }
}

/// The rename box over its row. Hidden unless a rename is in progress.
pub(crate) unsafe fn place_side_edit() {
    let (edit, id, host, scroll, dpi) = app(|a| (a.side_edit, a.side_edit_at.clone(), a.side_hwnd, a.side_scroll, a.dpi));
    if edit.0.is_null() || id.is_empty() || host.0.is_null() {
        return;
    }
    let list = rows();
    let rh = row_h();
    let y = tree_pos(&list, &id).map(|i| i as i32 * rh - scroll).unwrap_or(0);
    let s = |v: i32| v * dpi / 96;
    let outer = RECT { left: s(8), top: y, right: s(8) + s(220), bottom: y + rh };
    place_edit(edit, outer, true);
}

pub(crate) unsafe fn begin_side_rename(id: &[usize]) {
    let list = rows();
    let label = list.iter().find(|r| r.in_tree() && r.id == id).map(|r| if r.name.is_empty() { short_name(&r.path) } else { r.name.clone() }).unwrap_or_default();
    let (host, font) = app(|a| (a.side_hwnd, a.font));
    if host.0.is_null() {
        return;
    }
    let edit = app(|a| a.side_edit);
    let edit = if edit.0.is_null() {
        let e = child(host, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), ID_SIDE_EDIT, font);
        app(|a| a.side_edit = e);
        e
    } else {
        edit
    };
    if edit.0.is_null() {
        return;
    }
    app(|a| {
        a.side_edit_at = id.to_vec();
        a.side_edit_key = node_key(&a.s.tree, id);
    });
    _ = SetWindowTextW(edit, &HSTRING::from(label.as_str()));
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    place_side_edit();
    _ = SetFocus(Some(edit));
    let len = GetWindowTextLengthW(edit) as usize;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(len as isize)));
}

pub(crate) unsafe fn end_side_rename(save: bool) {
    let (edit, id, key) = app(|a| (a.side_edit, std::mem::take(&mut a.side_edit_at), a.side_edit_key.take()));
    if edit.0.is_null() || id.is_empty() {
        return;
    }
    if save {
        let name = text_of(edit);
        // The tree may have changed under the edit box; rename the node it was opened on, or nothing.
        let done = app(|a| {
            let Some(id) = key.as_ref().and_then(|k| locate(&a.s.tree, &id, k)) else { return false };
            rename_at(&mut a.s.tree, &id, name.trim());
            a.s.sync_favs();
            true
        });
        if done {
            favs_changed();
        }
    }
    _ = ShowWindow(edit, SW_HIDE);
    let host = app(|a| a.side_hwnd);
    if !host.0.is_null() {
        _ = InvalidateRect(Some(host), None, false);
    }
}

fn changed() {
    unsafe { favs_changed() };
}

fn new_group(parent: &[usize]) {
    let name = tr("Group", "그룹", "グループ").to_string();
    app(|a| {
        a.s.ensure_fav_tree();
        if !parent.is_empty() {
            set_open(&mut a.s.tree, parent, true);
        }
        add_group(&mut a.s.tree, parent, &name);
        a.s.sync_favs();
    });
    let id = rows().iter().rev().find(|r| r.in_tree() && r.group && r.name == name).map(|r| r.id.clone());
    changed();
    if let Some(id) = id {
        unsafe { begin_side_rename(&id) };
    }
}

fn toggle_row(row: &FavRow) {
    if row.group && !row.in_tree() {
        let open = app(|a| match row.kind {
            RowKind::Pins => {
                a.s.pins_open = !a.s.pins_open;
                a.s.pins_open
            }
            RowKind::Files => {
                a.s.files_open = !a.s.files_open;
                a.s.files_open
            }
            RowKind::Tree => false,
        });
        if open {
            request_pins(false);
        }
        changed();
        clamp_side_scroll();
        return;
    }
    if !row.in_tree() || !row.group {
        return;
    }
    app(|a| {
        toggle_open(&mut a.s.tree, &row.id);
    });
    changed();
    clamp_side_scroll();
}

unsafe fn side_menu(h: HWND, index: Option<usize>, x: i32, y: i32) {
    let row = index.and_then(|i| rows().get(i).cloned());
    if row.as_ref().is_some_and(|r| r.group && !r.in_tree()) {
        return;
    }
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, label: &str| {
        _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(label));
    };
    let pin_leaf = row.as_ref().is_some_and(|r| r.kind == RowKind::Pins && !r.group);
    let file_leaf = row.as_ref().is_some_and(|r| r.kind == RowKind::Files && !r.group);
    if pin_leaf {
        add(6, tr("Unpin from Explorer", "탐색기에서 고정 해제", "エクスプローラーからピン留めを外す"));
        add(4, tr("Open", "열기", "開く"));
    } else if file_leaf {
        add(6, tr("Remove from Explorer favourites", "탐색기 즐겨찾기에서 빼기", "エクスプローラーのお気に入りから削除"));
        add(4, tr("Open", "열기", "開く"));
        add(5, tr("Reveal", "위치 표시", "場所を表示"));
    } else {
        add(1, tr("New group", "새 그룹", "新しいグループ"));
        if row.is_some() {
            add(2, tr("Rename", "이름 변경", "名前の変更"));
            add(3, tr("Delete", "삭제", "削除"));
        }
        if row.as_ref().is_some_and(|r| !r.group) {
            add(4, tr("Open", "열기", "開く"));
        }
        if row.as_ref().is_some_and(|r| r.file) {
            add(5, tr("Reveal", "위치 표시", "場所を表示"));
        }
    }
    let mut pt = POINT { x, y };
    _ = ClientToScreen(h, &mut pt);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, h, None).0;
    _ = DestroyMenu(menu);
    let Some(row) = row else {
        if cmd == 1 {
            new_group(&[]);
        }
        return;
    };
    if !row.in_tree() {
        match cmd {
            4 => open_fav_row(&row),
            5 if row.file => {
                if let Some(parent) = parent_of(&row.path) {
                    let i = active_pane();
                    navigate(i, &parent);
                    focus_view(i);
                }
            }
            6 => unpin_path(&row.path, row.kind == RowKind::Files),
            _ => {}
        }
        return;
    }
    match cmd {
        1 => {
            let parent: Vec<usize> = if row.group { row.id.clone() } else { row.id[..row.id.len() - 1].to_vec() };
            new_group(&parent);
        }
        2 => begin_side_rename(&row.id),
        3 => {
            app(|a| {
                delete_at(&mut a.s.tree, &row.id);
                a.s.sync_favs();
                a.side_sel = None;
            });
            changed();
        }
        4 => open_fav_row(&row),
        6 => unpin_path(&row.path, row.kind == RowKind::Files),
        5 => {
            if let Some(parent) = parent_of(&row.path) {
                let i = active_pane();
                navigate(i, &parent);
                focus_view(i);
            }
        }
        _ => {}
    }
}

fn set_drop(next: Option<(usize, u8)>) -> bool {
    app(|a| {
        let paint = a.side_drop != next;
        a.side_drop = next;
        paint
    })
}

fn track_drop(y: i32) -> bool {
    let drag = app(|a| a.side_drag.clone());
    let Some(drag) = drag else { return false };
    let list = rows();
    let Some(i) = row_at(y) else { return set_drop(None) };
    let Some(target) = list.get(i) else { return set_drop(None) };
    let kind = side_act(drag.kind, drag.group, drag.file, drag.path.is_empty(), target.kind);
    let next = match kind {
        SideAct::None => None,
        SideAct::Pin => list.iter().position(|r| r.kind == RowKind::Pins && r.group).map(|row| (row, 1)),
        SideAct::PinFile => list.iter().position(|r| r.kind == RowKind::Files && r.group).map(|row| (row, 1)),
        SideAct::Copy | SideAct::Move => {
            let scroll = app(|a| a.side_scroll);
            let rh = row_h();
            let top = i as i32 * rh - scroll;
            Some((i, drop_zone(target.group, y - top, rh)))
        }
    };
    set_drop(next)
}

fn repaint_side() {
    clamp_side_scroll();
    let h = app(|a| a.side_hwnd);
    if !h.0.is_null() {
        unsafe { _ = InvalidateRect(Some(h), None, false) };
    }
}

/// The shell folder updates after the verb returns. One read shortly after, off the UI thread.
fn refresh_pins_soon() {
    let h = app(|a| a.side_hwnd);
    if h.0.is_null() {
        request_pins(true);
        return;
    }
    unsafe { _ = SetTimer(Some(h), TIMER_PINS, 700, None) };
}

fn pin_home(path: &str, name: &str, file: bool) {
    if path.is_empty() {
        return;
    }
    let added = remember_pinned(path, name, file);
    if !shell_home_verb(path, true) {
        if added {
            forget_pinned(path);
        }
        repaint_side();
        unsafe { _ = MessageBeep(MB_ICONWARNING) };
        return;
    }
    let was_open = app(|a| if file { a.s.files_open } else { a.s.pins_open });
    app(|a| {
        if file {
            a.s.files_open = true;
        } else {
            a.s.pins_open = true;
        }
    });
    if was_open {
        unsafe {
            across_windows(|| {
                let h = app(|a| a.side_hwnd);
                if !h.0.is_null() {
                    _ = InvalidateRect(Some(h), None, false);
                }
            });
        }
    } else {
        changed();
    }
    refresh_pins_soon();
}

fn unpin_path(path: &str, file: bool) {
    if file {
        // The row stays until a Home re-read omits it. A verb that returns success and
        // leaves the file in place used to hide it for 3 seconds, then the next read put it back.
        shell_unfavorite_file(path);
        repaint_side();
        return;
    }
    forget_pinned(path);
    repaint_side();
    let removed = shell_home_verb(path, false);
    refresh_pins_soon();
    if !removed {
        unsafe { _ = MessageBeep(MB_ICONWARNING) };
    }
}

fn side_inside(h: HWND, x: i32, y: i32) -> bool {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(h, &mut rc) };
    x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
}

fn nudge_side(down: bool) {
    let list = rows();
    let Some(row) = app(|a| a.side_sel).and_then(|i| list.get(i).cloned()).filter(|r| r.in_tree()) else { return };
    let moved = app(|a| {
        let id = nudge_node(&mut a.s.tree, &row.id, down);
        if id.is_some() {
            a.s.sync_favs();
        }
        id
    });
    if let Some(id) = moved {
        let i = tree_pos(&rows(), &id);
        app(|a| a.side_sel = i);
        changed();
    }
}

fn finish_drop() {
    let list = rows();
    let (drag, onto) = app(|a| {
        let onto = a.side_drop.and_then(|(i, zone)| list.get(i).cloned().map(|r| (r, zone)));
        let drag = a.side_drag.clone();
        a.side_drag = None;
        a.side_drop = None;
        a.side_press = None;
        (drag, onto)
    });
    let Some(drag) = drag else { return };
    let Some((target, zone)) = onto else {
        repaint_side();
        return;
    };
    match side_act(drag.kind, drag.group, drag.file, drag.path.is_empty(), target.kind) {
        SideAct::None => repaint_side(),
        SideAct::Pin => pin_home(&drag.path, &drag.name, false),
        SideAct::PinFile => pin_home(&drag.path, &drag.name, true),
        SideAct::Copy => {
            let name = if drag.name.is_empty() || drag.name == short_name(&drag.path) { String::new() } else { drag.name.clone() };
            let copied = app(|a| {
                let id = copy_leaf(&mut a.s.tree, &drag.path, &name, &target.id, zone, drag.file);
                if id.is_some() {
                    if zone == 1 {
                        set_open(&mut a.s.tree, &target.id, true);
                    }
                    a.s.sync_favs();
                }
                id
            });
            if let Some(id) = copied {
                let i = tree_pos(&rows(), &id);
                app(|a| a.side_sel = i);
                changed();
            } else {
                repaint_side();
            }
        }
        SideAct::Move => {
            let moved = app(|a| {
                let id = apply_drop(&mut a.s.tree, &drag.id, &target.id, zone);
                if id.is_some() {
                    reveal_drop(&mut a.s.tree, &drag.id, &target.id, zone);
                    a.s.sync_favs();
                }
                id
            });
            if let Some(id) = moved {
                let i = tree_pos(&rows(), &id);
                app(|a| a.side_sel = i);
                changed();
            } else {
                repaint_side();
            }
        }
    }
}

fn fav_exts(nodes: &[FavNode], seen: &mut std::collections::HashSet<String>, out: &mut Vec<String>) {
    for n in nodes {
        if n.is_group() {
            fav_exts(&n.children, seen, out);
        } else if n.file {
            let ext = file_ext(&n.path);
            if !ext.is_empty() && !per_file_icon(&ext) && seen.insert(ext.clone()) {
                out.push(ext);
            }
        }
    }
}

fn prewarm_fav_icons() {
    let exts = app(|a| {
        a.s.ensure_fav_tree();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        fav_exts(&a.s.tree, &mut seen, &mut out);
        for p in pin_files() {
            let ext = file_ext(&p.path);
            if !ext.is_empty() && !per_file_icon(&ext) && seen.insert(ext.clone()) {
                out.push(ext);
            }
        }
        out
    });
    prewarm_exts(&exts);
}

pub(crate) extern "system" fn side_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| side_proc_inner(h, msg, wp, lp)))
}
fn side_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        note_side(h);
        note_pins_hwnd(h);
        match msg {
            WM_DESTROY => {
                hide_side_fav_tip();
                drop_side(h);
                drop_pins_hwnd(h);
                _ = RevokeDragDrop(h);
                _ = KillTimer(Some(h), TIMER_ICON);
                _ = KillTimer(Some(h), TIMER_PINS);
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_FAV_ICON => {
                clear_icon_refresh();
                _ = InvalidateRect(Some(h), None, false);
                // A row may have just been marked missing. Refresh the note if the pointer is still on it.
                if app(|a| a.fav_tip_hold.is_none() && a.side_drag.is_none() && !a.side_external) {
                    if let Some((x, y)) = cursor_in_client(h) {
                        let hot = row_at(y);
                        let list = rows();
                        show_side_fav_tip(x, y, hot.and_then(|i| list.get(i)));
                    }
                }
            }
            WM_TIMER if wp.0 == TIMER_ICON => {
                _ = KillTimer(Some(h), TIMER_ICON);
                icon_prewarm_done();
                prewarm_fav_icons();
            }
            WM_TIMER if wp.0 == TIMER_PINS => {
                _ = KillTimer(Some(h), TIMER_PINS);
                request_pins(true);
            }
            WM_PINS => {
                // Row count changed under the panel. Reclamp the scroll, then paint now:
                // a later WM_PAINT may never arrive while the queue stays busy.
                // Pin reads retry until Explorer answers. Only a real change of the rows hides the tip.
                let mark = fav_rows_mark();
                let changed = app(|a| {
                    let changed = a.side_rows_mark.is_some_and(|prev| prev != mark);
                    a.side_rows_mark = Some(mark);
                    changed
                });
                if changed {
                    dismiss_side_fav_tip();
                }
                clamp_side_scroll();
                _ = InvalidateRect(Some(h), None, false);
                _ = UpdateWindow(h);
                if !pins_loaded() {
                    _ = SetTimer(Some(h), TIMER_PINS, 700, None);
                }
            }
            WM_PAINT => paint_side(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = row_at(y);
                let dragging = app(|a| a.side_drag.is_some());
                if dragging {
                    // The tip sits below the pointer. Hide it before the drag moves onto that spot.
                    hide_side_fav_tip();
                    if track_drop(y) {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                } else if let Some((px, py, i)) = app(|a| a.side_press) {
                    if (x - px).abs() + (y - py).abs() > 4 {
                        if let Some(row) = rows().get(i).filter(|r| !(r.group && !r.in_tree())) {
                            hide_side_fav_tip();
                            let drag = drag_of(row);
                            app(|a| a.side_drag = Some(drag));
                            if track_drop(y) {
                                _ = InvalidateRect(Some(h), None, false);
                            }
                        }
                    }
                } else if app(|a| a.side_external) {
                    hide_side_fav_tip();
                } else {
                    let paint = app(|a| {
                        let paint = a.side_hot != hot;
                        a.side_hot = hot;
                        paint
                    });
                    if paint {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    let list = rows();
                    show_side_fav_tip(x, y, hot.and_then(|i| list.get(i)));
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    _ = TrackMouseEvent(&mut t);
                }
            }
            WM_MOUSELEAVE => {
                hide_side_fav_tip();
                app(|a| a.side_hot = None);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_MOUSEWHEEL => {
                dismiss_side_fav_tip();
                let rh = row_h();
                let delta = ((wp.0 >> 16) as i16 as i32) * rh / 120;
                let max = max_side_scroll();
                app(|a| a.side_scroll = (a.side_scroll - delta).clamp(0, max));
                sync_side_scrollbar();
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_SIZE => {
                clamp_side_scroll();
                place_side_edit();
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_VSCROLL => {
                dismiss_side_fav_tip();
                let mut si = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
                _ = GetScrollInfo(h, SB_VERT, &mut si);
                let rh = row_h();
                let cur = app(|a| a.side_scroll);
                let want = match SCROLLBAR_COMMAND((wp.0 & 0xFFFF) as i32) {
                    SB_LINEUP => cur - rh,
                    SB_LINEDOWN => cur + rh,
                    SB_PAGEUP => cur - si.nPage as i32,
                    SB_PAGEDOWN => cur + si.nPage as i32,
                    SB_THUMBTRACK | SB_THUMBPOSITION => si.nTrackPos,
                    SB_TOP => 0,
                    SB_BOTTOM => i32::MAX,
                    _ => cur,
                };
                let max = max_side_scroll();
                app(|a| a.side_scroll = want.clamp(0, max));
                sync_side_scrollbar();
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_LBUTTONDOWN => {
                // Gone before the click is handled, so the tip cannot take the click or the double-click.
                hide_side_fav_tip();
                let (x, y) = xy(lp);
                _ = SetFocus(Some(h));
                if let Some(i) = row_at(y) {
                    let list = rows();
                    let dpi = app(|a| a.dpi);
                    if list.get(i).is_some_and(|r| chevron_hit(x, r, dpi)) {
                        let row = list[i].clone();
                        toggle_row(&row);
                    } else {
                        app(|a| {
                            a.side_sel = Some(i);
                            a.side_press = Some((x, y, i));
                        });
                        _ = SetCapture(h);
                        _ = InvalidateRect(Some(h), None, false);
                    }
                } else {
                    app(|a| a.side_sel = None);
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let dragged = app(|a| a.side_drag.is_some());
                if dragged {
                    // Outside the panel the drop is cancelled. Inside, the slot is this release point.
                    if !side_inside(h, x, y) {
                        app(|a| {
                            a.side_drag = None;
                            a.side_drop = None;
                            a.side_press = None;
                        });
                        repaint_side();
                    } else {
                        _ = track_drop(y);
                        finish_drop();
                    }
                } else {
                    app(|a| {
                        a.side_press = None;
                    });
                }
                _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => {
                let paint = app(|a| {
                    let paint = a.side_drag.is_some();
                    a.side_drag = None;
                    a.side_drop = None;
                    a.side_press = None;
                    paint
                });
                if paint {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_SYSKEYDOWN => {
                let vk = wp.0 as u16;
                let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
                if fav_step(vk, down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT)).is_none() || app(|a| a.side_drag.is_some() || !a.side_edit_at.is_empty()) {
                    return DefWindowProcW(h, msg, wp, lp);
                }
                nudge_side(vk == VK_DOWN.0);
            }
            WM_LBUTTONDBLCLK => {
                let y = xy(lp).1;
                if let Some(row) = row_at(y).and_then(|i| rows().get(i).cloned()) {
                    if row.group {
                        toggle_row(&row);
                    } else {
                        open_fav_row(&row);
                    }
                }
            }
            WM_RBUTTONUP => {
                // The menu opens at the pointer. The tip would sit on top of it.
                hide_side_fav_tip();
                let (x, y) = xy(lp);
                let i = row_at(y);
                if let Some(i) = i {
                    app(|a| a.side_sel = Some(i));
                }
                side_menu(h, i, x, y);
            }
            WM_KEYDOWN => match VIRTUAL_KEY(wp.0 as u16) {
                VK_DELETE => {
                    let row = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned());
                    if let Some(row) = row.filter(|r| r.in_tree()) {
                        app(|a| {
                            delete_at(&mut a.s.tree, &row.id);
                            a.s.sync_favs();
                            a.side_sel = None;
                        });
                        changed();
                    }
                }
                VK_F2 => {
                    if let Some(row) = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned()).filter(|r| r.in_tree()) {
                        begin_side_rename(&row.id);
                    }
                }
                VK_RETURN => {
                    if let Some(row) = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned()) {
                        if row.group {
                            toggle_row(&row);
                        } else {
                            open_fav_row(&row);
                        }
                    }
                }
                _ => {}
            },
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().card));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => {
                if (wp.0 & 0xffff) as i32 == ID_SIDE_EDIT && ((wp.0 >> 16) & 0xffff) as u32 == EN_KILLFOCUS {
                    end_side_rename(true);
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// A pane selection that can land on one Explorer group. Mixed files and folders are refused.
#[derive(Clone)]
enum HomeDrop {
    Folders(Vec<String>),
    Files(Vec<String>),
}

/// Paths from the drop, plus whether each one is a file when the drag data already says so.
struct RawDrop {
    path: String,
    file: Option<bool>,
}

#[derive(Clone)]
struct Pending {
    items: Vec<Dropped>,
    home: Option<HomeDrop>,
}

fn pending_of(raw: Vec<RawDrop>) -> Pending {
    let home = home_of(&raw);
    let items = raw.into_iter().map(|r| Dropped { path: r.path, file: r.file.unwrap_or(false) }).collect();
    Pending { items, home }
}

/// Pinning needs a known file-or-folder bit for every item. An unknown bit is not guessed:
/// the same drop can still be added to favourites.
fn home_of(raw: &[RawDrop]) -> Option<HomeDrop> {
    if raw.is_empty() || raw.iter().any(|r| r.file.is_none()) {
        return None;
    }
    let pairs: Vec<(String, bool)> = raw.iter().map(|r| (r.path.clone(), r.file == Some(false))).collect();
    if let Some(paths) = pin_drop_paths(&pairs) {
        Some(HomeDrop::Folders(paths))
    } else {
        file_drop_paths(&pairs).map(HomeDrop::Files)
    }
}

fn hits_of(list: &[FavRow]) -> Vec<DropHit> {
    list.iter().map(|r| DropHit { tree: r.in_tree(), group: r.group, id: r.id.clone() }).collect()
}

fn fav_target(y: i32) -> (DropSpot, Option<(usize, u8)>) {
    let list = rows();
    let hits = hits_of(&list);
    let index = row_at(y);
    let zone = match index.and_then(|i| list.get(i).map(|row| (i, row.group))) {
        Some((i, group)) => {
            let scroll = app(|a| a.side_scroll);
            let rh = row_h();
            let top = i as i32 * rh - scroll;
            drop_zone(group, y - top, rh)
        }
        None => 2,
    };
    let spot = drop_spot(&hits, index, zone);
    let line = drop_line(&hits, &spot);
    (spot, line)
}

fn home_accepts(drop: &HomeDrop, kind: RowKind) -> bool {
    matches!((drop, kind), (HomeDrop::Folders(_), RowKind::Pins) | (HomeDrop::Files(_), RowKind::Files))
}

#[implement(IDropTarget)]
struct SideDrop {
    hwnd: HWND,
    pending: RefCell<Option<Pending>>,
}

impl SideDrop {
    unsafe fn client_y(&self, pt: &POINTL) -> Option<i32> {
        let mut p = POINT { x: pt.x, y: pt.y };
        ScreenToClient(self.hwnd, &mut p).as_bool().then_some(p.y)
    }

    unsafe fn home_at(&self, y: i32) -> Option<RowKind> {
        let list = rows();
        let kind = row_at(y).and_then(|i| list.get(i)).map(|r| r.kind)?;
        matches!(kind, RowKind::Pins | RowKind::Files).then_some(kind)
    }

    unsafe fn paint_line(&self, line: Option<(usize, u8)>) {
        if set_drop(line) {
            _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Pins and favourite-file headers keep the old Explorer pin drop. Everywhere else the
    /// cursor is a link and the insertion line follows the pointer. Shift must not turn this
    /// into a move of the real files.
    unsafe fn hover(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        if effect.is_null() {
            return;
        }
        bind_hwnd(self.hwnd);
        let pending = self.pending.borrow().clone();
        let y = self.client_y(pt);
        let at = y.and_then(|y| self.home_at(y));
        let home = pending.as_ref().and_then(|p| p.home.as_ref());
        if home.zip(at).is_some_and(|(d, k)| home_accepts(d, k)) {
            *effect = drop_pick(*effect);
            let next = (*effect != DROPEFFECT_NONE).then(|| {
                let want = match home {
                    Some(HomeDrop::Folders(_)) => RowKind::Pins,
                    _ => RowKind::Files,
                };
                rows().iter().position(|r| r.group && r.kind == want).map(|row| (row, 1u8))
            }).flatten();
            self.paint_line(next);
            return;
        }
        let has = pending.as_ref().is_some_and(|p| !p.items.is_empty());
        if !has {
            *effect = DROPEFFECT_NONE;
            self.paint_line(None);
            return;
        }
        *effect = DROPEFFECT(fav_link_effect((*effect).0));
        if (*effect).0 == 0 {
            self.paint_line(None);
            return;
        }
        self.paint_line(y.map(|yy| fav_target(yy).1).unwrap_or(None));
    }

    unsafe fn clear(&self) {
        bind_hwnd(self.hwnd);
        app(|a| a.side_external = false);
        *self.pending.borrow_mut() = None;
        self.paint_line(None);
    }
}

impl IDropTarget_Impl for SideDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            // Hide before the drop target is live, so the tip cannot sit under the drag cursor.
            app(|a| a.side_external = true);
            hide_side_fav_tip();
            *self.pending.borrow_mut() = Some(pending_of(read_drop(&d)));
            self.hover(pt, effect);
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { self.hover(pt, effect) };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe { self.clear() };
        Ok(())
    }
    fn Drop(&self, _: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            let pending = self.pending.borrow().clone();
            let y = self.client_y(pt);
            let at = y.and_then(|y| self.home_at(y));
            let allowed = if effect.is_null() { DROPEFFECT_NONE } else { *effect };
            self.clear();
            if !effect.is_null() {
                *effect = DROPEFFECT_NONE;
            }
            let Some(pending) = pending else { return Ok(()) };
            if let (Some(home), Some(at)) = (pending.home.as_ref(), at) {
                if home_accepts(home, at) {
                    let eff = drop_pick(allowed);
                    if eff == DROPEFFECT_NONE {
                        return Ok(());
                    }
                    if !effect.is_null() {
                        *effect = eff;
                    }
                    match home {
                        HomeDrop::Folders(paths) => {
                            for path in paths {
                                pin_home(path, "", false);
                            }
                        }
                        HomeDrop::Files(paths) => {
                            for path in paths {
                                pin_home(path, "", true);
                            }
                        }
                    }
                    return Ok(());
                }
            }
            let link = fav_link_effect(allowed.0);
            if link == 0 || pending.items.is_empty() {
                return Ok(());
            }
            if !effect.is_null() {
                *effect = DROPEFFECT(link);
            }
            let spot = y.map(|yy| fav_target(yy).0).unwrap_or(DropSpot::End);
            let id = app(|a| {
                a.s.ensure_fav_tree();
                let id = insert_dropped(&mut a.s.tree, &pending.items, &spot);
                if id.is_some() {
                    a.s.sync_favs();
                }
                id
            });
            if let Some(id) = id {
                let sel = tree_pos(&rows(), &id);
                app(|a| a.side_sel = sel);
                changed();
            } else {
                repaint_side();
            }
        }
        Ok(())
    }
}

/// `CF_HDROP` first: the paths are already in the drop memory, so a UNC name is not resolved.
/// Shell items are only asked for a display name when that format is missing.
unsafe fn read_drop(d: &Ref<IDataObject>) -> Vec<RawDrop> {
    let paths = hdrop_paths(d);
    if !paths.is_empty() {
        let flags = shell_flags(d, &paths);
        return paths.into_iter().zip(flags).filter(|(p, _)| is_fs(p)).map(|(path, file)| RawDrop { path, file }).collect();
    }
    shell_paths(d)
}

unsafe fn hdrop_paths(d: &Ref<IDataObject>) -> Vec<String> {
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::ReleaseStgMedium;
    let Some(obj) = d.as_ref() else { return Vec::new() };
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = obj.GetData(&fmt) else { return Vec::new() };
    let drop = HDROP(medium.u.hGlobal.0);
    let mut paths = Vec::new();
    for k in 0..DragQueryFileW(drop, u32::MAX, None) {
        let need = DragQueryFileW(drop, k, None) as usize;
        let mut buf = vec![0u16; need + 1];
        let got = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
        let path = canon_drop_path(&String::from_utf16_lossy(buf.get(..got).unwrap_or(&[])));
        if !path.is_empty() {
            paths.push(path);
        }
    }
    ReleaseStgMedium(&mut medium);
    paths
}

/// File bits aligned with `paths`. A count mismatch does not fall back to display names:
/// those can open a share. An unknown bit stays unknown.
unsafe fn shell_flags(d: &Ref<IDataObject>, paths: &[String]) -> Vec<Option<bool>> {
    let Some(arr) = drop_items(d) else {
        return paths.iter().map(|p| path_is_file(p)).collect();
    };
    let n = arr.GetCount().unwrap_or(0) as usize;
    if n != paths.len() {
        return paths.iter().map(|p| path_is_file(p)).collect();
    }
    (0..n)
        .map(|k| match arr.GetItemAt(k as u32) {
            Ok(item) => file_bit(&item, &paths[k]).or_else(|| path_is_file(&paths[k])),
            Err(_) => path_is_file(&paths[k]),
        })
        .collect()
}

unsafe fn shell_paths(d: &Ref<IDataObject>) -> Vec<RawDrop> {
    let Some(arr) = drop_items(d) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(item) = arr.GetItemAt(k) else { continue };
        let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
        let path = canon_drop_path(&pwstr(name));
        if path.is_empty() || !is_fs(&path) {
            continue;
        }
        let file = file_bit(&item, &path).or_else(|| path_is_file(&path));
        out.push(RawDrop { path, file });
    }
    out
}

/// `Some(true)` is a file. Fast cached attributes first. `GetAttributes` and
/// `GetFileAttributesW` are skipped for UNC and mapped drives: either can bind the share.
unsafe fn file_bit(item: &IShellItem, path: &str) -> Option<bool> {
    if let Some(dir) = fast_is_dir(item) {
        return Some(!dir);
    }
    if !unc_path(path) && !share_path(path) {
        if let Ok(flags) = item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM) {
            return Some(!shell_is_folder(flags));
        }
    }
    None
}

unsafe fn path_is_file(path: &str) -> Option<bool> {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES};
    if unc_path(path) || share_path(path) {
        return None;
    }
    let attr = GetFileAttributesW(&HSTRING::from(path));
    if attr == INVALID_FILE_ATTRIBUTES {
        return None;
    }
    Some(attr & FILE_ATTRIBUTE_DIRECTORY.0 == 0)
}

const PKEY_FILE_ATTRIBUTES: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac),
    pid: 13,
};

unsafe fn fast_is_dir(item: &IShellItem) -> Option<bool> {
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
    use windows::Win32::UI::Shell::PropertiesSystem::{GPS_FASTPROPERTIESONLY, IPropertyStore};
    let item2 = item.cast::<IShellItem2>().ok()?;
    let store = item2.GetPropertyStore::<IPropertyStore>(GPS_FASTPROPERTIESONLY).ok()?;
    let mut v: PROPVARIANT = store.GetValue(&PKEY_FILE_ATTRIBUTES).ok()?;
    let dir = prop_directory(&v);
    _ = PropVariantClear(&mut v);
    dir
}

fn prop_directory(v: &windows::Win32::System::Com::StructuredStorage::PROPVARIANT) -> Option<bool> {
    use windows::Win32::System::Variant::{VT_I4, VT_INT, VT_UI4, VT_UINT};
    unsafe {
        let head = &*v.Anonymous.Anonymous;
        let n = if head.vt == VT_UI4 || head.vt == VT_UINT {
            head.Anonymous.ulVal
        } else if head.vt == VT_I4 || head.vt == VT_INT {
            head.Anonymous.lVal as u32
        } else {
            return None;
        };
        Some(n & 0x10 != 0)
    }
}

pub(crate) unsafe fn register_side_drop(h: HWND) {
    if h.0.is_null() {
        return;
    }
    let dt: IDropTarget = SideDrop { hwnd: h, pending: RefCell::new(None) }.into();
    _ = RegisterDragDrop(h, &dt);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{fav_row_height, fav_tip_text, mark_rows, FavRow, RowKind};
    use crate::i18n::UI;

    #[test]
    fn row_height_uses_the_tree_and_font_fallback_scales() {
        assert_eq!(fav_row_height(19, 16), 19);
        assert_eq!(fav_row_height(38, 32), 38);
        assert_eq!(fav_row_height(0, 16), 20);
        assert_eq!(fav_row_height(0, 32), 36);
        assert_eq!(fav_row_height(-1, 16), 20);
    }

    #[test]
    fn favourite_tip_is_the_full_location_and_a_missing_note() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        let jp = r"F:\CAC\サンプル";
        assert_eq!(fav_tip_text(jp, "表示", false, false), jp);
        assert_eq!(fav_tip_text(r"\\srv\share\資料", "資料", false, false), r"\\srv\share\資料");
        assert_eq!(fav_tip_text(r"C:\Users\me\Documents", "Docs", false, false), r"C:\Users\me\Documents");
        assert_eq!(fav_tip_text("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "This PC", false, false), "This PC");
        assert_eq!(fav_tip_text("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "", false, false), "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}");
        assert_eq!(fav_tip_text("", "ごみ箱", false, false), "ごみ箱");
        assert_eq!(fav_tip_text("", "", false, false), "");
        assert_eq!(fav_tip_text("", "", false, true), "");
        // A group has no location, even when a path was passed in by mistake.
        assert_eq!(fav_tip_text(r"C:\gone", "gone", true, true), "");
        // A multibyte name is not a drive letter. Slicing it by bytes would panic or drop a character.
        assert_eq!(fav_tip_text("サンプル", "", false, false), "サンプル");
        assert_eq!(fav_tip_text(r"D:\없는\폴더", "폴더", false, true), "D:\\없는\\폴더\r\nNot found");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(fav_tip_text(r"D:\없는\폴더", "폴더", false, true), "D:\\없는\\폴더\r\n찾을 수 없음");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(fav_tip_text(r"D:\없는\폴더", "폴더", false, true), "D:\\없는\\폴더\r\n見つかりません");
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn row_mark_includes_the_whole_path() {
        let row = |path: &str| FavRow {
            id: vec![0],
            depth: 0,
            group: false,
            file: false,
            open: false,
            name: "n".into(),
            path: path.into(),
            kind: RowKind::Tree,
        };
        let a = mark_rows(&[row(r"D:\없는\폴더")]);
        let b = mark_rows(&[row(r"D:\없는\폴더2")]);
        let c = mark_rows(&[row("サンプル")]);
        let d = mark_rows(&[row("サンフ")]);
        assert_ne!(a, b);
        assert_ne!(c, d);
        assert_eq!(a, mark_rows(&[row(r"D:\없는\폴더")]));
    }
}
