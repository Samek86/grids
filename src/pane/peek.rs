use std::time::Duration;

use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::i18n::*;
use super::preview::*;
use super::{focus_find_list, focus_view, step_find_sel};
use windows::Win32::UI::Controls::ILD_NORMAL;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};

/// Quick look was opened on a search result, so arrows walk that list and closing it returns there.
static FROM_FIND: AtomicBool = AtomicBool::new(false);
static FROM_FIND_PANE: AtomicUsize = AtomicUsize::new(usize::MAX);

pub(crate) fn peek_shows_find(i: usize) -> bool {
    FROM_FIND.load(Ordering::SeqCst) && FROM_FIND_PANE.load(Ordering::SeqCst) == i
}

fn mark_find_peek(i: Option<usize>) {
    match i {
        Some(i) => {
            FROM_FIND_PANE.store(i, Ordering::SeqCst);
            FROM_FIND.store(true, Ordering::SeqCst);
        }
        None => {
            FROM_FIND.store(false, Ordering::SeqCst);
            FROM_FIND_PANE.store(usize::MAX, Ordering::SeqCst);
        }
    }
}

pub(crate) const WM_PEEK_DONE: u32 = WM_APP + 45; // wParam = generation; the pixels stay in PEEK
pub(crate) const WM_PEEK_LOST: u32 = WM_APP + 51; // the preview went inactive: close it unless it is active again
pub(crate) const WM_PEEK_HOSTED: u32 = WM_APP + 52; // wParam = generation, lParam = 1 shown / 0 failed

const ICON_PREV: &str = "";
const ICON_NEXT: &str = "";

// ---------- F3 quick preview ----------

#[derive(Clone)]
pub(crate) struct PeekReq {
    pub(crate) path: String,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) hwnd: isize, // popup told by WM_PEEK_DONE
    pub(crate) plain: bool, // the preview handler failed: read the file ourselves
}

pub(crate) struct Pix {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) bgra: Vec<u8>, // premultiplied, top-down
}

// One worker, newest request only. gen moves on every want, so a result for an item already left
// (or a closed popup) is dropped instead of shown.
pub(crate) struct PeekJob {
    pub(crate) gen: usize,
    pub(crate) req: Option<PeekReq>,
    pub(crate) done: Option<Peeked>,
}

pub(crate) struct Peeked {
    pub(crate) body: Body,
    pub(crate) icon: i32,          // system image list index for the header, -1 = none
    pub(crate) meta: Vec<String>,  // header facts: type, dimensions / pages / lines, size, date
}

impl PeekJob {
    pub(crate) fn want(&mut self, req: Option<PeekReq>) {
        (self.gen, self.req, self.done) = (self.gen + 1, req, None);
    }

    pub(crate) fn publish(&mut self, gen: usize, r: Peeked) -> bool {
        if self.gen != gen {
            return false;
        }
        self.done = Some(r);
        true
    }

    // One more PDF page for the item still on screen.
    pub(crate) fn add_page(&mut self, gen: usize, p: Pix) -> bool {
        match &mut self.done {
            Some(Peeked { body: Body::Pages { pages, .. }, .. }) if self.gen == gen => {
                pages.push(p);
                true
            }
            _ => false,
        }
    }
}

pub(crate) static PEEK: Mutex<PeekJob> = Mutex::new(PeekJob { gen: 0, req: None, done: None });
pub(crate) static PEEK_WAKE: Condvar = Condvar::new();

pub(crate) fn peek_job() -> std::sync::MutexGuard<'static, PeekJob> {
    PEEK.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_peek(req: Option<PeekReq>) {
    let wake = req.is_some();
    peek_job().want(req);
    if wake {
        static WORKER: Once = Once::new();
        WORKER.call_once(|| _ = std::thread::Builder::new().name("grids-peek".into()).spawn(peek_worker));
        PEEK_WAKE.notify_one();
    }
}

// Lives until the process ends. COM is up only while one preview is made; the lock is never held then.
pub(crate) fn peek_worker() {
    let mut seen = 0;
    loop {
        let (gen, req) = {
            let mut j = peek_job();
            while j.req.is_none() || j.gen == seen {
                j = PEEK_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.gen, j.req.clone().unwrap())
        };
        seen = gen;
        unsafe {
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let body = content(&req);
            let (icon, meta) = meta(&req.path, &body);
            let more = match &body {
                Body::Pages { pages, total } if (pages.len() as u32) < *total => Some(*total),
                _ => None,
            };
            let tell = || _ = PostMessageW(Some(HWND(req.hwnd as _)), WM_PEEK_DONE, WPARAM(gen), LPARAM(0));
            if peek_job().publish(gen, Peeked { body, icon, meta }) {
                tell();
                // the first page is up; the rest follow while this is still the item shown
                if let (Some(total), Ok(doc)) = (more, pdf_open(&req.path)) {
                    for i in 1..total {
                        let Some(p) = (peek_job().gen == gen).then(|| pdf_page(&doc, i, req.w, req.h)).flatten() else { break };
                        if !peek_job().add_page(gen, p) {
                            break;
                        }
                        tell();
                    }
                }
            }
            if com {
                CoUninitialize();
            }
        }
    }
}

// Photos come back with every alpha byte 0: make them opaque. Icons come back with straight alpha (a colour
// brighter than its alpha says so), which AlphaBlend would draw as a dirty fringe: premultiply those.
pub(crate) fn fix_alpha(bgra: &mut [u8]) {
    if bgra.chunks_exact(4).all(|p| p[3] == 0) {
        bgra.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    } else if bgra.chunks_exact(4).any(|p| p[0].max(p[1]).max(p[2]) > p[3]) {
        for p in bgra.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

// Accent line around the popup: 3px at 96 dpi, and never thinner than one pixel.
fn peek_edge(dpi: i32) -> i32 {
    (3 * dpi / 96).max(1)
}

// The popup is laid out like the main window: the frame colour around, content on a plate.
// Everything sits inside `peek_edge`, so the accent border neither covers it nor gets covered.
pub(crate) struct PeekGeo {
    pub(crate) icon: RECT,  // file icon in the header
    pub(crate) title: RECT, // name, then the facts
    pub(crate) close: RECT,
    pub(crate) stage: RECT, // pictures and pages sit here; everything else on a plate filling it
    pub(crate) inner: RECT, // the plate's content box
    pub(crate) prev: RECT,  // ‹ and › in the side margins
    pub(crate) next: RECT,
}

pub(crate) fn peek_geo(rc: RECT, dpi: i32) -> PeekGeo {
    let s = |v: i32| v * dpi / 96;
    let e = peek_edge(dpi);
    let rc = RECT { left: rc.left + e, top: rc.top + e, right: rc.right - e, bottom: rc.bottom - e };
    let (head, side, b, ic, pad) = (s(48), s(56), s(32), s(16), s(20));
    let close = RECT { left: rc.right - s(12) - b, top: rc.top + (head - b) / 2, right: rc.right - s(12), bottom: rc.top + (head + b) / 2 };
    let icon = RECT { left: rc.left + side, top: rc.top + (head - ic) / 2, right: rc.left + side + ic, bottom: rc.top + (head + ic) / 2 };
    let title = RECT { left: icon.right + s(10), top: rc.top, right: close.left - s(12), bottom: rc.top + head };
    let stage = RECT { left: rc.left + side, top: rc.top + head, right: rc.right - side, bottom: rc.bottom - s(24) };
    let inner = RECT { left: stage.left + pad, top: stage.top + pad, right: stage.right - pad, bottom: stage.bottom - pad };
    let mid = (stage.top + stage.bottom) / 2;
    let arrow = |x: i32| RECT { left: x, top: mid - b / 2, right: x + b, bottom: mid + b / 2 };
    PeekGeo { icon, title, close, stage, inner, prev: arrow(rc.left + (side - b) / 2), next: arrow(rc.right - side + (side - b) / 2) }
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(crate) enum Hot {
    #[default]
    None,
    Prev,
    Next,
    Close,
}

pub(crate) fn peek_hit(g: &PeekGeo, x: i32, y: i32) -> Hot {
    match () {
        _ if inside(g.close, x, y) => Hot::Close,
        _ if inside(g.prev, x, y) => Hot::Prev,
        _ if inside(g.next, x, y) => Hot::Next,
        _ => Hot::None,
    }
}

unsafe fn geo_of(h: HWND) -> PeekGeo {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    peek_geo(rc, app(|a| a.dpi))
}

// UI-thread state of the open preview: scroll position, laid-out text, the hosted handler.
#[derive(Default)]
pub(crate) struct PeekView {
    pub(crate) y: i32, // scrolled down, px
    pub(crate) x: i32, // scrolled right (tables, code), px
    pub(crate) max_y: i32, // how far it can scroll down, from the last paint; 0 = it all fits
    pub(crate) page_h: i32, // viewport height from that paint; the thumb is sized from it
    pub(crate) edge_at: Option<Instant>, // the wheel reached the top / bottom then
    pub(crate) wheel: i32, // wheel delta toward the next file step (120 a notch)
    pub(crate) layout: Option<(usize, i32, Vec<Line>, i32)>, // (gen, width, lines, total height)
    pub(crate) fonts: Option<(i32, [HFONT; 4])>,              // (dpi, [mono, bold, h1, h2])
    pub(crate) hosted: Option<usize>, // generation a preview handler is showing
    pub(crate) host_win: Option<HWND>, // the picture area's child window the handler draws in
    pub(crate) hover: bool,            // the mouse is over the popup: ‹ › show
    pub(crate) hot: Hot,
    pub(crate) bar_hot: bool,        // pointer over the scrollbar thumb
    pub(crate) bar_drag: Option<i32>, // mouse y minus thumb top, while that thumb is dragged
    pub(crate) bar_down: bool,        // mouse-down hit the bar, so mouse-up is not ‹ › / ×
}

thread_local! {
    pub(crate) static VIEW: RefCell<PeekView> = RefCell::new(PeekView::default());
}

fn view<R>(f: impl FnOnce(&mut PeekView) -> R) -> R {
    VIEW.with(|v| f(&mut v.borrow_mut()))
}

// New file: back to the top. A drag still holding the mouse lets go too.
unsafe fn reset_scroll() {
    let captured = view(|v| {
        (v.y, v.x, v.layout, v.max_y, v.edge_at, v.wheel) = (0, 0, None, 0, None, 0);
        let captured = v.bar_drag.take().is_some() || v.bar_down;
        (v.page_h, v.bar_hot, v.bar_down) = (0, false, false);
        captured
    });
    if captured {
        _ = ReleaseCapture();
    }
}

/// Heading faces are cloned from the UI font. Drop them when that font changes; the next paint builds new ones.
pub(crate) unsafe fn drop_peek_fonts() {
    let old = view(|v| {
        v.layout = None;
        v.fonts.take()
    });
    if let Some((_, fonts)) = old {
        for f in fonts {
            if !f.is_invalid() {
                _ = DeleteObject(f.into());
            }
        }
    }
}

unsafe fn create_peek(i: usize) -> bool {
    let (main, dpi, open) = app(|a| (a.main, a.dpi, a.peek));
    if !open.0.is_null() {
        return false;
    }
    let s = |v: i32| v * dpi / 96;
    let mut rc = RECT::default();
    _ = GetClientRect(main, &mut rc);
    let (w, h) = ((rc.right * 86 / 100).min(s(1120)), (rc.bottom * 88 / 100).min(s(860)));
    let mut pt = POINT { x: (rc.right - w) / 2, y: (rc.bottom - h) / 2 };
    _ = ClientToScreen(main, &mut pt);
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsPeek"), w!(""), style, pt.x, pt.y, w, h, Some(main), None, None, None) else {
        return false;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| (a.peek, a.peek_pane) = (h, i));
    true
}

pub(crate) unsafe fn open_peek(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    if !app(|a| a.peek.0.is_null()) {
        return;
    }
    if fv.GetFocusedItem().unwrap_or(-1) < 0 {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    if !create_peek(i) {
        return;
    }
    mark_find_peek(None);
    load_peek(false);
}

/// Quick look at one search result. Arrows then walk the result list, not the folder behind it.
pub(crate) unsafe fn open_peek_path(i: usize, path: &str, name: &str) {
    if !app(|a| a.peek.0.is_null()) {
        return;
    }
    if path.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    // Set before the popup exists, so a hide timer that lands during creation keeps this pane's rows.
    mark_find_peek(Some(i));
    if !create_peek(i) {
        mark_find_peek(None);
        return;
    }
    app(|a| a.peek_name = name.to_string());
    show_path(path, false);
}

unsafe fn show_path(path: &str, plain: bool) {
    let (h, dpi) = app(|a| (a.peek, a.dpi));
    if h.0.is_null() {
        return;
    }
    unload_handler();
    reset_scroll();
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let st = peek_geo(rc, dpi).stage;
    want_peek(Some(PeekReq { path: path.to_string(), w: st.right - st.left, h: st.bottom - st.top, hwnd: h.0 as isize, plain }));
    _ = InvalidateRect(Some(h), None, false);
}

// Asks the worker for the pane's focused item at the picture area's size. plain: skip the preview handler.
pub(crate) unsafe fn load_peek(plain: bool) {
    if FROM_FIND.load(Ordering::SeqCst) {
        let path = peek_job().req.as_ref().map(|r| r.path.clone());
        if let Some(path) = path {
            show_path(&path, plain);
            return;
        }
    }
    let (h, i, dpi) = app(|a| (a.peek, a.peek_pane, a.dpi));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    unload_handler();
    reset_scroll();
    let it = fv.GetItem::<IShellItem>(fv.GetFocusedItem().unwrap_or(-1)).ok();
    let shown = |kind| it.as_ref().and_then(|it| it.GetDisplayName(kind).ok()).map(|p| pwstr(p));
    app(|a| a.peek_name = shown(SIGDN_NORMALDISPLAY).unwrap_or_default());
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let st = peek_geo(rc, dpi).stage;
    match shown(SIGDN_FILESYSPATH).or_else(|| shown(SIGDN_DESKTOPABSOLUTEPARSING)) {
        Some(path) => want_peek(Some(PeekReq { path, w: st.right - st.left, h: st.bottom - st.top, hwnd: h.0 as isize, plain })),
        None => {
            let mut j = peek_job();
            j.want(None);
            j.done = Some(Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new() });
        }
    }
    _ = InvalidateRect(Some(h), None, false);
}

// Next / previous in the view's own order. Only a real neighbour (S_OK, in range) that got selected moves it.
// A result preview walks the result list instead, and stays on this row at either end.
pub(crate) unsafe fn step_peek(prev: bool) {
    if FROM_FIND.load(Ordering::SeqCst) {
        let i = app(|a| a.peek_pane);
        if let Some(hit) = step_find_sel(i, prev) {
            app(|a| a.peek_name = hit.name);
            show_path(&hit.path, false);
        }
    } else {
        let Some(fv) = view_of::<IFolderView2>(app(|a| a.peek_pane)) else { return };
        let cur = fv.GetFocusedItem().unwrap_or(-1);
        let mut next = -1;
        let hr = (Interface::vtable(&fv).GetVisibleItem)(Interface::as_raw(&fv), cur, prev.into(), &mut next);
        let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
        let flags = SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
        if hr == S_OK && (0..n).contains(&next) && fv.SelectItem(next, flags as u32).is_ok() {
            load_peek(false);
        }
    }
    // selecting puts the focus in the list; the preview stays open and keeps the keys, like Quick Look
    let h = app(|a| a.peek);
    if !h.0.is_null() && GetActiveWindow() != h {
        _ = SetActiveWindow(h);
        _ = SetFocus(Some(h));
    }
}

pub(crate) unsafe fn close_peek(h: HWND) {
    let i = app(|a| a.peek_pane);
    let from_find = FROM_FIND.load(Ordering::SeqCst);
    _ = DestroyWindow(h);
    if !(from_find && focus_find_list(i)) {
        focus_view(i);
    }
}

// ---------- Windows preview handlers (Office, Outlook, HTML, fonts, PDF readers) ----------

// The handler wanted now (its preview generation), 0 = none. A host thread whose generation is no longer
// this one unloads and quits, so Esc or an arrow never waits for a slow handler.
pub(crate) static HOST_GEN: AtomicUsize = AtomicUsize::new(0);

pub(crate) unsafe fn unload_handler() {
    HOST_GEN.store(0, Ordering::SeqCst);
    if let Some(w) = view(|v| {
        v.hosted = None;
        v.host_win.take()
    }) {
        _ = DestroyWindow(w); // the handler's window goes with it, even while it is still starting
    }
}

// UTF-8 HTML that doesn't name its charset: the old HTML previewer reads it as the ANSI code page, so it
// gets a BOM in front. Everything else is handed over as the file.
unsafe fn handler_stream(path: &str) -> Result<IStream> {
    let name = HSTRING::from(path);
    if matches!(ext_of(path).as_str(), "html" | "htm") {
        if let Ok(b) = std::fs::read(path) {
            let head = String::from_utf8_lossy(&b[..b.len().min(4096)]).to_ascii_lowercase();
            if !b.starts_with(&[0xEF, 0xBB, 0xBF]) && !head.contains("charset") && std::str::from_utf8(&b).is_ok() && !b.is_ascii() {
                let with_bom = [&[0xEF, 0xBB, 0xBF][..], &b].concat();
                return SHCreateMemStream(Some(&with_bom)).ok_or_else(|| Error::from(E_FAIL));
            }
        }
    }
    SHCreateStreamOnFileEx(&name, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None)
}

// Made where Explorer makes them (prevhost.exe, out of process), given the file as a stream when they take one.
unsafe fn host_handler(h: HWND, clsid: &GUID, path: &str, area: RECT) -> Result<IPreviewHandler> {
    let ph: IPreviewHandler = CoCreateInstance(clsid, None, CLSCTX_LOCAL_SERVER).or_else(|_| CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER))?;
    let name = HSTRING::from(path);
    if let Ok(init) = ph.cast::<IInitializeWithStream>() {
        init.Initialize(&handler_stream(path)?, STGM_READ.0)?;
    } else if let Ok(init) = ph.cast::<IInitializeWithItem>() {
        let item: IShellItem = SHCreateItemFromParsingName(&name, None)?;
        init.Initialize(&item, STGM_READ.0)?;
    } else {
        ph.cast::<IInitializeWithFile>()?.Initialize(&name, STGM_READ.0)?;
    }
    if let Ok(vis) = ph.cast::<IPreviewHandlerVisuals>() {
        _ = vis.SetBackgroundColor(cref(pal().card));
        _ = vis.SetTextColor(cref(pal().text));
    }
    ph.SetWindow(h, &area)?;
    ph.DoPreview()?;
    Ok(ph)
}

// One thread per hosted preview: it starts the handler, tells the popup (WM_PEEK_HOSTED, lParam 1 = shown),
// keeps it while it is the one wanted, then unloads it on its own apartment.
fn host_thread(gen: usize, clsid: GUID, path: String, hwnd: isize, popup: isize, area: RECT) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let h = HWND(hwnd as _);
        let ph = host_handler(h, &clsid, &path, area);
        let wanted = || HOST_GEN.load(Ordering::SeqCst) == gen;
        if wanted() {
            _ = PostMessageW(Some(HWND(popup as _)), WM_PEEK_HOSTED, WPARAM(gen), LPARAM(ph.is_ok() as isize));
        }
        if let Ok(ph) = ph {
            let mut msg = MSG::default();
            while wanted() && IsWindow(Some(h)).as_bool() {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
            }
            _ = ph.Unload();
        }
        if com {
            CoUninitialize();
        }
    }
}

// The worker said "a handler shows this": start it on its own thread, once per item.
unsafe fn start_handler(h: HWND, gen: usize) {
    let (clsid, path) = {
        let j = peek_job();
        match (&j.done, &j.req) {
            (Some(Peeked { body: Body::Handler(g), .. }), Some(r)) if j.gen == gen => (*g, r.path.clone()),
            _ => return,
        }
    };
    if HOST_GEN.swap(gen, Ordering::SeqCst) == gen {
        return;
    }
    // The handler fills the plate. `stage` is already inside the accent border (peek_geo); the extra
    // pixel keeps the child off the plate's own line, which WS_CLIPCHILDREN would otherwise clip.
    let st = geo_of(h).stage;
    let line = 1;
    let (w, ht) = (st.right - st.left - line * 2, st.bottom - st.top - line * 2);
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(win) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsPeekHost"), w!(""), style, st.left + line, st.top + line, w, ht, Some(h), None, None, None) else {
        return load_peek(true);
    };
    view(|v| v.host_win = Some(win));
    let (hwnd, popup, area) = (win.0 as isize, h.0 as isize, RECT { left: 0, top: 0, right: w, bottom: ht });
    if std::thread::Builder::new().spawn(move || host_thread(gen, clsid, path, hwnd, popup, area)).is_err() {
        load_peek(true);
    }
}

// The handler's parent: the plate's colour (and "Opening…") until the handler draws over it.
pub(crate) extern "system" fn peek_host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let font = app(|a| a.font);
                buffered(h, |dc, rc| {
                    fill(dc, rc, pal().card);
                    text(dc, rc, tr("Opening…   Esc: cancel", "여는 중…   Esc: 취소", "開いています…   Esc: キャンセル"), font, pal().sub, DT_CENTER);
                });
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// The host thread is done starting: shown, or read the file ourselves.
unsafe fn handler_ready(h: HWND, gen: usize, ok: bool) {
    if HOST_GEN.load(Ordering::SeqCst) != gen || peek_job().gen != gen {
        return;
    }
    if ok {
        view(|v| v.hosted = Some(gen));
        // the handler's window may have taken the focus: arrows and Esc stay ours
        if GetActiveWindow() == h {
            _ = SetFocus(Some(h));
        }
    } else {
        load_peek(true);
    }
    _ = InvalidateRect(Some(h), None, false);
}

// ---------- text layout ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Style {
    Plain,
    Mono, // not wrapped
    H1,
    H2,
    H3,
    Bullet,
    Quote,
}

pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) y: i32,
    pub(crate) h: i32,
    pub(crate) no: usize, // source line number, for the code gutter
}

/// Markdown lines with their look: headings, bullets, quotes, fenced code. Inline marks are dropped.
pub(crate) fn md_lines(text: &str) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    let mut code = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code {
            out.push((l.to_string(), Style::Mono));
            continue;
        }
        let clean = |s: &str| s.replace("**", "").replace("__", "").replace('`', "");
        let h3 = t.strip_prefix("### ").or_else(|| t.strip_prefix("#### ")).or_else(|| t.strip_prefix("##### "));
        let item = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "));
        let (text, style) = if let Some(h) = h3 {
            (clean(h), Style::H3)
        } else if let Some(h) = t.strip_prefix("## ") {
            (clean(h), Style::H2)
        } else if let Some(h) = t.strip_prefix("# ") {
            (clean(h), Style::H1)
        } else if let Some(b) = item {
            let indent = "    ".repeat((l.len() - t.len()) / 2);
            (format!("{indent}• {}", clean(b)), Style::Bullet)
        } else if let Some(q) = t.strip_prefix("> ").or_else(|| t.strip_prefix('>')) {
            (clean(q), Style::Quote)
        } else if t.len() >= 3 && t.chars().all(|c| c == '-') {
            ("────────".into(), Style::Quote)
        } else {
            (clean(l), Style::Plain)
        };
        out.push((text, style));
    }
    out
}

unsafe fn fonts(dpi: i32, base: HFONT) -> [HFONT; 4] {
    if let Some((d, f)) = view(|v| v.fonts) {
        if d == dpi {
            return f;
        }
        for x in f {
            _ = DeleteObject(x.into());
        }
    }
    let mut lf = LOGFONTW::default();
    GetObjectW(base.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut _ as _));
    let mono = CreateFontIndirectW(&LOGFONTW { lfFaceName: logfont("Consolas", 0).lfFaceName, ..lf });
    let bold = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, ..lf });
    let h1 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 16 / 10, ..lf });
    let h2 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 13 / 10, ..lf });
    let f = [mono, bold, h1, h2];
    view(|v| v.fonts = Some((dpi, f)));
    f
}

fn style_font(st: Style, font: HFONT, f: &[HFONT; 4]) -> HFONT {
    match st {
        Style::Mono => f[0],
        Style::H1 => f[2],
        Style::H2 => f[3],
        Style::H3 => f[1],
        _ => font,
    }
}

// Heights of every line at this width, measured once per item and width; painting then draws only what shows.
unsafe fn layout(dc: HDC, gen: usize, width: i32, text: &str, kind: TextKind, font: HFONT, f: &[HFONT; 4], dpi: i32) -> i32 {
    if let Some(total) = view(|v| v.layout.as_ref().filter(|(g, w, ..)| *g == gen && *w == width).map(|l| l.3)) {
        return total;
    }
    let src: Vec<(String, Style)> = match kind {
        TextKind::Markdown => md_lines(text),
        TextKind::Code => text.lines().map(|l| (l.to_string(), Style::Mono)).collect(),
        TextKind::Plain => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
    };
    let mut y = 0;
    let mut lines = Vec::with_capacity(src.len().min(40_000));
    for (no, (t, st)) in src.into_iter().take(40_000).enumerate() {
        let old = SelectObject(dc, style_font(st, font, f).into());
        let mut tm = TEXTMETRICW::default();
        _ = GetTextMetricsW(dc, &mut tm);
        let lh = tm.tmHeight + tm.tmExternalLeading;
        let h = if st == Style::Mono || t.is_empty() {
            lh
        } else {
            let mut w: Vec<u16> = t.encode_utf16().collect();
            let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
            DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_EXPANDTABS | DT_NOPREFIX | DT_EDITCONTROL);
            (r.bottom - r.top).max(lh)
        };
        SelectObject(dc, old);
        y += match st {
            Style::H1 | Style::H2 => lh / 2,
            Style::H3 => lh / 3,
            _ => 0,
        };
        lines.push(Line { text: t, style: st, y, h, no: no + 1 });
        y += h + if matches!(st, Style::H1 | Style::H2) { dpi / 24 } else { 0 };
    }
    view(|v| v.layout = Some((gen, width, lines, y)));
    y
}

// ---------- scrollbar ----------
// Pure geometry: thumb size, thumb position, and clamping. No window, so the tests can pin it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarHit {
    None,
    Thumb,
    Track,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BarLayout {
    pub max_y: i32,
    pub thumb_h: i32,
    pub thumb_top: i32,
}

#[derive(Clone, Copy)]
pub(crate) struct BarRects {
    paint: RECT, // the drawn track
    hit: RECT,   // wider grab target, ending where › starts
}

/// `y` clamped into `0..=max_y`. A negative range is empty.
pub(crate) fn clamp_y(y: i32, max_y: i32) -> i32 {
    y.clamp(0, max_y.max(0))
}

/// Move `y` by `dy`. Once the content is taller than the view (`max_y > 0`) the result stays
/// inside it. Before the first layout `max_y` is 0, and a large `dy` is left for the paint to clamp.
pub(crate) fn nudge(y: i32, max_y: i32, dy: i32) -> i32 {
    let y = y.saturating_add(dy).max(0);
    if max_y > 0 {
        y.min(max_y)
    } else {
        y
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArrowAct {
    Scroll(i32),
    Step,
    Stay, // a key repeat ran into the end: don't walk the file list
}

/// Up / Down while the page is taller than the window. `dy` is positive.
pub(crate) fn arrow_act(y: i32, max_y: i32, down: bool, repeat: bool, dy: i32) -> ArrowAct {
    if max_y > 0 {
        let next = nudge(y, max_y, if down { dy } else { -dy });
        if next != y {
            return ArrowAct::Scroll(next);
        }
        if repeat {
            return ArrowAct::Stay;
        }
    }
    ArrowAct::Step
}

/// Thumb length along a track of `span` px. Short content gets a long thumb; a huge document
/// keeps at least `min_thumb`, and never more than the track.
pub(crate) fn thumb_len(content: i32, view: i32, span: i32, min_thumb: i32) -> i32 {
    if span <= 0 {
        return 0;
    }
    let min_thumb = min_thumb.max(1).min(span);
    if content <= view || view <= 0 || content <= 0 {
        return span;
    }
    let raw = (span as i64 * view as i64 / content as i64) as i32;
    raw.clamp(min_thumb, span)
}

/// Thumb top, relative to the track, for scroll position `y`.
pub(crate) fn thumb_offset(y: i32, max_y: i32, travel: i32) -> i32 {
    if max_y <= 0 || travel <= 0 {
        0
    } else {
        let y = y.clamp(0, max_y);
        ((y as i64 * travel as i64) / max_y as i64) as i32
    }
}

/// Scroll position for a thumb whose top is `rel` px into a travel of `travel`.
pub(crate) fn thumb_scroll(rel: i32, travel: i32, max_y: i32) -> i32 {
    if max_y <= 0 || travel <= 0 {
        0
    } else {
        let rel = rel.clamp(0, travel) as i64;
        ((rel * max_y as i64) / travel as i64).clamp(0, max_y as i64) as i32
    }
}

/// `None` when everything fits. Otherwise the thumb sits inside the track for `y`.
pub(crate) fn bar_layout(content: i32, view: i32, track_top: i32, track_h: i32, min_thumb: i32, y: i32) -> Option<BarLayout> {
    if view <= 0 || track_h <= 0 || content <= view {
        return None;
    }
    let max_y = content - view;
    let thumb_h = thumb_len(content, view, track_h, min_thumb);
    let travel = track_h - thumb_h;
    let thumb_top = track_top + thumb_offset(y, max_y, travel);
    Some(BarLayout { max_y, thumb_h, thumb_top })
}

/// One page toward the click. A click on the thumb leaves `y` alone.
pub(crate) fn page_toward(y: i32, max_y: i32, thumb_top: i32, thumb_h: i32, click_y: i32, page: i32) -> i32 {
    let dir: i32 = if click_y < thumb_top {
        -1
    } else if click_y >= thumb_top.saturating_add(thumb_h) {
        1
    } else {
        0
    };
    nudge(y, max_y, dir.saturating_mul(page.max(0)))
}

fn thumb_min(dpi: i32) -> i32 {
    (24 * dpi / 96).max(8)
}

/// Track in the gutter peek_geo leaves between the stage and ›. The hit rect is wider
/// (a 6 DIP bar is hard to grab) and stops on ›'s left edge.
pub(crate) fn peek_bar_rects(stage: RECT, dpi: i32) -> BarRects {
    let s = |v: i32| v * dpi / 96;
    let gutter = (s(56) - s(32)) / 2;
    let w = s(6).max(4).min((gutter - s(2)).max(1));
    let limit = stage.right + gutter;
    let mut right = stage.right + (gutter - w).max(0) / 2 + w;
    if right > limit {
        right = limit;
    }
    let left = (right - w).max(stage.right);
    let paint = RECT { left, top: stage.top + s(4), right, bottom: stage.bottom - s(4) };
    let hit = RECT { left: paint.left - s(8), top: paint.top, right: limit, bottom: paint.bottom };
    BarRects { paint, hit }
}

/// Where the pointer is on the bar. The thumb wins over the track, across the wider hit width.
pub(crate) fn bar_hit(hit: RECT, thumb: RECT, x: i32, y: i32) -> BarHit {
    let thumb = RECT { left: hit.left, right: hit.right, ..thumb };
    if inside(thumb, x, y) {
        BarHit::Thumb
    } else if inside(hit, x, y) {
        BarHit::Track
    } else {
        BarHit::None
    }
}

/// Track and thumb colours. The track is a theme mix; the thumb carries the accent, nudged
/// toward the theme text so a dim swatch still reads on both ivory and charcoal.
pub(crate) fn bar_rgb(bg: u32, text: u32, accent: u32, hot: bool) -> (u32, u32) {
    let track = blend(bg, text, 0.16);
    let thumb = if hot { blend(accent, on_color(accent), 0.25) } else { blend(accent, text, 0.12) };
    (track, thumb)
}

// A drag or a track click moved `y`. Restart the wheel's "rested at the end" wait when we
// arrive there, so the fling doesn't also step to the next file.
fn note_scroll(v: &mut PeekView, prev: i32) {
    let moved = v.y != prev;
    let at_edge = v.max_y > 0 && (v.y <= 0 || v.y >= v.max_y);
    if !at_edge {
        v.edge_at = None;
    } else if moved {
        v.edge_at = Some(Instant::now());
    }
    v.wheel = 0;
}

// ---------- painting ----------

fn clamp_scroll(total: i32, page: i32) {
    view(|v| {
        let page = page.max(0);
        v.page_h = page;
        v.max_y = total.saturating_sub(page).max(0);
        v.y = clamp_y(v.y, v.max_y);
    });
}

#[derive(Debug, PartialEq)]
pub(crate) enum Wheel {
    Scroll,
    Step,
    Hold, // at the end, but only just: a fling to the bottom doesn't also jump to the next file
}

// A wheel turn scrolls while there is room that way; past the end it moves to the previous / next file,
// at once when nothing scrolls, else once the view has rested at that end for a moment.
pub(crate) fn wheel_action(y: i32, max_y: i32, down: bool, rested: bool) -> Wheel {
    let room = if down { y < max_y } else { y > 0 };
    match () {
        _ if room => Wheel::Scroll,
        _ if max_y == 0 || rested => Wheel::Step,
        _ => Wheel::Hold,
    }
}

unsafe fn wheel_peek(h: HWND, raw: i32) {
    let down = raw < 0;
    let px = -raw * app(|a| a.fh) * 3 / 120;
    let act = view(|v| {
        let rested = v.edge_at.is_some_and(|t| t.elapsed() >= Duration::from_millis(350));
        let act = wheel_action(v.y, v.max_y, down, rested);
        match act {
            Wheel::Scroll => {
                v.y = (v.y + px).clamp(0, v.max_y);
                let end = if down { v.y >= v.max_y } else { v.y <= 0 };
                (v.edge_at, v.wheel) = (end.then(Instant::now), 0);
            }
            Wheel::Hold => {
                v.edge_at.get_or_insert_with(Instant::now);
            }
            Wheel::Step => {}
        }
        act
    });
    match act {
        Wheel::Scroll => _ = InvalidateRect(Some(h), None, false),
        Wheel::Step => step_by_wheel(raw),
        Wheel::Hold => {}
    }
}

// One file per notch; a touchpad's small deltas add up to one.
unsafe fn step_by_wheel(raw: i32) {
    let go = view(|v| {
        if (v.wheel < 0) != (raw < 0) {
            v.wheel = 0;
        }
        v.wheel += raw;
        let go = v.wheel.abs() >= 120;
        if go {
            v.wheel = 0;
        }
        go
    });
    if go {
        step_peek(raw > 0);
    }
}

// DrawTextW without text()'s single-line, end-ellipsis defaults.
pub(crate) unsafe fn text_flags(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // see text()
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, flags | DT_NOPREFIX);
    SelectObject(dc, old);
}

unsafe fn text_w(dc: HDC, s: &str, font: HFONT) -> i32 {
    let w: Vec<u16> = s.chars().take(200).collect::<String>().encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    SelectObject(dc, old);
    sz.cx
}

unsafe fn clip(dc: HDC, r: RECT, draw: impl FnOnce()) {
    let rgn = CreateRectRgn(r.left, r.top, r.right, r.bottom);
    SelectClipRgn(dc, Some(rgn));
    draw();
    SelectClipRgn(dc, None);
    _ = DeleteObject(rgn.into());
}

unsafe fn paint_text(dc: HDC, area: RECT, gen: usize, text: &str, kind: TextKind, cut: bool, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(dpi, font);
    let gutter = if kind == TextKind::Code { s(44) } else { 0 };
    let width = area.right - area.left - gutter - s(8);
    let total = layout(dc, gen, width, text, kind, font, &f, dpi) + if cut { s(28) } else { 0 };
    clamp_scroll(total, area.bottom - area.top);
    let (sy, sx) = view(|v| (v.y, v.x));
    clip(dc, area, || {
        view(|v| {
            let Some((_, _, lines, _)) = v.layout.as_ref() else { return };
            let first = lines.partition_point(|l| l.y + l.h < sy);
            for l in lines[first..].iter().take_while(|l| l.y - sy < area.bottom - area.top) {
                let top = area.top + l.y - sy;
                if gutter > 0 {
                    let g = RECT { left: area.left, top, right: area.left + gutter - s(10), bottom: top + l.h };
                    text_flags(dc, g, &l.no.to_string(), f[0], pal().sub, DT_RIGHT | DT_SINGLELINE);
                }
                let mono = l.style == Style::Mono;
                let (ink, fnt) = match l.style {
                    Style::Quote => (pal().sub, font),
                    st => (pal().text, style_font(st, font, &f)),
                };
                let left = area.left + gutter - if mono { sx } else { 0 };
                let r = RECT { left, top, right: if mono { left + 100_000 } else { area.right - s(8) }, bottom: top + l.h };
                let flags = if mono { DT_SINGLELINE | DT_EXPANDTABS } else { DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL };
                text_flags(dc, r, &l.text, fnt, ink, flags);
            }
        });
        if cut {
            let y = area.top + total - s(24) - sy;
            let r = RECT { left: area.left, top: y, right: area.right, bottom: y + s(24) };
            text_flags(dc, r, tr("… only the first 512 KB is shown", "… 앞부분 512 KB만 표시", "… 先頭 512 KB だけ表示"), font, pal().sub, DT_SINGLELINE);
        }
    });
}

// A grid: column widths from the first rows' text, the first row as a header, scrolled both ways.
unsafe fn paint_table(dc: HDC, area: RECT, rows: &[Vec<String>], cut: bool, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(dpi, font);
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        text(dc, area, tr("Empty", "비어 있음", "空です"), font, pal().sub, DT_CENTER);
        return;
    }
    let mut widths = vec![s(36); cols];
    for (k, r) in rows.iter().take(200).enumerate() {
        for (c, cell) in r.iter().enumerate() {
            let w = text_w(dc, cell, if k == 0 { f[1] } else { font }) + s(14);
            widths[c] = widths[c].max(w.min(s(260)));
        }
    }
    let num = text_w(dc, &rows.len().to_string(), font) + s(14);
    let rh = app(|a| a.fh) + s(8);
    let total = rh * rows.len() as i32 + if cut { rh } else { 0 };
    clamp_scroll(total, area.bottom - area.top);
    let full: i32 = num + widths.iter().sum::<i32>();
    view(|v| v.x = v.x.min((full - (area.right - area.left)).max(0)).max(0));
    let (sy, sx) = view(|v| (v.y, v.x));
    clip(dc, area, || {
        let first = (sy / rh) as usize;
        let shown = ((area.bottom - area.top) / rh + 2) as usize;
        for (k, r) in rows.iter().enumerate().skip(first).take(shown) {
            let top = area.top + k as i32 * rh - sy;
            let row = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k == 0 {
                fill(dc, row, pal().pill);
            }
            fill(dc, RECT { top: row.bottom - 1, ..row }, pal().line);
            text(dc, RECT { left: area.left, right: area.left + num - s(6), ..row }, &(k + 1).to_string(), font, pal().sub, DT_RIGHT);
            let mut x = area.left + num - sx;
            for (c, w) in widths.iter().enumerate() {
                if x + w > area.left + num && x < area.right {
                    let cell = r.get(c).map(String::as_str).unwrap_or("");
                    let cr = RECT { left: (x + s(7)).max(area.left + num), top, right: x + w - s(7), bottom: top + rh };
                    let right = k > 0 && !cell.is_empty() && cell.replace(',', "").parse::<f64>().is_ok();
                    text(dc, cr, cell, if k == 0 { f[1] } else { font }, pal().text, if right { DT_RIGHT } else { DT_LEFT });
                    fill(dc, RECT { left: x + w - 1, top, right: x + w, bottom: top + rh }, pal().line);
                }
                x += w;
            }
        }
        if cut {
            let top = area.top + rows.len() as i32 * rh - sy;
            let r = RECT { left: area.left + s(6), top, right: area.right, bottom: top + rh };
            text(dc, r, tr("… more rows not shown", "… 이후 행은 생략", "… これ以降の行は省略"), font, pal().sub, DT_LEFT);
        }
    });
}

unsafe fn paint_list(dc: HDC, area: RECT, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let rh = app(|a| a.fh) + s(8);
    let top0 = area.top;
    if rows.is_empty() {
        text(dc, area, tr("Empty", "비어 있음", "空です"), font, pal().sub, DT_CENTER);
        return;
    }
    let page = area.bottom - top0;
    clamp_scroll(rh * rows.len() as i32, page);
    let sy = view(|v| v.y);
    let sw = s(90);
    clip(dc, RECT { top: top0, ..area }, || {
        for (k, (name, size)) in rows.iter().enumerate().skip((sy / rh) as usize).take((page / rh + 2) as usize) {
            let top = top0 + k as i32 * rh - sy;
            let r = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k % 2 == 1 {
                fill(dc, RECT { left: r.left - s(8), right: r.right + s(8), ..r }, blend(pal().card, pal().bg, 0.45));
            }
            text(dc, RECT { right: r.right - sw - s(8), ..r }, name, font, pal().text, DT_LEFT);
            text(dc, RECT { left: r.right - sw, ..r }, size, font, pal().sub, DT_RIGHT);
        }
    });
}

// Big icon (or thumbnail) on the left, the facts beside it.
unsafe fn paint_card(dc: HDC, area: RECT, icon: Option<&Pix>, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    // drawn at its own size (only shrunk if it doesn't fit): scaling an icon blurs and fringes it
    let side = icon.map_or(s(96), |p| p.w.max(p.h)).min((area.right - area.left) / 3).min(area.bottom - area.top);
    // icon and facts as one block, centred
    let lw = rows.iter().map(|(l, _)| text_w(dc, l, font)).max().unwrap_or(0) + s(16);
    let vw = rows.iter().map(|(_, v)| text_w(dc, v, font)).max().unwrap_or(0);
    let block = side + s(32) + lw + vw;
    let x0 = area.left + ((area.right - area.left - block) / 2).max(s(8));
    if let Some(p) = icon {
        let k = (side as f32 / p.w.max(p.h) as f32).min(1.0);
        let (dw, dh) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
        let (x, y) = (x0 + (side - dw) / 2, area.top + (area.bottom - area.top - dh) / 2);
        blit_bgra(dc, RECT { left: x, top: y, right: x + dw, bottom: y + dh }, p.w, p.h, &p.bgra);
    }
    let left = x0 + side + s(32);
    let rh = app(|a| a.fh) + s(8);
    let mut y = area.top + ((area.bottom - area.top) - rh * rows.len() as i32).max(0) / 2;
    for (label, value) in rows {
        text(dc, RECT { left, top: y, right: left + lw, bottom: y + rh }, label, font, pal().sub, DT_LEFT);
        let v = RECT { left: left + lw, top: y + s(4), right: area.right, bottom: y + rh * 3 };
        let mut w: Vec<u16> = value.encode_utf16().collect();
        let mut calc = v;
        let old = SelectObject(dc, font.into());
        DrawTextW(dc, &mut w, &mut calc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL);
        SelectObject(dc, old);
        text_flags(dc, v, value, font, pal().text, DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
        y += (calc.bottom - calc.top).clamp(rh - s(8), rh * 3 - s(8)) + s(8);
    }
}

// PDF pages one under another, each its own sheet of paper on the stage.
unsafe fn paint_pages(dc: HDC, area: RECT, pages: &[Pix], total: u32, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let gap = s(16);
    let more = (pages.len() as u32) < total;
    let height: i32 = pages.iter().map(|p| p.h + gap).sum::<i32>() - gap + if more { s(48) } else { 0 };
    clamp_scroll(height, area.bottom - area.top);
    let sy = view(|v| v.y);
    clip(dc, area, || {
        let mut y = area.top - sy;
        for p in pages {
            let (w, h) = (p.w.min(area.right - area.left), p.h * p.w.min(area.right - area.left) / p.w.max(1));
            let x = (area.left + area.right - w) / 2;
            if y + h > area.top && y < area.bottom {
                let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
                blit_bgra(dc, r, p.w, p.h, &p.bgra);
                frame(dc, RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 }, 1, pal().line);
            }
            y += h + gap;
        }
        if more {
            let r = RECT { left: area.left, top: y, right: area.right, bottom: y + s(32) };
            let t = format!("{} {} / {}", tr("Rendering pages…", "페이지 그리는 중…", "ページを描いています…"), pages.len(), total);
            text(dc, r, &t, font, pal().sub, DT_CENTER);
        }
    });
}

pub(crate) unsafe fn paint_peek(h: HWND) {
    let (dpi, font, icon_font, name, ilist, accent) = app(|a| (a.dpi, a.font, a.icon_font, a.peek_name.clone(), a.ilist.clone(), a.s.accent_now()));
    let (hosted, hover, hot) = view(|v| (v.hosted, v.hover, v.hot));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let g = peek_geo(rc, dpi);
        let f = fonts(dpi, font);
        fill(dc, rc, pal().bg);
        let j = peek_job();
        let gen = j.gen;
        // header: icon, name in bold, the facts after it in the secondary colour
        if let (Some(p), Some(il)) = (&j.done, &ilist) {
            if let Some(icon) = (p.icon >= 0).then(|| il.GetIcon(p.icon, ILD_NORMAL.0).ok()).flatten() {
                let r = g.icon;
                _ = DrawIconEx(dc, r.left, r.top, icon, r.right - r.left, r.bottom - r.top, 0, None, DI_NORMAL);
                _ = DestroyIcon(icon);
            }
        }
        let room = g.title.right - g.title.left;
        let name_w = text_w(dc, &name, f[1]).min(room * 6 / 10);
        text(dc, RECT { right: g.title.left + name_w, ..g.title }, &name, f[1], pal().text, DT_LEFT);
        if let Some(p) = &j.done {
            let facts = RECT { left: g.title.left + name_w + s(16), ..g.title };
            text(dc, facts, &p.meta.join("   ·   "), font, pal().sub, DT_LEFT);
        }
        if hot == Hot::Close {
            fill(dc, g.close, blend(pal().bg, pal().text, 0.08));
        }
        text(dc, g.close, ICON_CLOSE, icon_font, if hot == Hot::Close { pal().text } else { pal().sub }, DT_CENTER);
        // content: pictures and pages on the stage; everything else on a plate, as panes are
        let plate = || {
            fill(dc, g.stage, pal().card);
            frame(dc, g.stage, 1, pal().line);
        };
        match j.done.as_ref().map(|p| &p.body) {
            Some(Body::Pic(p)) => {
                // shrunk to fit if the shell handed back something bigger, never enlarged
                let st = g.stage;
                let k = ((st.right - st.left) as f32 / p.w as f32).min((st.bottom - st.top) as f32 / p.h as f32).min(1.0);
                let (dw, dh) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
                let (x, y) = ((st.left + st.right - dw) / 2, (st.top + st.bottom - dh) / 2);
                blit_bgra(dc, RECT { left: x, top: y, right: x + dw, bottom: y + dh }, p.w, p.h, &p.bgra);
                frame(dc, RECT { left: x - 1, top: y - 1, right: x + dw + 1, bottom: y + dh + 1 }, 1, pal().line);
            }
            Some(Body::Pages { pages, total }) => paint_pages(dc, g.stage, pages, *total, font, dpi),
            Some(Body::Text { text: t, kind, cut }) => {
                plate();
                paint_text(dc, g.inner, gen, t, *kind, *cut, font, dpi);
            }
            Some(Body::Table { rows, cut }) => {
                plate();
                paint_table(dc, g.inner, rows, *cut, font, dpi);
            }
            Some(Body::List { rows, .. }) => {
                plate();
                paint_list(dc, g.inner, rows, font, dpi);
            }
            Some(Body::Card { icon, rows }) => {
                plate();
                paint_card(dc, g.inner, icon.as_ref(), rows, font, dpi);
            }
            Some(Body::Handler(_)) => {
                plate();
                if hosted != Some(gen) {
                    text(dc, g.stage, tr("Opening…   Esc: cancel", "여는 중…   Esc: 취소", "開いています…   Esc: キャンセル"), font, pal().sub, DT_CENTER);
                }
            }
            None => text(dc, g.stage, tr("Loading…", "불러오는 중…", "読み込み中…"), font, pal().sub, DT_CENTER),
        }
        drop(j);
        paint_bar(dc, g.stage, dpi, accent);
        // ‹ › while the mouse is over the popup, square like the app's buttons
        if hover {
            for (r, glyph, which) in [(g.prev, ICON_PREV, Hot::Prev), (g.next, ICON_NEXT, Hot::Next)] {
                let on = hot == which;
                fill(dc, r, if on { pal().card } else { pal().bg });
                frame(dc, r, 1, if on { pal().sub } else { pal().line });
                text(dc, r, glyph, icon_font, if on { pal().text } else { pal().sub }, DT_CENTER);
            }
        }
        // Last, so nothing drawn above covers it. Content rects stay inside this thickness.
        frame(dc, rc, peek_edge(dpi), accent);
    });
}

// Wheel, PgUp / PgDn, Home / End, and Up / Down scroll. Shift+wheel moves sideways (tables, code).
unsafe fn scroll_peek(h: HWND, dy: i32, dx: i32) {
    view(|v| {
        v.y = nudge(v.y, v.max_y, dy);
        v.x = v.x.saturating_add(dx).max(0);
    });
    _ = InvalidateRect(Some(h), None, false);
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe fn bar_state(h: HWND) -> Option<(BarRects, BarLayout)> {
    let dpi = app(|a| a.dpi);
    let rects = peek_bar_rects(geo_of(h).stage, dpi);
    let span = rects.paint.bottom - rects.paint.top;
    view(|v| {
        if v.max_y <= 0 || v.page_h <= 0 {
            return None;
        }
        bar_layout(v.max_y.saturating_add(v.page_h), v.page_h, rects.paint.top, span, thumb_min(dpi), v.y).map(|b| (rects, b))
    })
}

fn thumb_of(rects: BarRects, b: &BarLayout) -> RECT {
    RECT { left: rects.paint.left, top: b.thumb_top, right: rects.paint.right, bottom: b.thumb_top + b.thumb_h }
}

unsafe fn thumb_hot(h: HWND, x: i32, y: i32) -> bool {
    let Some((rects, b)) = bar_state(h) else { return false };
    bar_hit(rects.hit, thumb_of(rects, &b), x, y) == BarHit::Thumb
}

// Rounded like the settings window's bar. No pen: the fill is the whole mark.
unsafe fn round_fill(dc: HDC, r: RECT, rad: i32, fill: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(cref(fill));
    let pen = HPEN(GetStockObject(NULL_PEN).0);
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, rad.max(1), rad.max(1));
    SelectObject(dc, op);
    SelectObject(dc, ob);
    _ = DeleteObject(brush.into());
}

unsafe fn paint_bar(dc: HDC, stage: RECT, dpi: i32, accent: u32) {
    let rects = peek_bar_rects(stage, dpi);
    let span = rects.paint.bottom - rects.paint.top;
    let drawn = view(|v| {
        if v.max_y <= 0 || v.page_h <= 0 {
            return None;
        }
        let hot = v.bar_hot || v.bar_drag.is_some();
        bar_layout(v.max_y.saturating_add(v.page_h), v.page_h, rects.paint.top, span, thumb_min(dpi), v.y).map(|b| (b, hot))
    });
    let Some((b, hot)) = drawn else { return };
    let s = |v: i32| v * dpi / 96;
    let (track, thumb) = bar_rgb(pal().bg, pal().text, accent, hot);
    round_fill(dc, rects.paint, s(4), track);
    round_fill(dc, thumb_of(rects, &b), s(4), thumb);
}

unsafe fn press_bar(h: HWND, x: i32, y: i32) -> bool {
    let Some((rects, b)) = bar_state(h) else { return false };
    let thumb = thumb_of(rects, &b);
    match bar_hit(rects.hit, thumb, x, y) {
        BarHit::None => false,
        BarHit::Thumb => {
            view(|v| v.bar_drag = Some(y - b.thumb_top));
            true
        }
        BarHit::Track => {
            let page = view(|v| v.page_h.max(1));
            view(|v| {
                let prev = v.y;
                v.y = page_toward(v.y, v.max_y, b.thumb_top, b.thumb_h, y, page);
                note_scroll(v, prev);
            });
            _ = InvalidateRect(Some(h), None, false);
            true
        }
    }
}

unsafe fn drag_bar(h: HWND, y: i32) {
    let Some(off) = view(|v| v.bar_drag) else { return };
    let Some((rects, b)) = bar_state(h) else { return };
    let travel = (rects.paint.bottom - rects.paint.top - b.thumb_h).max(0);
    if travel <= 0 {
        return;
    }
    let top = (y - off).clamp(rects.paint.top, rects.paint.bottom - b.thumb_h);
    let rel = top - rects.paint.top;
    view(|v| {
        let prev = v.y;
        v.y = thumb_scroll(rel, travel, b.max_y);
        note_scroll(v, prev);
    });
    _ = InvalidateRect(Some(h), None, false);
}

// Up / Down scroll a long page. At the end a fresh press steps to the previous / next file;
// a repeat stops, so holding the key doesn't run through the list. Left / Right always step.
unsafe fn arrow_scroll(h: HWND, down: bool, repeat: bool) {
    let line = app(|a| a.fh).max(1) * 3;
    let act = view(|v| {
        let dy = line.max(v.page_h / 10).max(1);
        let act = arrow_act(v.y, v.max_y, down, repeat, dy);
        if let ArrowAct::Scroll(y) = act {
            let prev = v.y;
            v.y = y;
            note_scroll(v, prev);
        }
        act
    });
    match act {
        ArrowAct::Scroll(_) => _ = InvalidateRect(Some(h), None, false),
        ArrowAct::Step => step_peek(!down),
        ArrowAct::Stay => {}
    }
}

pub(crate) extern "system" fn peek_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let page = || {
            let st = geo_of(h).stage;
            (st.bottom - st.top) * 9 / 10
        };
        let notch = |w: usize| ((w >> 16) as i16 as i32) * app(|a| a.fh) * 3 / 120;
        match msg {
            WM_PAINT => paint_peek(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_PEEK_DONE => {
                start_handler(h, wp.0);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_PEEK_HOSTED => handler_ready(h, wp.0, lp.0 != 0),
            WM_MOUSEMOVE => {
                let (x, y) = lp_xy(lp);
                if view(|v| v.bar_drag.is_some()) {
                    drag_bar(h, y);
                } else {
                    let hot = peek_hit(&geo_of(h), x, y);
                    let over = thumb_hot(h, x, y);
                    let (first, changed) = view(|v| {
                        let r = (!v.hover, !v.hover || v.hot != hot || v.bar_hot != over);
                        (v.hover, v.hot, v.bar_hot) = (true, hot, over);
                        r
                    });
                    if first {
                        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                        _ = TrackMouseEvent(&mut t);
                    }
                    if changed {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                }
            }
            WM_MOUSELEAVE => {
                view(|v| (v.hover, v.hot, v.bar_hot) = (false, Hot::None, false));
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lp_xy(lp);
                if press_bar(h, x, y) {
                    view(|v| v.bar_down = true);
                    _ = SetCapture(h);
                }
            }
            // only × and ‹ › act on a click; the content itself never closes the preview.
            // a press that started on the bar is a scroll, even if the pointer has slid onto ›
            WM_LBUTTONUP => {
                let on_bar = view(|v| {
                    let on = v.bar_down;
                    v.bar_down = false;
                    v.bar_drag = None;
                    on
                });
                if on_bar {
                    _ = ReleaseCapture();
                    _ = InvalidateRect(Some(h), None, false);
                } else {
                    match view(|v| v.hot) {
                        Hot::Close => close_peek(h),
                        Hot::Prev => step_peek(true),
                        Hot::Next => step_peek(false),
                        Hot::None => {}
                    }
                }
            }
            WM_CAPTURECHANGED => {
                view(|v| (v.bar_drag, v.bar_down) = (None, false));
            }
            WM_MOUSEWHEEL if (wp.0 & 0xffff) as u32 & MK_SHIFT.0 != 0 => scroll_peek(h, 0, -notch(wp.0)),
            WM_MOUSEWHEEL => wheel_peek(h, (wp.0 >> 16) as i16 as i32),
            // tilting the wheel: previous / next file, like the arrows
            WM_MOUSEHWHEEL => step_by_wheel(-((wp.0 >> 16) as i16 as i32)),
            WM_KEYDOWN => {
                let k = VIRTUAL_KEY(wp.0 as u16);
                let repeat = lp.0 & (1 << 30) != 0;
                match k {
                    // a held F3 only opens it; its repeats must not close it again
                    VK_ESCAPE | VK_SPACE | VK_F3 if !repeat => close_peek(h),
                    VK_LEFT | VK_RIGHT => step_peek(k == VK_LEFT),
                    VK_UP | VK_DOWN => arrow_scroll(h, k == VK_DOWN, repeat),
                    VK_NEXT => scroll_peek(h, page(), 0),
                    VK_PRIOR => scroll_peek(h, -page(), 0),
                    VK_HOME => scroll_peek(h, i32::MIN, i32::MIN),
                    VK_END => scroll_peek(h, i32::MAX, 0),
                    _ => {}
                }
            }
            // another app or a click behind closes it; stepping takes the activation away only for a moment
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_PEEK_LOST, WPARAM(0), LPARAM(0)),
            WM_PEEK_LOST if GetActiveWindow() != h => _ = DestroyWindow(h),
            WM_DESTROY => {
                let from_find = FROM_FIND.load(Ordering::SeqCst);
                let i = app(|a| a.peek_pane);
                mark_find_peek(None);
                unload_handler();
                want_peek(None);
                reset_scroll();
                app(|a| a.peek = HWND::default());
                // Focus has left the result list. Ask the pane to hide those rows unless
                // close puts the keyboard back on them (or the search box still has it).
                if from_find {
                    let pane = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)).unwrap_or_default();
                    if !pane.0.is_null() {
                        _ = SetTimer(Some(pane), TIMER_FIND, 120, None);
                    }
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_newest_only() {
        let req = |p: &str| Some(PeekReq { path: p.into(), w: 10, h: 10, hwnd: 0, plain: false });
        let pix = || Peeked { body: Body::Pic(Pix { w: 1, h: 1, bgra: vec![0; 4] }), icon: -1, meta: Vec::new() };
        let card = || Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new() };
        let mut j = PeekJob { gen: 0, req: None, done: None };
        j.want(req("a"));
        let a = j.gen;
        j.want(req("b"));
        assert!(!j.publish(a, pix()) && j.done.is_none());
        assert!(j.publish(j.gen, card()) && matches!(j.done, Some(Peeked { body: Body::Card { .. }, .. })));
        let b = j.gen;
        j.want(None);
        assert!(j.gen != b && j.req.is_none() && j.done.is_none());
        assert!(!j.publish(b, pix()) && j.done.is_none());
    }

    #[test]
    fn peek_alpha() {
        let mut photo = [10u8, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut photo);
        assert_eq!(photo, [10u8, 20, 30, 255, 40, 50, 60, 255]);
        let mut icon = [0u8, 0, 0, 0, 40, 50, 60, 128];
        fix_alpha(&mut icon);
        assert_eq!(icon, [0u8, 0, 0, 0, 40, 50, 60, 128]);
    }

    #[test]
    fn icon_alpha() {
        let mut photo = vec![10, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut photo);
        assert_eq!(photo, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        let mut icon = vec![255, 255, 255, 128, 200, 100, 0, 255, 0, 0, 0, 0];
        fix_alpha(&mut icon);
        assert_eq!(icon, vec![128, 128, 128, 128, 200, 100, 0, 255, 0, 0, 0, 0]);
        let mut pre = vec![100, 100, 100, 128, 0, 0, 0, 0];
        fix_alpha(&mut pre);
        assert_eq!(pre, vec![100, 100, 100, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn markdown_lines() {
        let l = md_lines("# T\n\nsome **bold** `x`\n- a\n  - b\n> q\n```\ncode **\n```\n---");
        let styles: Vec<Style> = l.iter().map(|x| x.1).collect();
        use Style::*;
        assert_eq!(styles, vec![H1, Plain, Plain, Bullet, Bullet, Quote, Mono, Quote]);
        assert_eq!((l[2].0.as_str(), l[3].0.as_str(), l[4].0.as_str(), l[6].0.as_str()), ("some bold x", "• a", "    • b", "code **"));
    }

    #[test]
    fn peek_wheel() {
        use Wheel::*;
        assert_eq!(wheel_action(0, 0, true, false), Step);
        assert_eq!(wheel_action(0, 0, false, false), Step);
        assert_eq!(wheel_action(0, 500, true, false), Scroll);
        assert_eq!(wheel_action(0, 500, false, false), Hold);
        assert_eq!(wheel_action(0, 500, false, true), Step);
        assert_eq!(wheel_action(500, 500, true, false), Hold);
        assert_eq!(wheel_action(500, 500, true, true), Step);
        assert_eq!(wheel_action(500, 500, false, true), Scroll);
    }

    #[test]
    fn peek_layout() {
        let rc = RECT { left: 0, top: 0, right: 1120, bottom: 810 };
        let g = peek_geo(rc, 96);
        assert_eq!((g.stage.left, g.stage.top, g.stage.right, g.stage.bottom), (59, 51, 1061, 783));
        assert_eq!((g.inner.left, g.inner.top), (79, 71));
        assert!(g.close.right <= rc.right - 3 && g.close.top >= 3 && g.title.top >= 3 && g.title.right < g.close.left);
        assert!(g.prev.right <= g.stage.left && g.next.left >= g.stage.right);
        assert_eq!((g.prev.top + g.prev.bottom) / 2, (g.stage.top + g.stage.bottom) / 2);
        assert_eq!(peek_hit(&g, g.next.left + 4, g.next.top + 4), Hot::Next);
        assert_eq!(peek_hit(&g, g.prev.left + 4, g.prev.top + 4), Hot::Prev);
        assert_eq!(peek_hit(&g, g.close.left + 4, g.close.top + 4), Hot::Close);
        assert_eq!(peek_hit(&g, 500, 400), Hot::None);
    }

    #[test]
    fn peek_border_inset() {
        let rc = RECT { left: 0, top: 0, right: 1120, bottom: 810 };
        assert_eq!(peek_edge(1), 1);
        assert_eq!((peek_edge(96), peek_edge(144)), (3, 4));
        for dpi in [96, 144] {
            let e = peek_edge(dpi);
            let s = |v: i32| v * dpi / 96;
            let g = peek_geo(rc, dpi);
            assert_eq!(g.stage.left, e + s(56));
            assert_eq!(g.stage.top, e + s(48));
            assert_eq!(g.stage.right, rc.right - e - s(56));
            assert_eq!(g.stage.bottom, rc.bottom - e - s(24));
            assert_eq!((g.inner.left, g.inner.top, g.inner.right, g.inner.bottom), (g.stage.left + s(20), g.stage.top + s(20), g.stage.right - s(20), g.stage.bottom - s(20)));
            for r in [g.icon, g.title, g.close, g.stage, g.inner, g.prev, g.next] {
                assert!(r.left >= e && r.top >= e && r.right <= rc.right - e && r.bottom <= rc.bottom - e);
            }
        }
    }

    #[test]
    fn scroll_clamps_and_reaches_the_end() {
        assert_eq!(clamp_y(-4, 100), 0);
        assert_eq!(clamp_y(40, 100), 40);
        assert_eq!(clamp_y(400, 100), 100);
        assert_eq!(clamp_y(5, 0), 0);
        assert_eq!(clamp_y(5, -3), 0);

        assert_eq!(nudge(0, 500, 100), 100);
        assert_eq!(nudge(450, 500, 100), 500);
        assert_eq!(nudge(10, 500, -40), 0);
        assert_eq!(nudge(10, 200, i32::MAX), 200);
        assert_eq!(nudge(10, 200, i32::MIN), 0);
        // not laid out yet: a jump is kept for the paint to clamp
        assert_eq!(nudge(10, 0, i32::MAX), i32::MAX);
        assert_eq!(nudge(10, 0, i32::MIN), 0);

        let max_y = 10_000;
        let mut y = 0;
        let mut n = 0;
        while y < max_y {
            let next = nudge(y, max_y, 48);
            assert!(next > y && next <= max_y);
            y = next;
            n += 1;
            assert!(n < 1000);
        }
        assert_eq!(nudge(y, max_y, 48), max_y);
        assert_eq!(nudge(0, max_y, i32::MIN), 0);
    }

    #[test]
    fn arrow_scrolls_until_the_end() {
        use ArrowAct::*;
        assert_eq!(arrow_act(0, 0, true, false, 40), Step);
        assert_eq!(arrow_act(0, 0, true, true, 40), Step);
        assert_eq!(arrow_act(0, 0, false, true, 40), Step);
        assert_eq!(arrow_act(0, 500, true, false, 40), Scroll(40));
        assert_eq!(arrow_act(480, 500, true, false, 40), Scroll(500));
        assert_eq!(arrow_act(500, 500, true, false, 40), Step);
        assert_eq!(arrow_act(500, 500, true, true, 40), Stay);
        assert_eq!(arrow_act(0, 500, false, false, 40), Step);
        assert_eq!(arrow_act(0, 500, false, true, 40), Stay);
        assert_eq!(arrow_act(20, 500, false, false, 40), Scroll(0));
        assert_eq!(arrow_act(100, 500, false, true, 40), Scroll(60));
    }

    #[test]
    fn thumb_size_position_and_drag() {
        assert!(bar_layout(100, 100, 0, 200, 24, 0).is_none());
        assert!(bar_layout(50, 100, 0, 200, 24, 0).is_none());
        assert!(bar_layout(200, 0, 0, 200, 24, 0).is_none());
        assert!(bar_layout(200, 100, 0, 0, 24, 0).is_none());

        // content twice the view: thumb is half the track, ends meet the track ends
        let top = 10;
        let span = 200;
        let b = bar_layout(400, 200, top, span, 24, 0).unwrap();
        assert_eq!(b.max_y, 200);
        assert_eq!(b.thumb_h, 100);
        assert_eq!(b.thumb_top, top);
        let end = bar_layout(400, 200, top, span, 24, 200).unwrap();
        assert_eq!(end.thumb_top, top + span - end.thumb_h);
        assert_eq!(end.thumb_top + end.thumb_h, top + span);
        let mid = bar_layout(400, 200, top, span, 24, 100).unwrap();
        assert_eq!(mid.thumb_top, top + 50);
        let travel = span - b.thumb_h;
        assert_eq!(thumb_scroll(0, travel, b.max_y), 0);
        assert_eq!(thumb_scroll(travel, travel, b.max_y), b.max_y);
        assert_eq!(thumb_scroll(50, travel, b.max_y), 100);

        // y outside the range pins the thumb to the ends
        assert_eq!(bar_layout(400, 200, top, span, 24, -50).unwrap().thumb_top, top);
        assert_eq!(bar_layout(400, 200, top, span, 24, 9_000).unwrap().thumb_top, end.thumb_top);

        // a long document keeps the minimum thumb, and the bottom still lands on the track end
        let long = bar_layout(100_000, 100, 0, span, 24, 0).unwrap();
        assert_eq!(long.thumb_h, 24);
        let long_end = bar_layout(100_000, 100, 0, span, 24, long.max_y).unwrap();
        assert_eq!(long_end.thumb_top + long_end.thumb_h, span);
        assert_eq!(thumb_scroll(span - 24, span - 24, long.max_y), long.max_y);
        assert_eq!(thumb_len(100_000, 100, span, 24), 24);
        // min thumb larger than the track cannot overflow it
        assert_eq!(thumb_len(10_000, 100, 10, 24), 10);
        let tiny = bar_layout(10_000, 100, 5, 10, 24, 50).unwrap();
        assert_eq!((tiny.thumb_h, tiny.thumb_top), (10, 5));
        assert_eq!(thumb_scroll(0, 0, tiny.max_y), 0);

        let mut prev = -1;
        for y in [0, 1, 10, 100, 1000, 5000, 9999, 10_000] {
            let b = bar_layout(20_000, 100, 0, 400, 24, y).unwrap();
            assert!(b.thumb_top >= prev);
            assert!(b.thumb_top + b.thumb_h <= 400);
            assert!(b.thumb_h >= 24);
            prev = b.thumb_top;
        }
    }

    #[test]
    fn track_click_pages_to_the_end() {
        assert_eq!(page_toward(100, 500, 40, 20, 10, 80), 20);
        assert_eq!(page_toward(100, 500, 40, 20, 70, 80), 180);
        assert_eq!(page_toward(100, 500, 40, 20, 40, 80), 100);
        assert_eq!(page_toward(100, 500, 40, 20, 59, 80), 100);
        assert_eq!(page_toward(10, 500, 40, 20, 0, 80), 0);
        assert_eq!(page_toward(490, 500, 40, 20, 100, 80), 500);

        let max_y = 5_000;
        let page = 400;
        let mut y = 0;
        let mut n = 0;
        while y < max_y {
            let next = page_toward(y, max_y, 0, 10, 50, page);
            assert!(next > y);
            y = next;
            n += 1;
            assert!(n < 100);
        }
        assert_eq!(y, max_y);
        assert_eq!(page_toward(0, max_y, 0, 30, 10, page), 0);
    }

    #[test]
    fn bar_hit_and_gutter() {
        let hit = RECT { left: 0, top: 0, right: 20, bottom: 200 };
        let thumb = RECT { left: 4, top: 30, right: 12, bottom: 60 };
        assert_eq!(bar_hit(hit, thumb, 2, 40), BarHit::Thumb);
        assert_eq!(bar_hit(hit, thumb, 18, 59), BarHit::Thumb);
        assert_eq!(bar_hit(hit, thumb, 10, 10), BarHit::Track);
        assert_eq!(bar_hit(hit, thumb, 10, 60), BarHit::Track);
        assert_eq!(bar_hit(hit, thumb, 20, 40), BarHit::None);
        assert_eq!(bar_hit(hit, thumb, 10, 200), BarHit::None);

        for dpi in [96, 120, 144, 168, 192] {
            let rc = RECT { left: 0, top: 0, right: 1400, bottom: 900 };
            let g = peek_geo(rc, dpi);
            let b = peek_bar_rects(g.stage, dpi);
            assert!(b.paint.left >= g.stage.right, "dpi {dpi}");
            assert!(b.paint.right <= g.next.left, "dpi {dpi}");
            assert!(b.hit.right <= g.next.left, "dpi {dpi}");
            assert!(b.hit.left < b.paint.left, "dpi {dpi}");
            assert!(b.paint.top >= g.stage.top && b.paint.bottom <= g.stage.bottom, "dpi {dpi}");
            assert!(b.paint.right > b.paint.left && b.paint.bottom > b.paint.top, "dpi {dpi}");
            // ›'s own pixel is not part of the bar
            assert_eq!(bar_hit(b.hit, b.paint, g.next.left, g.next.top + 1), BarHit::None);
        }
    }

    #[test]
    fn bar_colors_read_on_both_themes() {
        for (bg, text, accent) in [(LIGHT.bg, LIGHT.text, 0x0067C0u32), (DARK.bg, DARK.text, 0x4CC2FFu32), (DARK.bg, DARK.text, 0x4B4B4Bu32)] {
            let (track, thumb) = bar_rgb(bg, text, accent, false);
            let (track_hot, thumb_hot) = bar_rgb(bg, text, accent, true);
            assert_ne!(track, bg);
            assert_ne!(thumb, bg);
            assert_ne!(thumb, track);
            assert_ne!(thumb_hot, thumb);
            assert_ne!(thumb_hot, bg);
            assert_eq!(track_hot, track);
        }
    }
}
