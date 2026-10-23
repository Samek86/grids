// Settings file: %APPDATA%\grids\grids.json, or next to the exe when portable.txt is there.
// Named .grids sessions stay in the ini shape. A leftover grids.ini is read once and renamed to grids.ini.bak.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use windows::core::HSTRING;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::{fav_path, fix, Lang, Session, MAXP, RECENT};

fn v1() -> u32 {
    1
}
fn yes() -> bool {
    true
}

fn half_split() -> f64 {
    0.5
}
fn pins_open() -> bool {
    true
}
fn pins_are_open(v: &bool) -> bool {
    *v
}
fn is_on(v: &bool) -> bool {
    *v
}
fn is_off(v: &bool) -> bool {
    !*v
}
fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}
fn files_open() -> bool {
    true
}
fn files_are_open(v: &bool) -> bool {
    *v
}
fn is_half_split(v: &f64) -> bool {
    (v - 0.5).abs() < 1e-6
}
fn accent_light() -> String {
    "0067C0".into()
}
fn accent_dark() -> String {
    "4CC2FF".into()
}
fn theme_auto() -> String {
    "auto".into()
}
fn lang_auto() -> String {
    "auto".into()
}
fn view_details() -> i32 {
    4
}
fn two_by_two() -> Vec<usize> {
    vec![2, 2]
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct WinJson {
    #[serde(default)]
    left: i32,
    #[serde(default)]
    top: i32,
    #[serde(default)]
    right: i32,
    #[serde(default)]
    bottom: i32,
    #[serde(default)]
    maximized: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct LayoutJson {
    #[serde(default = "two_by_two")]
    columns: Vec<usize>,
    #[serde(default)]
    colr: Vec<f64>,
    #[serde(default)]
    rowr: Vec<Vec<f64>>,
    #[serde(default)]
    active: usize,
}

impl Default for LayoutJson {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or(LayoutJson { columns: two_by_two(), colr: Vec::new(), rowr: Vec::new(), active: 0 })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct PaneJson {
    #[serde(default)]
    index: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    view: Vec<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    columns: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    filter: String,
    /// Pane tabs: folders this pane switches between. The current one is also `path`. Empty means one folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    folder: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct FavJson {
    path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
}

/// Version 1. `tabs` is the focused window. `windows` is every window, focused first.
/// `favorites_tree` is the favourites tree. Unknown keys land in `extra` and are written back.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SettingsFile {
    #[serde(default = "v1")]
    version: u32,
    #[serde(default)]
    window: Option<WinJson>,
    #[serde(default)]
    side: i32,
    #[serde(default)]
    layout: LayoutJson,
    /// Permutation of pane slots since the named session was saved. Empty means they match the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    order: Vec<u8>,
    #[serde(default)]
    panes: Vec<PaneJson>,
    #[serde(default = "accent_light")]
    accent: String,
    #[serde(default = "accent_dark")]
    accent_dark: String,
    #[serde(default)]
    dark: bool,
    #[serde(default = "theme_auto")]
    theme: String,
    #[serde(default)]
    font: u8,
    /// UI face. Missing or empty uses the language default. Not a size (that is `font`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    font_face: String,
    #[serde(default = "yes")]
    bar: bool,
    #[serde(default = "yes")]
    stat: bool,
    #[serde(default = "view_details")]
    view_new: i32,
    #[serde(default)]
    start: u8,
    #[serde(default)]
    start_path: String,
    #[serde(default = "yes")]
    ext: bool,
    #[serde(default = "yes")]
    tree: bool,
    /// Retired activity-bar choice ("tree" / "fav"). Read so it is not kept, never written.
    #[serde(default, skip_serializing)]
    #[allow(dead_code)]
    panel: String,
    /// Folder tree's share of the left panel. Favourites use the rest. Missing means even.
    #[serde(default = "half_split", skip_serializing_if = "is_half_split")]
    side_split: f64,
    /// Explorer pins group expanded. The pinned folders themselves are not stored.
    #[serde(default = "pins_open", skip_serializing_if = "pins_are_open")]
    explorer_pins_open: bool,
    /// Explorer favourite-files group expanded. The files themselves are not stored.
    /// Omitted when open, which is the default. The group is hidden when Explorer has no such files.
    #[serde(default = "files_open", skip_serializing_if = "files_are_open")]
    explorer_files_open: bool,
    #[serde(default = "yes")]
    edge: bool,
    #[serde(default = "yes")]
    confirm: bool,
    #[serde(default)]
    del: u8,
    #[serde(default)]
    new_name: String,
    #[serde(default)]
    drop: u8,
    #[serde(default)]
    boot: u8,
    #[serde(default)]
    boot_file: String,
    #[serde(default)]
    autosave: bool,
    #[serde(default = "yes")]
    dot: bool,
    #[serde(default = "yes")]
    dblup: bool,
    /// Pane-tab strips. Missing means off. Separate from `tabs`, which are window tabs.
    #[serde(default, skip_serializing_if = "is_off")]
    pane_tabs: bool,
    /// Global hotkey. Missing means on, so an older grids.json keeps trying Win+Alt+E.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    hotkey: bool,
    #[serde(default = "lang_auto")]
    lang: String,
    #[serde(default = "yes")]
    single: bool,
    /// One window. Missing means off, so an older grids.json still opens windows.
    #[serde(default, skip_serializing_if = "is_off")]
    one_window: bool,
    /// Subfolder search uses Everything when it is running. Missing means on.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    use_everything: bool,
    #[serde(default)]
    session: String,
    #[serde(default)]
    favorites: Vec<FavJson>,
    #[serde(default)]
    recent: Vec<String>,
    #[serde(default)]
    tabs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    windows: Vec<Value>,
    #[serde(default)]
    keybindings: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    favorites_tree: Vec<Value>,
    #[serde(default, flatten)]
    extra: Map<String, Value>,
}

impl Default for SettingsFile {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or_else(|_| SettingsFile::blank())
    }
}

impl SettingsFile {
    fn blank() -> Self {
        SettingsFile {
            version: 1,
            window: None,
            side: 0,
            layout: LayoutJson::default(),
            order: Vec::new(),
            panes: Vec::new(),
            accent: accent_light(),
            accent_dark: accent_dark(),
            dark: false,
            theme: theme_auto(),
            font: 0,
            font_face: String::new(),
            bar: true,
            stat: true,
            view_new: 4,
            start: 0,
            start_path: String::new(),
            ext: true,
            tree: true,
            panel: String::new(),
            side_split: 0.5,
            explorer_pins_open: true,
            explorer_files_open: true,
            edge: true,
            confirm: true,
            del: 0,
            new_name: String::new(),
            drop: 0,
            boot: 0,
            boot_file: String::new(),
            autosave: false,
            dot: true,
            dblup: true,
            pane_tabs: false,
            hotkey: true,
            lang: lang_auto(),
            single: true,
            one_window: false,
            use_everything: true,
            session: String::new(),
            favorites: Vec::new(),
            recent: Vec::new(),
            tabs: Vec::new(),
            windows: Vec::new(),
            keybindings: Map::new(),
            favorites_tree: Vec::new(),
            extra: Map::new(),
        }
    }

    fn from_session(s: &Session) -> Self {
        let shown = s.view_tabs();
        let at = s.tab.min(shown.len().saturating_sub(1));
        let filters = shown.get(at).map(|t| t.filters.clone()).unwrap_or_default();
        let mut panes = Vec::new();
        for i in 0..MAXP {
            let filter = filters.get(i).cloned().unwrap_or_default();
            let folders = s.pane_tab_list(i);
            let folders = if folders.len() > 1 { folders } else { Vec::new() };
            if s.paths[i].is_empty() && s.views[i] == (0, 0) && s.cols[i].is_empty() && filter.is_empty() && folders.is_empty() {
                continue;
            }
            panes.push(PaneJson {
                index: i,
                path: s.paths[i].clone(),
                view: if s.views[i].0 == 0 { Vec::new() } else { vec![s.views[i].0, s.views[i].1] },
                columns: s.cols[i].clone(),
                filter,
                folders: folders.clone(),
                folder: if folders.len() > 1 { s.pane_cur[i].min(folders.len() - 1) } else { 0 },
            });
        }
        let tree = if s.tree.is_empty() { crate::tree_from_flat(&s.favs) } else { s.tree.clone() };
        let flat = if s.tree.is_empty() { s.favs.clone() } else { crate::flat_favs(&s.tree) };
        let favorites = flat.iter().map(|f| {
            let path = fav_path(f).to_string();
            let name = f.split_once('\t').map(|(_, n)| n.to_string()).unwrap_or_default();
            FavJson { path, name }
        }).collect();
        SettingsFile {
            version: 1,
            window: s.win.map(|w| WinJson { left: w[0], top: w[1], right: w[2], bottom: w[3], maximized: w[4] != 0 }),
            side: s.side,
            layout: LayoutJson { columns: s.heights.clone(), colr: s.colr.clone(), rowr: s.rowr.clone(), active: s.active },
            order: s.order.clone(),
            panes,
            accent: format!("{:06X}", s.accent & 0xFFFFFF),
            accent_dark: format!("{:06X}", s.accent_dark & 0xFFFFFF),
            dark: s.dark,
            theme: ["auto", "light", "dark"][s.theme.min(2) as usize].into(),
            font: s.font.min(2),
            font_face: s.font_face.trim().to_string(),
            bar: s.bar,
            stat: s.stat,
            view_new: s.view_new,
            start: s.start_new.min(2),
            start_path: s.start_path.clone(),
            ext: s.ext,
            tree: s.tree_on,
            panel: String::new(),
            side_split: crate::sane_split(s.side_split),
            explorer_pins_open: s.pins_open,
            explorer_files_open: s.files_open,
            edge: s.edge,
            confirm: s.confirm,
            del: s.del,
            new_name: s.new_name.clone(),
            drop: s.dropm.min(2),
            boot: s.boot.min(2),
            boot_file: s.boot_file.clone(),
            autosave: s.autosave,
            dot: s.dot,
            dblup: s.dblup,
            pane_tabs: s.pane_tabs_on,
            hotkey: s.hotkey,
            lang: s.lang.ini().into(),
            single: s.single,
            one_window: s.one_window,
            use_everything: s.use_everything,
            session: s.file.clone(),
            favorites,
            recent: s.recent.clone(),
            tabs: serde_json::from_str(&crate::tabs_json(&shown, at)).unwrap_or_default(),
            windows: crate::take_snaps(),
            keybindings: crate::keys::bindings_json(),
            favorites_tree: crate::tree_to_values(&tree),
            extra: Map::new(),
        }
    }

    /// Keep a newer file's version and any keys this build does not edit.
    /// `tabs` is this build's: the session just wrote it.
    fn merge_forward(&mut self, previous: &SettingsFile) {
        if previous.version >= 1 {
            self.version = previous.version;
        }
        // Command ids come from the live keymap. Anything else (a future build's keys) stays.
        let mut kb = self.keybindings.clone();
        for (k, v) in &previous.keybindings {
            if crate::keys::is_command(k) {
                continue;
            }
            kb.entry(k.clone()).or_insert_with(|| v.clone());
        }
        self.keybindings = kb;
        // An empty list means this save did not see live windows (a test, or a build that
        // does not write them). A real save always has the window that is still open.
        if self.windows.is_empty() {
            self.windows = previous.windows.clone();
        }
        self.extra = previous.extra.clone();
        self.extra.remove("panel");
        self.extra.remove("one_window");
        self.extra.remove("use_everything");
    }

    fn to_session(&self) -> Session {
        let mut s = Session::default();
        if let Some(w) = &self.window {
            s.win = Some([w.left, w.top, w.right, w.bottom, w.maximized as i32]);
        }
        s.side = self.side;
        let heights: Vec<usize> = self.layout.columns.iter().take(4).copied().map(|n| n.clamp(1, 4)).collect();
        s.heights = if heights.is_empty() { vec![2, 2] } else { heights };
        s.colr = fix(self.layout.colr.clone(), s.heights.len());
        let rowr = &self.layout.rowr;
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        s.order = if self.order.len() == MAXP && self.order.iter().all(|&n| (n as usize) < MAXP) { self.order.clone() } else { Vec::new() };
        s.active = self.layout.active;
        if s.active >= s.panes() {
            s.active = 0;
        }
        s.paths = vec![String::new(); MAXP];
        s.views = vec![(0, 0); MAXP];
        s.cols = vec![String::new(); MAXP];
        for p in &self.panes {
            if p.index >= MAXP {
                continue;
            }
            s.paths[p.index] = p.path.clone();
            if p.view.len() >= 2 {
                s.views[p.index] = (p.view[0], p.view[1]);
            } else if let Some(m) = p.view.first() {
                s.views[p.index] = (*m, 0);
            }
            s.cols[p.index] = p.columns.clone();
            if p.folders.len() > 1 {
                s.pane_cur[p.index] = p.folder.min(p.folders.len() - 1);
                s.pane_folders[p.index] = p.folders.clone();
            }
        }
        let mut filters = vec![String::new(); MAXP];
        for p in &self.panes {
            if p.index < MAXP {
                filters[p.index] = p.filter.clone();
            }
        }
        s.accent = parse_color(&self.accent).unwrap_or(s.accent);
        s.accent_dark = parse_color(&self.accent_dark).unwrap_or(s.accent_dark);
        s.dark = self.dark;
        s.theme = match self.theme.as_str() {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        };
        s.font = self.font.min(2);
        s.font_face = self.font_face.trim().to_string();
        s.bar = self.bar;
        s.stat = self.stat;
        s.view_new = match self.view_new {
            3 => 3,
            1 => 1,
            _ => 4,
        };
        s.start_new = self.start.min(2);
        s.start_path = self.start_path.clone();
        s.ext = self.ext;
        s.tree_on = self.tree;
        s.side_split = crate::sane_split(self.side_split);
        s.pins_open = self.explorer_pins_open;
        s.files_open = self.explorer_files_open;
        s.edge = self.edge;
        s.confirm = self.confirm;
        s.del = if self.del == 1 { 1 } else { 0 };
        s.new_name = self.new_name.clone();
        s.dropm = self.drop.min(2);
        s.boot = self.boot.min(2);
        s.boot_file = self.boot_file.clone();
        s.autosave = self.autosave;
        s.dot = self.dot;
        s.dblup = self.dblup;
        s.pane_tabs_on = self.pane_tabs;
        s.hotkey = self.hotkey;
        s.lang = Lang::from_ini(&self.lang);
        s.single = self.single;
        s.one_window = self.one_window;
        s.use_everything = self.use_everything;
        s.file = self.session.clone();
        let parsed_tree = crate::tree_from_values(&self.favorites_tree);
        if !parsed_tree.is_empty() {
            s.tree = parsed_tree;
            s.favs = crate::flat_favs(&s.tree);
        } else {
            s.favs = self.favorites.iter().filter(|f| !f.path.trim().is_empty()).map(|f| {
                if f.name.trim().is_empty() { f.path.trim().to_string() } else { format!("{}\t{}", f.path.trim(), f.name.trim()) }
            }).collect();
            s.tree = crate::tree_from_flat(&s.favs);
        }
        s.recent = self.recent.iter().filter(|r| !r.trim().is_empty()).take(RECENT).cloned().collect();
        if self.tabs.is_empty() {
            s.tabs.clear();
            s.tab = 0;
            s.anchor();
            if let Some(t) = s.tabs.get_mut(0) {
                t.filters = filters;
            }
        } else {
            crate::adopt_tabs(&mut s, &self.tabs);
        }
        if self.windows.is_empty() {
            crate::stash_windows(Vec::new());
            s
        } else {
            let first = crate::session_from_window(self.windows.first().unwrap(), &s);
            crate::stash_windows(self.windows.iter().skip(1).cloned().collect());
            first
        }
    }
}

/// Drop `font_face` when that family is not installed. The pure decision is `resolve_face`.
fn forget_missing_face(s: &mut Session) {
    if s.font_face.is_empty() {
        return;
    }
    let ok = unsafe { crate::face_installed(&s.font_face) };
    let only = [s.font_face.as_str()];
    let resolved = crate::resolve_face(&s.font_face, s.lang.resolve(), if ok { &only[..] } else { &[] });
    if ok && resolved == s.font_face {
        return;
    }
    s.font_face.clear();
    SETTINGS_KEEP.with(|k| k.borrow_mut().font_face.clear());
}

fn parse_color(v: &str) -> Option<u32> {
    let t = v.trim().trim_start_matches('#');
    if t.is_empty() {
        return None;
    }
    u32::from_str_radix(t, 16).ok().map(|n| n & 0xFFFFFF)
}

fn pretty(file: &SettingsFile) -> String {
    let mut s = serde_json::to_string_pretty(file).unwrap_or_else(|_| "{}".into());
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

thread_local! {
    static SETTINGS_KEEP: RefCell<SettingsFile> = RefCell::new(SettingsFile::default());
}

fn remember(file: SettingsFile) {
    SETTINGS_KEEP.with(|k| *k.borrow_mut() = file);
}

fn strip_bom(text: &str) -> &str {
    let text = text.trim_start();
    text.strip_prefix('\u{FEFF}').unwrap_or(text).trim_start()
}

fn adopt(text: &str) -> Option<Session> {
    let text = strip_bom(text);
    let file: SettingsFile = serde_json::from_str(text).ok()?;
    let s = file.to_session();
    crate::keys::load_bindings(&file.keybindings);
    remember(file);
    Some(s)
}

/// Serialize `s`, keeping version and unknown fields from the file that was loaded.
pub(crate) fn sync_text(s: &Session) -> String {
    let mut file = SettingsFile::from_session(s);
    let prev = SETTINGS_KEEP.with(|k| k.borrow().clone());
    file.merge_forward(&prev);
    let text = pretty(&file);
    remember(file);
    text
}

pub(crate) fn current_text() -> String {
    SETTINGS_KEEP.with(|k| pretty(&k.borrow()))
}

/// A leading `{` is JSON. Anything else is a legacy ini. Invalid JSON is refused.
pub(crate) fn import_text(text: &str) -> Result<Session, ()> {
    let text = strip_bom(text);
    if text.starts_with('{') {
        adopt(text).ok_or(())
    } else {
        let s = Session::parse(text);
        remember(SettingsFile::from_session(&s));
        Ok(s)
    }
}

pub(crate) fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn portable_mode() -> bool {
    exe_dir().join("portable.txt").is_file()
}

fn roaming_appdata() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as _));
        if path.is_empty() { None } else { Some(PathBuf::from(path)) }
    }
}

pub(crate) fn config_dir() -> PathBuf {
    if portable_mode() {
        return exe_dir();
    }
    match roaming_appdata() {
        Some(p) => p.join("grids"),
        None => std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(exe_dir).join("grids"),
    }
}

pub(crate) fn settings_path() -> PathBuf {
    config_dir().join("grids.json")
}

fn legacy_candidates(exe: &Path, cfg: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for p in [exe.join("grids.ini"), cfg.join("grids.ini"), exe.join("qdir.ini")] {
        if !v.iter().any(|e| e == &p) {
            v.push(p);
        }
    }
    v
}

fn bak_path(ini: &Path) -> PathBuf {
    ini.with_extension("ini.bak")
}

/// grids.json bytes. Missing is `Ok(None)`. A locked or unreadable file is an error, never defaults.
pub(crate) fn read_settings_file(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Load grids.json, or migrate a legacy ini. An existing file that cannot be read or parsed is an error,
/// so a later save does not replace it with defaults.
pub(crate) fn try_load_at(exe: &Path, cfg: &Path) -> Result<Session, String> {
    let _ = std::fs::create_dir_all(cfg);
    let json_path = cfg.join("grids.json");
    match read_settings_file(&json_path)? {
        Some(raw) => {
            let text = strip_bom(&raw);
            let Some(mut s) = adopt(text) else {
                return Err("grids.json is not valid settings".into());
            };
            // A name that is not installed falls back to the language default. Empty skips the lookup.
            forget_missing_face(&mut s);
            // The activity-bar choice is retired. Drop it once so the next launch does not see it.
            let had_panel = serde_json::from_str::<Value>(text).ok().is_some_and(|v| v.get("panel").is_some());
            if had_panel {
                let migrated = current_text();
                if migrated != text {
                    let _ = write_atomic(&json_path, migrated.as_bytes());
                }
            }
            Ok(s)
        }
        None => Ok(load_legacy(exe, cfg)),
    }
}

fn load_legacy(exe: &Path, cfg: &Path) -> Session {
    let json_path = cfg.join("grids.json");
    for cand in legacy_candidates(exe, cfg) {
        let Ok(text) = std::fs::read_to_string(&cand) else { continue };
        let s = Session::parse(&text);
        let file = SettingsFile::from_session(&s);
        if write_atomic(&json_path, pretty(&file).as_bytes()).is_ok() {
            remember(file);
            let bak = bak_path(&cand);
            if bak.exists() {
                let _ = std::fs::remove_file(&bak);
            }
            let _ = std::fs::rename(&cand, bak);
            return s;
        }
    }
    let s = Session::default();
    remember(SettingsFile::from_session(&s));
    s
}

/// Same as `try_load_at` for tests that only see a readable file.
#[cfg(test)]
pub(crate) fn load_at(exe: &Path, cfg: &Path) -> Session {
    try_load_at(exe, cfg).unwrap_or_else(|_| {
        remember(SettingsFile::default());
        Session::default()
    })
}

pub(crate) fn load() -> Result<Session, String> {
    let cfg = config_dir();
    let _ = std::fs::create_dir_all(&cfg);
    try_load_at(&exe_dir(), &cfg)
}

pub(crate) fn write_settings(text: &str) -> std::io::Result<()> {
    write_atomic(&settings_path(), text.as_bytes())
}

/// Write `bytes` next to `path` and swap it in. A failed swap leaves the previous file in place.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if path.is_dir() {
        return Err(std::io::Error::other("settings path is a directory"));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp = path.to_path_buf();
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "grids.json".into());
    tmp.set_file_name(format!("{name}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let src = HSTRING::from(tmp.to_string_lossy().as_ref());
    let dst = HSTRING::from(path.to_string_lossy().as_ref());
    let moved = unsafe { MoveFileExW(&src, &dst, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
    if moved.is_ok() {
        return Ok(());
    }
    // Replace by renaming aside, then back if the new file does not land.
    let bak = tmp.with_extension("bak");
    if path.exists() && std::fs::rename(path, &bak).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(std::io::Error::other("could not replace settings"));
    }
    if std::fs::rename(&tmp, path).is_ok() {
        let _ = std::fs::remove_file(&bak);
        return Ok(());
    }
    if bak.exists() {
        let _ = std::fs::rename(&bak, path);
    }
    let _ = std::fs::remove_file(&tmp);
    Err(std::io::Error::other("could not replace settings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("grids-json-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn sample() -> Session {
        let mut s = Session::default();
        s.win = Some([10, 20, 900, 700, 1]);
        s.side = 260;
        s.set_heights(vec![1, 3]);
        s.colr = vec![0.25, 0.75];
        s.rowr = vec![vec![1.0], vec![0.5, 0.25, 0.25]];
        s.active = 2;
        s.paths[1] = "C:\\a=b\\폴더".into();
        s.views[1] = (4, 16);
        s.cols[1] = "{guid}/10=220;{guid}/12=-1".into();
        s.accent = 0xA8624A;
        s.accent_dark = 0x4CC2FF;
        s.dark = true;
        s.dblup = false;
        s.file = "C:\\s\\work.grids".into();
        s.favs = vec!["C:\\WORK\tWork".into(), "D:\\Photos".into(), "E:\\\tLater".into()];
        s.recent = vec!["D:\\Photos".into(), "C:\\WORK".into()];
        s.lang = Lang::Ko;
        s.single = false;
        s.theme = 0;
        s.font = 2;
        s.bar = false;
        s.stat = false;
        s.view_new = 1;
        s.start_new = 2;
        s.start_path = "C:\\Work".into();
        s.ext = false;
        s.tree_on = false;
        s.edge = false;
        s.confirm = false;
        s.del = 1;
        s.new_name = "note.txt".into();
        s.dropm = 2;
        s.boot = 1;
        s.boot_file = "C:\\s.grids".into();
        s.autosave = true;
        s.dot = false;
        s
    }

    #[test]
    fn json_round_trip_keeps_settings_and_forward_fields() {
        let parsed = Session::parse(&sample().to_ini());
        let text = pretty(&SettingsFile::from_session(&parsed));
        let back = adopt(&text).unwrap();
        assert_eq!(back, parsed);
        assert_eq!(adopt("{}").unwrap(), Session::default());

        let mut file = SettingsFile::from_session(&parsed);
        file.version = 2;
        file.tabs = vec![serde_json::json!({"id": "later"})];
        file.keybindings.insert("copy".into(), serde_json::json!("Ctrl+C"));
        file.favorites_tree = vec![serde_json::json!("group")];
        file.extra.insert("future".into(), serde_json::json!({"keep": true}));
        let loaded = adopt(&pretty(&file)).unwrap();
        assert_eq!(loaded, parsed);
        let saved: Value = serde_json::from_str(&sync_text(&loaded)).unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["tabs"][0]["name"], "1");
        assert_eq!(saved["tabs"][0]["locked"], false);
        assert_eq!(saved["tabs"][0]["layout"]["columns"][0], 1);
        assert_eq!(saved["tabs"][0]["layout"]["columns"][1], 3);
        assert!(saved["tabs"][0]["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\a=b\\폴더"));
        assert_eq!(saved["keybindings"]["copy"], "Ctrl+C");
        assert_eq!(saved["favorites_tree"][0]["path"], "C:\\WORK");
        assert_eq!(saved["favorites_tree"][0]["name"], "Work");
        assert_eq!(saved["future"]["keep"], true);
        assert_eq!(saved["lang"], "ko");
        assert_eq!(saved["favorites"][0]["name"], "Work");
        assert_eq!(saved["favorites"][1]["path"], "D:\\Photos");
        assert!(saved["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\a=b\\폴더" && p["columns"] == "{guid}/10=220;{guid}/12=-1"));

        let ini = import_text(&parsed.to_ini()).unwrap();
        assert_eq!(ini, parsed);
        assert!(import_text("{").is_err());
    }

    #[test]
    fn favorites_tree_nests_files_and_migrates_a_flat_list() {
        let old = r#"{"version":1,"favorites":[{"path":"C:\\Work","name":"Work"},{"path":"D:\\Photos"}]}"#;
        let migrated = adopt(old).unwrap();
        assert_eq!(migrated.favs.len(), 2);
        assert_eq!(migrated.tree.len(), 2);
        assert!(migrated.is_fav(r"c:\work"));
        let again: Value = serde_json::from_str(&sync_text(&migrated)).unwrap();
        assert_eq!(again["favorites_tree"][0]["name"], "Work");
        assert!(again["favorites_tree"][1].get("name").is_none());

        let mut s = Session::default();
        s.tree.push(crate::FavNode::group("Jobs"));
        s.tree[0].children.push(crate::FavNode::leaf(r"C:\a\note.txt", "note", true));
        s.tree.push(crate::FavNode::leaf(r"D:\Photos", "", false));
        s.sync_favs();
        let back = adopt(&sync_text(&s)).unwrap();
        assert_eq!(back.tree, s.tree);
        assert_eq!(back.favs, vec![r"C:\a\note.txt".to_string() + "\tnote", r"D:\Photos".into()]);
        assert!(back.is_fav(r"C:\a\note.txt"));
        assert!(back.tree[0].children[0].file);
    }

    #[test]
    fn explorer_pins_open_round_trips_and_stays_out_of_the_tree() {
        let fresh = adopt("{}").unwrap();
        assert!(fresh.pins_open);
        let text = sync_text(&fresh);
        assert!(serde_json::from_str::<Value>(&text).unwrap().get("explorer_pins_open").is_none());

        let mut closed = fresh.clone();
        closed.pins_open = false;
        closed.tree.push(crate::FavNode::leaf(r"D:\Photos", "", false));
        closed.sync_favs();
        let saved: Value = serde_json::from_str(&sync_text(&closed)).unwrap();
        assert_eq!(saved["explorer_pins_open"], false);
        let tree = saved["favorites_tree"].as_array().unwrap();
        assert!(tree.iter().all(|n| n.get("name").and_then(|v| v.as_str()) != Some("Explorer pins")));
        let back = adopt(&saved.to_string()).unwrap();
        assert!(!back.pins_open);
        assert!(back.files_open);
        assert_eq!(back.tree.len(), 1);
        assert_eq!(back.tree[0].path, r"D:\Photos");
    }

    #[test]
    fn explorer_files_open_round_trips_and_stays_out_of_the_tree() {
        let fresh = adopt("{}").unwrap();
        assert!(fresh.files_open);
        let text = sync_text(&fresh);
        let saved_fresh: Value = serde_json::from_str(&text).unwrap();
        assert!(saved_fresh.get("explorer_files_open").is_none());

        let mut closed = fresh.clone();
        closed.files_open = false;
        closed.tree.push(crate::FavNode::leaf(r"C:\a\note.txt", "note", true));
        closed.sync_favs();
        let saved: Value = serde_json::from_str(&sync_text(&closed)).unwrap();
        assert_eq!(saved["explorer_files_open"], false);
        let tree = saved["favorites_tree"].as_array().unwrap();
        assert!(tree.iter().all(|n| n.get("name").and_then(|v| v.as_str()) != Some("Explorer favourite files")));
        let back = adopt(&saved.to_string()).unwrap();
        assert!(!back.files_open);
        assert!(back.pins_open);
        assert!(back.tree[0].file);
        assert_eq!(back.tree[0].path, r"C:\a\note.txt");
    }

    #[test]
    fn tabs_round_trip_in_json() {
        let mut s = Session::default();
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\one".into();
        s.anchor();
        let mut other = Session::default();
        other.set_heights(vec![2]);
        other.paths[0] = r"D:\two".into();
        s.tabs.push(crate::Tab::from_live(&other, "Notes".into(), true, vec!["*.txt".into()]));
        let back = adopt(&sync_text(&s)).unwrap();
        assert_eq!(back, s);
        assert!(back.tabs[1].locked);
        assert_eq!(back.tabs[1].filters[0], "*.txt");
        assert_eq!(back.tabs[1].paths[0], r"D:\two");
        assert_eq!(back.paths[0], r"C:\one");

        let old = r#"{"version":1,"layout":{"columns":[1,3],"active":2},"panes":[{"index":1,"path":"C:\\z","filter":"*.pdf"}]}"#;
        let loaded = adopt(old).unwrap();
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].name, "1");
        assert!(!loaded.tabs[0].locked);
        assert_eq!(loaded.paths[1], r"C:\z");
        assert_eq!(loaded.tabs[0].filters[1], "*.pdf");
        assert_eq!((loaded.heights.clone(), loaded.active), (vec![1, 3], 2));
        let again: Value = serde_json::from_str(&sync_text(&loaded)).unwrap();
        assert_eq!(again["tabs"][0]["name"], "1");
        assert_eq!(again["tabs"][0]["panes"][0]["filter"], "*.pdf");
    }

    #[test]
    fn ini_migrates_to_json_and_is_renamed() {
        let root = scratch("migrate");
        let exe = root.join("exe");
        let cfg = root.join("appdata").join("grids");
        std::fs::create_dir_all(&exe).unwrap();
        let parsed = Session::parse(&sample().to_ini());
        std::fs::write(exe.join("grids.ini"), parsed.to_ini()).unwrap();
        std::fs::create_dir_all(exe.join("sessions")).unwrap();
        std::fs::write(exe.join("sessions").join("work.grids"), parsed.snapshot()).unwrap();

        let loaded = load_at(&exe, &cfg);
        assert_eq!(loaded, parsed);
        assert!(!exe.join("grids.ini").exists());
        assert!(exe.join("grids.ini.bak").is_file());
        let json = std::fs::read_to_string(cfg.join("grids.json")).unwrap();
        assert!(json.contains("\"version\": 1"));
        assert_eq!(adopt(&json).unwrap(), parsed);
        assert_eq!(std::fs::read_to_string(exe.join("sessions").join("work.grids")).unwrap(), parsed.snapshot());

        // A second launch reads the json and leaves the backup alone.
        let again = load_at(&exe, &cfg);
        assert_eq!(again, parsed);
        assert!(exe.join("grids.ini.bak").is_file());

        // An ini that only exists in the config dir is the one that moves.
        let root2 = scratch("appdata-ini");
        let exe2 = root2.join("exe");
        let cfg2 = root2.join("cfg");
        std::fs::create_dir_all(&exe2).unwrap();
        std::fs::create_dir_all(&cfg2).unwrap();
        std::fs::write(cfg2.join("grids.ini"), "lang=ja\n").unwrap();
        assert_eq!(load_at(&exe2, &cfg2).lang, Lang::Ja);
        assert!(cfg2.join("grids.json").is_file());
        assert!(cfg2.join("grids.ini.bak").is_file());
        assert!(!cfg2.join("grids.ini").exists());

        // The exe ini wins when both exist, and an existing json is not replaced.
        let root3 = scratch("keep-json");
        let exe3 = root3.join("exe");
        let cfg3 = root3.join("cfg");
        std::fs::create_dir_all(&exe3).unwrap();
        std::fs::create_dir_all(&cfg3).unwrap();
        std::fs::write(exe3.join("grids.ini"), "lang=en\n").unwrap();
        std::fs::write(cfg3.join("grids.json"), pretty(&SettingsFile::from_session(&Session { lang: Lang::Ko, ..Session::default() }))).unwrap();
        assert_eq!(load_at(&exe3, &cfg3).lang, Lang::Ko);
        assert!(exe3.join("grids.ini").is_file());

        // qdir.ini is the older name, used when grids.ini is absent.
        let root4 = scratch("qdir");
        let exe4 = root4.join("exe");
        let cfg4 = root4.join("cfg");
        std::fs::create_dir_all(&exe4).unwrap();
        std::fs::write(exe4.join("qdir.ini"), "lang=en\nsingle=0\n").unwrap();
        let q = load_at(&exe4, &cfg4);
        assert_eq!((q.lang, q.single), (Lang::En, false));
        assert!(exe4.join("qdir.ini.bak").is_file());
    }

    #[test]
    fn portable_json_sits_beside_the_exe() {
        let dir = scratch("portable");
        std::fs::write(dir.join("portable.txt"), b"").unwrap();
        std::fs::write(dir.join("grids.ini"), sample().to_ini()).unwrap();
        let loaded = load_at(&dir, &dir);
        assert_eq!(loaded, Session::parse(&sample().to_ini()));
        assert!(dir.join("grids.json").is_file());
        assert!(dir.join("grids.ini.bak").is_file());
        assert!(!dir.join("grids.json.tmp").exists());
    }

    #[test]
    fn keybinding_round_trip_keeps_unknown_keys_and_drops_defaults() {
        crate::keys::load_bindings(&Map::new());
        let _ = adopt("{}");
        let i = crate::keys::index("tab.new").unwrap();
        crate::keys::rebind(i, crate::keys::parse_chord("Ctrl+K").unwrap());
        let saved: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert_eq!(saved["keybindings"]["tab.new"], "Ctrl+K");

        let mut file = SettingsFile::from_session(&Session::default());
        file.keybindings.insert("copy".into(), serde_json::json!("Ctrl+C"));
        let _ = adopt(&pretty(&file));
        assert_eq!(crate::keys::lookup(b'K' as u16, true, false, false, false), Some(i));
        let again: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert_eq!(again["keybindings"]["tab.new"], "Ctrl+K");
        assert_eq!(again["keybindings"]["copy"], "Ctrl+C");

        crate::keys::reset_all();
        let cleared: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert!(cleared["keybindings"].get("tab.new").is_none());
        assert_eq!(cleared["keybindings"]["copy"], "Ctrl+C");
        crate::keys::load_bindings(&Map::new());
        let _ = adopt("{}");
    }

    #[test]
    fn atomic_write_replaces_the_file() {
        let dir = scratch("atomic");
        let path = dir.join("grids.json");
        write_atomic(&path, b"{\"version\":1}\n").unwrap();
        write_atomic(&path, b"{\"version\":1,\"lang\":\"ja\"}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"version\":1,\"lang\":\"ja\"}\n");
        assert!(!dir.join("grids.json.tmp").exists());
        let blocked = dir.join("blocked.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_atomic(&blocked, b"{\"version\":1}\n").is_err());
        assert!(blocked.is_dir());
        assert!(!dir.join("blocked.json.tmp").exists());
    }

    #[test]
    fn unreadable_settings_are_not_defaults() {
        let dir = scratch("unread");
        assert!(read_settings_file(&dir.join("missing.json")).unwrap().is_none());
        std::fs::write(dir.join("grids.json"), "{\"lang\":\"ko\"}").unwrap();
        assert!(try_load_at(&dir, &dir).is_ok());
        let locked = scratch("locked");
        std::fs::create_dir(locked.join("grids.json")).unwrap();
        assert!(read_settings_file(&locked.join("grids.json")).is_err());
        assert!(try_load_at(&locked, &locked).is_err());
        assert!(locked.join("grids.json").is_dir());
        let bad = scratch("badjson");
        std::fs::write(bad.join("grids.json"), "{").unwrap();
        assert!(try_load_at(&bad, &bad).is_err());
        assert_eq!(std::fs::read_to_string(bad.join("grids.json")).unwrap(), "{");
    }

    #[test]
    fn windows_round_trip_keeps_each_path_and_an_empty_save_does_not_wipe_them() {
        let mut a = sample();
        a.paths[0] = "C:\\win-a".into();
        a.anchor();
        let mut b = sample();
        b.paths[0] = "D:\\win-b".into();
        b.win = Some([40, 50, 800, 600, 0]);
        b.file.clear();
        b.anchor();
        crate::set_snaps(vec![crate::window_value(&a), crate::window_value(&b)]);
        let text = sync_text(&a);
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert!(saved["windows"][0]["tabs"][0]["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\win-a"));
        assert!(saved["windows"][1]["tabs"][0]["panes"].as_array().unwrap().iter().any(|p| p["path"] == "D:\\win-b"));
        assert!(saved["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\win-a"));
        let loaded = adopt(&text).unwrap();
        assert_eq!(loaded.paths[0], "C:\\win-a");
        let rest = crate::take_stashed();
        assert_eq!(rest.len(), 1);
        let back = crate::session_from_window(&rest[0], &loaded);
        assert_eq!(back.paths[0], "D:\\win-b");
        assert_eq!(back.win, Some([40, 50, 800, 600, 0]));
        assert_eq!(back.accent, loaded.accent);
        // A save that did not snapshot windows must leave the stored list in place.
        let kept: Value = serde_json::from_str(&sync_text(&loaded)).unwrap();
        assert_eq!(kept["windows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn old_panel_key_is_dropped_and_side_split_round_trips() {
        let mut s = Session::default();
        s.side_split = 0.25;
        s.anchor();
        let saved: Value = serde_json::from_str(&sync_text(&s)).unwrap();
        assert!(saved.get("panel").is_none());
        assert!((saved["side_split"].as_f64().unwrap() - 0.25).abs() < 1e-9);
        let back = adopt(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert!((back.side_split - 0.25).abs() < 1e-9);
        let migrated = adopt(r#"{"version":1,"panel":"fav","side_split":0.25}"#).unwrap();
        assert!((migrated.side_split - 0.25).abs() < 1e-9);
        let again: Value = serde_json::from_str(&sync_text(&migrated)).unwrap();
        assert!(again.get("panel").is_none());
        assert!((again["side_split"].as_f64().unwrap() - 0.25).abs() < 1e-9);
        let even: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert!(even.get("panel").is_none());
        assert!(even.get("side_split").is_none());
        let only = adopt(r#"{"panel":"fav"}"#).unwrap();
        assert!((only.side_split - 0.5).abs() < 1e-9);
        let dropped: Value = serde_json::from_str(&sync_text(&only)).unwrap();
        assert!(dropped.get("panel").is_none());
        let wide = adopt(r#"{"side_split":9}"#).unwrap();
        assert!((wide.side_split - 1.0).abs() < 1e-9);
    }

    #[test]
    fn load_drops_a_retired_panel_key_once() {
        let root = scratch("drop-panel");
        let exe = root.join("exe");
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("grids.json"), "{\"version\":1,\"panel\":\"fav\",\"side_split\":0.25,\"lang\":\"en\"}\n").unwrap();
        let s = load_at(&exe, &cfg);
        assert!((s.side_split - 0.25).abs() < 1e-9);
        let json = std::fs::read_to_string(cfg.join("grids.json")).unwrap();
        assert!(!json.contains("\"panel\""));
        assert!(json.contains("0.25"));
        let again = load_at(&exe, &cfg);
        assert!((again.side_split - 0.25).abs() < 1e-9);
        assert_eq!(std::fs::read_to_string(cfg.join("grids.json")).unwrap(), json);
    }

    #[test]
    fn one_window_defaults_off_and_round_trips() {
        assert!(!Session::default().one_window);
        assert!(!adopt(r#"{"version":1,"single":true}"#).unwrap().one_window);
        let mut s = Session::default();
        s.one_window = true;
        assert!(Session::parse(&s.to_ini()).one_window);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"one_window\": true"), "{text}");
        assert!(adopt(&text).unwrap().one_window);
        assert!(!text_of_off());
    }

    fn text_of_off() -> bool {
        pretty(&SettingsFile::from_session(&Session::default())).contains("one_window")
    }

    #[test]
    fn use_everything_defaults_on_and_round_trips() {
        assert!(Session::default().use_everything);
        assert!(adopt(r#"{"version":1}"#).unwrap().use_everything);
        let mut s = Session::default();
        s.use_everything = false;
        assert!(!Session::parse(&s.to_ini()).use_everything);
        assert!(Session::parse("everything=1").use_everything);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"use_everything\": false"), "{text}");
        assert!(!adopt(&text).unwrap().use_everything);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("use_everything"));
    }

    #[test]
    fn utf8_bom_loads_the_favorites_tree() {
        let root = scratch("bom-fav");
        let exe = root.join("exe");
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend(br#"{"version":1,"favorites_tree":[{"name":"Jobs","children":[{"name":"note","path":"C:\\a\\note.txt","file":true}]}]}"#);
        std::fs::write(cfg.join("grids.json"), raw).unwrap();
        let s = load_at(&exe, &cfg);
        assert_eq!(s.tree.len(), 1);
        assert_eq!(s.tree[0].name, "Jobs");
        assert!(s.tree[0].is_group());
        assert_eq!(s.tree[0].children[0].name, "note");
        assert!(s.tree[0].children[0].file);
        assert!(s.is_fav(r"C:\a\note.txt"));
        let imported = import_text("\u{FEFF}{\"version\":1,\"favorites_tree\":[{\"name\":\"Places\",\"path\":\"D:\\\\Photos\"}]}").unwrap();
        assert_eq!(imported.tree[0].name, "Places");
        assert!(imported.is_fav(r"D:\Photos"));
    }

    #[test]
    fn font_face_round_trips_present_absent_and_unknown_keys() {
        let _ = adopt("{}");
        assert!(Session::default().font_face.is_empty());
        let bare = adopt(r#"{"version":1,"lang":"ja","future":{"keep":true}}"#).unwrap();
        assert!(bare.font_face.is_empty());
        assert_eq!(bare.lang, Lang::Ja);
        let saved_bare: Value = serde_json::from_str(&sync_text(&bare)).unwrap();
        assert!(saved_bare.get("font_face").is_none());
        assert_eq!(saved_bare["future"]["keep"], true);
        assert_eq!(saved_bare["lang"], "ja");

        let mut s = bare.clone();
        s.font_face = "Yu Gothic UI".into();
        let saved: Value = serde_json::from_str(&sync_text(&s)).unwrap();
        assert_eq!(saved["font_face"], "Yu Gothic UI");
        assert_eq!(saved["future"]["keep"], true);
        let back = adopt(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(back.font_face, "Yu Gothic UI");
        assert_eq!(back.lang, Lang::Ja);

        let mut named = Session::default();
        named.font_face = "Consolas".into();
        named.lang = Lang::En;
        assert!(!named.to_ini().contains("font_face"));
        assert!(Session::parse(&format!("font_face=Consolas\n{}", named.to_ini())).font_face.is_empty());
    }

    #[test]
    fn load_drops_a_font_face_that_is_not_installed() {
        let root = scratch("missing-face");
        let exe = root.join("exe");
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("grids.json"), "{\"version\":1,\"lang\":\"ja\",\"font_face\":\"Not A Real Font Zzz\",\"future\":1}\n").unwrap();
        let s = load_at(&exe, &cfg);
        assert!(s.font_face.is_empty(), "missing face should fall back");
        assert_eq!(s.lang, Lang::Ja);
        let kept: Value = serde_json::from_str(&current_text()).unwrap();
        assert!(kept.get("font_face").is_none());
        assert_eq!(kept["future"], 1);
        assert_eq!(kept["lang"], "ja");
    }
}
