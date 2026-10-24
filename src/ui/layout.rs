use crate::imports::*;
use super::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;

// ---------- grid geometry ----------

/// Shrinks `r` into `work` and slides it inside. A rect already inside comes back unchanged.
pub(crate) fn fit(r: RECT, work: RECT) -> RECT {
    let w = (r.right - r.left).min(work.right - work.left).max(0);
    let h = (r.bottom - r.top).min(work.bottom - work.top).max(0);
    let left = r.left.clamp(work.left, work.right - w);
    let top = r.top.clamp(work.top, work.bottom - h);
    RECT { left, top, right: left + w, bottom: top + h }
}

/// `fit` for a saved normal position, which is in workspace coordinates (origin at the work area's corner).
pub(crate) fn fit_saved(saved: RECT, mon: RECT, work: RECT) -> RECT {
    let (dx, dy) = (work.left - mon.left, work.top - mon.top);
    let r = fit(RECT { left: saved.left + dx, top: saved.top + dy, right: saved.right + dx, bottom: saved.bottom + dy }, work);
    RECT { left: r.left - dx, top: r.top - dy, right: r.right - dx, bottom: r.bottom - dy }
}

/// Splits `len` px starting at `start` into cells by `ratios`, with `gap` px splitters between them.
pub(crate) fn cells(start: i32, len: i32, ratios: &[f64], gap: i32) -> Vec<(i32, i32)> {
    let avail = (len - gap * (ratios.len() as i32 - 1)).max(0) as f64;
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(ratios.len());
    for (i, r) in ratios.iter().enumerate() {
        let a = start + (acc * avail).round() as i32 + i as i32 * gap;
        acc += r;
        out.push((a, start + (acc * avail).round() as i32 + i as i32 * gap));
    }
    out
}

/// Moves the boundary after cell `k` to `pos`, keeping both neighbours at least `min` px.
pub(crate) fn drag_ratio(r: &mut [f64], k: usize, pos: i32, start: i32, len: i32, gap: i32, min: i32) {
    let avail = (len - gap * (r.len() as i32 - 1)).max(1) as f64;
    let before: f64 = r[..k].iter().sum();
    let pair = r[k] + r[k + 1];
    let m = (min as f64 / avail).min(pair / 2.0);
    let t = ((pos - start - k as i32 * gap) as f64 / avail - before).clamp(m, pair - m);
    (r[k], r[k + 1]) = (t, pair - t);
}

/// Minimum height of the folder tree and of the favourites list, in px at this DPI.
pub(crate) fn side_min(dpi: i32) -> i32 {
    80 * dpi.max(1) / 96
}

/// Pixel heights of the folder tree and the favourites list inside a plate of `height` px.
/// `ratio` is the tree's share of the space left after the `gap` px splitter.
/// Each part keeps at least `min` px when the plate is tall enough.
pub(crate) fn split_heights(height: i32, ratio: f64, gap: i32, min: i32) -> (i32, i32) {
    let avail = (height - gap).max(0);
    if avail == 0 {
        return (0, 0);
    }
    let min = min.clamp(0, avail / 2);
    let tree = ((avail as f64) * sane_split(ratio)).round() as i32;
    let tree = tree.clamp(min, avail - min);
    (tree, avail - tree)
}

/// Ratio that places the splitter so its top edge is at `pos`. Both parts stay at least `min` px.
pub(crate) fn split_ratio_at(pos: i32, top: i32, height: i32, gap: i32, min: i32) -> f64 {
    let avail = (height - gap).max(1);
    let raw = (pos - top).clamp(0, avail) as f64 / avail as f64;
    let (tree, _) = split_heights(height, raw, gap, min);
    tree as f64 / avail as f64
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Hit {
    Side,
    SideSplit,         // horizontal divider between the folder tree and favourites
    Col(usize),        // boundary after column k
    Row(usize, usize), // (column, boundary after row k in it)
}

// Main window: tab strip on top, status line at the bottom, plates (sidebar + grid) in between.
// The gaps between plates are the splitters. Drive buttons and the icon group sit on the strip.
#[derive(Clone)]
pub(crate) struct Geo {
    pub(crate) w: i32,
    pub(crate) h: i32,   // client height
    pub(crate) top: i32, // tab strip bottom = plates top
    pub(crate) bot: i32, // plates bottom = status line top
    pub(crate) pad: i32,
    pub(crate) gap: i32,
    pub(crate) side: i32,   // sidebar plate width
    pub(crate) tree_h: i32, // folder-tree height inside the plate; favourites sit below the gap
    pub(crate) gx: i32,     // grid left
    pub(crate) xs: Vec<(i32, i32)>,
    pub(crate) ys: Vec<Vec<(i32, i32)>>, // per column
    pub(crate) zoom: bool, // one pane fills the grid: no splitters
}

/// A splitter release stores the clamped panel width. While the panel is hidden, the kept width stays.
pub(crate) fn kept_side(panel: bool, stored: i32, clamped: i32) -> i32 {
    if panel { clamped } else { stored }
}

pub(crate) fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap) = (s(8), s(8));
    let side = if a.s.tree_on { a.s.side.clamp(s(120), (rc.right - gap - pad - s(240)).max(s(120))) } else { 0 };
    let gx = if a.s.tree_on { side + gap } else { pad };
    let top = s(32); // tab strip. `bar` hides the icon group except the gear, not this row.
    let bot = if a.s.stat { (rc.bottom - s(24)).max(top) } else { rc.bottom };
    let plate = (bot - top).max(0);
    let tree_h = if side > 0 { split_heights(plate, a.s.side_split, gap, side_min(a.dpi)).0 } else { 0 };
    Geo {
        xs: cells(gx, rc.right - pad - gx, &a.s.colr, gap),
        ys: a.s.rowr.iter().map(|r| cells(top, bot - top, r, gap)).collect(),
        w: rc.right,
        h: rc.bottom,
        top,
        bot,
        pad,
        gap,
        side,
        tree_h,
        gx,
        zoom: a.zoom.is_some(),
    }
}

pub(crate) fn side_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.top, right: g.side, bottom: g.bot }
}

pub(crate) fn status_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.bot, right: g.w, bottom: g.h }
}

/// Pane slot -> (column, row): panes are numbered down each column, left to right.
pub(crate) fn slot(h: &[usize], mut i: usize) -> (usize, usize) {
    for (c, &n) in h.iter().enumerate() {
        if i < n {
            return (c, i);
        }
        i -= n;
    }
    (h.len().saturating_sub(1), h.last().copied().unwrap_or(1).saturating_sub(1))
}

/// The cell each off-screen slot's pane last had. Runtime only.
pub(crate) type Homes = [Option<(usize, usize)>; MAXP];

/// Layout `old` to `new`. Slot j shows the pane that was in slot `perm[j]`.
/// A pane whose cell is in both stays there. A cell that leaves is remembered and comes back.
/// New cells take the lowest slots that have no home.
pub(crate) fn remap(old: &[usize], new: &[usize], home: &Homes) -> ([usize; MAXP], Homes) {
    let has = |(c, r): (usize, usize)| new.get(c).is_some_and(|&n| r < n);
    let (on, n) = (old.iter().sum::<usize>(), new.iter().sum::<usize>());
    let mut home = *home;
    let mut perm = [usize::MAX; MAXP];
    for j in 0..MAXP {
        let Some(c) = (if j < on { Some(slot(old, j)) } else { home[j] }) else { continue };
        if has(c) {
            perm[new[..c.0].iter().sum::<usize>() + c.1] = j;
            home[j] = None;
        } else {
            home[j] = Some(c);
        }
    }
    let free: Vec<usize> = (0..MAXP).filter(|j| !perm.contains(j) && home[*j].is_none()).collect();
    let empty: Vec<usize> = (0..n).filter(|&k| perm[k] == usize::MAX).collect();
    for (k, j) in empty.into_iter().zip(free) {
        perm[k] = j;
    }
    let rest: Vec<usize> = (0..MAXP).filter(|j| !perm[..n].contains(j)).collect();
    let mut out = [None; MAXP];
    for (k, j) in (n..MAXP).zip(rest) {
        (perm[k], out[k]) = (j, home[j]);
    }
    (perm, out)
}

pub(crate) fn cell_rect(g: &Geo, h: &[usize], i: usize) -> RECT {
    let (c, r) = slot(h, i);
    let ((x0, x1), (y0, y1)) = (g.xs[c], g.ys[c][r]);
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; a pane-tab
// strip under that bar when this pane has two or more tabs; the Explorer view below.
// One tab hides the strip (the list uses it) and shows `add` just to the right of ⇄.
// Window tabs are a different strip, on the main window.
pub(crate) struct PaneGeo {
    pub(crate) num: RECT,      // pane number (Ctrl+N); zero width when the pane is too narrow
    pub(crate) btn: [RECT; 4], // back, forward, up, open-in
    pub(crate) addr: RECT,
    pub(crate) add: RECT, // add-tab button; zero width while the strip is showing
    pub(crate) filt: RECT,
    pub(crate) fav: RECT,
    pub(crate) tabs: RECT, // pane-tab strip between the bar and the list; empty while this pane has one tab
    pub(crate) view: RECT,
}

/// `strip` when this pane has two or more tabs. Callers that have a window use `pane_geo_of`.
pub(crate) fn pane_geo(w: i32, h: i32, dpi: i32, strip: bool) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(30), s(22), s(24), s(4));
    let strip_h = if strip { s(26) } else { 0 };
    let y = (bar - bh) / 2;
    let r = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let fav = r(w - pad - b, b);
    // back, forward, up, ⇄, favourite; the add-tab button joins them while the strip is hidden
    let nbtn = if strip { 5 } else { 6 };
    let fw = s(150).min((w - 2 * pad - nbtn * b) / 3).max(0);
    let filt = r(fav.left - s(2) - fw, fw);
    let add = if strip { RECT { left: filt.left, top: y, right: filt.left, bottom: y + bh } } else { r(filt.left - s(2) - b, b) };
    let open = r(add.left - s(2) - b, b);
    // pane number at the far left, only while the address edit (pill minus its s(8)/s(6) insets) keeps s(32)
    let nw = s(18);
    let nw = if open.left - s(4) - (pad + 3 * b + s(4) + nw) >= s(8) + s(6) + s(32) { nw } else { 0 };
    let x = pad + nw;
    let addr_x = x + 3 * b + s(4);
    PaneGeo {
        num: r(pad, nw),
        btn: [r(x, b), r(x + b, b), r(x + 2 * b, b), open],
        addr: r(addr_x, open.left - s(4) - addr_x),
        add,
        filt,
        fav,
        tabs: RECT { left: s(2), top: bar, right: w - s(2), bottom: bar + strip_h },
        // 2px in, so the active ring stays visible around the list
        view: RECT { left: s(2), top: bar + strip_h, right: w - s(2), bottom: h - s(2) },
    }
}

/// This window's bar. The strip is on only while that pane has two or more tabs.
pub(crate) fn pane_geo_of(h: HWND, w: i32, height: i32, dpi: i32) -> PaneGeo {
    let strip = pane_of(h).is_some_and(|i| app(|a| a.s.pane_tab_list(i).len() > 1));
    pane_geo(w, height, dpi, strip)
}

/// Pane tabs share the strip evenly up to a comfortable width, then a '+' after the last. (tabs, '+')
pub(crate) fn pane_tab_rects(strip: RECT, n: usize, dpi: i32) -> (Vec<RECT>, RECT) {
    let s = |v: i32| v * dpi / 96;
    let (pad, plus) = (s(4), s(24));
    let room = (strip.right - strip.left - 2 * pad - plus).max(0);
    let w = (room / n.max(1) as i32).min(s(170));
    let tab = |k: i32| RECT { left: strip.left + pad + k * w, top: strip.top + s(3), right: strip.left + pad + (k + 1) * w, bottom: strip.bottom };
    let tabs: Vec<RECT> = (0..n as i32).map(tab).collect();
    let x = tabs.last().map_or(strip.left + pad, |r| r.right);
    (tabs, RECT { left: x, top: strip.top + s(3), right: x + plus, bottom: strip.bottom })
}

/// The × on a pane tab, at its right end. Only on tabs wide enough to keep some name.
pub(crate) fn pane_tab_close_rect(tab: RECT, dpi: i32) -> Option<RECT> {
    let s = |v: i32| v * dpi / 96;
    (tab.right - tab.left >= s(64)).then(|| RECT { left: tab.right - s(22), right: tab.right - s(4), ..tab })
}

pub(crate) fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// The search icon or filter chip at the left of the filter pill. The edit starts at 24 DIP, so this stays clickable.
pub(crate) fn filter_icon_rect(filt: RECT, s24: i32) -> Option<RECT> {
    let r = RECT { right: filt.right.min(filt.left + s24), ..filt };
    (r.right > r.left).then_some(r)
}

/// Colour-label strip along the pane's top edge: inside the border, in the gap above the buttons.
pub(crate) fn tint_band(w: i32, dpi: i32, active: bool) -> RECT {
    let s = |v: i32| v * dpi / 96;
    let inset = if active { s(2) } else { 1 };
    let top = pane_geo(w, s(100), dpi, false).num.top;
    let thick = s(3).min((top - inset).max(0));
    RECT { left: inset, top: inset, right: w - inset, bottom: inset + thick }
}

/// A pane's colour label in the current theme: one of the selection-colour presets. 0 is none.
pub(crate) fn tint_color(k: u8) -> Option<u32> {
    (1..=6).contains(&k).then(|| pal().accents[k as usize - 1].1)
}

pub(crate) fn tint_name(k: u8) -> &'static str {
    let i = (k as usize).saturating_sub(1).min(5);
    tr(ACCENT_EN[i], LIGHT.accents[i].0, ACCENT_JA[i])
}

/// A `t` px line down the middle of splitter `d`. None when that splitter is not on screen.
pub(crate) fn gap_line(g: &Geo, d: Hit, t: i32) -> Option<RECT> {
    let vbar = |x: i32| RECT { left: x - t / 2, top: g.top, right: x - t / 2 + t, bottom: g.bot };
    match d {
        Hit::Side if g.side > 0 => Some(vbar(g.side + g.gap / 2)),
        Hit::SideSplit if g.side > 0 && g.tree_h > 0 && g.gap > 0 => {
            let y = g.top + g.tree_h + g.gap / 2;
            Some(RECT { left: 0, top: y - t / 2, right: g.side, bottom: y - t / 2 + t })
        }
        Hit::Col(k) if !g.zoom => {
            g.xs.get(k + 1)?;
            Some(vbar(g.xs[k].1 + g.gap / 2))
        }
        Hit::Row(c, k) if !g.zoom => {
            let (x0, x1) = *g.xs.get(c)?;
            g.ys.get(c)?.get(k + 1)?;
            let y = g.ys[c][k].1 + g.gap / 2;
            Some(RECT { left: x0, top: y - t / 2, right: x1, bottom: y - t / 2 + t })
        }
        _ => None,
    }
}

pub(crate) fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y < g.top || y >= g.bot {
        return None;
    }
    if g.side > 0 && x >= 0 && x < g.side {
        let y0 = g.top + g.tree_h;
        if y >= y0 && y < y0 + g.gap {
            return Some(Hit::SideSplit);
        }
    }
    if g.side > 0 && x >= g.side && x < g.gx {
        return Some(Hit::Side);
    }
    if x < g.gx || g.zoom {
        return None;
    }
    let between = |v: &[(i32, i32)], p: i32| v.windows(2).position(|w| p >= w[0].1 && p < w[1].0);
    if let Some(k) = between(&g.xs, x) {
        return Some(Hit::Col(k));
    }
    // row gaps belong to one column: rows no longer line up across columns
    let c = g.xs.iter().position(|&(a, b)| x >= a && x < b)?;
    between(&g.ys[c], y).map(|k| Hit::Row(c, k))
}

// ---------- layout picker: click = even grid, press and drag across columns = rows per column, presets ----------

pub(crate) const PRESETS: [&[usize]; 6] = [&[1, 2], &[1, 3], &[2, 1], &[3, 1], &[1, 1, 2], &[2, 3]];

pub(crate) struct PickGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) pad: i32,
    pub(crate) cell: i32,
    pub(crate) gap: i32,
    pub(crate) label: RECT,
    pub(crate) presets: [RECT; 6],
}

pub(crate) fn pick_geo(dpi: i32) -> PickGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, cell, gap) = (s(12), s(24), s(6));
    let gw = 4 * cell + 3 * gap;
    let label = RECT { left: pad, top: pad + gw, right: pad + gw, bottom: pad + gw + s(26) };
    let (pw, ph) = ((gw - 2 * gap) / 3, s(24));
    let presets = std::array::from_fn(|i| {
        let (x, y) = (pad + (i as i32 % 3) * (pw + gap), label.bottom + (i as i32 / 3) * (ph + gap));
        RECT { left: x, top: y, right: x + pw, bottom: y + ph }
    });
    PickGeo { w: 2 * pad + gw, h: label.bottom + 2 * ph + gap + pad, pad, cell, gap, label, presets }
}

pub(crate) fn pick_cell(g: &PickGeo, r: usize, c: usize) -> RECT {
    let (x, y) = (g.pad + c as i32 * (g.cell + g.gap), g.pad + r as i32 * (g.cell + g.gap));
    RECT { left: x, top: y, right: x + g.cell, bottom: y + g.cell }
}

/// (row, column) under the point; `clamp` keeps a drag that leaves the cells on the nearest one.
pub(crate) fn pick_at(g: &PickGeo, x: i32, y: i32, clamp: bool) -> Option<(usize, usize)> {
    let span = 4 * g.cell + 3 * g.gap;
    let inside = (g.pad..g.pad + span).contains(&x) && (g.pad..g.pad + span).contains(&y);
    let at = |v: i32| ((v - g.pad).max(0) / (g.cell + g.gap)).min(3) as usize;
    (inside || clamp).then(|| (at(y), at(x)))
}

/// Dragging: columns follow the cursor; the column under it takes the cursor's row.
pub(crate) fn drag_heights(mut h: Vec<usize>, r: usize, c: usize) -> Vec<usize> {
    h.resize(c + 1, r + 1);
    h[c] = r + 1;
    h
}

pub(crate) unsafe fn open_grid() {
    let (main, dpi, open, closed, cur) = app(|a| (a.main, a.dpi, a.grid, a.grid_closed, a.s.heights.clone()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        _ = GetWindowRect(b, &mut rc);
    }
    let g = pick_geo(dpi);
    app(|a| (a.grid_preview, a.grid_drag, a.grid_preset) = (cur, None, None));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsPicker"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.grid = h);
}

pub(crate) unsafe fn paint_grid(h: HWND) {
    let (preview, cur, hot, dpi, font, accent) =
        app(|a| (a.grid_preview.clone(), a.s.heights.clone(), a.grid_preset, a.dpi, a.font, a.s.accent_now()));
    let soft = blend(accent, pal().card, 0.8);
    let g = pick_geo(dpi);
    let on = |v: &[usize], r: usize, c: usize| c < v.len() && r < v[c];
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        for r in 0..4 {
            for c in 0..4 {
                let col = if on(&preview, r, c) {
                    accent
                } else if on(&cur, r, c) {
                    soft
                } else {
                    pal().pill
                };
                fill(dc, pick_cell(&g, r, c), col);
            }
        }
        let label = if preview.iter().all(|&n| n == preview[0]) {
            match ui() {
                1 => format!("{}행 × {}열", preview[0], preview.len()),
                2 => format!("{} 行 × {} 列", preview[0], preview.len()),
                _ => format!("{} × {}", preview[0], preview.len()),
            }
        } else {
            format!("{}  {}", tr("Per column", "열마다", "列ごと"), layout_label(&preview))
        };
        text(dc, g.label, &label, font, pal().text, DT_CENTER);
        // preset thumbnails: the panes of each layout on a tile
        for (i, (p, t)) in PRESETS.iter().zip(g.presets).enumerate() {
            let (tile, pane) = if hot == Some(i) { (soft, accent) } else { (pal().pill, blend(pal().card, pal().text, 0.25)) };
            fill(dc, t, tile);
            let inner = RECT { left: t.left + 3, top: t.top + 3, right: t.right - 3, bottom: t.bottom - 3 };
            for (c, (x0, x1)) in cells(inner.left, inner.right - inner.left, &even(p.len()), 2).into_iter().enumerate() {
                for (y0, y1) in cells(inner.top, inner.bottom - inner.top, &even(p[c]), 2) {
                    fill(dc, RECT { left: x0, top: y0, right: x1, bottom: y1 }, pane);
                }
            }
        }
    });
}

pub(crate) extern "system" fn grid_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let g = pick_geo(app(|a| a.dpi));
        let show = |v: Vec<usize>, preset: Option<usize>| {
            let changed = app(|a| {
                let changed = a.grid_preview != v || a.grid_preset != preset;
                (a.grid_preview, a.grid_preset) = (v, preset);
                changed
            });
            if changed {
                _ = InvalidateRect(Some(h), None, false);
            }
        };
        let apply = |v: Vec<usize>| {
            _ = DestroyWindow(h);
            set_layout(v);
        };
        match msg {
            WM_PAINT => paint_grid(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some((r, c)) = pick_at(&g, x, y, false) {
                    SetCapture(h);
                    let v = vec![r + 1; c + 1];
                    app(|a| a.grid_drag = Some(v.clone()));
                    show(v, None);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| a.grid_drag.clone()) {
                    if let Some((r, c)) = pick_at(&g, x, y, true) {
                        let v = drag_heights(d, r, c);
                        app(|a| a.grid_drag = Some(v.clone()));
                        show(v, None);
                    }
                } else if let Some((r, c)) = pick_at(&g, x, y, false) {
                    show(vec![r + 1; c + 1], None);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    show(PRESETS[i].to_vec(), Some(i));
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                if let Some(v) = app(|a| a.grid_drag.take()) {
                    _ = ReleaseCapture();
                    apply(v);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    apply(PRESETS[i].to_vec());
                }
            }
            WM_KEYDOWN => {
                // keyboard picks even grids: arrows move the corner, Enter applies
                let v = app(|a| a.grid_preview.clone());
                let (r, c) = (v[v.len() - 1] - 1, v.len() - 1);
                let (r, c) = match VIRTUAL_KEY(wp.0 as u16) {
                    VK_LEFT => (r, c.saturating_sub(1)),
                    VK_RIGHT => (r, (c + 1).min(3)),
                    VK_UP => (r.saturating_sub(1), c),
                    VK_DOWN => ((r + 1).min(3), c),
                    VK_RETURN => {
                        apply(v);
                        return LRESULT(0);
                    }
                    VK_ESCAPE => {
                        _ = DestroyWindow(h);
                        return LRESULT(0);
                    }
                    _ => (r, c),
                };
                show(vec![r + 1; c + 1], None);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.grid, a.grid_drag, a.grid_closed) = (HWND::default(), None, Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn address_icon_slot_is_fixed() {
        use crate::{addr_icon, crumb_field};
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            let narrow = RECT { left: 0, top: 0, right: s(90) - 1, bottom: s(22) };
            let wide = RECT { left: 10, top: 4, right: 10 + s(200), bottom: 4 + s(22) };
            assert!(addr_icon(narrow, dpi).is_none());
            assert_eq!(crumb_field(narrow, dpi).left, narrow.left + s(8));
            let r = addr_icon(wide, dpi).unwrap();
            assert_eq!((r.right - r.left, r.bottom - r.top), (s(16), s(16)));
            assert_eq!(crumb_field(wide, dpi).left, wide.left + s(26));
            assert_eq!(crumb_field(wide, dpi).left, crumb_field(wide, dpi).left);
        }
    }

    #[test]
    fn theme_follow_switches_only_when_it_differs() {
        use crate::theme_target;
        assert_eq!(theme_target(true, Some(true), false), Some(true));
        assert_eq!(theme_target(true, Some(false), true), Some(false));
        assert_eq!(theme_target(true, Some(true), true), None);
        assert_eq!(theme_target(false, Some(true), false), None);
        assert_eq!(theme_target(true, None, false), None);
    }

    #[test]
    fn gap_lines_mark_real_splitters() {
        let g = Geo {
            w: 1000,
            h: 600,
            top: 34,
            bot: 576,
            pad: 8,
            gap: 8,
            side: 240,
            tree_h: 200,
            gx: 248,
            zoom: false,
            xs: vec![(256, 620), (628, 992)],
            ys: vec![vec![(34, 300), (308, 576)], vec![(34, 576)]],
        };
        for d in [Hit::Side, Hit::SideSplit, Hit::Col(0), Hit::Row(0, 0)] {
            let r = gap_line(&g, d, 2).unwrap();
            assert!(r.right > r.left && r.bottom > r.top, "{d:?}");
        }
        for d in [Hit::Col(1), Hit::Col(5), Hit::Row(1, 0), Hit::Row(0, 1), Hit::Row(3, 0)] {
            assert_eq!(gap_line(&g, d, 2), None, "{d:?}");
        }
        let hidden = Geo { side: 0, tree_h: 0, gx: 8, ..g.clone() };
        assert_eq!(gap_line(&hidden, Hit::Side, 2), None);
        assert_eq!(gap_line(&hidden, Hit::SideSplit, 2), None);
        let zoomed = Geo { zoom: true, ..g };
        assert_eq!(gap_line(&zoomed, Hit::Col(0), 2), None);
        assert_eq!(gap_line(&zoomed, Hit::Row(0, 0), 2), None);
        assert!(gap_line(&zoomed, Hit::Side, 2).is_some());
    }

    #[test]
    fn filter_icon_is_the_pill_s_left_edge() {
        let wide = RECT { left: 10, top: 2, right: 160, bottom: 24 };
        let r = filter_icon_rect(wide, 24).unwrap();
        assert_eq!((r.left, r.right), (10, 34));
        assert!(filter_icon_rect(RECT { left: 10, right: 10, ..wide }, 24).is_none());
    }

    #[test]
    fn tint_strip_sits_above_the_buttons() {
        for dpi in [96, 120, 144, 192] {
            for w in [120, 400] {
                for active in [false, true] {
                    let (b, top) = (tint_band(w, dpi, active), pane_geo(w, 300, dpi, false).num.top);
                    let border = if active { 2 * dpi / 96 } else { 1 };
                    assert!(b.top >= border && b.bottom <= top && b.bottom - b.top >= 2, "{dpi} {w} {active}");
                }
            }
        }
    }

    #[test]
    fn split_keeps_a_minimum_and_defaults_to_even() {
        assert_eq!(split_heights(400, 0.5, 8, 80), (196, 196));
        assert_eq!(split_heights(400, 0.0, 8, 80), (80, 312));
        assert_eq!(split_heights(400, 1.0, 8, 80), (312, 80));
        assert_eq!(split_heights(400, f64::NAN, 8, 80), (196, 196));
        // shorter than two minimums: the plate is shared, and neither part vanishes
        let (top, bot) = split_heights(100, 0.9, 8, 80);
        assert!(top >= 1 && bot >= 1);
        assert_eq!(top + bot, 92);
        assert!(side_min(96) == 80 && side_min(192) == 160);
    }

    #[test]
    fn window_fits_work_area() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        let work = r(0, 0, 1024, 728);
        assert_eq!(fit(r(100, 50, 900, 650), work), r(100, 50, 900, 650));
        assert_eq!(fit(r(26, 26, 1426, 926), work), work);
        assert_eq!(fit(r(500, 400, 1300, 1000), work), r(224, 128, 1024, 728));
        let left_mon = r(-1920, 0, 0, 1040);
        assert_eq!(fit(r(-2100, -50, -1300, 550), left_mon), r(-1920, 0, -1120, 600));
        assert_eq!(fit(r(-300, 100, 500, 700), left_mon), r(-800, 100, 0, 700));
        let mon = r(0, 0, 1920, 1080);
        assert_eq!(fit_saved(r(1500, 900, 2300, 1500), mon, r(0, 40, 1920, 1080)), r(1120, 440, 1920, 1040));
        let saved = r(100, 100, 900, 700);
        assert_eq!(fit_saved(saved, mon, r(0, 40, 1920, 1080)), saved);
    }

    #[test]
    fn hidden_panel_keeps_its_width() {
        // Hidden, then a column drag: the clamped 0 must not replace 600.
        assert_eq!(kept_side(false, 600, 0), 600);
        assert_eq!(kept_side(true, 600, 536), 536);
    }

    #[test]
    fn drag_ratio_stops_at_the_minimum_and_double_click_is_even() {
        assert!(near(split_ratio_at(0, 0, 400, 8, 80), 80.0 / 392.0));
        assert!(near(split_ratio_at(400, 0, 400, 8, 80), 312.0 / 392.0));
        assert!(near(split_ratio_at(196, 0, 400, 8, 80), 196.0 / 392.0));
        let again = split_heights(400, split_ratio_at(50, 0, 400, 8, 80), 8, 80);
        assert_eq!(again.0, 80);
    }

    #[test]
    fn side_split_hit_is_the_gap_inside_the_plate() {
        let g = Geo {
            w: 1000,
            h: 800,
            top: 66,
            bot: 700,
            pad: 8,
            gap: 8,
            side: 240,
            tree_h: 300,
            gx: 248,
            xs: vec![(248, 600), (608, 992)],
            ys: vec![vec![(66, 700)], vec![(66, 700)]],
            zoom: false,
        };
        assert_eq!(hit(&g, 100, 66 + 300 + 3), Some(Hit::SideSplit));
        assert_eq!(hit(&g, 100, 66 + 200), None);
        assert_eq!(hit(&g, 244, 66 + 300 + 3), Some(Hit::Side));
        let closed = Geo { side: 0, tree_h: 0, gx: 8, ..g };
        assert_eq!(hit(&closed, 4, 400), None);
    }

    #[test]
    fn remap_keeps_cells() {
        let ident: [usize; MAXP] = std::array::from_fn(|k| k);
        let run = |steps: &[Vec<usize>]| {
            let (mut id, mut home) = (ident, [None; MAXP]);
            for w in steps.windows(2) {
                let (p, h) = remap(&w[0], &w[1], &home);
                (id, home) = (p.map(|k| id[k]), h);
            }
            id
        };
        let tail = |v: &[usize]| v.iter().copied().chain(v.len()..MAXP).collect::<Vec<_>>();
        assert_eq!(run(&[vec![2, 2], vec![3, 2]])[..], tail(&[0, 1, 4, 2, 3])[..]);
        assert_eq!(run(&[vec![1, 1, 1, 1], vec![4]])[..], tail(&[0, 4, 5, 6, 1, 2, 3])[..]);
        for steps in [
            vec![vec![1, 1, 1, 1], vec![4], vec![1, 1, 1, 1]],
            vec![vec![3, 3], vec![2, 2], vec![2, 3], vec![3, 3]],
            vec![vec![3, 2], vec![2, 3], vec![3, 2]],
            vec![vec![2, 2], vec![2, 2]],
        ] {
            assert_eq!(run(&steps), ident, "{steps:?}");
        }
    }

    #[test]
    fn pane_tab_strip_shares_the_row() {
        let strip = RECT { left: 2, top: 30, right: 402, bottom: 56 };
        let (tabs, plus) = pane_tab_rects(strip, 2, 96);
        assert_eq!((tabs[0].left, tabs[0].right - tabs[0].left, tabs[1].right, plus.left, plus.right), (6, 170, 346, 346, 370));
        let (tabs, plus) = pane_tab_rects(strip, 10, 96); // squeezed evenly, '+' still inside
        assert!(tabs.iter().all(|r| r.right - r.left == 36) && plus.right <= strip.right);
        assert!(pane_tab_close_rect(tabs[0], 96).is_none());
        assert!(pane_tab_close_rect(RECT { left: 0, top: 0, right: 64, bottom: 20 }, 96).is_some());
        let pg = PaneGeo {
            num: RECT::default(),
            btn: [RECT::default(); 4],
            addr: RECT::default(),
            filt: RECT::default(),
            fav: RECT::default(),
            add: RECT::default(),
            tabs: strip,
            view: RECT::default(),
        };
        assert!(matches!(crate::pane_tab_hit(&pg, 2, 6, 40, 96), Some(crate::PaneTabHit::Tab(0))));
        assert!(matches!(crate::pane_tab_hit(&pg, 2, 350, 40, 96), Some(crate::PaneTabHit::Plus)));
        assert!(crate::pane_tab_hit(&pg, 2, 1, 40, 96).is_none());
    }

    #[test]
    fn one_tab_hides_the_strip_and_keeps_an_add_button() {
        let one = pane_geo(800, 500, 96, false);
        let many = pane_geo(800, 500, 96, true);
        assert_eq!(one.tabs.bottom, one.tabs.top);
        assert_eq!(many.tabs.bottom - many.tabs.top, 26);
        assert_eq!(many.view.top - one.view.top, 26);
        assert_eq!(one.view.bottom, many.view.bottom);
        assert_eq!(one.add.right - one.add.left, 24);
        assert_eq!(many.add.left, many.add.right);
        assert_eq!(one.btn[3].right + 2, one.add.left);
        assert_eq!(one.add.right + 2, one.filt.left);
        assert_eq!(many.btn[3].right + 2, many.filt.left);
        assert!(one.addr.right < many.addr.right);
        // the number needs a wider pane once the add button takes a slot
        let edge = |strip| (60..=800).find(|&w| pane_geo(w, 500, 96, strip).num.right > pane_geo(w, 500, 96, strip).num.left).unwrap();
        assert!(edge(false) > edge(true));
        for (dpi, strip) in [(96, false), (144, false), (96, true), (144, true)] {
            let s = |v: i32| v * dpi / 96;
            for w in [400, 800, 1200] {
                let p = pane_geo(w, 400, dpi, strip);
                let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.add, p.filt, p.fav];
                assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi} strip={strip}");
                assert!(p.fav.right <= w && p.view.top >= p.btn[0].bottom, "{w}@{dpi}");
            }
            for w in 60..=700 {
                let p = pane_geo(w, 400, dpi, strip);
                assert!(p.fav.right <= w && p.view.top >= p.btn[0].bottom, "{w}@{dpi}");
                if strip {
                    assert_eq!((p.add.right, p.tabs.bottom - p.tabs.top), (p.add.left, s(26)));
                    assert_eq!(p.view.top, p.tabs.bottom);
                } else {
                    assert_eq!((p.tabs.bottom, p.add.right - p.add.left), (p.tabs.top, s(24)));
                    assert_eq!(p.view.top, p.tabs.top);
                    if p.btn[3].left >= p.btn[2].right {
                        assert!(p.btn[3].right <= p.add.left && p.add.right <= p.filt.left, "{w}@{dpi}");
                    }
                }
            }
        }
    }
}

