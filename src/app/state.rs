use std::ops::{Deref, DerefMut};

use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::session::*;

// ---------- app state ----------
// Single UI thread, one or more top-level windows. `app()` is the window bound to the
// message being handled. Rule: never call into Win32/COM that can re-enter our window procs while inside app().

static NEXT_WIN: AtomicUsize = AtomicUsize::new(1);

pub(crate) struct Pane {
    pub(crate) hwnd: HWND,
    pub(crate) eb: IExplorerBrowser,
    pub(crate) cookie: u32,
    pub(crate) disp: String,
    pub(crate) pending: Option<String>, // columns to put on when the next folder shows (session restore, open in)
    pub(crate) settle: Option<(String, u8)>, // columns just put on, and passes left to hold narrow widths (TIMER_SETTLE)
    pub(crate) recover: Option<(String, String)>, // missing folder, and the parent opened in its place
    pub(crate) icon: i32,   // current folder in the system image list, -1 = none yet
    pub(crate) ticket: u32, // that lookup; 0 = none asked
    /// Where this browser sits now. Swap updates it; the Explorer callbacks read it.
    pub(crate) slot: std::sync::Arc<AtomicUsize>,
}

pub(crate) struct Desk {
    #[allow(dead_code)] // stable per window; reserved for later window targeting
    pub(crate) id: u64,
    pub(crate) s: Session,
    pub(crate) main: HWND,
    pub(crate) tree: Option<INameSpaceTreeControl>,
    pub(crate) tree_hwnd: HWND,
    pub(crate) side_hwnd: HWND, // favourites list, bottom of the left panel
    pub(crate) side_hot: Option<usize>,
    pub(crate) side_sel: Option<usize>,
    pub(crate) side_scroll: i32,
    pub(crate) side_drag: Option<SideDrag>, // node being dragged
    pub(crate) side_drop: Option<(usize, u8)>, // visible row, drop zone
    pub(crate) side_press: Option<(i32, i32, usize)>,
    pub(crate) side_edit: HWND,
    pub(crate) side_edit_at: Vec<usize>,
    pub(crate) tree_input: bool, // a click/key reached the tree since its last selection change
    pub(crate) focus_keep: HWND, // focus when this window last went to the back
    pub(crate) click_nc: bool,   // the activating click was outside the client (title bar)
    pub(crate) focus_wait: bool, // TIMER_FOCUS is running
    pub(crate) font: HFONT,
    pub(crate) icon_font: HFONT,
    pub(crate) bold_font: HFONT, // favourite group labels; created on first paint, freed on DPI change and close
    pub(crate) group_brush: HBRUSH, // favourite group wash; rebuilt when the accent or theme changes
    pub(crate) group_brush_rgb: u32,
    pub(crate) fh: i32, // UI font line height in px
    pub(crate) dpi: i32,
    pub(crate) panes: Vec<Option<Pane>>,
    pub(crate) drag: Option<Hit>,
    pub(crate) gap_hot: Option<Hit>, // splitter under the pointer, drawn in the accent colour
    pub(crate) sizing: bool, // inside the window's own move/size loop
    pub(crate) was_max: bool, // maximized when last not minimized (kept while minimized)
    pub(crate) status: String,
    pub(crate) status_key: String,
    pub(crate) status_base: (String, Option<(u64, bool, bool)>, String), // text before / after the selection size, (file bytes, has folders, partial)
    pub(crate) view_menu: HMENU,
    pub(crate) base: Option<Session>, // last loaded/saved (or startup) state; differs -> green dot
    pub(crate) dirty: bool,
    pub(crate) small_font: HFONT,
    pub(crate) load: HWND, // load popup while open
    pub(crate) load_closed: Option<Instant>,
    pub(crate) load_hot: Option<usize>, // card index, or LOAD_SAVE_AS / LOAD_FOLDER / LOAD_DEL + card
    pub(crate) load_edit: HWND,         // rename box on a card; made on first use, hidden when done
    pub(crate) load_editing: Option<usize>, // card being renamed
    pub(crate) fav_popup: HWND,
    pub(crate) fav_closed: Option<Instant>,
    pub(crate) fav_hot: Option<(usize, bool)>, // (row, on its ✕)
    pub(crate) fav_prev: HWND,                 // focus before the popup opened; Esc goes back there
    pub(crate) keys_popup: HWND,               // shortcut window; null while closed
    pub(crate) keys_prev: HWND,                // focus to restore when that window is closed on purpose
    pub(crate) peek: HWND,                     // F3 quick look while open
    pub(crate) peek_pane: usize,               // pane whose focused item it shows
    pub(crate) peek_name: String,
    pub(crate) keys_scroll: i32,
    pub(crate) crumb_hot: Option<(HWND, usize)>, // pane, crumb under the pointer
    pub(crate) kids: Option<KidsAsk>,            // a '›' whose subfolder list has not come back yet
    pub(crate) lost: u32,                        // missing-folder notes merged while one is showing
    pub(crate) cards: Vec<(PathBuf, Session, String)>,
    pub(crate) ilist: Option<windows::Win32::UI::Controls::IImageList>, // system small icons, shared, never freed
    pub(crate) drives: Vec<(u8, u32, HWND)>, // tab-row buttons, A→Z: letter, drive type, button
    pub(crate) drive_mask: u32,              // GetLogicalDrives() they were built for
    pub(crate) drive_mark: Option<u8>,       // the active pane's drive, underlined
    pub(crate) hover: HWND, // owner-drawn button under the mouse
    pub(crate) tip: HWND,
    pub(crate) bars: [HWND; 2], // splitter guide bars (vertical, horizontal)
    pub(crate) grid: HWND,      // layout picker popup while open
    pub(crate) reveal_tries: u8, // sidebar reveal passes left
    pub(crate) grid_preview: Vec<usize>,       // layout shown in the picker
    pub(crate) grid_drag: Option<Vec<usize>>, // press-and-drag in progress
    pub(crate) grid_preset: Option<usize>,    // preset tile under the mouse
    pub(crate) grid_closed: Option<Instant>,
    pub(crate) zoom: Option<usize>, // pane maximized over the grid (runtime only, never saved)
    pub(crate) home: Homes,         // cell each off-screen slot last had, so a returning cell gets its pane back
    pub(crate) focus_req: HWND,     // pane to focus once, when the folder set_layout put in its place opens
    pub(crate) note: Option<(String, Instant)>, // status line confirmation and when it expires (runtime only)
    pub(crate) held: Option<u16>,               // key whose auto-repeat is swallowed until release
    pub(crate) match_last: String,              // last pattern typed into the select box
    pub(crate) matcher: Option<MatchBox>,
    pub(crate) drop_hover: HWND,                         // pane whose top bar something openable is being dragged over
    pub(crate) drop_open: Option<(HWND, usize, IShellItem)>, // folder dropped on a top bar, opened by WM_DROP_OPEN
    pub(crate) park: Vec<Vec<Option<Pane>>>,             // hidden browsers, parallel to s.tabs; empty once evicted
    pub(crate) park_lru: Vec<usize>,                     // parked tabs that still have browsers, oldest first
    pub(crate) save_since: Option<Instant>,              // first unsaved change in this debounce window
    pub(crate) closed: Vec<Tab>,                         // recently closed tabs, newest first (not saved)
    pub(crate) tabs_hwnd: HWND,
    pub(crate) tab_edit: HWND,
    pub(crate) tab_hot: Option<usize>,
    pub(crate) tab_hot_close: bool,
    pub(crate) tab_lock_tip: bool, // tracking tooltip for a locked tab's lock glyph is installed
    pub(crate) tab_hot_plus: bool,
    pub(crate) tab_hot_over: bool, // pointer over the overflow chevron
    pub(crate) tab_scroll: usize,  // first visible window tab while the strip overflows
    pub(crate) tab_drag: Option<usize>,
    pub(crate) tab_press: Option<(usize, i32, i32, bool)>, // tab, x, y, press landed on its close box
    pub(crate) tab_renaming: Option<usize>,
    pub(crate) tab_rename_skip: bool,
    /// Filter text last seen while this window was current. `FILTERS` holds the current window.
    pub(crate) live_filters: [String; MAXP],
}

/// The small pattern box over a file list (Num+ / Num−).
pub(crate) struct MatchBox {
    pub(crate) hwnd: HWND,
    pub(crate) edit: HWND,
    pub(crate) pane: usize,
    pub(crate) path: String,
    pub(crate) desel: bool,
}

/// A favourites row the user is dragging. Explorer rows are copies, not tree moves.
#[derive(Clone)]
pub(crate) struct SideDrag {
    pub(crate) id: Vec<usize>,
    pub(crate) kind: RowKind,
    pub(crate) group: bool,
    pub(crate) file: bool,
    pub(crate) path: String,
    pub(crate) name: String,
}

pub(crate) struct App {
    pub(crate) desks: Vec<Desk>,
    pub(crate) cur: usize,
    /// A failed grids.json write is tried again after this instant. Not mid-drag.
    pub(crate) save_after: Option<Instant>,
    pub(crate) hot_on: bool,    // the global hotkey is really registered (s.hotkey is only the wish)
    pub(crate) hot_hwnd: HWND,  // the window that owns that registration
    pub(crate) last_front: usize, // desk that was active most recently
}

impl Deref for App {
    type Target = Desk;
    fn deref(&self) -> &Desk {
        &self.desks[self.cur]
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut Desk {
        let i = self.cur;
        &mut self.desks[i]
    }
}

pub(crate) fn blank_desk() -> Desk {
    Desk {
        id: NEXT_WIN.fetch_add(1, Ordering::Relaxed) as u64,
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        side_hwnd: HWND::default(),
        side_hot: None,
        side_sel: None,
        side_scroll: 0,
        side_drag: None,
        side_drop: None,
        side_press: None,
        side_edit: HWND::default(),
        side_edit_at: Vec::new(),
        tree_input: false,
        focus_keep: HWND::default(),
        click_nc: false,
        focus_wait: false,
        font: HFONT::default(),
        icon_font: HFONT::default(),
        bold_font: HFONT::default(),
        group_brush: HBRUSH::default(),
        group_brush_rgb: 0,
        fh: 16,
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        gap_hot: None,
        sizing: false,
        was_max: false,
        status: String::new(),
        status_key: String::new(),
        status_base: Default::default(),
        view_menu: HMENU::default(),
        base: None,
        dirty: false,
        small_font: HFONT::default(),
        load: HWND::default(),
        load_closed: None,
        load_hot: None,
        load_edit: HWND::default(),
        load_editing: None,
        fav_popup: HWND::default(),
        fav_closed: None,
        fav_hot: None,
        fav_prev: HWND::default(),
        keys_popup: HWND::default(),
        keys_prev: HWND::default(),
        peek: HWND::default(),
        peek_pane: 0,
        peek_name: String::new(),
        keys_scroll: 0,
        crumb_hot: None,
        kids: None,
        lost: 0,
        cards: Vec::new(),
        ilist: None,
        drives: Vec::new(),
        drive_mask: 0,
        drive_mark: None,
        hover: HWND::default(),
        tip: HWND::default(),
        bars: [HWND::default(); 2],
        grid: HWND::default(),
        reveal_tries: 0,
        grid_preview: Vec::new(),
        grid_drag: None,
        grid_preset: None,
        grid_closed: None,
        zoom: None,
        home: [None; MAXP],
        focus_req: HWND::default(),
        note: None,
        held: None,
        match_last: String::new(),
        matcher: None,
        drop_hover: HWND::default(),
        drop_open: None,
        park: Vec::new(),
        park_lru: Vec::new(),
        save_since: None,
        closed: Vec::new(),
        tabs_hwnd: HWND::default(),
        tab_edit: HWND::default(),
        tab_hot: None,
        tab_hot_close: false,
        tab_lock_tip: false,
        tab_hot_plus: false,
        tab_hot_over: false,
        tab_scroll: 0,
        tab_drag: None,
        tab_press: None,
        tab_renaming: None,
        tab_rename_skip: false,
        live_filters: [const { String::new() }; MAXP],
    }
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App { desks: vec![blank_desk()], cur: 0, save_after: None, hot_on: false, hot_hwnd: HWND::default(), last_front: 0 });
}

pub(crate) fn no_panes() -> Vec<Option<Pane>> {
    (0..MAXP).map(|_| None).collect()
}

// Per-pane filter spec. Not in APP: the shell view may call IncludeObject from its own thread.
pub(crate) static FILTERS: Mutex<[String; MAXP]> = Mutex::new([const { String::new() }; MAXP]);

// Folder created by Ctrl+Shift+N, waiting for the view to list it so it can be selected and renamed.
pub(crate) struct PendingNew {
    pub(crate) pane: usize,
    pub(crate) name: String,
    pub(crate) left: u8,
}
pub(crate) static PENDING_NEW: Mutex<Option<PendingNew>> = Mutex::new(None);

/// "New folder", then "New folder (2)", "New folder (3)", … The first free name wins.
pub(crate) fn fresh_folder(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let name = format!("{base} ({n})");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

pub(crate) fn filter_of(i: usize) -> String {
    FILTERS.lock().map(|f| f[i].clone()).unwrap_or_default()
}

/// Shell-view hwnd → filter. IncludeObject can run off the UI thread, so this is not the current window's array.
pub(crate) static FILTER_VIEWS: Mutex<Vec<(isize, String)>> = Mutex::new(Vec::new());

pub(crate) fn note_filter_view(h: HWND, spec: &str) {
    if h.0.is_null() {
        return;
    }
    if let Ok(mut m) = FILTER_VIEWS.lock() {
        m.retain(|(k, _)| *k != h.0 as isize);
        if !spec.is_empty() {
            m.push((h.0 as isize, spec.to_string()));
        }
    }
}

pub(crate) fn clear_filter_view(h: HWND) {
    if h.0.is_null() {
        return;
    }
    if let Ok(mut m) = FILTER_VIEWS.lock() {
        m.retain(|(k, _)| *k != h.0 as isize);
    }
}

pub(crate) fn filter_for_view(h: HWND) -> Option<String> {
    if h.0.is_null() {
        return None;
    }
    FILTER_VIEWS.lock().ok().and_then(|m| m.iter().find(|(k, _)| *k == h.0 as isize).map(|(_, s)| s.clone()))
}

pub(crate) unsafe fn sync_filter_hwnds() {
    for i in 0..MAXP {
        let spec = filter_of(i);
        if let Some(sv) = view_of::<IShellView>(i) {
            if let Ok(h) = sv.GetWindow() {
                note_filter_view(h, &spec);
            }
        }
    }
}

pub(crate) fn window_count() -> usize {
    app(|a| a.desks.len())
}

/// Point `app()` at desk `i`, keeping each window's live filter text with that window.
pub(crate) fn bind_index(i: usize) {
    app(|a| {
        if a.desks.is_empty() || i >= a.desks.len() || i == a.cur {
            return;
        }
        if let Ok(mut f) = FILTERS.lock() {
            let prev = a.cur;
            a.desks[prev].live_filters = f.clone();
            *f = a.desks[i].live_filters.clone();
        }
        a.cur = i;
    });
}

fn desk_owns(d: &Desk, h: HWND) -> bool {
    if h.0.is_null() || d.main.0.is_null() {
        return false;
    }
    if h == d.main || h == d.tabs_hwnd || h == d.tab_edit || h == d.tree_hwnd || h == d.fav_popup || h == d.load || h == d.grid || h == d.tip || h == d.keys_popup || h == d.peek || d.matcher.as_ref().is_some_and(|m| m.hwnd == h || m.edit == h) || d.bars.contains(&h) {
        return true;
    }
    unsafe { IsChild(d.main, h).as_bool() }
}

/// Make the window that owns `h` current. A settings window is not a child and does not switch.
pub(crate) fn bind_hwnd(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let found = app(|a| {
        if a.desks.is_empty() {
            return None;
        }
        if a.cur < a.desks.len() && desk_owns(&a.desks[a.cur], h) {
            return Some(a.cur);
        }
        a.desks.iter().position(|d| desk_owns(d, h))
    });
    if let Some(i) = found {
        bind_index(i);
        true
    } else {
        false
    }
}

/// Drop the current desk. Loads the neighbour's filters. Fonts and the view menu go back to the caller.
pub(crate) fn drop_current_desk() -> (HFONT, HFONT, HFONT, HFONT, HMENU) {
    app(|a| {
        if a.desks.is_empty() {
            return (HFONT::default(), HFONT::default(), HFONT::default(), HFONT::default(), HMENU::default());
        }
        let i = a.cur.min(a.desks.len() - 1);
        let d = a.desks.remove(i);
        let brush = d.group_brush;
        if !brush.is_invalid() {
            unsafe { _ = DeleteObject(brush.into()) };
        }
        if a.desks.is_empty() {
            a.cur = 0;
        } else {
            if a.cur >= a.desks.len() {
                a.cur = a.desks.len() - 1;
            }
            if let Ok(mut f) = FILTERS.lock() {
                *f = a.desks[a.cur].live_filters.clone();
            }
        }
        (d.font, d.icon_font, d.small_font, d.bold_font, d.view_menu)
    })
}

pub(crate) unsafe fn release_desk_handles(font: HFONT, icon: HFONT, small: HFONT, bold: HFONT, menu: HMENU) {
    for f in [font, icon, small, bold] {
        if !f.0.is_null() {
            _ = DeleteObject(f.into());
        }
    }
    if !menu.0.is_null() {
        _ = DestroyMenu(menu);
    }
}

fn copy_shared(dst: &mut Session, src: &Session) {
    dst.accent = src.accent;
    dst.accent_dark = src.accent_dark;
    dst.dark = src.dark;
    dst.theme = src.theme;
    dst.font_face = src.font_face.clone();
    dst.bar = src.bar;
    dst.drive_buttons = src.drive_buttons;
    dst.stat = src.stat;
    dst.view_new = src.view_new;
    dst.start_new = src.start_new;
    dst.start_path = src.start_path.clone();
    dst.ext = src.ext;
    dst.tree_on = src.tree_on;
    dst.pins_open = src.pins_open;
    dst.files_open = src.files_open;
    dst.edge = src.edge;
    dst.confirm = src.confirm;
    dst.del = src.del;
    dst.new_name = src.new_name.clone();
    dst.dropm = src.dropm;
    dst.layout_new = src.layout_new;
    dst.boot = src.boot;
    dst.boot_file = src.boot_file.clone();
    dst.autosave = src.autosave;
    dst.dot = src.dot;
    dst.dblup = src.dblup;
    dst.hotkey = src.hotkey;
    dst.lang = src.lang;
    dst.single = src.single;
    dst.one_window = src.one_window;
    dst.favs = src.favs.clone();
    dst.tree = src.tree.clone();
    dst.recent = src.recent.clone();
}

/// Copy process-wide preferences from the current window onto the others.
pub(crate) fn publish_shared() {
    app(|a| {
        if a.desks.len() < 2 || a.cur >= a.desks.len() {
            return;
        }
        let src = a.desks[a.cur].s.clone();
        for (k, d) in a.desks.iter_mut().enumerate() {
            if k != a.cur {
                copy_shared(&mut d.s, &src);
            }
        }
    });
}

static ACROSS: AtomicBool = AtomicBool::new(false);

struct AcrossGuard;
impl Drop for AcrossGuard {
    fn drop(&mut self) {
        ACROSS.store(false, Ordering::Relaxed);
    }
}

/// Run `f` once per window, on that window. A nested call runs only for the window already current.
pub(crate) unsafe fn across_windows(mut f: impl FnMut()) {
    if ACROSS.swap(true, Ordering::Relaxed) {
        f();
        return;
    }
    let _guard = AcrossGuard;
    let (n, cur) = app(|a| (a.desks.len(), a.cur));
    for i in 0..n {
        bind_index(i);
        f();
    }
    if cur < app(|a| a.desks.len()) {
        bind_index(cur);
    }
}

pub(crate) fn app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

/// Pane that commands act on. A tab strip would return the active tab's pane from here.
pub(crate) fn active_pane() -> usize {
    app(|a| a.s.active)
}

pub(crate) fn eb_of(i: usize) -> Option<IExplorerBrowser> {
    app(|a| a.panes.get(i)?.as_ref().map(|p| p.eb.clone()))
}

pub(crate) unsafe fn view_of<T: Interface>(i: usize) -> Option<T> {
    eb_of(i)?.GetCurrentView::<T>().ok()
}

pub(crate) fn pane_of(h: HWND) -> Option<usize> {
    app(|a| {
        a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h || unsafe { IsChild(p.hwnd, h).as_bool() }))
    })
}

pub(crate) fn pane_ctl(i: usize, id: i32) -> Option<HWND> {
    let h = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd))?;
    unsafe { GetDlgItem(Some(h), id).ok() }
}

pub(crate) unsafe fn text_of(h: HWND) -> String {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..n])
}

// scroll an unfocused, untouched address bar to its end, so a cut-off path shows the current folder
pub(crate) unsafe fn show_path_end(e: HWND, disp: &str) {
    let (mut s, mut en) = (0u32, 0u32);
    SendMessageW(e, EM_GETSEL, Some(WPARAM(&mut s as *mut u32 as usize)), Some(LPARAM(&mut en as *mut u32 as isize)));
    if GetFocus() == e || s != en || text_of(e) != disp {
        return;
    }
    let len = GetWindowTextLengthW(e) as usize;
    // back to the start first: drops any scroll left from when the box was 0 px wide
    for at in [0, len] {
        SendMessageW(e, EM_SETSEL, Some(WPARAM(at)), Some(LPARAM(at as isize)));
        SendMessageW(e, EM_SCROLLCARET, None, None);
    }
}

