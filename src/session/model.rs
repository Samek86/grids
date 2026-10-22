use crate::imports::*;
use super::*;
use crate::app::*;
use crate::i18n::*;
use crate::store;

// ---------- session (settings live in grids.json; named sessions stay .grids files in the ini shape) ----------

/// One tab: a whole pane grid (layout, folders, view, columns, filter). The active tab's grid
/// also lives on the session fields the rest of the window already reads.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tab {
    pub(crate) name: String,
    pub(crate) locked: bool,
    /// Stripe colour 0xRRGGBB. None draws the tab in the theme colours.
    pub(crate) color: Option<u32>,
    pub(crate) heights: Vec<usize>,
    pub(crate) colr: Vec<f64>,
    pub(crate) rowr: Vec<Vec<f64>>,
    pub(crate) active: usize,
    pub(crate) paths: Vec<String>,
    pub(crate) views: Vec<(i32, i32)>,
    pub(crate) cols: Vec<String>,
    pub(crate) filters: Vec<String>,
    /// Per pane: 0 none, 1..=6 a selection-colour preset of the current theme. Separate from the tab stripe.
    pub(crate) tints: Vec<u8>,
}

pub(crate) fn pad_filters(mut f: Vec<String>) -> Vec<String> {
    f.resize(MAXP, String::new());
    f.truncate(MAXP);
    f
}

impl Tab {
    pub(crate) fn from_live(s: &Session, name: String, locked: bool, filters: Vec<String>) -> Self {
        Tab {
            name,
            locked,
            color: None,
            heights: s.heights.clone(),
            colr: s.colr.clone(),
            rowr: s.rowr.clone(),
            active: s.active,
            paths: s.paths.clone(),
            views: s.views.clone(),
            cols: s.cols.clone(),
            filters: pad_filters(filters),
            tints: s.tints.clone(),
        }
    }
}

/// The next default name: 1, then 2, skipping numbers already used.
pub(crate) fn next_tab_name(tabs: &[Tab]) -> String {
    let mut n = 1u32;
    loop {
        let s = n.to_string();
        if !tabs.iter().any(|t| t.name == s) {
            return s;
        }
        n += 1;
        if n > 10_000 {
            return s;
        }
    }
}

/// A copied tab keeps a numbered name as the next free number, and any other name as "Name (2)".
pub(crate) fn duplicate_name(tabs: &[Tab], src: &str) -> String {
    if !src.is_empty() && src.chars().all(|c| c.is_ascii_digit()) {
        return next_tab_name(tabs);
    }
    let mut n = 2u32;
    loop {
        let s = format!("{src} ({n})");
        if !tabs.iter().any(|t| t.name == s) {
            return s;
        }
        n += 1;
        if n > 1000 {
            return s;
        }
    }
}

/// A locked tab, and the last remaining tab, stay open.
pub(crate) fn can_close_tab(tabs: &[Tab], i: usize) -> bool {
    tabs.len() > 1 && tabs.get(i).is_some_and(|t| !t.locked)
}

/// Indexes closed by "close other tabs": every unlocked tab except `keep`, low to high.
pub(crate) fn close_other_indexes(tabs: &[Tab], keep: usize) -> Vec<usize> {
    tabs.iter().enumerate().filter(|(i, t)| *i != keep && !t.locked).map(|(i, _)| i).collect()
}

pub(crate) fn reorder_vec<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= v.len() || to >= v.len() {
        return;
    }
    let item = v.remove(from);
    v.insert(to, item);
}

/// Where the active index lands after a tab moves from `from` to `to`.
pub(crate) fn moved_active(active: usize, from: usize, to: usize) -> usize {
    if active == from {
        return to;
    }
    if from < active && to >= active {
        active - 1
    } else if from > active && to <= active {
        active + 1
    } else {
        active
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Session {
    pub(crate) win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    pub(crate) side: i32,             // sidebar width in px
    pub(crate) heights: Vec<usize>,   // panes per column, left to right (1..=4 columns of 1..=4)
    pub(crate) colr: Vec<f64>,        // column width ratios
    pub(crate) rowr: Vec<Vec<f64>>,   // row height ratios, per column
    pub(crate) active: usize,
    pub(crate) paths: Vec<String>,     // parsing names per pane slot (numbered down each column, left to right)
    pub(crate) views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    pub(crate) accent: u32,            // selection colour 0xRRGGBB, light theme
    pub(crate) accent_dark: u32,       // selection colour, dark theme
    pub(crate) dark: bool,
    pub(crate) dblup: bool,            // double-click on empty space goes to the parent folder
    pub(crate) file: String,           // named session (.grids) the layout was last loaded from / saved to
    pub(crate) favs: Vec<String>,      // favourite leaves, depth-first (`path` or `path\tname`)
    pub(crate) tree: Vec<FavNode>,     // groups and leaves; `favs` is derived from this
    pub(crate) recent: Vec<String>,    // recently visited folders from every pane, newest first, app-wide
    pub(crate) lang: Lang,             // UI language preference; Auto follows Windows
    pub(crate) single: bool,           // a second launch opens a new window in this process
    pub(crate) cols: Vec<String>,      // per pane: columns and sort of its view (cols_text)
    pub(crate) tints: Vec<u8>,         // per pane: colour label, 0 = none, 1..=6 = selection colour preset
    pub(crate) theme: u8,              // 0 follow Windows, 1 light, 2 dark
    pub(crate) font: u8,               // 0 default, 1 large, 2 larger
    pub(crate) bar: bool,              // toolbar
    pub(crate) stat: bool,             // status line
    pub(crate) view_new: i32,          // FOLDERVIEWMODE for a pane that has none yet (4 details, 3 list, 1 icons)
    pub(crate) start_new: u8,          // 0 This PC, 1 Desktop, 2 start_path
    pub(crate) start_path: String,
    pub(crate) ext: bool,              // show file extensions (the Windows Explorer setting)
    pub(crate) tree_on: bool,          // left panel open
    pub(crate) side_split: f64,        // folder tree's share of the left panel; favourites take the rest
    pub(crate) pins_open: bool,        // Explorer pins group expanded; the rows are not saved
    pub(crate) files_open: bool,       // Explorer favourite-files group expanded; hidden when empty; rows are not saved
    pub(crate) edge: bool,             // accent border on the active pane
    pub(crate) confirm: bool,          // ask before delete
    pub(crate) del: u8,                // 0 recycle bin, 1 permanent
    pub(crate) new_name: String,       // empty = the name for the display language
    pub(crate) dropm: u8,              // 0 Windows rules, 1 always copy, 2 always ask (the ⇄ menu)
    pub(crate) boot: u8,               // 0 restore last layout, 1 open boot_file, 2 default 2×2
    pub(crate) boot_file: String,
    pub(crate) autosave: bool,         // write the named session on exit
    pub(crate) dot: bool,              // green unsaved mark on the save button
    pub(crate) tabs: Vec<Tab>,         // every tab's grid; the active one's grid is also the fields above
    pub(crate) tab: usize,             // which tab is on screen
}

impl Default for Session {
    fn default() -> Self {
        let mut s = Session {
            win: None,
            side: 0,
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            accent: 0x0067C0, // Windows Settings blue; a chosen swatch replaces it
            accent_dark: 0x4CC2FF,
            dark: false,
            dblup: true,
            file: String::new(),
            favs: Vec::new(),
            tree: Vec::new(),
            recent: Vec::new(),
            lang: Lang::Auto,
            single: true,
            cols: vec![String::new(); MAXP],
            tints: vec![0; MAXP],
            theme: 0,
            font: 0,
            bar: true,
            stat: true,
            view_new: 4,
            start_new: 0,
            start_path: String::new(),
            ext: true,
            tree_on: true,
            side_split: 0.5,
            pins_open: true,
            files_open: true,
            edge: true,
            confirm: true,
            del: 0,
            new_name: String::new(),
            dropm: 0,
            boot: 0,
            boot_file: String::new(),
            autosave: false,
            dot: true,
            tabs: Vec::new(),
            tab: 0,
        };
        s.anchor();
        s
    }
}

impl PartialEq for Session {
    fn eq(&self, o: &Self) -> bool {
        self.win == o.win
            && self.side == o.side
            && self.heights == o.heights
            && self.colr == o.colr
            && self.rowr == o.rowr
            && self.active == o.active
            && self.paths == o.paths
            && self.views == o.views
            && self.accent == o.accent
            && self.accent_dark == o.accent_dark
            && self.dark == o.dark
            && self.dblup == o.dblup
            && self.file == o.file
            && self.favs == o.favs
            && self.tree == o.tree
            && self.recent == o.recent
            && self.lang == o.lang
            && self.single == o.single
            && self.cols == o.cols
            && self.tints == o.tints
            && self.theme == o.theme
            && self.font == o.font
            && self.bar == o.bar
            && self.stat == o.stat
            && self.view_new == o.view_new
            && self.start_new == o.start_new
            && self.start_path == o.start_path
            && self.ext == o.ext
            && self.tree_on == o.tree_on
            && self.side_split == o.side_split
            && self.pins_open == o.pins_open
            && self.files_open == o.files_open
            && self.edge == o.edge
            && self.confirm == o.confirm
            && self.del == o.del
            && self.new_name == o.new_name
            && self.dropm == o.dropm
            && self.boot == o.boot
            && self.boot_file == o.boot_file
            && self.autosave == o.autosave
            && self.dot == o.dot
            && self.tab == o.tab
            && self.view_tabs() == o.view_tabs()
    }
}

/// Folder tree's share of the side plate. Out of range or NaN becomes an even split.
pub(crate) fn sane_split(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 }
}

/// Favourite entries are `path` or `path<tab>display name`. The path is what gets opened.
pub(crate) fn fav_path(f: &str) -> &str {
    f.split('\t').next().unwrap_or(f)
}

pub(crate) fn fav_label(f: &str) -> String {
    match f.split_once('\t') {
        Some((_, n)) if !n.is_empty() => n.to_string(),
        _ => short_name(fav_path(f)),
    }
}

pub(crate) fn even(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

pub(crate) fn nums<T: std::str::FromStr>(v: &str) -> Vec<T> {
    v.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

pub(crate) fn join<T: Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

// Ratios from a hand-edited ini can be anything: wrong length, zero, negative -> fall back to equal.
pub(crate) fn fix(r: Vec<f64>, n: usize) -> Vec<f64> {
    if r.len() != n || r.iter().any(|x| !(*x > 0.0)) {
        return even(n);
    }
    let sum: f64 = r.iter().sum();
    r.iter().map(|x| x / sum).collect()
}

// "2 × 2" for an even grid, "1 | 3" when the columns differ.
pub(crate) fn layout_label(h: &[usize]) -> String {
    match h.iter().all(|&n| n == h[0]) {
        true => format!("{} × {}", h[0], h.len()),
        false => h.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | "),
    }
}

impl Session {
    /// Tabs as saved: the active tab's grid is the live fields, its name and lock stay.
    pub(crate) fn view_tabs(&self) -> Vec<Tab> {
        let mut tabs = if self.tabs.is_empty() {
            vec![Tab::from_live(self, "1".into(), false, vec![String::new(); MAXP])]
        } else {
            self.tabs.clone()
        };
        let i = self.tab.min(tabs.len() - 1);
        let name = tabs[i].name.clone();
        let locked = tabs[i].locked;
        let color = tabs[i].color;
        let filters = tabs[i].filters.clone();
        tabs[i] = Tab::from_live(self, name, locked, filters);
        tabs[i].color = color;
        tabs
    }

    /// Write the live grid into the active tab. An empty list becomes one tab named 1.
    pub(crate) fn anchor(&mut self) {
        if self.tabs.is_empty() {
            self.tabs.push(Tab::from_live(self, "1".into(), false, vec![String::new(); MAXP]));
            self.tab = 0;
            return;
        }
        if self.tab >= self.tabs.len() {
            self.tab = 0;
        }
        let name = self.tabs[self.tab].name.clone();
        let locked = self.tabs[self.tab].locked;
        let color = self.tabs[self.tab].color;
        let filters = pad_filters(std::mem::take(&mut self.tabs[self.tab].filters));
        self.tabs[self.tab] = Tab::from_live(self, name, locked, filters);
        self.tabs[self.tab].color = color;
    }

    /// Copy the active tab onto the live grid fields.
    pub(crate) fn publish(&mut self) {
        if self.tabs.is_empty() {
            self.anchor();
            return;
        }
        self.tab = self.tab.min(self.tabs.len() - 1);
        let t = self.tabs[self.tab].clone();
        self.heights = t.heights;
        self.colr = t.colr;
        self.rowr = t.rowr;
        let n = self.heights.iter().sum::<usize>().max(1);
        self.active = t.active.min(n - 1);
        self.paths = t.paths;
        self.views = t.views;
        self.cols = t.cols;
        self.tints = t.tints;
    }

    pub(crate) fn accent_now(&self) -> u32 {
        if self.dark {
            self.accent_dark
        } else {
            self.accent
        }
    }

    pub(crate) fn panes(&self) -> usize {
        self.heights.iter().sum()
    }

    /// New layout with even splits.
    pub(crate) fn set_heights(&mut self, h: Vec<usize>) {
        self.colr = even(h.len());
        self.rowr = h.iter().map(|&n| even(n)).collect();
        self.heights = h;
        self.active = self.active.min(self.panes() - 1);
    }

    pub(crate) fn parse(text: &str) -> Session {
        let mut s = Session::default();
        s.tabs.clear();
        let mut rowr: Vec<Vec<f64>> = Vec::new();
        let (mut saw_theme, mut saw_dark) = (false, false);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                "side" => s.side = v.parse().unwrap_or(0),
                "split" => s.side_split = sane_split(v.parse().unwrap_or(0.5)),
                "cols" => {
                    let h: Vec<usize> = nums::<usize>(v).into_iter().take(4).map(|n| n.clamp(1, 4)).collect();
                    if !h.is_empty() {
                        s.heights = h;
                    }
                }
                // older ini: even rows x cols grid
                "grid" => {
                    if let [r, c] = nums::<usize>(v)[..] {
                        s.heights = vec![r.clamp(1, 4); c.clamp(1, 4)];
                    }
                }
                "colr" => s.colr = nums(v),
                "rowr" => rowr = v.split('|').map(nums).collect(),
                "active" => s.active = v.parse().unwrap_or(0),
                "accent" => s.accent = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).unwrap_or(s.accent) & 0xFFFFFF,
                "accent_dark" => s.accent_dark = u32::from_str_radix(v.trim(), 16).unwrap_or(s.accent_dark) & 0xFFFFFF,
                "dark" => {
                    s.dark = v.trim() == "1";
                    saw_dark = true;
                }
                "theme" => {
                    s.theme = match v.trim() {
                        "light" => 1,
                        "dark" => 2,
                        "auto" => 0,
                        _ => s.theme,
                    };
                    saw_theme = true;
                }
                "font" => s.font = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "bar" => s.bar = v.trim() != "0",
                "stat" => s.stat = v.trim() != "0",
                "viewnew" => s.view_new = match v.trim() {
                    "3" | "list" => 3,
                    "1" | "icon" => 1,
                    _ => 4,
                },
                "start" => s.start_new = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "startp" => s.start_path = v.trim().to_string(),
                "ext" => s.ext = v.trim() != "0",
                "tree" => s.tree_on = v.trim() != "0",
                "edge" => s.edge = v.trim() != "0",
                "confirm" => s.confirm = v.trim() != "0",
                "del" => s.del = if v.trim() == "1" { 1 } else { 0 },
                "newname" => s.new_name = v.trim().to_string(),
                "drop" => s.dropm = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "boot" => s.boot = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "bootf" => s.boot_file = v.trim().to_string(),
                "autosave" => s.autosave = v.trim() == "1",
                "dot" => s.dot = v.trim() != "0",
                "dblup" => s.dblup = v.trim() != "0",
                "lang" => s.lang = Lang::from_ini(v),
                "single" => s.single = v.trim() != "0",
                "session" => s.file = v.trim().to_string(),
                "tabs" => {
                    if let Some((tabs, active)) = tabs_from_json(v) {
                        s.tabs = tabs;
                        s.tab = active;
                    }
                }
                "fav" if !v.trim().is_empty() => s.favs.push(v.trim().to_string()),
                "recent" if !v.trim().is_empty() && s.recent.len() < RECENT => s.recent.push(v.trim().to_string()),
                _ => {
                    if let Some(i) = slot('p') {
                        s.paths[i] = v.to_string();
                    } else if let (Some(i), [m, z]) = (slot('v'), &nums::<i32>(v)[..]) {
                        s.views[i] = (*m, *z);
                    } else if let Some(i) = slot('c') {
                        s.cols[i] = v.trim().to_string();
                    } else if let Some(i) = slot('t') {
                        s.tints[i] = v.trim().parse().ok().filter(|&k| k <= 6).unwrap_or(0);
                    }
                }
            }
        }
        s.colr = fix(std::mem::take(&mut s.colr), s.heights.len());
        // one group per column; an older single group applies to every column
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        if s.active >= s.panes() {
            s.active = 0;
        }
        // An older ini has dark= and no theme=. That stays an explicit light or dark choice.
        if !saw_theme && saw_dark {
            s.theme = if s.dark { 2 } else { 1 };
        }
        if s.tabs.is_empty() {
            s.anchor();
        } else {
            s.publish();
        }
        s.tree = super::tree_from_flat(&s.favs);
        s
    }

    pub(crate) fn to_ini(&self) -> String {
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        let rowr = self.rowr.iter().map(|r| join(r)).collect::<Vec<_>>().join("|");
        o += &format!("side={}\nsplit={}\ncols={}\n", self.side, self.side_split, join(&self.heights));
        o += &format!("colr={}\nrowr={rowr}\nactive={}\n", join(&self.colr), self.active);
        o += &format!("accent={:06X}\naccent_dark={:06X}\ndark={}\n", self.accent, self.accent_dark, self.dark as u8);
        let theme = ["auto", "light", "dark"][self.theme.min(2) as usize];
        o += &format!("theme={theme}\nfont={}\nbar={}\nstat={}\n", self.font, self.bar as u8, self.stat as u8);
        o += &format!("viewnew={}\nstart={}\n", self.view_new, self.start_new);
        if !self.start_path.is_empty() {
            o += &format!("startp={}\n", self.start_path);
        }
        o += &format!("ext={}\ntree={}\nedge={}\n", self.ext as u8, self.tree_on as u8, self.edge as u8);
        o += &format!("confirm={}\ndel={}\n", self.confirm as u8, self.del);
        if !self.new_name.is_empty() {
            o += &format!("newname={}\n", self.new_name);
        }
        o += &format!("drop={}\nboot={}\n", self.dropm, self.boot);
        if !self.boot_file.is_empty() {
            o += &format!("bootf={}\n", self.boot_file);
        }
        o += &format!("autosave={}\ndot={}\n", self.autosave as u8, self.dot as u8);
        o += &format!("dblup={}\nlang={}\nsingle={}\n", self.dblup as u8, self.lang.ini(), self.single as u8);
        if !self.file.is_empty() {
            o += &format!("session={}\n", self.file);
        }
        for f in &self.favs {
            o += &format!("fav={f}\n");
        }
        for r in &self.recent {
            o += &format!("recent={r}\n");
        }
        for i in 0..MAXP {
            if !self.paths[i].is_empty() {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
            if !self.cols[i].is_empty() {
                o += &format!("c{i}={}\n", self.cols[i]);
            }
            if self.tints[i] != 0 {
                o += &format!("t{i}={}\n", self.tints[i]);
            }
        }
        let tabs = self.view_tabs();
        let active = self.tab.min(tabs.len() - 1);
        o += &format!("tabs={}\n", tabs_json(&tabs, active));
        o
    }

    pub(crate) fn is_fav(&self, path: &str) -> bool {
        !path.is_empty()
            && (super::contains_path(&self.tree, path) || self.favs.iter().any(|f| fav_path(f).eq_ignore_ascii_case(path)))
    }

    pub(crate) fn default_view(&self) -> (i32, i32) {
        match self.view_new {
            3 => (3, 16),
            1 => (1, 96),
            _ => (4, 16),
        }
    }

    /// What goes into a named session file: no window position, no pointer to itself, no favourites or recent folders.
    pub(crate) fn snapshot(&self) -> String {
        let mut c = self.clone();
        c.anchor();
        c.win = None;
        c.file.clear();
        c.favs.clear();
        c.tree.clear();
        c.recent.clear();
        c.to_ini()
    }

    /// Takes every tab from a named session; window position and sidebar width stay as they are.
    pub(crate) fn take_layout(&mut self, o: &Session) {
        (self.heights, self.colr, self.rowr) = (o.heights.clone(), o.colr.clone(), o.rowr.clone());
        (self.active, self.paths, self.views, self.cols) = (o.active, o.paths.clone(), o.views.clone(), o.cols.clone());
        self.tints = o.tints.clone();
        self.tabs = o.view_tabs();
        self.tab = o.tab.min(self.tabs.len().saturating_sub(1));
    }
}

/// Named-session text for one tab: its grid only.
pub(crate) fn tab_session_text(s: &Session, i: usize) -> Option<String> {
    let t = s.view_tabs().get(i)?.clone();
    let mut one = s.clone();
    one.tabs = vec![t];
    one.tab = 0;
    one.publish();
    Some(one.snapshot())
}

pub(crate) fn parse_tab_color(v: &str) -> Option<u32> {
    let t = v.trim().trim_start_matches('#');
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(t, 16).ok().map(|n| n & 0xFFFFFF)
}

pub(crate) fn pane_json(t: &Tab) -> Vec<Value> {
    let mut panes = Vec::new();
    for i in 0..MAXP {
        let filter = t.filters.get(i).cloned().unwrap_or_default();
        let tint = t.tints.get(i).copied().unwrap_or(0);
        if t.paths[i].is_empty() && t.views[i] == (0, 0) && t.cols[i].is_empty() && filter.is_empty() && tint == 0 {
            continue;
        }
        let mut p = serde_json::Map::new();
        p.insert("index".into(), Value::from(i));
        if !t.paths[i].is_empty() {
            p.insert("path".into(), Value::String(t.paths[i].clone()));
        }
        if t.views[i].0 != 0 {
            p.insert("view".into(), Value::Array(vec![Value::from(t.views[i].0), Value::from(t.views[i].1)]));
        }
        if !t.cols[i].is_empty() {
            p.insert("columns".into(), Value::String(t.cols[i].clone()));
        }
        if !filter.is_empty() {
            p.insert("filter".into(), Value::String(filter));
        }
        if tint != 0 {
            p.insert("tint".into(), Value::from(tint));
        }
        panes.push(Value::Object(p));
    }
    panes
}

pub(crate) fn tabs_json(tabs: &[Tab], active: usize) -> String {
    let v: Vec<Value> = tabs.iter().enumerate().map(|(i, t)| {
        let mut obj = serde_json::json!({
            "name": t.name,
            "locked": t.locked,
            "active": i == active,
            "layout": { "columns": t.heights, "colr": t.colr, "rowr": t.rowr, "active": t.active },
            "panes": pane_json(t),
        });
        if let Some(c) = t.color {
            if let Some(m) = obj.as_object_mut() {
                m.insert("color".into(), Value::String(format!("{c:06X}")));
            }
        }
        obj
    }).collect();
    serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
}

/// A tab object needs a name, a layout, panes, or a lock. Anything else is left for a newer build.
pub(crate) fn tab_from_value(v: &Value) -> Option<Tab> {
    let obj = v.as_object()?;
    if !obj.contains_key("name") && !obj.contains_key("layout") && !obj.contains_key("panes") && !obj.contains_key("locked") {
        return None;
    }
    let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    let locked = obj.get("locked").and_then(|n| n.as_bool()).unwrap_or(false);
    let color = obj.get("color").and_then(|c| c.as_str()).and_then(parse_tab_color);
    let mut heights = vec![2, 2];
    let mut colr = Vec::new();
    let mut rowr: Vec<Vec<f64>> = Vec::new();
    let mut active = 0usize;
    if let Some(l) = obj.get("layout").and_then(|l| l.as_object()) {
        if let Some(c) = l.get("columns").and_then(|c| c.as_array()) {
            let h: Vec<usize> = c.iter().filter_map(|n| n.as_u64()).map(|n| (n as usize).clamp(1, 4)).take(4).collect();
            if !h.is_empty() {
                heights = h;
            }
        }
        if let Some(c) = l.get("colr").and_then(|c| c.as_array()) {
            colr = c.iter().filter_map(|n| n.as_f64()).collect();
        }
        if let Some(rows) = l.get("rowr").and_then(|c| c.as_array()) {
            rowr = rows.iter().map(|r| r.as_array().map(|a| a.iter().filter_map(|n| n.as_f64()).collect()).unwrap_or_default()).collect();
        }
        active = l.get("active").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    }
    colr = fix(colr, heights.len());
    let group = |c: usize| if rowr.len() == heights.len() { rowr.get(c) } else { rowr.first() };
    let rowr = heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
    let n: usize = heights.iter().sum();
    if active >= n {
        active = 0;
    }
    let mut paths = vec![String::new(); MAXP];
    let mut views = vec![(0i32, 0i32); MAXP];
    let mut cols = vec![String::new(); MAXP];
    let mut filters = vec![String::new(); MAXP];
    let mut tints = vec![0u8; MAXP];
    if let Some(panes) = obj.get("panes").and_then(|p| p.as_array()) {
        for p in panes {
            let Some(o) = p.as_object() else { continue };
            let i = o.get("index").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
            if i >= MAXP {
                continue;
            }
            if let Some(path) = o.get("path").and_then(|x| x.as_str()) {
                paths[i] = path.to_string();
            }
            if let Some(view) = o.get("view").and_then(|x| x.as_array()) {
                let m = view.first().and_then(|n| n.as_i64()).unwrap_or(0) as i32;
                let z = view.get(1).and_then(|n| n.as_i64()).unwrap_or(0) as i32;
                if m != 0 {
                    views[i] = (m, z);
                }
            }
            if let Some(c) = o.get("columns").and_then(|x| x.as_str()) {
                cols[i] = c.to_string();
            }
            if let Some(f) = o.get("filter").and_then(|x| x.as_str()) {
                filters[i] = f.to_string();
            }
            if let Some(t) = o.get("tint").and_then(|x| x.as_u64()) {
                tints[i] = if t <= 6 { t as u8 } else { 0 };
            }
        }
    }
    let name = if name.is_empty() { "1".into() } else { name };
    Some(Tab { name, locked, color, heights, colr, rowr, active, paths, views, cols, filters, tints })
}

pub(crate) fn tabs_from_json(v: &str) -> Option<(Vec<Tab>, usize)> {
    let values: Vec<Value> = serde_json::from_str(v).ok()?;
    let mut tabs = Vec::new();
    let mut active = None;
    for val in &values {
        let Some(t) = tab_from_value(val) else { continue };
        if val.get("active").and_then(|a| a.as_bool()) == Some(true) && active.is_none() {
            active = Some(tabs.len());
        }
        tabs.push(t);
    }
    if tabs.is_empty() {
        return None;
    }
    let active = active.unwrap_or(0).min(tabs.len() - 1);
    Some((tabs, active))
}

/// Put `values` on the session. Invalid entries are skipped; none left means one tab from the live grid.
pub(crate) fn adopt_tabs(s: &mut Session, values: &[Value]) {
    let mut tabs = Vec::new();
    let mut active = None;
    for v in values {
        let Some(t) = tab_from_value(v) else { continue };
        if v.get("active").and_then(|a| a.as_bool()) == Some(true) && active.is_none() {
            active = Some(tabs.len());
        }
        tabs.push(t);
    }
    if tabs.is_empty() {
        s.tabs.clear();
        s.tab = 0;
        s.anchor();
        return;
    }
    s.tabs = tabs;
    s.tab = active.unwrap_or(0).min(s.tabs.len() - 1);
    s.publish();
}

/// Layout changes worth a save prompt: grid, split ratios, view modes, which tab is open. Folder changes don't count.
pub(crate) fn layout_differs(cur: &Session, base: &Session) -> bool {
    let near = |x: &[f64], y: &[f64]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| (a - b).abs() < 0.005);
    let grid = |c: &Session, b: &Session| {
        c.heights != b.heights
            || !near(&c.colr, &b.colr)
            || c.rowr.len() != b.rowr.len()
            || c.rowr.iter().zip(&b.rowr).any(|(a, b)| !near(a, b))
            || (0..c.panes()).any(|i| b.views[i].0 != 0 && c.views[i] != b.views[i])
            || (0..c.panes()).any(|i| !b.cols[i].is_empty() && c.cols[i] != b.cols[i])
            || (0..c.panes()).any(|i| c.tints.get(i) != b.tints.get(i))
    };
    if grid(cur, base) || cur.tab != base.tab || cur.tabs.len() != base.tabs.len() {
        return true;
    }
    cur.tabs.iter().zip(&base.tabs).enumerate().any(|(i, (a, b))| {
        if i == cur.tab {
            return false;
        }
        let n = a.heights.iter().sum::<usize>().min(MAXP);
        a.heights != b.heights
            || !near(&a.colr, &b.colr)
            || a.rowr.len() != b.rowr.len()
            || a.rowr.iter().zip(&b.rowr).any(|(x, y)| !near(x, y))
            || (0..n).any(|k| b.views.get(k).is_some_and(|v| v.0 != 0) && a.views.get(k) != b.views.get(k))
            || (0..n).any(|k| b.cols.get(k).is_some_and(|c| !c.is_empty()) && a.cols.get(k) != b.cols.get(k))
            || (0..n).any(|k| a.tints.get(k) != b.tints.get(k))
    })
}

// Drives, folders and shares; not shell places like This PC or the Recycle Bin, whose columns are their own.
pub(crate) fn is_fs(path: &str) -> bool {
    path.get(1..3) == Some(":\\") || path.starts_with(r"\\")
}

/// A folder you can browse. ZIP, CAB and the like are FOLDER|STREAM, so a filter treats them as files.
pub(crate) fn real_folder(attr: u32) -> bool {
    attr & SFGAO_FOLDER.0 != 0 && attr & SFGAO_STREAM.0 == 0
}

/// Address crumbs of a parsing name: (shown name, folder to open). The drive or `\\server\share` is first.
/// Shell places such as This PC have none.
pub(crate) fn crumbs(parse: &str) -> Vec<(String, String)> {
    if !is_fs(parse) {
        return Vec::new();
    }
    let (root, go, rest) = match parse.strip_prefix(r"\\") {
        Some(unc) => {
            let mut it = unc.splitn(3, '\\');
            let root = format!(r"\\{}", it.by_ref().take(2).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\\"));
            (root.clone(), root, it.next().unwrap_or(""))
        }
        None => {
            if parse.len() < 3 {
                return Vec::new();
            }
            (parse[..2].to_string(), parse[..3].to_string(), &parse[3..])
        }
    };
    let mut path = root.clone();
    let mut out = vec![(root, go)];
    for part in rest.split('\\').filter(|p| !p.is_empty()) {
        path = format!(r"{path}\{part}");
        out.push((part.to_string(), path.clone()));
    }
    out
}

/// Crumb widths `w` into `avail` px: (first crumb shown, whether '…' is shown, width of the last).
/// Everything when it fits. Otherwise the current folder, cut to fit, then parents nearest first, leaving room for '…'.
pub(crate) fn crumb_fit(w: &[i32], avail: i32, sep: i32, ell: i32, min: i32) -> (usize, bool, i32) {
    let n = w.len();
    if n == 0 || avail < min {
        return (n, false, 0);
    }
    if w.iter().sum::<i32>() + sep * (n as i32 - 1) <= avail {
        return (0, false, w[n - 1]);
    }
    let last = w[n - 1].min(avail);
    let (mut first, mut used) = (n - 1, last);
    for k in (0..n - 1).rev() {
        if used + w[k] + sep + if k > 0 { ell + sep } else { 0 } > avail {
            break;
        }
        used += w[k] + sep;
        first = k;
    }
    (first, first > 0 && used + ell + sep <= avail, last)
}

pub(crate) const RECENT: usize = 20;

// A pane moving from `old` to `new` puts both at the front of the recent list. Coming back to the folder it
// already holds (session restore, theme/language rebuild, refresh) is no visit; neither is a fresh pane or a shell place.
pub(crate) fn visit(list: &mut Vec<String>, old: &str, new: &str) {
    if old.is_empty() || old.eq_ignore_ascii_case(new) || !is_fs(new) {
        return;
    }
    for p in [old, new].into_iter().filter(|p| is_fs(p)) {
        list.retain(|x| !x.eq_ignore_ascii_case(p));
        list.insert(0, p.to_string());
    }
    list.truncate(RECENT);
}

// The recent menu of a pane: everything but the folder it shows.
pub(crate) fn recent_items(list: &[String], cur: &str) -> Vec<String> {
    list.iter().filter(|p| !p.eq_ignore_ascii_case(cur)).cloned().collect()
}

/// Address text that names no path: the recent folder (newest first), then a favourite folder,
/// whose name equals it, else starts with it, else contains it. The folder on display is skipped.
pub(crate) fn jump_pick<'a>(q: &str, recent: &'a [String], favs: &'a [String], cur: &str) -> Option<&'a str> {
    let q = q.trim().to_lowercase();
    if q.is_empty() || q.contains(['\\', '/', ':', '"']) {
        return None;
    }
    let score_name = |n: &str| {
        let n = n.to_lowercase();
        if n == q {
            Some(0u8)
        } else if n.starts_with(&q) {
            Some(1)
        } else {
            n.contains(&q).then_some(2)
        }
    };
    let score = |p: &str| {
        let path = fav_path(p);
        [short_name(path), fav_label(p)].iter().filter_map(|n| score_name(n)).min()
    };
    let all = recent.iter().chain(favs).filter(|p| is_fs(fav_path(p)) && !fav_path(p).eq_ignore_ascii_case(cur));
    all.filter_map(|p| Some((score(p)?, p))).min_by_key(|(s, _)| *s).map(|(_, p)| fav_path(p))
}

/// Enter in the address bar fell through (`done` is None): jump only when the text named no path.
/// A path that resolved but would not open just beeps.
pub(crate) fn addr_jumps(done: Option<bool>, resolved: bool) -> bool {
    done.is_none() && !resolved
}

// A view's columns: visible ones in order with their widths, then the sort columns, each property as
// "format GUID/property id": "GUID/pid=width,...;GUID/pid=direction,...".
pub(crate) fn cols_text(cols: &[(PROPERTYKEY, u32)], sort: &[(PROPERTYKEY, i32)]) -> String {
    let key = |k: &PROPERTYKEY| format!("{:?}/{}", k.fmtid, k.pid);
    let cols: Vec<String> = cols.iter().map(|(k, w)| format!("{}={w}", key(k))).collect();
    let sort: Vec<String> = sort.iter().map(|(k, d)| format!("{}={d}", key(k))).collect();
    format!("{};{}", cols.join(","), sort.join(","))
}

pub(crate) fn parse_cols(t: &str) -> (Vec<(PROPERTYKEY, u32)>, Vec<(PROPERTYKEY, i32)>) {
    fn items<T: std::str::FromStr>(s: &str) -> Vec<(PROPERTYKEY, T)> {
        let item = |e: &str| {
            let (k, v) = e.split_once('=')?;
            let (g, pid) = k.split_once('/')?;
            Some((PROPERTYKEY { fmtid: GUID::try_from(g).ok()?, pid: pid.parse().ok()? }, v.parse().ok()?))
        };
        s.split(',').filter_map(item).collect()
    }
    let (cols, sort) = t.split_once(';').unwrap_or((t, ""));
    (items(cols), items(sort))
}

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
pub(crate) fn resolve(p: String) -> String {
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
}

pub(crate) fn is_session_file(p: &Path) -> bool {
    // .qdr: sessions saved before the rename
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("grids") || x.eq_ignore_ascii_case("qdr"))
}

// A lone session-file argument means "load this session".
pub(crate) fn session_arg(args: &[String]) -> Option<&str> {
    match args {
        [f] if is_session_file(Path::new(f)) => Some(f),
        _ => None,
    }
}

// `grids.exe A B C` opens the paths in pane order, growing the grid if needed.
pub(crate) fn apply_args(s: &mut Session, args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    let n = args.len().min(MAXP);
    for (i, p) in args.into_iter().take(MAXP).enumerate() {
        s.paths[i] = p;
    }
    if n > s.panes() {
        let c = (n as f64).sqrt().ceil() as usize;
        s.set_heights(vec![n.div_ceil(c); c]);
    }
    s.active = 0;
    s.anchor();
}

pub(crate) fn sessions_dir() -> PathBuf {
    store::exe_dir().join("sessions")
}

pub(crate) fn human(b: u64) -> String {
    let (mut v, mut k) = (b as f64, 0);
    while v >= 1024.0 && k < 4 {
        v /= 1024.0;
        k += 1;
    }
    match k {
        0 => format!("{b} B"),
        _ => format!("{v:.1} {}", ["", "KB", "MB", "GB", "TB"][k]),
    }
}

/// "pdf; *.txt" -> "*pdf*;*.txt" for PathMatchSpec: plain words match anywhere in the name.
pub(crate) fn filter_spec(text: &str) -> String {
    let parts = text.split(';').map(str::trim).filter(|t| !t.is_empty());
    let parts = parts.map(|t| if t.contains(['*', '?']) { t.to_string() } else { format!("*{t}*") });
    parts.collect::<Vec<_>>().join(";")
}

#[cfg(test)]
mod crumb_tests {
    use super::{addr_jumps, crumb_fit, crumbs, jump_pick, layout_differs, real_folder, Session};
    use crate::imports::{SFGAO_FOLDER, SFGAO_STREAM};
    use crate::MAXP;
    use crate::THIS_PC;

    #[test]
    fn address_crumbs() {
        let c = |p: &str| crumbs(p).into_iter().map(|(n, g)| format!("{n}={g}")).collect::<Vec<_>>();
        assert_eq!(c("C:\\"), [r"C:=C:\"]);
        assert_eq!(c(r"C:\Users\Public\"), [r"C:=C:\", r"Users=C:\Users", r"Public=C:\Users\Public"]);
        assert_eq!(c(r"\\srv\share\a"), [r"\\srv\share=\\srv\share", r"a=\\srv\share\a"]);
        assert!(crumbs(THIS_PC).is_empty());
        assert!(crumbs("C:").is_empty());
        let p = r"D:\x\y y\z";
        assert!(crumbs(p).iter().all(|(_, g)| p.starts_with(g.as_str())));
    }

    #[test]
    fn crumb_fits() {
        let f = |w: &[i32], avail| crumb_fit(w, avail, 4, 20, 16);
        assert_eq!(f(&[30, 40, 50], 500), (0, false, 50));
        assert_eq!(f(&[14, 15, 15], 52), (0, false, 15));
        assert_eq!(f(&[14, 15, 15], 51), (2, true, 15));
        assert_eq!(f(&[10, 200], 100), (1, false, 100));
        assert_eq!(f(&[30, 10, 50], 70), (2, false, 50));
        assert_eq!(f(&[30, 10, 50], 78), (2, true, 50));
        assert_eq!(f(&[30, 40], 15), (2, false, 0));
        assert_eq!(f(&[30, 40], 0), (2, false, 0));
        assert_eq!(f(&[], 100), (0, false, 0));
    }

    #[test]
    fn jump_picks() {
        let v = |s: &[&str]| s.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let recent = v(&[r"C:\Users\me\Downloads", r"C:\Work\down", r"D:\Old Downloads", r"D:\문서\보고서 2026", THIS_PC]);
        let favs = v(&[r"E:\Down", r"\\srv\share", r"C:\", "C:\\Work\\proj\t보고서"]);
        let pick = |q: &str, cur: &str| jump_pick(q, &recent, &favs, cur);
        assert_eq!(pick("down", ""), Some(r"C:\Work\down"));
        assert_eq!(pick("DOWNL", ""), Some(r"C:\Users\me\Downloads"));
        assert_eq!(pick("loads", ""), Some(r"C:\Users\me\Downloads"));
        assert_eq!(pick("  down  ", r"c:\work\DOWN"), Some(r"E:\Down"));
        assert_eq!(pick("보고", ""), Some(r"D:\문서\보고서 2026"));
        assert_eq!(pick("보고서", ""), Some(r"C:\Work\proj"));
        assert_eq!(pick("share", ""), Some(r"\\srv\share"));
        assert_eq!(pick("c", ""), Some(r"C:\"));
        assert_eq!(pick("this", ""), None);
        for q in ["", "   ", r"down\x", "a/b", "c:", "\"down\"", "nothing"] {
            assert_eq!(pick(q, ""), None, "{q:?}");
        }
        assert!(addr_jumps(None, false));
        assert!(!addr_jumps(None, true));
        assert!(!addr_jumps(Some(true), false) && !addr_jumps(Some(false), false));
    }

    #[test]
    fn pane_tint_round_trip() {
        let mut s = Session::default();
        s.tints[3] = 4;
        s.anchor();
        let text = s.to_ini();
        assert!(text.contains("t3=4\n"));
        assert!(text.contains("\"tint\":4"));
        let back = Session::parse(&text);
        assert_eq!(back.tints[3], 4);
        assert_eq!(back.view_tabs()[back.tab].tints[3], 4);
        assert_eq!(Session::parse("t5=9\nt6=x\nt99=2").tints, vec![0; MAXP]);
        assert_eq!(Session::parse(&Session::default().to_ini()).tints, vec![0; MAXP]);
        let mut base = s.clone();
        base.tints[3] = 0;
        base.anchor();
        assert!(layout_differs(&s, &base));
        assert!(!s.snapshot().contains("win="));
    }

    #[test]
    fn archives_match_by_name() {
        assert!(real_folder(SFGAO_FOLDER.0));
        assert!(!real_folder(SFGAO_FOLDER.0 | SFGAO_STREAM.0));
        assert!(!real_folder(SFGAO_STREAM.0));
        assert!(!real_folder(0));
    }
}

