use crate::imports::*;
use super::{inside, pane_tab_close_rect, pane_tab_rects, PaneGeo};
use crate::{fix, slot, MAXP};

// Dragging a pane by its header. The grid stays columns of rows (the layout picker and the
// session both stop at 4 × 4). A drop never invents a free split, a gap setting, or a fifth column.

/// Columns, and rows in one column. Past this the session file would drop the extra cell.
pub(crate) const GRID_CAP: usize = 4;

/// Where a pane lands when it is dropped on another pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

/// What a header press will do if the pointer never moves far enough to drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneGrab {
    Header,
    Tab(usize),
    Strip,
}

/// Header press, in that pane's client pixels. Becomes a drag only after `drag_slack`.
#[derive(Clone, Copy)]
pub(crate) struct PanePress {
    pub(crate) hwnd: HWND,
    pub(crate) pane: usize,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) grab: PaneGrab,
}

/// A pane drag that has passed the click slack. `hot` is the pane and zone under the pointer.
#[derive(Clone, Copy)]
pub(crate) struct PaneMove {
    pub(crate) hwnd: HWND,
    pub(crate) from: usize,
    pub(crate) hot: Option<(usize, DropZone)>,
}

/// New grid after a drop. `perm[slot]` is the slot whose pane lands there (same rule as `reorder`).
#[derive(Clone, Debug)]
pub(crate) struct LayoutShift {
    pub(crate) heights: Vec<usize>,
    pub(crate) colr: Vec<f64>,
    pub(crate) rowr: Vec<Vec<f64>>,
    pub(crate) perm: [usize; MAXP],
    pub(crate) focus: usize,
}

/// Movement, in px, before a header press becomes a drag.
/// Matches the window-tab reorder slack, so a click on a pane tab still selects it.
pub(crate) fn drag_slack(dpi: i32) -> i32 {
    (4 * dpi.max(1) / 96).max(4)
}

/// Header or pane-tab strip, and not a control. The file list is below `view` and is left to OLE.
/// Tab close and '+' stay clicks. Address, filter and the icon buttons do too.
pub(crate) fn pane_grab(pg: &PaneGeo, tabs: usize, x: i32, y: i32, dpi: i32) -> Option<PaneGrab> {
    if x < 0 || y < 0 || y >= pg.view.top {
        return None;
    }
    let mut controls = pg.btn.iter().copied().chain([pg.addr, pg.filt, pg.fav, pg.add]);
    if controls.any(|r| r.right > r.left && inside(r, x, y)) {
        return None;
    }
    if pg.tabs.bottom > pg.tabs.top && inside(pg.tabs, x, y) {
        // Same hit order as pane_tab_hit. Close and '+' stay clicks; the rest can drag.
        let (tabs_r, plus) = pane_tab_rects(pg.tabs, tabs, dpi);
        if inside(plus, x, y) {
            return None;
        }
        if let Some(k) = tabs_r.iter().position(|r| inside(*r, x, y)) {
            let on_close = pane_tab_close_rect(tabs_r[k], dpi).is_some_and(|c| tabs > 1 && inside(c, x, y));
            return if on_close { None } else { Some(PaneGrab::Tab(k)) };
        }
        return Some(PaneGrab::Strip);
    }
    Some(PaneGrab::Header)
}

/// Edge bands are a quarter of the pane. A corner belongs to whichever edge is closer, as a fraction.
/// A tie prefers left/right. Outside the pane is no target.
pub(crate) fn pane_zone(r: RECT, x: i32, y: i32) -> Option<DropZone> {
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    if w <= 0 || h <= 0 {
        return None;
    }
    let (px, py) = (x - r.left, y - r.top);
    if px < 0 || py < 0 || px >= w || py >= h {
        return None;
    }
    let (ex, ey) = (w / 4, h / 4);
    let left = px < ex;
    let right = ex > 0 && px >= w - ex;
    let top = py < ey;
    let bottom = ey > 0 && py >= h - ey;
    let horiz = if left {
        Some(DropZone::Left)
    } else if right {
        Some(DropZone::Right)
    } else {
        None
    };
    let vert = if top {
        Some(DropZone::Top)
    } else if bottom {
        Some(DropZone::Bottom)
    } else {
        None
    };
    Some(match (horiz, vert) {
        (Some(hzn), Some(vzn)) => {
            let fx = if left { px as f64 } else { (w - px) as f64 } / w as f64;
            let fy = if top { py as f64 } else { (h - py) as f64 } / h as f64;
            if fx <= fy { hzn } else { vzn }
        }
        (Some(hzn), None) => hzn,
        (None, Some(vzn)) => vzn,
        (None, None) => DropZone::Center,
    })
}

/// Rectangle to tint for this drop, in the same coordinates as `cells` (one rect per live slot).
/// Center is the whole target pane. Top/bottom is the half of that pane the new row will take.
/// Left/right is half of the target column's width across the full column, which is the new column.
pub(crate) fn drop_preview(heights: &[usize], cells: &[RECT], onto: usize, zone: DropZone) -> Option<RECT> {
    let n: usize = heights.iter().sum();
    if onto >= n || onto >= cells.len() || heights.is_empty() {
        return None;
    }
    let target = cells[onto];
    let r = if zone == DropZone::Center {
        target
    } else {
        let (tc, _) = slot(heights, onto);
        let mut span = None::<(i32, i32, i32, i32)>;
        let mut i = 0usize;
        for (c, &rows) in heights.iter().enumerate() {
            for _ in 0..rows {
                if c == tc {
                    if let Some(cell) = cells.get(i) {
                        span = Some(match span {
                            None => (cell.left, cell.top, cell.right, cell.bottom),
                            Some((l, t, r, b)) => (l.min(cell.left), t.min(cell.top), r.max(cell.right), b.max(cell.bottom)),
                        });
                    }
                }
                i += 1;
            }
        }
        let (left, top, right, bot) = span?;
        let mid_x = left + (right - left) / 2;
        let mid_y = target.top + (target.bottom - target.top) / 2;
        match zone {
            DropZone::Left => RECT { left, right: mid_x, top, bottom: bot },
            DropZone::Right => RECT { left: mid_x, right, top, bottom: bot },
            DropZone::Top => RECT { bottom: mid_y, ..target },
            DropZone::Bottom => RECT { top: mid_y, ..target },
            DropZone::Center => target,
        }
    };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

#[derive(Clone)]
struct Col {
    slots: Vec<usize>,
    rows: Vec<f64>,
    width: f64,
}

fn grid_cols(heights: &[usize], colr: &[f64], rowr: &[Vec<f64>]) -> Vec<Col> {
    let colr = fix(colr.to_vec(), heights.len());
    let mut slot_i = 0usize;
    heights
        .iter()
        .enumerate()
        .map(|(c, &n)| {
            let rows = fix(rowr.get(c).cloned().unwrap_or_default(), n);
            let slots = (0..n)
                .map(|_| {
                    let s = slot_i;
                    slot_i += 1;
                    s
                })
                .collect();
            Col { slots, rows, width: colr[c] }
        })
        .collect()
}

fn locate(cols: &[Col], slot_i: usize) -> Option<(usize, usize)> {
    cols.iter().enumerate().find_map(|(c, col)| col.slots.iter().position(|&s| s == slot_i).map(|r| (c, r)))
}

fn slot_grid(cols: &[Col]) -> Vec<Vec<usize>> {
    cols.iter().map(|c| c.slots.clone()).collect()
}

fn renorm(v: &mut [f64]) {
    let sum: f64 = v.iter().copied().filter(|x| x.is_finite() && *x > 0.0).sum();
    if !(sum > 0.0 && sum.is_finite()) {
        let n = v.len().max(1) as f64;
        for x in v.iter_mut() {
            *x = 1.0 / n;
        }
        return;
    }
    for x in v.iter_mut() {
        *x = if x.is_finite() && *x > 0.0 { *x / sum } else { 0.0 };
    }
}

fn split_col(cols: &mut Vec<Col>, tc: usize, right: bool, slot_i: usize) -> bool {
    if cols.len() >= GRID_CAP || tc >= cols.len() {
        return false;
    }
    let half = cols[tc].width / 2.0;
    cols[tc].width = half;
    let at = if right { tc + 1 } else { tc };
    cols.insert(at, Col { slots: vec![slot_i], rows: vec![1.0], width: half });
    true
}

fn split_row(col: &mut Col, tr: usize, below: bool, slot_i: usize) -> bool {
    if col.slots.len() >= GRID_CAP || tr >= col.slots.len() {
        return false;
    }
    let half = col.rows[tr] / 2.0;
    col.rows[tr] = half;
    let at = if below { tr + 1 } else { tr };
    col.rows.insert(at, half);
    col.slots.insert(at, slot_i);
    true
}

/// Pull `from` out and put it on `zone` of `onto`. False when that would pass `GRID_CAP`.
fn shift_edge(cols: &mut Vec<Col>, from: usize, onto: usize, zone: DropZone) -> bool {
    let Some((sc, sr)) = locate(cols, from) else { return false };
    let Some((mut tc, mut tr)) = locate(cols, onto) else { return false };
    let row_ratio = cols[sc].rows[sr];
    let col_width = cols[sc].width;
    cols[sc].slots.remove(sr);
    cols[sc].rows.remove(sr);
    let emptied = cols[sc].slots.is_empty();
    if emptied {
        cols.remove(sc);
        if sc < tc {
            tc -= 1;
        }
    } else if sc == tc && sr < tr {
        tr -= 1;
    }
    if cols.is_empty() || tc >= cols.len() {
        return false;
    }
    match zone {
        // The dragged pane was a whole column. Move that column; keep its width.
        DropZone::Left | DropZone::Right if emptied => {
            if cols.len() >= GRID_CAP {
                return false;
            }
            let at = if zone == DropZone::Right { tc + 1 } else { tc };
            cols.insert(at, Col { slots: vec![from], rows: vec![1.0], width: col_width });
            true
        }
        // Taken out of a taller column: the new column borrows half the target column.
        DropZone::Left | DropZone::Right => {
            renorm(&mut cols[sc].rows);
            split_col(cols, tc, zone == DropZone::Right, from)
        }
        // Reorder inside one column. Each pane keeps the height it had.
        DropZone::Top | DropZone::Bottom if !emptied && sc == tc => {
            if cols[tc].slots.len() >= GRID_CAP {
                return false;
            }
            let at = if zone == DropZone::Bottom { tr + 1 } else { tr };
            cols[tc].slots.insert(at, from);
            cols[tc].rows.insert(at, row_ratio);
            true
        }
        // A row in another column. The old column closes up; the target row is split in half.
        DropZone::Top | DropZone::Bottom => {
            if emptied {
                let mut widths: Vec<f64> = cols.iter().map(|c| c.width).collect();
                renorm(&mut widths);
                for (c, w) in cols.iter_mut().zip(widths) {
                    c.width = w;
                }
            } else {
                renorm(&mut cols[sc].rows);
            }
            split_row(&mut cols[tc], tr, zone == DropZone::Bottom, from)
        }
        DropZone::Center => false,
    }
}

fn kept_ratios(heights: &[usize], colr: &[f64], rowr: &[Vec<f64>]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let colr = if colr.len() == heights.len() { colr.to_vec() } else { fix(colr.to_vec(), heights.len()) };
    let rowr = heights
        .iter()
        .enumerate()
        .map(|(c, &rows)| match rowr.get(c) {
            Some(r) if r.len() == rows => r.clone(),
            other => fix(other.cloned().unwrap_or_default(), rows),
        })
        .collect();
    (colr, rowr)
}

/// Swap on `Center`. An edge moves the dragged pane to that side of the target.
/// None when the drop is the pane itself, would pass 4 columns or 4 rows, or would not change the grid.
pub(crate) fn pane_drop(heights: &[usize], colr: &[f64], rowr: &[Vec<f64>], from: usize, onto: usize, zone: DropZone) -> Option<LayoutShift> {
    if heights.is_empty() || heights.iter().any(|&n| n == 0) {
        return None;
    }
    let n: usize = heights.iter().sum();
    if n == 0 || n > MAXP || from >= n || onto >= n || from == onto {
        return None;
    }
    if zone == DropZone::Center {
        let mut perm = std::array::from_fn(|i| i);
        perm.swap(from, onto);
        let (colr, rowr) = kept_ratios(heights, colr, rowr);
        return Some(LayoutShift { heights: heights.to_vec(), colr, rowr, perm, focus: onto });
    }
    let mut cols = grid_cols(heights, colr, rowr);
    let before = slot_grid(&cols);
    if !shift_edge(&mut cols, from, onto, zone) || slot_grid(&cols) == before {
        return None;
    }
    let order: Vec<usize> = cols.iter().flat_map(|c| c.slots.iter().copied()).collect();
    if order.len() != n {
        return None;
    }
    let mut seen = vec![false; n];
    for &old in &order {
        if old >= n || seen[old] {
            return None;
        }
        seen[old] = true;
    }
    let mut perm = std::array::from_fn(|i| i);
    for (i, &old) in order.iter().enumerate() {
        perm[i] = old;
    }
    let focus = order.iter().position(|&s| s == from)?;
    let heights: Vec<usize> = cols.iter().map(|c| c.slots.len()).filter(|&k| k > 0).collect();
    if heights.iter().sum::<usize>() != n {
        return None;
    }
    let colr = fix(cols.iter().map(|c| c.width).collect(), heights.len());
    let rowr = cols.iter().zip(&heights).map(|(c, &rows)| fix(c.rows.clone(), rows)).collect();
    Some(LayoutShift { heights, colr, rowr, perm, focus })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane_geo;
    use crate::pane_tab_close_rect;
    use crate::pane_tab_rects;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn near_slice(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| near(*x, *y))
    }

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    fn assert_shift(plan: &LayoutShift, n: usize, from: usize) {
        assert_eq!(plan.perm[plan.focus], from);
        assert!(plan.focus < n);
        let mut seen = [false; MAXP];
        for &k in &plan.perm {
            assert!(k < MAXP && !seen[k], "perm {:?}", plan.perm);
            seen[k] = true;
        }
        let mut live: Vec<usize> = plan.perm[..n].to_vec();
        live.sort_unstable();
        assert_eq!(live, (0..n).collect::<Vec<_>>());
        assert!(plan.perm[n..].iter().enumerate().all(|(i, &k)| k == n + i));
        let sum: f64 = plan.colr.iter().sum();
        assert!(near(sum, 1.0), "colr {:?}", plan.colr);
        for rows in &plan.rowr {
            assert!(near(rows.iter().sum(), 1.0), "rowr {rows:?}");
        }
        assert_eq!(plan.heights.iter().sum::<usize>(), n);
        assert!(plan.heights.len() <= GRID_CAP && plan.heights.iter().all(|&k| (1..=GRID_CAP).contains(&k)));
    }

    #[test]
    fn slack_tracks_dpi_and_stays_a_few_pixels() {
        assert_eq!(drag_slack(96), 4);
        assert_eq!(drag_slack(144), 6);
        assert_eq!(drag_slack(192), 8);
        assert_eq!(drag_slack(1), 4);
    }

    #[test]
    fn grab_is_the_header_and_the_strip_not_the_list_or_controls() {
        let bar = pane_geo(900, 600, 96, false);
        assert_eq!(pane_grab(&bar, 1, bar.num.left, bar.num.top, 96), Some(PaneGrab::Header));
        assert!(pane_grab(&bar, 1, bar.addr.left + 2, bar.addr.top + 2, 96).is_none());
        assert!(pane_grab(&bar, 1, bar.filt.left + 2, bar.filt.top + 2, 96).is_none());
        assert!(pane_grab(&bar, 1, bar.btn[0].left + 1, bar.btn[0].top + 1, 96).is_none());
        assert!(pane_grab(&bar, 1, bar.view.left + 4, bar.view.top + 4, 96).is_none());
        let wide = pane_geo(900, 600, 144, false);
        assert_eq!(pane_grab(&wide, 1, wide.num.left + 1, wide.num.top + 1, 144), Some(PaneGrab::Header));
        assert!(pane_grab(&wide, 1, wide.view.left, wide.view.top, 144).is_none());

        let strip = pane_geo(900, 600, 96, true);
        let (tabs, plus) = pane_tab_rects(strip.tabs, 2, 96);
        assert_eq!(pane_grab(&strip, 2, tabs[0].left + 8, tabs[0].top + 4, 96), Some(PaneGrab::Tab(0)));
        let close = pane_tab_close_rect(tabs[0], 96).unwrap();
        assert!(pane_grab(&strip, 2, close.left + 2, close.top + 2, 96).is_none());
        assert!(pane_grab(&strip, 2, plus.left + 2, plus.top + 2, 96).is_none());
        assert_eq!(pane_grab(&strip, 2, plus.right + 8, strip.tabs.top + 8, 96), Some(PaneGrab::Strip));

        // A narrow column used to drop the number, and the buttons then covered the whole header.
        for (w, dpi, strip_on) in [(240, 96, true), (200, 96, false), (384, 192, true), (80, 96, true)] {
            let p = pane_geo(w, 400, dpi, strip_on);
            let tabs = if strip_on { 2 } else { 1 };
            let x = p.num.left + (p.num.right - p.num.left) / 2;
            let y = p.num.top + (p.num.bottom - p.num.top) / 2;
            assert_eq!(pane_grab(&p, tabs, x, y, dpi), Some(PaneGrab::Header), "{w}@{dpi} strip={strip_on}");
        }
    }

    #[test]
    fn zones_are_quarters_and_a_corner_picks_the_closer_edge() {
        let pane = r(0, 0, 100, 100);
        assert_eq!(pane_zone(pane, 50, 50), Some(DropZone::Center));
        assert_eq!(pane_zone(pane, 10, 50), Some(DropZone::Left));
        assert_eq!(pane_zone(pane, 80, 50), Some(DropZone::Right));
        assert_eq!(pane_zone(pane, 50, 10), Some(DropZone::Top));
        assert_eq!(pane_zone(pane, 50, 90), Some(DropZone::Bottom));
        assert_eq!(pane_zone(pane, 10, 20), Some(DropZone::Left));
        assert_eq!(pane_zone(pane, 20, 10), Some(DropZone::Top));
        assert_eq!(pane_zone(pane, 10, 10), Some(DropZone::Left));
        assert_eq!(pane_zone(pane, -1, 10), None);
        assert_eq!(pane_zone(pane, 100, 10), None);
        // Physical pixels of a 4× pane: the band is still a quarter of that rect.
        let big = r(0, 0, 400, 200);
        assert_eq!(pane_zone(big, 50, 100), Some(DropZone::Left));
        assert_eq!(pane_zone(big, 200, 20), Some(DropZone::Top));
        assert_eq!(pane_zone(big, 200, 100), Some(DropZone::Center));
    }

    #[test]
    fn preview_shows_the_swap_or_the_half_the_pane_will_take() {
        let heights = vec![2, 1];
        let cells = vec![r(0, 0, 100, 50), r(0, 58, 100, 108), r(108, 0, 208, 108)];
        assert_eq!(drop_preview(&heights, &cells, 2, DropZone::Center), Some(cells[2]));
        assert_eq!(drop_preview(&heights, &cells, 2, DropZone::Left), Some(r(108, 0, 158, 108)));
        assert_eq!(drop_preview(&heights, &cells, 2, DropZone::Right), Some(r(158, 0, 208, 108)));
        assert_eq!(drop_preview(&heights, &cells, 1, DropZone::Top), Some(r(0, 58, 100, 83)));
        assert_eq!(drop_preview(&heights, &cells, 1, DropZone::Bottom), Some(r(0, 83, 100, 108)));
        assert!(drop_preview(&heights, &cells, 9, DropZone::Center).is_none());
    }

    #[test]
    fn center_swaps_two_panes_and_leaves_the_grid_alone() {
        let heights = vec![2, 2];
        let colr = vec![0.25, 0.75];
        let rowr = vec![vec![0.4, 0.6], vec![0.3, 0.7]];
        let plan = pane_drop(&heights, &colr, &rowr, 0, 3, DropZone::Center).unwrap();
        assert_eq!(plan.heights, heights);
        assert_eq!(plan.colr, colr);
        assert_eq!(plan.rowr, rowr);
        assert_eq!(&plan.perm[..4], &[3, 1, 2, 0]);
        assert_eq!(plan.focus, 3);
        assert_shift(&plan, 4, 0);
        assert!(pane_drop(&heights, &colr, &rowr, 1, 1, DropZone::Center).is_none());
        assert!(pane_drop(&heights, &colr, &rowr, 0, 4, DropZone::Center).is_none());
    }

    #[test]
    fn edge_reorders_inside_a_column_and_keeps_each_height() {
        let rowr = vec![vec![0.2, 0.3, 0.5]];
        let plan = pane_drop(&[3], &[1.0], &rowr, 2, 0, DropZone::Top).unwrap();
        assert_eq!(plan.heights, vec![3]);
        assert!(near_slice(&plan.rowr[0], &[0.5, 0.2, 0.3]));
        assert_eq!(&plan.perm[..3], &[2, 0, 1]);
        assert_eq!(plan.focus, 0);
        assert_shift(&plan, 3, 2);
        // Already above that pane: not a change.
        assert!(pane_drop(&[3], &[1.0], &rowr, 0, 1, DropZone::Top).is_none());
        let down = pane_drop(&[3], &[1.0], &rowr, 0, 2, DropZone::Bottom).unwrap();
        assert_eq!(&down.perm[..3], &[1, 2, 0]);
        assert!(near_slice(&down.rowr[0], &[0.3, 0.5, 0.2]));
        assert_eq!(down.focus, 2);
    }

    #[test]
    fn moving_a_column_keeps_its_width() {
        let colr = vec![0.1, 0.2, 0.3, 0.4];
        let rowr = vec![vec![1.0]; 4];
        let plan = pane_drop(&[1, 1, 1, 1], &colr, &rowr, 0, 3, DropZone::Right).unwrap();
        assert_eq!(plan.heights, vec![1, 1, 1, 1]);
        assert!(near_slice(&plan.colr, &[0.2, 0.3, 0.4, 0.1]));
        assert_eq!(&plan.perm[..4], &[1, 2, 3, 0]);
        assert_eq!(plan.focus, 3);
        assert_shift(&plan, 4, 0);
        // Already the column on that side.
        assert!(pane_drop(&[1, 1], &[0.2, 0.8], &[vec![1.0], vec![1.0]], 0, 1, DropZone::Left).is_none());
        let swapped = pane_drop(&[1, 1], &[0.2, 0.8], &[vec![1.0], vec![1.0]], 0, 1, DropZone::Right).unwrap();
        assert_eq!(swapped.heights, vec![1, 1]);
        assert!(near_slice(&swapped.colr, &[0.8, 0.2]));
        assert_eq!(&swapped.perm[..2], &[1, 0]);
    }

    #[test]
    fn an_edge_splits_the_target_when_the_pane_leaves_another_cell() {
        // Stack becomes two columns. The new column takes half the old width.
        let side = pane_drop(&[2], &[1.0], &[vec![0.4, 0.6]], 0, 1, DropZone::Left).unwrap();
        assert_eq!(side.heights, vec![1, 1]);
        assert!(near_slice(&side.colr, &[0.5, 0.5]));
        assert_eq!(&side.perm[..2], &[0, 1]);
        assert_eq!(side.focus, 0);
        assert_shift(&side, 2, 0);

        // Pane 0 leaves the first column and takes the top half of the pane in the second.
        let rowr = vec![vec![0.25, 0.75], vec![1.0]];
        let down = pane_drop(&[2, 1], &[0.4, 0.6], &rowr, 0, 2, DropZone::Top).unwrap();
        assert_eq!(down.heights, vec![1, 2]);
        assert!(near_slice(&down.colr, &[0.4, 0.6]));
        assert!(near_slice(&down.rowr[0], &[1.0]));
        assert!(near_slice(&down.rowr[1], &[0.5, 0.5]));
        assert_eq!(&down.perm[..3], &[1, 0, 2]);
        assert_eq!(down.focus, 1);
        assert_shift(&down, 3, 0);

        // A whole column dropped under another pane gives its width to the survivor, then splits that row.
        let under = pane_drop(&[1, 2], &[0.5, 0.5], &[vec![1.0], vec![0.25, 0.75]], 0, 2, DropZone::Bottom).unwrap();
        assert_eq!(under.heights, vec![3]);
        assert!(near_slice(&under.colr, &[1.0]));
        assert!(near_slice(&under.rowr[0], &[0.25, 0.375, 0.375]));
        assert_eq!(&under.perm[..3], &[1, 2, 0]);
        assert_shift(&under, 3, 0);
    }

    #[test]
    fn a_fifth_column_or_row_is_refused_and_a_reorder_at_the_cap_is_not() {
        let cols = vec![0.25; 4];
        let rows = vec![vec![1.0], vec![1.0], vec![1.0], vec![0.5, 0.5]];
        assert!(pane_drop(&[1, 1, 1, 2], &cols, &rows, 4, 0, DropZone::Right).is_none());
        let moved = pane_drop(&[1, 1, 1, 1], &vec![0.1, 0.2, 0.3, 0.4], &vec![vec![1.0]; 4], 3, 0, DropZone::Left).unwrap();
        assert_eq!(moved.heights, vec![1, 1, 1, 1]);
        assert_eq!(&moved.perm[..4], &[3, 0, 1, 2]);
        assert_shift(&moved, 4, 3);

        let tall = vec![vec![0.25; 4], vec![1.0]];
        assert!(pane_drop(&[4, 1], &[0.5, 0.5], &tall, 4, 0, DropZone::Top).is_none());
        assert!(pane_drop(&[4, 1], &[0.5, 0.5], &tall, 4, 0, DropZone::Center).is_some());
        let reorder = pane_drop(&[4], &[1.0], &[vec![0.25; 4]], 0, 3, DropZone::Bottom).unwrap();
        assert_eq!(reorder.heights, vec![4]);
        assert_eq!(&reorder.perm[..4], &[1, 2, 3, 0]);
        assert_shift(&reorder, 4, 0);
    }
}
