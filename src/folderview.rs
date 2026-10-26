// Per-folder view memory, separate from panegrid.json and from any grids.ini.
// The file is folder-views.json in the config folder (roaming Panegrid, or the exe folder when portable.txt is present).
// A folder key is its shell parsing name, not the localized display name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use windows::Win32::Foundation::PROPERTYKEY;

use crate::settings::store::{self, write_atomic};
use crate::{is_fs, parse_cols};

pub(crate) const FILE_NAME: &str = "folder-views.json";
/// Oldest entries fall off once the file would grow past this.
pub(crate) const CAP: usize = 5000;
/// Widths within this many pixels are the same column layout. The shell's own floor is noisier than a real drag.
const WIDTH_SLOP: i32 = 4;

/// One remembered view: mode, icon size, columns (order, widths, visibility, sort) and grouping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewSnap {
    pub(crate) mode: i32,
    pub(crate) icon: i32,
    pub(crate) columns: String,
    /// None: grouping was not read (leave the shell alone).
    /// Some(""): not grouped. Some(text): one `guid/pid=dir` column, the same shape as a sort column.
    pub(crate) group: Option<String>,
}

/// What this pane applied when the folder opened, and the live view last accepted as "no user edit".
#[derive(Clone, Debug)]
pub(crate) struct ViewWatch {
    pub(crate) key: String,
    pub(crate) applied: ViewSnap,
    pub(crate) baseline: Option<ViewSnap>,
    /// Polls to ignore after a programmatic apply, so the shell finishing its own layout is not a user edit.
    pub(crate) quiet: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Saved(ViewSnap),
    Default(ViewSnap),
    None,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Store {
    version: u32,
    default: Option<ViewSnap>,
    /// Oldest first. The map holds the same keys.
    order: Vec<String>,
    map: HashMap<String, ViewSnap>,
    extra: Map<String, Value>,
    dirty: bool,
    /// The file on disk could not be read. Never replace it.
    blocked: bool,
}

#[derive(Serialize, Deserialize)]
struct FileDoc {
    #[serde(default = "one")]
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<SnapJson>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<EntryJson>,
    #[serde(default, flatten)]
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SnapJson {
    #[serde(default)]
    mode: i32,
    #[serde(default)]
    icon: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    columns: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct EntryJson {
    path: String,
    #[serde(default)]
    mode: i32,
    #[serde(default)]
    icon: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    columns: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<String>,
}

fn one() -> u32 {
    1
}

impl From<SnapJson> for ViewSnap {
    fn from(j: SnapJson) -> Self {
        ViewSnap { mode: j.mode, icon: j.icon, columns: j.columns, group: j.group }
    }
}

impl From<&ViewSnap> for SnapJson {
    fn from(v: &ViewSnap) -> Self {
        SnapJson { mode: v.mode, icon: v.icon, columns: v.columns.clone(), group: v.group.clone() }
    }
}

pub(crate) fn folder_views_path() -> PathBuf {
    store::config_dir().join(FILE_NAME)
}

/// Parsing name of a folder, or None when the text is not a folder we can reopen.
/// Filesystem paths are case-insensitive and lose a trailing slash. Drive roots stay `c:\`.
/// Shell and special folders stay on their parsing name (`::{guid}`, `shell:…`), lowercased.
pub(crate) fn normalize_folder_key(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if is_shell_parsing(path) {
        let key = normalize_shell(path);
        return (!key.is_empty()).then_some(key);
    }
    let key = normalize_fs(path)?;
    is_fs(&key).then_some(key)
}

fn is_shell_parsing(path: &str) -> bool {
    let p = path.trim_start();
    p.starts_with("::") || p.len() >= 6 && p[..6].eq_ignore_ascii_case("shell:")
}

fn normalize_shell(path: &str) -> String {
    let mut s = path.trim().replace('/', "\\");
    while s.contains("\\\\") {
        s = s.replace("\\\\", "\\");
    }
    s.trim_end_matches('\\').to_lowercase()
}

fn normalize_fs(path: &str) -> Option<String> {
    let mut s = path.trim().replace('/', "\\");
    if let Some(rest) = strip_prefix_ci(&s, "\\\\?\\UNC\\") {
        s = format!("\\\\{rest}");
    } else if let Some(rest) = strip_prefix_ci(&s, "\\\\?\\") {
        s = rest.to_string();
    }
    let unc = s.starts_with("\\\\");
    let body = if unc { &s[2..] } else { s.as_str() };
    let parts: Vec<&str> = body.split('\\').filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.is_empty() {
        return None;
    }
    let mut out = if unc { format!("\\\\{}", parts.join("\\")) } else { parts.join("\\") };
    out = out.to_lowercase();
    if out.len() == 2 && out.as_bytes()[1] == b':' {
        out.push('\\');
    }
    if !(out.len() == 3 && out.as_bytes()[1] == b':' && out.as_bytes()[2] == b'\\') {
        out = out.trim_end_matches('\\').to_string();
    }
    Some(out)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)).then(|| &s[prefix.len()..])
}

/// Column text the shell reported, replaced by `applied` when only the widths drifted a few pixels.
pub(crate) fn align_columns(applied: &str, live: &str) -> String {
    if !applied.is_empty() && !live.is_empty() && columns_close(applied, live) {
        applied.to_string()
    } else {
        live.to_string()
    }
}

pub(crate) fn columns_close(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (ca, sa) = parse_cols(a);
    let (cb, sb) = parse_cols(b);
    if ca.len() != cb.len() || sa.len() != sb.len() || ca.is_empty() && cb.is_empty() && a != b {
        return false;
    }
    let same = |x: &PROPERTYKEY, y: &PROPERTYKEY| x.fmtid == y.fmtid && x.pid == y.pid;
    ca.iter().zip(&cb).all(|(x, y)| same(&x.0, &y.0) && (x.1 as i32 - y.1 as i32).abs() <= WIDTH_SLOP)
        && sa.iter().zip(&sb).all(|(x, y)| same(&x.0, &y.0) && x.1 == y.1)
}

/// True when `live` is a user edit worth storing for this folder.
pub(crate) fn observe(watch: &mut ViewWatch, live: &ViewSnap) -> bool {
    if watch.key.is_empty() || live.mode == 0 {
        return false;
    }
    // Details with no columns yet: the list is still building. Icon views may legitimately have none.
    if live.mode == 4 && live.columns.is_empty() {
        return false;
    }
    if !watch.applied.columns.is_empty() && live.columns.is_empty() {
        return false;
    }
    if watch.quiet > 0 {
        if !differs_from_applied(&watch.applied, live) {
            watch.quiet = 0;
            watch.baseline = Some(absorb(watch.baseline.as_ref(), live));
            return false;
        }
        watch.quiet -= 1;
        return false;
    }
    let Some(base) = watch.baseline.clone() else {
        let save = differs_from_applied(&watch.applied, live);
        watch.baseline = Some(live.clone());
        return save;
    };
    if let Some(next) = absorb_fill(&base, live) {
        watch.baseline = Some(next);
        return false;
    }
    if same_settled(&base, live) {
        return false;
    }
    watch.baseline = Some(live.clone());
    true
}

fn differs_from_applied(applied: &ViewSnap, live: &ViewSnap) -> bool {
    if applied.mode != 0 && live.mode != 0 && live.mode != applied.mode {
        return true;
    }
    // Zero means the shell has not published a size yet. Two real sizes that disagree are an edit.
    if applied.icon != 0 && live.icon != 0 && live.icon != applied.icon {
        return true;
    }
    if !applied.columns.is_empty() && !live.columns.is_empty() && live.columns != applied.columns {
        return true;
    }
    match (&applied.group, &live.group) {
        (Some(a), Some(b)) if a != b => true,
        _ => false,
    }
}

/// The shell publishing icon size or grouping after the first look is not an edit.
fn absorb_fill(base: &ViewSnap, live: &ViewSnap) -> Option<ViewSnap> {
    if base.mode != live.mode || base.columns != live.columns {
        return None;
    }
    let icon_fill = base.icon == 0 && live.icon != 0;
    let group_fill = base.group.is_none() && live.group.is_some();
    if !icon_fill && !group_fill {
        return None;
    }
    if !icon_fill && base.icon != live.icon {
        return None;
    }
    if !group_fill && !groups_equal(&base.group, &live.group) {
        return None;
    }
    Some(live.clone())
}

fn absorb(prev: Option<&ViewSnap>, live: &ViewSnap) -> ViewSnap {
    let mut out = live.clone();
    if let Some(prev) = prev {
        if out.icon == 0 {
            out.icon = prev.icon;
        }
        if out.group.is_none() {
            out.group = prev.group.clone();
        }
    }
    out
}

fn same_settled(base: &ViewSnap, live: &ViewSnap) -> bool {
    base.mode == live.mode && base.icon == live.icon && base.columns == live.columns && groups_equal(&base.group, &live.group)
}

fn groups_equal(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        (None, None) => true,
        (Some(x), None) | (None, Some(x)) => x.is_empty(),
    }
}

impl Store {
    pub(crate) fn choose(&mut self, path: &str) -> Choice {
        let Some(key) = normalize_folder_key(path) else {
            return Choice::None;
        };
        if self.map.contains_key(&key) {
            let view = self.map.get(&key).cloned().unwrap();
            self.touch(&key);
            return Choice::Saved(view);
        }
        match &self.default {
            Some(v) => Choice::Default(v.clone()),
            None => Choice::None,
        }
    }

    pub(crate) fn remember(&mut self, path: &str, snap: ViewSnap) -> bool {
        let Some(key) = normalize_folder_key(path) else {
            return false;
        };
        if self.blocked {
            return false;
        }
        let changed = self.map.get(&key) != Some(&snap);
        self.map.insert(key.clone(), snap);
        let moved = self.touch(&key);
        self.evict();
        if changed || moved {
            self.dirty = true;
        }
        changed || moved
    }

    pub(crate) fn forget(&mut self, path: &str) -> bool {
        let Some(key) = normalize_folder_key(path) else {
            return false;
        };
        if self.blocked {
            return false;
        }
        let had = self.map.remove(&key).is_some();
        let before = self.order.len();
        self.order.retain(|k| k != &key);
        if had || self.order.len() != before {
            self.dirty = true;
        }
        had
    }

    pub(crate) fn clear_folders(&mut self) {
        if self.blocked || self.map.is_empty() {
            return;
        }
        self.map.clear();
        self.order.clear();
        self.dirty = true;
    }

    pub(crate) fn set_default(&mut self, snap: ViewSnap) {
        if self.blocked || self.default.as_ref() == Some(&snap) {
            return;
        }
        self.default = Some(snap);
        self.dirty = true;
    }

    pub(crate) fn default_view(&self) -> Option<ViewSnap> {
        self.default.clone()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, path: &str) -> bool {
        normalize_folder_key(path).is_some_and(|k| self.map.contains_key(&k))
    }

    fn touch(&mut self, key: &str) -> bool {
        if self.order.last().is_some_and(|k| k == key) {
            return false;
        }
        self.order.retain(|k| k != key);
        self.order.push(key.to_string());
        true
    }

    fn evict(&mut self) {
        while self.order.len() > CAP {
            if let Some(old) = self.order.first().cloned() {
                self.order.remove(0);
                self.map.remove(&old);
                self.dirty = true;
            } else {
                break;
            }
        }
    }

    pub(crate) fn to_json(&self) -> String {
        let folders = self
            .order
            .iter()
            .filter_map(|k| {
                let view = self.map.get(k)?;
                Some(EntryJson {
                    path: k.clone(),
                    mode: view.mode,
                    icon: view.icon,
                    columns: view.columns.clone(),
                    group: view.group.clone(),
                })
            })
            .collect();
        let doc = FileDoc {
            version: self.version.max(1),
            default: self.default.as_ref().map(SnapJson::from),
            folders,
            extra: self.extra.clone(),
        };
        let mut text = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{\"version\":1}".into());
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    pub(crate) fn parse(text: &str) -> Option<Store> {
        let doc: FileDoc = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        let mut store = Store { version: doc.version.max(1), extra: doc.extra, ..Store::default() };
        if let Some(d) = doc.default {
            store.default = Some(d.into());
        }
        for e in doc.folders {
            let Some(key) = normalize_folder_key(&e.path) else { continue };
            let snap = ViewSnap { mode: e.mode, icon: e.icon, columns: e.columns, group: e.group };
            store.map.insert(key.clone(), snap);
            store.order.retain(|k| k != &key);
            store.order.push(key);
        }
        let trimmed = store.order.len() > CAP;
        store.evict();
        store.dirty = trimmed;
        Some(store)
    }

    pub(crate) fn load_from(path: &Path) -> Store {
        match std::fs::read(path) {
            Ok(b) => match Store::parse(&String::from_utf8_lossy(&b)) {
                Some(s) => s,
                None => Store { blocked: true, ..Store::default() },
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(_) => Store { blocked: true, ..Store::default() },
        }
    }

    pub(crate) fn save_to(&mut self, path: &Path) -> std::io::Result<()> {
        if self.blocked {
            return Err(std::io::Error::other("folder-views.json could not be read"));
        }
        write_atomic(path, self.to_json().as_bytes())?;
        self.dirty = false;
        Ok(())
    }
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    use std::sync::Mutex;
    static STORE: Mutex<Option<Store>> = Mutex::new(None);
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Store::load_from(&folder_views_path()));
    }
    f(g.as_mut().unwrap())
}

pub(crate) fn choose(path: &str) -> Choice {
    with_store(|s| s.choose(path))
}

pub(crate) fn remember(path: &str, snap: ViewSnap) -> bool {
    with_store(|s| s.remember(path, snap))
}

pub(crate) fn forget(path: &str) -> bool {
    with_store(|s| s.forget(path))
}

pub(crate) fn clear_folders() {
    with_store(|s| s.clear_folders());
}

pub(crate) fn set_default(snap: ViewSnap) {
    with_store(|s| s.set_default(snap));
}

pub(crate) fn default_view() -> Option<ViewSnap> {
    with_store(|s| s.default_view())
}

pub(crate) fn is_dirty() -> bool {
    with_store(|s| s.dirty)
}

/// Write folder-views.json if something changed. A file that could not be read is left in place.
pub(crate) fn flush() {
    with_store(|s| {
        if !s.dirty || s.blocked {
            s.dirty = false;
            return;
        }
        if s.save_to(&folder_views_path()).is_err() {
            // Keep dirty so a later timer or shutdown tries again. The previous file stays.
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(mode: i32, columns: &str) -> ViewSnap {
        ViewSnap { mode, icon: 16, columns: columns.into(), group: None }
    }

    fn watch(applied: ViewSnap) -> ViewWatch {
        ViewWatch { key: "c:\\docs".into(), applied, baseline: None, quiet: 0 }
    }

    #[test]
    fn filesystem_keys_ignore_case_slash_and_prefix() {
        let want = Some("c:\\foo\\bar".to_string());
        for p in ["C:\\Foo\\Bar", "c:/foo/bar/", "C:\\Foo\\Bar\\", "  c:\\foo\\bar  ", "\\\\?\\C:\\Foo\\Bar"] {
            assert_eq!(normalize_folder_key(p), want, "{p}");
        }
        assert_eq!(normalize_folder_key("C:\\"), Some("c:\\".into()));
        assert_eq!(normalize_folder_key("c:"), Some("c:\\".into()));
        assert_eq!(normalize_folder_key("C:/"), Some("c:\\".into()));
        assert_eq!(normalize_folder_key("\\\\Server\\Share\\Dir\\"), Some("\\\\server\\share\\dir".into()));
        assert_eq!(normalize_folder_key("\\\\?\\UNC\\Server\\Share\\"), Some("\\\\server\\share".into()));
        assert_eq!(normalize_folder_key("C:\\Foo\\.\\Bar"), Some("c:\\foo\\bar".into()));
        assert_eq!(normalize_folder_key(""), None);
        assert_eq!(normalize_folder_key("   "), None);
        assert_eq!(normalize_folder_key("not-a-path"), None);
        assert_eq!(normalize_folder_key("search-ms:query=a"), None);
    }

    #[test]
    fn shell_folders_use_the_parsing_name() {
        let guid = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        let want = Some(guid.to_lowercase());
        assert_eq!(normalize_folder_key(guid), want);
        assert_eq!(normalize_folder_key(&format!("{guid}\\")), want);
        assert_eq!(normalize_folder_key(&guid.to_lowercase()), want);
        assert_eq!(normalize_folder_key("SHELL:Downloads\\"), Some("shell:downloads".into()));
        assert_eq!(normalize_folder_key("shell:Desktop"), Some("shell:desktop".into()));
        let chain = "::{11111111-1111-1111-1111-111111111111}\\::{22222222-2222-2222-2222-222222222222}";
        assert_eq!(normalize_folder_key(&format!("{chain}\\")), Some(chain.to_lowercase()));
        // A display name is not a parsing name, so two languages do not share a key by accident.
        assert_eq!(normalize_folder_key("This PC"), None);
        assert_eq!(normalize_folder_key("내 PC"), None);
    }

    #[test]
    fn lru_drops_the_oldest_and_a_touch_keeps_it() {
        let mut s = Store::default();
        for i in 0..CAP {
            assert!(s.remember(&format!("C:\\f\\{i}"), snap(4, "")));
        }
        assert_eq!(s.len(), CAP);
        assert!(s.contains("C:\\f\\0"));
        assert!(s.remember(&format!("C:\\f\\{CAP}"), snap(4, "")));
        assert_eq!(s.len(), CAP);
        assert!(!s.contains("C:\\f\\0"));
        assert!(s.contains(&format!("C:\\f\\{CAP}")));
        // The oldest survivor is 1. Opening it moves it to the front of the recency list.
        assert!(matches!(s.choose("C:\\f\\1"), Choice::Saved(_)));
        assert!(s.remember(&format!("C:\\f\\{}", CAP + 1), snap(1, "")));
        assert!(s.contains("C:\\f\\1"));
        assert!(!s.contains("C:\\f\\2"));
        assert_eq!(s.len(), CAP);
    }

    #[test]
    fn spellings_share_one_entry_and_the_default_is_separate() {
        let mut s = Store::default();
        let custom = snap(1, "cols");
        assert!(s.remember("C:\\Docs\\", custom.clone()));
        assert!(!s.remember("c:/docs", custom.clone())); // same view, already newest: nothing to write
        assert_eq!(s.len(), 1);
        s.set_default(snap(4, "def"));
        assert!(matches!(s.choose("D:\\other"), Choice::Default(v) if v.columns == "def"));
        assert!(matches!(s.choose("c:\\docs"), Choice::Saved(v) if v.mode == 1));
        assert!(s.forget("C:\\docs\\"));
        assert!(!s.contains("c:\\docs"));
        assert!(s.default_view().is_some());
        s.remember("C:\\keep", snap(3, ""));
        s.clear_folders();
        assert_eq!(s.len(), 0);
        assert!(s.default_view().is_some());
        assert!(matches!(s.choose("C:\\keep"), Choice::Default(_)));
    }

    #[test]
    fn json_roundtrip_keeps_order_default_group_and_unknown_keys() {
        let mut s = Store::default();
        s.set_default(ViewSnap { mode: 4, icon: 16, columns: "a=1;b=-1".into(), group: Some(String::new()) });
        s.remember("C:\\Old", snap(4, "old"));
        s.remember("shell:Downloads", ViewSnap { mode: 1, icon: 96, columns: String::new(), group: Some("g/1=1".into()) });
        s.extra.insert("future".into(), Value::from(7));
        let back = Store::parse(&s.to_json()).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.order[0], "c:\\old");
        assert_eq!(back.order[1], "shell:downloads");
        assert_eq!(back.default.as_ref().unwrap().group.as_deref(), Some(""));
        assert_eq!(back.map["shell:downloads"].icon, 96);
        assert_eq!(back.map["shell:downloads"].group.as_deref(), Some("g/1=1"));
        assert_eq!(back.extra["future"], 7);
        let again = Store::parse(&back.to_json()).unwrap();
        assert_eq!(again.order, back.order);
        assert!(again.to_json().contains("\"future\": 7"));
    }

    #[test]
    fn a_broken_file_is_not_replaced() {
        let dir = std::env::temp_dir().join(format!("panegrid-fv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, b"{not json").unwrap();
        let mut s = Store::load_from(&path);
        assert!(s.blocked);
        s.remember("C:\\x", snap(4, ""));
        assert!(s.save_to(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
        assert!(!dir.join("grids.ini").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_save_roundtrips_and_leaves_no_grids_ini() {
        let dir = std::env::temp_dir().join(format!("panegrid-fv-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let mut s = Store::default();
        s.remember("C:\\A\\", snap(3, "name"));
        s.set_default(snap(4, ""));
        s.save_to(&path).unwrap();
        let back = Store::load_from(&path);
        assert!(back.contains("c:\\a"));
        assert_eq!(back.default_view().unwrap().mode, 4);
        assert!(!back.blocked);
        assert!(!dir.join("grids.ini").exists());
        assert!(!dir.join("panegrid.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_folder_does_not_store_the_shell_default_but_an_edit_does() {
        let mut w = watch(snap(4, ""));
        let shell = snap(4, "name=200;");
        assert!(!observe(&mut w, &ViewSnap { columns: String::new(), ..shell.clone() }));
        assert!(w.baseline.is_none());
        assert!(!observe(&mut w, &shell));
        assert_eq!(w.baseline.as_ref().unwrap().columns, shell.columns);
        let edited = snap(4, "name=80;");
        assert!(observe(&mut w, &edited));
        assert_eq!(w.baseline.as_ref().unwrap().columns, "name=80;");
    }

    #[test]
    fn quiet_polls_do_not_save_and_a_later_mode_change_does() {
        let mut w = watch(snap(4, "cols"));
        w.quiet = 2;
        let other = snap(1, "cols");
        assert!(!observe(&mut w, &other));
        assert!(!observe(&mut w, &other));
        assert!(w.baseline.is_none());
        assert!(observe(&mut w, &other));
        let mut w = watch(snap(4, "cols"));
        w.quiet = 2;
        assert!(!observe(&mut w, &snap(4, "cols")));
        assert_eq!(w.quiet, 0);
        assert!(!observe(&mut w, &snap(4, "cols")));
    }

    #[test]
    fn a_few_pixels_of_width_are_not_a_new_layout() {
        let k = |pid| PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
            pid,
        };
        let applied = crate::cols_text(&[(k(10), 200)], &[(k(10), 1)]);
        let live = crate::cols_text(&[(k(10), 203)], &[(k(10), 1)]);
        let moved = crate::cols_text(&[(k(10), 260)], &[(k(10), 1)]);
        assert!(columns_close(&applied, &live));
        assert!(!columns_close(&applied, &moved));
        assert_eq!(align_columns(&applied, &live), applied);
        let mut w = watch(ViewSnap { mode: 4, icon: 16, columns: applied.clone(), group: None });
        assert!(!observe(&mut w, &ViewSnap { mode: 4, icon: 16, columns: align_columns(&applied, &live), group: None }));
        assert!(observe(&mut w, &ViewSnap { mode: 4, icon: 16, columns: moved, group: None }));
    }

    #[test]
    fn a_real_icon_size_change_is_an_edit() {
        let applied = ViewSnap { mode: 1, icon: 48, columns: String::new(), group: None };
        let mut w = watch(applied.clone());
        assert!(!observe(&mut w, &applied));
        let bigger = ViewSnap { icon: 96, ..applied };
        assert!(observe(&mut w, &bigger));
        let mut w = watch(ViewSnap { mode: 1, icon: 48, columns: String::new(), group: None });
        w.quiet = 1;
        assert!(!observe(&mut w, &bigger));
        assert!(w.baseline.is_none());
        assert!(observe(&mut w, &bigger));
    }

    #[test]
    fn grouping_and_icon_size_arriving_later_are_not_edits() {
        let mut w = watch(snap(1, ""));
        let first = ViewSnap { mode: 1, icon: 0, columns: String::new(), group: None };
        assert!(!observe(&mut w, &first));
        let filled = ViewSnap { mode: 1, icon: 96, columns: String::new(), group: Some(String::new()) };
        assert!(!observe(&mut w, &filled));
        let grouped = ViewSnap { group: Some("g/4=1".into()), ..filled.clone() };
        assert!(observe(&mut w, &grouped));
    }
}
