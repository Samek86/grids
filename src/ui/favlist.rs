// Favourites list shared by the settings page and the favourites window.
// Filtering and row geometry are pure. Painting matches the settings table:
// indented name, shell icon, path, rename and delete.

use crate::imports::*;
use crate::i18n::tr;
use crate::session::{short_name, tree_rows, visible_rows, FavNode, FavRow};

use super::{draw_fav_icon, fill, pal, shell_icon, text};

fn px(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

fn contains_ci(hay: &str, q: &str) -> bool {
    q.is_empty() || hay.to_lowercase().contains(&q.to_lowercase())
}

/// Name, full path, or the path's last component. Empty `q` matches everything.
pub(crate) fn fav_row_matches(row: &FavRow, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    contains_ci(&row.name, q) || (!row.path.is_empty() && (contains_ci(&row.path, q) || contains_ci(&short_name(&row.path), q)))
}

/// Indices to show. An empty query keeps every row, in order.
/// A hit keeps its ancestors and its descendants so a nested row stays under its group.
pub(crate) fn filter_fav_rows(rows: &[FavRow], q: &str) -> Vec<usize> {
    if q.is_empty() {
        return (0..rows.len()).collect();
    }
    let mut on = vec![false; rows.len()];
    for row in rows {
        if !fav_row_matches(row, q) {
            continue;
        }
        for (j, other) in rows.iter().enumerate() {
            if other.id.starts_with(&row.id) || row.id.starts_with(&other.id) {
                on[j] = true;
            }
        }
    }
    on.into_iter().enumerate().filter(|(_, keep)| *keep).map(|(i, _)| i).collect()
}

/// Rows the favourites window shows. An empty query follows collapsed groups.
/// A query searches the whole tree, including children of a collapsed group, and keeps their parents.
pub(crate) fn rows_for_query(tree: &[FavNode], q: &str) -> Vec<FavRow> {
    let q = q.trim();
    if q.is_empty() {
        return visible_rows(tree);
    }
    let all = tree_rows(tree);
    filter_fav_rows(&all, q).into_iter().filter_map(|i| all.get(i).cloned()).collect()
}

pub(crate) fn fav_head_h(dpi: i32) -> i32 {
    px(dpi, 32)
}

pub(crate) fn fav_row_h(dpi: i32) -> i32 {
    px(dpi, 40)
}

/// One data row in the favourites table. `index` is into the slice passed as `keep`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FavSlot {
    pub(crate) index: usize,
    pub(crate) rect: RECT,
    pub(crate) rename: RECT,
    pub(crate) delete: RECT,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FavBoard {
    pub(crate) head: RECT,
    pub(crate) table: RECT,
    pub(crate) slots: Vec<FavSlot>,
    /// One placeholder row when `keep` is empty.
    pub(crate) empty: Option<RECT>,
    /// Y just below the last row. The head's top is the `top` argument.
    pub(crate) bottom: i32,
}

/// Name / path header plus one row per `keep` index. An empty `keep` is a single blank row.
/// Button rects match the settings favourites table (88px rename, 72px delete, 16px right inset).
pub(crate) fn layout_fav_board(keep: &[usize], left: i32, right: i32, top: i32, dpi: i32) -> FavBoard {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    let head_h = fav_head_h(dpi);
    let row_h = fav_row_h(dpi);
    let n = if keep.is_empty() { 1 } else { keep.len() as i32 };
    let head = RECT { left, top, right, bottom: top + head_h };
    let table = RECT { left, top, right, bottom: top + head_h + row_h * n };
    let mut y = top + head_h;
    let mut slots = Vec::new();
    let mut empty = None;
    if keep.is_empty() {
        empty = Some(RECT { left, top: y, right, bottom: y + row_h });
        y += row_h;
    } else {
        for &index in keep {
            let delete = RECT {
                left: right - s(16) - s(72),
                top: y + s(6),
                right: right - s(16),
                bottom: y + row_h - s(6),
            };
            let rename = RECT {
                left: delete.left - s(8) - s(88),
                top: delete.top,
                right: delete.left - s(8),
                bottom: delete.bottom,
            };
            slots.push(FavSlot {
                index,
                rect: RECT { left, top: y, right, bottom: y + row_h },
                rename,
                delete,
            });
            y += row_h;
        }
    }
    FavBoard { head, table, slots, empty, bottom: y }
}

pub(crate) enum FavHit {
    Row(usize),
    Rename(usize),
    Delete(usize),
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Button hits win over the row. `index` is the value stored on the slot.
pub(crate) fn hit_fav_board(board: &FavBoard, x: i32, y: i32) -> Option<FavHit> {
    for slot in &board.slots {
        if inside(slot.delete, x, y) {
            return Some(FavHit::Delete(slot.index));
        }
        if inside(slot.rename, x, y) {
            return Some(FavHit::Rename(slot.index));
        }
        if inside(slot.rect, x, y) {
            return Some(FavHit::Row(slot.index));
        }
    }
    None
}

/// Indent of a row's icon, capped so the name column still has room.
pub(crate) fn fav_row_indent(rect: RECT, depth: u8, dpi: i32) -> i32 {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    let name_right = rect.left + (rect.right - rect.left) * 32 / 100;
    let max_indent = (name_right - rect.left - s(12) - s(22) - s(4) - s(36)).max(0);
    (s(14) * depth.min(8) as i32).min(max_indent)
}

/// Chevron box for a group row. Sits where the icon starts; the icon shifts right when it is drawn.
pub(crate) fn fav_chevron_rect(rect: RECT, depth: u8, dpi: i32) -> RECT {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    let left = rect.left + s(12) + fav_row_indent(rect, depth, dpi);
    RECT { left, top: rect.top, right: left + s(16), bottom: rect.bottom }
}

pub(crate) unsafe fn paint_fav_head(dc: HDC, r: RECT, dpi: i32, font: HFONT) {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    let name = RECT { left: r.left + s(40), right: r.left + (r.right - r.left) * 32 / 100, ..r };
    text(dc, name, tr("Name", "이름", "名前"), font, pal().sub, DT_LEFT);
    text(dc, RECT { left: r.left + (r.right - r.left) * 32 / 100, ..r }, tr("Path", "경로", "パス"), font, pal().sub, DT_LEFT);
    fill(dc, RECT { top: r.bottom - 1, ..r }, pal().line);
}

/// `show_chevron` draws an open/closed mark on groups and shifts the icon. Settings passes false.
pub(crate) unsafe fn paint_fav_row(dc: HDC, row: Option<&FavRow>, r: RECT, dpi: i32, font: HFONT, small: HFONT, icon_font: HFONT, empty: bool, show_chevron: bool) {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    if empty {
        text(
            dc,
            r,
            tr(
                "No favourites yet · add one with ☆+ in a pane header",
                "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가",
                "お気に入りはまだありません · ペインヘッダーの ☆+ で追加",
            ),
            small,
            pal().sub,
            DT_CENTER,
        );
        return;
    }
    let Some(row) = row else { return };
    let name_right = r.left + (r.right - r.left) * 32 / 100;
    let slot = s(16).max(1);
    let icon_w = s(22);
    let indent = fav_row_indent(r, row.depth, dpi);
    let mut icon_left = r.left + s(12) + indent;
    if show_chevron && row.group {
        let chev = fav_chevron_rect(r, row.depth, dpi);
        let mark = if row.open { "\u{E70D}" } else { "\u{E76C}" };
        text(dc, chev, mark, icon_font, pal().sub, DT_CENTER);
        icon_left = chev.right;
    }
    let icon = RECT { left: icon_left, right: icon_left + icon_w, ..r };
    let (shell_path, shell_file) = if row.group { ("", false) } else { (row.path.as_str(), row.file) };
    if let Some((list, index, dim)) = shell_icon(shell_path, shell_file) {
        let iy = r.top + (r.bottom - r.top - slot).max(0) / 2;
        let ix = icon.left + (icon_w - slot).max(0) / 2;
        draw_fav_icon(dc, list, index, ix, iy, slot, dim);
    } else {
        let glyph = if shell_file { "\u{E8A5}" } else { "\u{E8B7}" };
        text(dc, icon, glyph, icon_font, pal().sub, DT_CENTER);
    }
    let name_r = RECT { left: icon.right + s(4), right: name_right.max(icon.right + s(4)), ..r };
    text(dc, name_r, &row.name, font, pal().text, DT_LEFT);
    let path_r = RECT { left: name_right + s(8), right: r.right - s(180), ..r };
    let path = if row.group { tr("Group", "그룹", "グループ").to_string() } else { row.path.clone() };
    if path_r.right > path_r.left {
        text(dc, path_r, &path, small, pal().sub, DT_LEFT);
    }
    fill(dc, RECT { top: r.bottom - 1, ..r }, pal().line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RowKind;

    fn row(id: &[usize], name: &str, path: &str, group: bool) -> FavRow {
        FavRow {
            id: id.to_vec(),
            depth: id.len().saturating_sub(1) as u8,
            group,
            file: false,
            open: true,
            name: name.to_string(),
            path: path.to_string(),
            kind: RowKind::Tree,
        }
    }

    fn sample() -> Vec<FavRow> {
        vec![
            row(&[0], "Work", "", true),
            row(&[0, 0], "notes", r"C:\work\notes.txt", false),
            row(&[1], "Home", "", true),
            row(&[1, 0], "photo", r"C:\home\photo.jpg", false),
        ]
    }

    #[test]
    fn filter_keeps_ancestors_and_descendants() {
        let rows = sample();
        assert_eq!(filter_fav_rows(&rows, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter_fav_rows(&rows, "zzz"), Vec::<usize>::new());
        // A file hit keeps its group and drops the other branch.
        assert_eq!(filter_fav_rows(&rows, "notes.txt"), vec![0, 1]);
        // A group hit keeps what is nested under it.
        assert_eq!(filter_fav_rows(&rows, "work"), vec![0, 1]);
        assert_eq!(filter_fav_rows(&rows, "HOME"), vec![2, 3]);
        // The last path component matches even when the row's label does not contain it.
        let pc = row(&[0], "PC", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", false);
        assert!(fav_row_matches(&pc, "this pc"));
        assert!(!fav_row_matches(&pc, "없는"));
        let ko = row(&[0], "사진", r"D:\사진", false);
        assert!(fav_row_matches(&ko, "사진"));
        assert!(fav_row_matches(&ko, "사"));
        assert!(!fav_row_matches(&ko, "없는"));
    }

    #[test]
    fn board_matches_the_settings_row_metrics() {
        let board = layout_fav_board(&[0, 2], 10, 410, 100, 96);
        assert_eq!(board.head, RECT { left: 10, top: 100, right: 410, bottom: 132 });
        assert_eq!(board.slots.len(), 2);
        assert_eq!(board.slots[0].index, 0);
        assert_eq!(board.slots[0].rect, RECT { left: 10, top: 132, right: 410, bottom: 172 });
        assert_eq!(board.slots[0].delete.right, 410 - 16);
        assert_eq!(board.slots[0].delete.left, 410 - 16 - 72);
        assert_eq!(board.slots[0].rename.right, board.slots[0].delete.left - 8);
        assert_eq!(board.slots[0].rename.right - board.slots[0].rename.left, 88);
        assert!(board.empty.is_none());
        assert_eq!(board.bottom, 132 + 80);
        assert!(matches!(hit_fav_board(&board, 380, 140), Some(FavHit::Delete(0))));
        assert!(matches!(hit_fav_board(&board, 300, 140), Some(FavHit::Rename(0))));
        assert!(matches!(hit_fav_board(&board, 40, 140), Some(FavHit::Row(0))));

        let empty = layout_fav_board(&[], 0, 200, 0, 96);
        assert!(empty.slots.is_empty());
        assert_eq!(empty.empty, Some(RECT { left: 0, top: 32, right: 200, bottom: 72 }));

        let hi = layout_fav_board(&[1], 0, 800, 0, 192);
        assert_eq!(hi.head.bottom, 64);
        assert_eq!(hi.slots[0].rect.bottom - hi.slots[0].rect.top, 80);
        assert_eq!(hi.slots[0].delete.right - hi.slots[0].delete.left, 144);
    }

    #[test]
    fn chevron_sits_at_the_icon_indent() {
        let rect = RECT { left: 0, top: 0, right: 400, bottom: 40 };
        let c = fav_chevron_rect(rect, 0, 96);
        assert_eq!(c.left, 12);
        assert_eq!(c.right - c.left, 16);
        assert_eq!(c.bottom, 40);
        let nested = fav_chevron_rect(rect, 2, 96);
        assert_eq!(nested.left, 12 + 28);
        assert!(nested.right < 400);
    }

    #[test]
    fn query_reveals_a_child_of_a_collapsed_group() {
        let mut tree = vec![FavNode::group("Work")];
        tree[0].open = false;
        tree[0].children.push(FavNode::leaf(r"C:\work\notes.txt", "notes", true));
        tree.push(FavNode::leaf(r"C:\home", "Home", false));
        let closed = rows_for_query(&tree, "");
        assert_eq!(closed.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Work", "Home"]);
        let found = rows_for_query(&tree, " notes.txt ");
        assert_eq!(found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Work", "notes"]);
        assert!(found[1].file);
        let none = rows_for_query(&tree, "없는");
        assert!(none.is_empty());
    }
}
