// Favourites side of the activity bar. One owner-drawn window: groups, files, and
// folders from the same tree the Ctrl+B list uses. No extra shell control.

use crate::imports::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;
use crate::shell::*;
use super::*;

const ID_SIDE_EDIT: i32 = 209;

fn row_h(dpi: i32) -> i32 {
    28 * dpi / 96
}

fn rows() -> Vec<FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        visible_rows(&a.s.tree)
    })
}

fn row_at(y: i32) -> Option<usize> {
    let n = rows().len();
    let (dpi, scroll) = app(|a| (a.dpi, a.side_scroll));
    if y < 0 || n == 0 {
        return None;
    }
    let i = (y + scroll) / row_h(dpi);
    (i >= 0 && (i as usize) < n).then_some(i as usize)
}

fn chevron_hit(x: i32, row: &FavRow, dpi: i32) -> bool {
    if !row.group {
        return false;
    }
    let s = |v: i32| v * dpi / 96;
    x < s(4) + s(14) * row.depth.min(8) as i32 + s(18)
}

pub(crate) unsafe fn paint_side(h: HWND) {
    let list = rows();
    let (dpi, font, icon, hot, sel, scroll, drop, accent) = app(|a| {
        (a.dpi, a.font, a.icon_font, a.side_hot, a.side_sel, a.side_scroll, a.side_drop, a.s.accent_now())
    });
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let rh = row_h(dpi);
        fill(dc, rc, pal().card);
        if list.is_empty() {
            text(dc, RECT { left: s(12), top: s(8), right: rc.right - s(8), bottom: s(48) }, tr("No favourites yet", "즐겨찾기가 없어요", "お気に入りはまだありません"), font, pal().sub, DT_LEFT);
            text(dc, RECT { left: s(12), top: s(28), right: rc.right - s(8), bottom: s(72) }, tr("Right-click to add a group", "우클릭해서 그룹을 만드세요", "右クリックでグループを作成"), font, pal().sub, DT_LEFT);
            return;
        }
        for (i, f) in list.iter().enumerate() {
            let y = i as i32 * rh - scroll;
            if y + rh < 0 || y >= rc.bottom {
                continue;
            }
            let r = RECT { left: 0, top: y, right: rc.right, bottom: y + rh };
            if sel == Some(i) {
                fill(dc, r, blend(accent, pal().card, 0.16));
            } else if hot == Some(i) {
                fill(dc, r, blend(pal().bg, pal().card, 0.35));
            }
            if drop.is_some_and(|(row, zone)| row == i && zone == 1) {
                fill(dc, r, blend(accent, pal().card, 0.28));
            }
            let indent = s(14) * f.depth.min(8) as i32;
            let mut x = s(4) + indent;
            if f.group {
                let chev = RECT { left: x, top: y, right: x + s(16), bottom: y + rh };
                text(dc, chev, if f.open { "\u{E70D}" } else { "\u{E76C}" }, icon, pal().sub, DT_CENTER);
                x += s(16);
            }
            let glyph = if f.file { "\u{E8A5}" } else { "\u{E8B7}" };
            let ir = RECT { left: x, top: y, right: x + s(18), bottom: y + rh };
            text(dc, ir, glyph, icon, pal().sub, DT_CENTER);
            let label = if f.name.is_empty() { short_name(&f.path) } else { f.name.clone() };
            text(dc, RECT { left: ir.right + s(4), right: rc.right - s(6), ..r }, &label, font, if f.group { pal().text } else { pal().text }, DT_LEFT);
            if drop.is_some_and(|(row, zone)| row == i && zone != 1) {
                let line_y = if drop.unwrap().1 == 0 { r.top } else { r.bottom - s(2) };
                fill(dc, RECT { left: s(8) + indent, top: line_y, right: rc.right - s(8), bottom: line_y + s(2) }, accent);
            }
        }
    });
}

pub(crate) unsafe fn begin_side_rename(id: &[usize]) {
    let list = rows();
    let label = list.iter().find(|r| r.id == id).map(|r| if r.name.is_empty() { short_name(&r.path) } else { r.name.clone() }).unwrap_or_default();
    let i = list.iter().position(|r| r.id == id);
    let (host, font, dpi, y) = app(|a| {
        let y = i.map(|i| i as i32 * row_h(a.dpi) - a.side_scroll).unwrap_or(0);
        (a.side_hwnd, a.font, a.dpi, y)
    });
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
    let s = |v: i32| v * dpi / 96;
    app(|a| a.side_edit_at = id.to_vec());
    _ = SetWindowTextW(edit, &HSTRING::from(label.as_str()));
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    _ = SetWindowPos(edit, None, s(8), y + s(2), s(220), row_h(dpi) - s(4), SWP_NOZORDER | SWP_SHOWWINDOW);
    _ = SetFocus(Some(edit));
    let len = GetWindowTextLengthW(edit) as usize;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(len as isize)));
}

pub(crate) unsafe fn end_side_rename(save: bool) {
    let (edit, id) = app(|a| (a.side_edit, std::mem::take(&mut a.side_edit_at)));
    if edit.0.is_null() || id.is_empty() {
        return;
    }
    if save {
        let name = text_of(edit);
        app(|a| {
            rename_at(&mut a.s.tree, &id, name.trim());
            a.s.sync_favs();
        });
        favs_changed();
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
    let id = rows().iter().rev().find(|r| r.group && r.name == name).map(|r| r.id.clone());
    changed();
    if let Some(id) = id {
        unsafe { begin_side_rename(&id) };
    }
}

unsafe fn side_menu(h: HWND, index: Option<usize>, x: i32, y: i32) {
    let row = index.and_then(|i| rows().get(i).cloned());
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, label: &str| {
        _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(label));
    };
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
        4 => open_fav_target(&row.path, row.file),
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

fn finish_drop() {
    let list = rows();
    let (from, onto) = app(|a| {
        let onto = a.side_drop.and_then(|(i, zone)| list.get(i).map(|r| (r.id.clone(), zone)));
        (a.side_drag.clone(), onto)
    });
    app(|a| {
        a.side_drag = None;
        a.side_drop = None;
        a.side_press = None;
    });
    let (Some(from), Some((onto, zone))) = (from, onto) else { return };
    let moved = app(|a| {
        let ok = apply_drop(&mut a.s.tree, &from, &onto, zone);
        if ok {
            a.s.sync_favs();
        }
        ok
    });
    if moved {
        changed();
    } else {
        let h = app(|a| a.side_hwnd);
        if !h.0.is_null() {
            unsafe { _ = InvalidateRect(Some(h), None, false) };
        }
    }
}

pub(crate) extern "system" fn side_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_PAINT => paint_side(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = row_at(y);
                let dragging = app(|a| a.side_drag.is_some());
                if dragging {
                    if let Some(i) = hot {
                        let list = rows();
                        let rh = row_h(app(|a| a.dpi));
                        let scroll = app(|a| a.side_scroll);
                        let top = i as i32 * rh - scroll;
                        let zone = list.get(i).map(|r| drop_zone(r.group, y - top, rh)).unwrap_or(2);
                        let paint = app(|a| {
                            let next = Some((i, zone));
                            let paint = a.side_drop != next;
                            a.side_drop = next;
                            paint
                        });
                        if paint {
                            _ = InvalidateRect(Some(h), None, false);
                        }
                    }
                } else if let Some((px, py, i)) = app(|a| a.side_press) {
                    if (x - px).abs() + (y - py).abs() > 4 {
                        let id = rows().get(i).map(|r| r.id.clone());
                        app(|a| a.side_drag = id);
                    }
                } else {
                    let paint = app(|a| {
                        let paint = a.side_hot != hot;
                        a.side_hot = hot;
                        paint
                    });
                    if paint {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    _ = TrackMouseEvent(&mut t);
                }
            }
            WM_MOUSELEAVE => {
                app(|a| a.side_hot = None);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) as i16 as i32) * row_h(app(|a| a.dpi)) / 120;
                let n = rows().len() as i32;
                app(|a| {
                    let max = (n * row_h(a.dpi) - 40).max(0);
                    a.side_scroll = (a.side_scroll - delta).clamp(0, max);
                });
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                _ = SetFocus(Some(h));
                if let Some(i) = row_at(y) {
                    let list = rows();
                    let dpi = app(|a| a.dpi);
                    if list.get(i).is_some_and(|r| chevron_hit(x, r, dpi)) {
                        let id = list[i].id.clone();
                        app(|a| {
                            toggle_open(&mut a.s.tree, &id);
                        });
                        changed();
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
                let dragged = app(|a| a.side_drag.is_some());
                if dragged {
                    finish_drop();
                } else {
                    app(|a| {
                        a.side_press = None;
                    });
                }
                _ = ReleaseCapture();
            }
            WM_LBUTTONDBLCLK => {
                let y = xy(lp).1;
                if let Some(row) = row_at(y).and_then(|i| rows().get(i).cloned()) {
                    if row.group {
                        app(|a| {
                            toggle_open(&mut a.s.tree, &row.id);
                        });
                        changed();
                    } else {
                        open_fav_target(&row.path, row.file);
                    }
                }
            }
            WM_RBUTTONUP => {
                let (x, y) = xy(lp);
                let i = row_at(y);
                if let Some(i) = i {
                    app(|a| a.side_sel = Some(i));
                }
                side_menu(h, i, x, y);
            }
            WM_KEYDOWN => match VIRTUAL_KEY(wp.0 as u16) {
                VK_DELETE => {
                    let id = app(|a| a.side_sel).and_then(|i| rows().get(i).map(|r| r.id.clone()));
                    if let Some(id) = id {
                        app(|a| {
                            delete_at(&mut a.s.tree, &id);
                            a.s.sync_favs();
                            a.side_sel = None;
                        });
                        changed();
                    }
                }
                VK_F2 => {
                    if let Some(id) = app(|a| a.side_sel).and_then(|i| rows().get(i).map(|r| r.id.clone())) {
                        begin_side_rename(&id);
                    }
                }
                VK_RETURN => {
                    if let Some(row) = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned()) {
                        if row.group {
                            app(|a| {
                                toggle_open(&mut a.s.tree, &row.id);
                            });
                            changed();
                        } else {
                            open_fav_target(&row.path, row.file);
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
