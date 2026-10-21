use crate::imports::*;
use super::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;

// ---------- grid geometry ----------

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

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Hit {
    Side,
    Col(usize),        // boundary after column k
    Row(usize, usize), // (column, boundary after row k in it)
}

// Main window: toolbar on top, status line at the bottom, plates (sidebar + grid) in between.
// The gaps between plates are the splitters.
pub(crate) struct Geo {
    pub(crate) w: i32,
    pub(crate) h: i32,   // client height
    pub(crate) top: i32, // toolbar bottom = plates top
    pub(crate) bot: i32, // plates bottom = status line top
    pub(crate) pad: i32,
    pub(crate) gap: i32,
    pub(crate) act: i32,  // activity bar width, always on the far left
    pub(crate) side: i32, // sidebar plate width
    pub(crate) gx: i32,   // grid left
    pub(crate) xs: Vec<(i32, i32)>,
    pub(crate) ys: Vec<Vec<(i32, i32)>>, // per column
    pub(crate) zoom: bool, // one pane fills the grid: no splitters
}

pub(crate) fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap) = (s(8), s(8));
    let act = s(36);
    let side = if a.s.tree_on { a.s.side.clamp(s(120), (rc.right - act - gap - pad - s(240)).max(s(120))) } else { 0 };
    let gx = if a.s.tree_on { act + side + gap } else { act + pad };
    let top = (if a.s.bar { s(34) } else { 0 }) + s(32); // toolbar, then the tab strip
    let bot = if a.s.stat { (rc.bottom - s(24)).max(top) } else { rc.bottom };
    Geo {
        xs: cells(gx, rc.right - pad - gx, &a.s.colr, gap),
        ys: a.s.rowr.iter().map(|r| cells(top, bot - top, r, gap)).collect(),
        w: rc.right,
        h: rc.bottom,
        top,
        bot,
        pad,
        gap,
        act,
        side,
        gx,
        zoom: a.zoom.is_some(),
    }
}

pub(crate) fn side_rect(g: &Geo) -> RECT {
    RECT { left: g.act, top: g.top, right: g.act + g.side, bottom: g.bot }
}

pub(crate) fn act_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.top, right: g.act, bottom: g.bot }
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
    (h.len() - 1, h[h.len() - 1] - 1)
}

pub(crate) fn cell_rect(g: &Geo, h: &[usize], i: usize) -> RECT {
    let (c, r) = slot(h, i);
    let ((x0, x1), (y0, y1)) = (g.xs[c], g.ys[c][r]);
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; the Explorer view below.
pub(crate) struct PaneGeo {
    pub(crate) num: RECT,      // pane number (Ctrl+N); zero width when the pane is too narrow
    pub(crate) btn: [RECT; 4], // back, forward, up, open-in
    pub(crate) addr: RECT,
    pub(crate) filt: RECT,
    pub(crate) fav: RECT,
    pub(crate) view: RECT,
}

pub(crate) fn pane_geo(w: i32, h: i32, dpi: i32) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(30), s(22), s(24), s(4));
    let y = (bar - bh) / 2;
    let r = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let fav = r(w - pad - b, b);
    let fw = s(150).min((w - 2 * pad - 5 * b) / 3).max(0);
    let filt = r(fav.left - s(2) - fw, fw);
    let open = r(filt.left - s(2) - b, b);
    // pane number at the far left, only while the address edit (pill minus its s(8)/s(6) insets) keeps s(32)
    let nw = s(18);
    let nw = if open.left - s(4) - (pad + 3 * b + s(4) + nw) >= s(8) + s(6) + s(32) { nw } else { 0 };
    let x = pad + nw;
    let addr_x = x + 3 * b + s(4);
    PaneGeo {
        num: r(pad, nw),
        btn: [r(x, b), r(x + b, b), r(x + 2 * b, b), open],
        addr: r(addr_x, open.left - s(4) - addr_x),
        filt,
        fav,
        // 2px in, so the active ring stays visible around the list
        view: RECT { left: s(2), top: bar, right: w - s(2), bottom: h - s(2) },
    }
}

pub(crate) fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y < g.top || y >= g.bot {
        return None;
    }
    if g.side > 0 && x >= g.act + g.side && x < g.gx {
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

