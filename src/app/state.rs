use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::session::*;

// ---------- app state ----------
// Single UI thread. Rule: never call into Win32/COM that can re-enter our window procs while inside app().

pub(crate) struct Pane {
    pub(crate) hwnd: HWND,
    pub(crate) eb: IExplorerBrowser,
    pub(crate) cookie: u32,
    pub(crate) disp: String,
    pub(crate) pending: Option<String>, // columns to put on when the next folder shows (session restore, open in)
    pub(crate) settle: Option<(String, u8)>, // columns just put on, and passes left to hold narrow widths (TIMER_SETTLE)
}

pub(crate) struct App {
    pub(crate) s: Session,
    pub(crate) main: HWND,
    pub(crate) tree: Option<INameSpaceTreeControl>,
    pub(crate) tree_hwnd: HWND,
    pub(crate) tree_input: bool, // a click/key reached the tree since its last selection change
    pub(crate) font: HFONT,
    pub(crate) icon_font: HFONT,
    pub(crate) fh: i32, // UI font line height in px
    pub(crate) dpi: i32,
    pub(crate) panes: Vec<Option<Pane>>,
    pub(crate) drag: Option<Hit>,
    pub(crate) sizing: bool, // inside the window's own move/size loop
    pub(crate) status: String,
    pub(crate) status_key: String,
    pub(crate) status_base: (String, Option<(u64, bool, bool)>, String), // text before / after the selection size, (file bytes, has folders, partial)
    pub(crate) view_menu: HMENU,
    pub(crate) base: Option<Session>, // last loaded/saved (or startup) state; differs -> green dot
    pub(crate) dirty: bool,
    pub(crate) small_font: HFONT,
    pub(crate) load: HWND, // load popup while open
    pub(crate) load_closed: Option<Instant>,
    pub(crate) load_hot: Option<usize>, // card index, or LOAD_SAVE_AS / LOAD_FOLDER
    pub(crate) fav_popup: HWND,
    pub(crate) fav_closed: Option<Instant>,
    pub(crate) fav_hot: Option<(usize, bool)>, // (row, on its ✕)
    pub(crate) fav_prev: HWND,                 // focus before the popup opened; Esc goes back there
    pub(crate) cards: Vec<(PathBuf, Session, String)>,
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
    pub(crate) note: Option<(String, Instant)>, // status line confirmation and when it was set (runtime only)
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
    pub(crate) tab_hot_plus: bool,
    pub(crate) tab_drag: Option<usize>,
    pub(crate) tab_press: Option<(usize, i32, i32, bool)>, // tab, x, y, press landed on its close box
    pub(crate) tab_renaming: Option<usize>,
    pub(crate) tab_rename_skip: bool,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        tree_input: false,
        font: HFONT::default(),
        icon_font: HFONT::default(),
        fh: 16,
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        sizing: false,
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
        fav_popup: HWND::default(),
        fav_closed: None,
        fav_hot: None,
        fav_prev: HWND::default(),
        cards: Vec::new(),
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
        note: None,
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
        tab_hot_plus: false,
        tab_drag: None,
        tab_press: None,
        tab_renaming: None,
        tab_rename_skip: false,
    });
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

