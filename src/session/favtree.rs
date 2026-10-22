// Favourites are a tree of groups and leaves. Leaves are folders or files.
// The flat `favs` list is the leaves, depth-first, so the star, the settings
// page, and an old `fav=` line keep working. Groups live only in the tree.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{fav_path, short_name, Session};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FavNode {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) file: bool,
    pub(crate) open: bool,
    pub(crate) children: Vec<FavNode>,
}

impl FavNode {
    pub(crate) fn group(name: &str) -> Self {
        FavNode { name: name.to_string(), path: String::new(), file: false, open: true, children: Vec::new() }
    }

    pub(crate) fn leaf(path: &str, name: &str, file: bool) -> Self {
        FavNode { name: name.to_string(), path: path.to_string(), file, open: false, children: Vec::new() }
    }

    pub(crate) fn is_group(&self) -> bool {
        self.path.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FavRow {
    pub(crate) id: Vec<usize>,
    pub(crate) depth: u8,
    pub(crate) group: bool,
    pub(crate) file: bool,
    pub(crate) open: bool,
    pub(crate) name: String,
    pub(crate) path: String,
    /// Explorer pins header or one of its folders. Those rows are not nodes of `favorites_tree`.
    pub(crate) pins: bool,
}

/// One child of Quick Access, before the pinned-folder filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PinCand {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) pinned: bool,
    pub(crate) folder: bool,
}

/// A filesystem folder that is pinned in Explorer. Order is Quick Access order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pinned {
    pub(crate) path: String,
    pub(crate) name: String,
}

/// What an internal favourites drag should do when it lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideAct {
    /// Move a node inside the favourites tree.
    Move,
    /// Pin this folder in Explorer. The favourites node stays where it is.
    Pin,
    /// Copy an Explorer pin into the favourites tree. The pin stays pinned.
    Copy,
    /// No indicator and no change. Pin rows do not reorder, and files do not pin.
    None,
}

/// Path kept for navigation and for case-insensitive identity. Drive roots keep their slash.
pub(crate) fn tidy_path(path: &str) -> String {
    let t = path.trim();
    let stripped = t.trim_end_matches(['\\', '/']);
    if stripped.is_empty() {
        return String::new();
    }
    if stripped.ends_with(':') {
        format!("{stripped}\\")
    } else {
        stripped.to_string()
    }
}

fn pin_key(path: &str) -> String {
    tidy_path(path).to_ascii_lowercase()
}

/// Pinned filesystem folders, in enumeration order. Files, unpinned items, empty paths, and
/// a later copy of the same path are left out.
pub(crate) fn pinned_folders(items: &[PinCand]) -> Vec<Pinned> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for it in items {
        if !it.pinned || !it.folder {
            continue;
        }
        let path = tidy_path(&it.path);
        let key = pin_key(&path);
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        let name = it.name.trim();
        let name = if name.is_empty() { short_name(&path) } else { name.to_string() };
        out.push(Pinned { path, name });
    }
    out
}

/// A drop onto the pins group is accepted only when every item is a filesystem folder.
/// Any file, or a folder with no filesystem path, refuses the whole drop.
pub(crate) fn pin_drop_paths(items: &[(String, bool)]) -> Option<Vec<String>> {
    if items.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for (path, folder) in items {
        let path = tidy_path(path);
        if !folder || path.is_empty() {
            return None;
        }
        if !out.iter().any(|p: &String| p.eq_ignore_ascii_case(&path)) {
            out.push(path);
        }
    }
    Some(out)
}

/// `src_path_empty` is a group or a leaf that has nothing to pin.
pub(crate) fn side_act(src_pins: bool, src_group: bool, src_file: bool, src_path_empty: bool, dst_pins: bool) -> SideAct {
    if dst_pins {
        if !src_pins && !src_group && !src_file && !src_path_empty {
            SideAct::Pin
        } else {
            SideAct::None
        }
    } else if src_pins && !src_group && !src_path_empty {
        SideAct::Copy
    } else if src_pins {
        SideAct::None
    } else {
        SideAct::Move
    }
}

/// True when a Quick Access re-read is allowed. `last` is when the previous read finished.
pub(crate) fn pins_due(last: Option<Instant>, now: Instant, wait: Duration) -> bool {
    match last {
        None => true,
        Some(t) => now.saturating_duration_since(t) >= wait,
    }
}

/// The pins group, then its folders when `open`, then the favourites tree. Favourite ids are unchanged.
pub(crate) fn compose_side(tree: &[FavNode], open: bool, pins: &[Pinned], label: &str) -> Vec<FavRow> {
    let mut out = Vec::new();
    out.push(FavRow {
        id: Vec::new(),
        depth: 0,
        group: true,
        file: false,
        open,
        name: label.to_string(),
        path: String::new(),
        pins: true,
    });
    if open {
        for (i, p) in pins.iter().enumerate() {
            out.push(FavRow {
                id: vec![i],
                depth: 1,
                group: false,
                file: false,
                open: false,
                name: p.name.clone(),
                path: p.path.clone(),
                pins: true,
            });
        }
    }
    out.extend(visible_rows(tree));
    out
}

/// Insert a copy of a folder at the drop position. Does nothing if that path is already a favourite.
pub(crate) fn copy_leaf(tree: &mut Vec<FavNode>, path: &str, name: &str, onto: &[usize], zone: u8) -> Option<Vec<usize>> {
    let path = tidy_path(path);
    if path.is_empty() || onto.is_empty() || contains_path(tree, &path) {
        return None;
    }
    let into = zone == 1 && is_group_id(tree, onto);
    if zone == 1 && !into {
        return None;
    }
    let parent: Vec<usize> = if into { onto.to_vec() } else { onto[..onto.len() - 1].to_vec() };
    let index = if into {
        child_len(tree, onto)
    } else if zone == 0 {
        onto[onto.len() - 1]
    } else {
        onto[onto.len() - 1] + 1
    };
    insert_child(tree, &parent, index, FavNode::leaf(&path, name, false))?;
    let index = index.min(child_len(tree, &parent).saturating_sub(1));
    let mut id = parent;
    id.push(index);
    Some(id)
}

pub(crate) fn node_label(n: &FavNode) -> String {
    if !n.name.is_empty() {
        return n.name.clone();
    }
    if n.path.is_empty() {
        return String::new();
    }
    short_name(&n.path)
}

/// Leaves, depth-first, in the `path` or `path\tname` form the rest of the app stores.
pub(crate) fn flat_favs(tree: &[FavNode]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], out: &mut Vec<String>) {
        for n in nodes {
            if n.is_group() {
                walk(&n.children, out);
                continue;
            }
            let custom = !n.name.is_empty() && n.name != short_name(&n.path);
            if custom {
                out.push(format!("{}\t{}", n.path, n.name));
            } else {
                out.push(n.path.clone());
            }
        }
    }
    walk(tree, &mut out);
    out
}

pub(crate) fn tree_from_flat(favs: &[String]) -> Vec<FavNode> {
    favs.iter()
        .filter_map(|f| {
            let path = fav_path(f).trim();
            if path.is_empty() {
                return None;
            }
            let name = f.split_once('\t').map(|(_, n)| n.trim().to_string()).unwrap_or_default();
            Some(FavNode::leaf(path, &name, false))
        })
        .collect()
}

pub(crate) fn contains_path(tree: &[FavNode], path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    tree.iter().any(|n| {
        (!n.path.is_empty() && n.path.eq_ignore_ascii_case(path)) || (n.is_group() && contains_path(&n.children, path))
    })
}

pub(crate) fn add_item(tree: &mut Vec<FavNode>, path: &str, file: bool) -> bool {
    let path = path.trim();
    if path.is_empty() || contains_path(tree, path) {
        return false;
    }
    tree.push(FavNode::leaf(path, "", file));
    true
}

/// Drop every leaf with this path. A group that only held it stays.
pub(crate) fn remove_path(tree: &mut Vec<FavNode>, path: &str) -> bool {
    fn rec(nodes: &mut Vec<FavNode>, path: &str) -> bool {
        let before = nodes.len();
        nodes.retain(|n| n.path.is_empty() || !n.path.eq_ignore_ascii_case(path));
        let mut hit = nodes.len() != before;
        for n in nodes.iter_mut() {
            if n.is_group() && rec(&mut n.children, path) {
                hit = true;
            }
        }
        hit
    }
    !path.is_empty() && rec(tree, path)
}

pub(crate) fn add_group(tree: &mut Vec<FavNode>, parent: &[usize], name: &str) -> bool {
    let node = FavNode::group(name);
    insert_child(tree, parent, usize::MAX, node).is_some()
}

pub(crate) fn rename_at(tree: &mut Vec<FavNode>, id: &[usize], name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let Some(n) = node_mut(tree, id) else { return false };
    n.name = name.chars().take(80).collect();
    true
}

pub(crate) fn delete_at(tree: &mut Vec<FavNode>, id: &[usize]) -> bool {
    take_node(tree, id).is_some()
}

pub(crate) fn set_open(tree: &mut Vec<FavNode>, id: &[usize], open: bool) {
    if let Some(n) = node_mut(tree, id) {
        if n.is_group() {
            n.open = open;
        }
    }
}

pub(crate) fn toggle_open(tree: &mut Vec<FavNode>, id: &[usize]) -> bool {
    let Some(n) = node_mut(tree, id) else { return false };
    if !n.is_group() {
        return false;
    }
    n.open = !n.open;
    true
}

pub(crate) fn visible_rows(tree: &[FavNode]) -> Vec<FavRow> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], prefix: &mut Vec<usize>, depth: u8, out: &mut Vec<FavRow>) {
        for (i, n) in nodes.iter().enumerate() {
            prefix.push(i);
            out.push(FavRow {
                id: prefix.clone(),
                depth,
                group: n.is_group(),
                file: n.file,
                open: n.open,
                name: node_label(n),
                path: n.path.clone(),
                pins: false,
            });
            if n.is_group() && n.open {
                walk(&n.children, prefix, depth.saturating_add(1), out);
            }
            prefix.pop();
        }
    }
    walk(tree, &mut Vec::new(), 0, &mut out);
    out
}

pub(crate) fn leaf_paths(tree: &[FavNode]) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, n) in nodes.iter().enumerate() {
            prefix.push(i);
            if n.is_group() {
                walk(&n.children, prefix, out);
            } else {
                out.push(prefix.clone());
            }
            prefix.pop();
        }
    }
    walk(tree, &mut Vec::new(), &mut out);
    out
}

pub(crate) fn remove_leaf(tree: &mut Vec<FavNode>, index: usize) -> bool {
    let Some(id) = leaf_paths(tree).get(index).cloned() else { return false };
    delete_at(tree, &id)
}

pub(crate) fn rename_leaf(tree: &mut Vec<FavNode>, index: usize, name: &str) -> bool {
    let Some(id) = leaf_paths(tree).get(index).cloned() else { return false };
    rename_at(tree, &id, name)
}

/// Same result as removing the flat leaf `from` and inserting it at `to`.
pub(crate) fn reorder_leaves(tree: &mut Vec<FavNode>, from: usize, to: usize) -> bool {
    if from == to {
        return false;
    }
    let paths = leaf_paths(tree);
    let (Some(src), Some(dst)) = (paths.get(from).cloned(), paths.get(to).cloned()) else { return false };
    let zone = if to < from { 0 } else { 2 };
    apply_drop(tree, &src, &dst, zone).is_some()
}

/// `zone`: 0 before `onto`, 1 into `onto` when it is a group, 2 after `onto`.
/// On success, the moved node's id in the tree afterwards.
pub(crate) fn apply_drop(tree: &mut Vec<FavNode>, from: &[usize], onto: &[usize], zone: u8) -> Option<Vec<usize>> {
    if from.is_empty() || onto.is_empty() || from == onto || onto.starts_with(from) {
        return None;
    }
    let into = zone == 1 && is_group_id(tree, onto);
    let parent: Vec<usize> = if into { onto.to_vec() } else { onto[..onto.len() - 1].to_vec() };
    let mut index = if into {
        child_len(tree, onto)
    } else if zone == 0 {
        onto[onto.len() - 1]
    } else {
        onto[onto.len() - 1] + 1
    };
    if from[..from.len() - 1] == parent[..] && from[from.len() - 1] < index {
        index -= 1;
    }
    let Some(node) = take_node(tree, from) else { return None };
    let parent = adjust(&parent, from).unwrap_or(parent);
    let index = index.min(child_len(tree, &parent));
    insert_child(tree, &parent, index, node)?;
    let mut id = parent;
    id.push(index);
    Some(id)
}

/// Top quarter / middle / bottom quarter of a row. A leaf has no middle.
pub(crate) fn drop_zone(group: bool, y: i32, rh: i32) -> u8 {
    if rh <= 0 {
        return 2;
    }
    if !group {
        return if y < rh / 2 { 0 } else { 2 };
    }
    if y < rh / 4 {
        0
    } else if y >= rh * 3 / 4 {
        2
    } else {
        1
    }
}

pub(crate) fn tree_to_values(tree: &[FavNode]) -> Vec<Value> {
    tree.iter().map(node_value).collect()
}

fn node_value(n: &FavNode) -> Value {
    let mut m = serde_json::Map::new();
    if !n.name.is_empty() {
        m.insert("name".into(), json!(n.name));
    }
    if !n.path.is_empty() {
        m.insert("path".into(), json!(n.path));
    }
    if n.file {
        m.insert("file".into(), json!(true));
    }
    if n.is_group() && !n.open {
        m.insert("open".into(), json!(false));
    }
    if !n.children.is_empty() {
        m.insert("children".into(), json!(tree_to_values(&n.children)));
    }
    Value::Object(m)
}

pub(crate) fn tree_from_values(v: &[Value]) -> Vec<FavNode> {
    v.iter().filter_map(node_from_value).collect()
}

fn node_from_value(v: &Value) -> Option<FavNode> {
    let o = v.as_object()?;
    let name = o.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let path = o.get("path").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let file = o.get("file").and_then(|x| x.as_bool()).unwrap_or(false);
    let children = o.get("children").and_then(|x| x.as_array()).map(|a| tree_from_values(a)).unwrap_or_default();
    if path.is_empty() && name.is_empty() && children.is_empty() {
        return None;
    }
    let open = if path.is_empty() { o.get("open").and_then(|x| x.as_bool()).unwrap_or(true) } else { false };
    Some(FavNode { name, path: path.clone(), file: file && !path.is_empty(), open, children })
}

impl Session {
    pub(crate) fn ensure_fav_tree(&mut self) {
        if self.tree.is_empty() && !self.favs.is_empty() {
            self.tree = tree_from_flat(&self.favs);
        }
    }

    pub(crate) fn sync_favs(&mut self) {
        self.favs = flat_favs(&self.tree);
    }
}

fn node_mut<'a>(tree: &'a mut [FavNode], id: &[usize]) -> Option<&'a mut FavNode> {
    let (&i, rest) = id.split_first()?;
    let n = tree.get_mut(i)?;
    if rest.is_empty() {
        Some(n)
    } else {
        node_mut(&mut n.children, rest)
    }
}

fn is_group_id(tree: &[FavNode], id: &[usize]) -> bool {
    let (&i, rest) = match id.split_first() {
        Some(x) => x,
        None => return false,
    };
    let n = match tree.get(i) {
        Some(n) => n,
        None => return false,
    };
    if rest.is_empty() {
        n.is_group()
    } else {
        is_group_id(&n.children, rest)
    }
}

fn child_len(tree: &[FavNode], parent: &[usize]) -> usize {
    if parent.is_empty() {
        return tree.len();
    }
    let (&i, rest) = match parent.split_first() {
        Some(x) => x,
        None => return 0,
    };
    match tree.get(i) {
        Some(n) if rest.is_empty() => n.children.len(),
        Some(n) => child_len(&n.children, rest),
        None => 0,
    }
}

fn insert_child(tree: &mut Vec<FavNode>, parent: &[usize], index: usize, node: FavNode) -> Option<()> {
    fn rec(nodes: &mut Vec<FavNode>, parent: &[usize], index: usize, node: FavNode) -> Option<()> {
        if parent.is_empty() {
            let index = index.min(nodes.len());
            nodes.insert(index, node);
            return Some(());
        }
        let (&i, rest) = parent.split_first()?;
        let n = nodes.get_mut(i)?;
        if !n.is_group() {
            return None;
        }
        rec(&mut n.children, rest, index, node)
    }
    rec(tree, parent, index, node)
}

fn take_node(tree: &mut Vec<FavNode>, id: &[usize]) -> Option<FavNode> {
    let (&i, parent) = id.split_last()?;
    fn rec(nodes: &mut Vec<FavNode>, parent: &[usize], i: usize) -> Option<FavNode> {
        if parent.is_empty() {
            return (i < nodes.len()).then(|| nodes.remove(i));
        }
        let (&p, rest) = parent.split_first()?;
        let n = nodes.get_mut(p)?;
        rec(&mut n.children, rest, i)
    }
    rec(tree, parent, i)
}

/// Visible row that an "after" marker belongs under.
/// An open group's next sibling sits below its children, so the line follows the last one.
pub(crate) fn after_row(rows: &[FavRow], i: usize) -> usize {
    let Some(row) = rows.get(i) else { return i };
    if !(row.group && row.open) {
        return i;
    }
    rows.iter().enumerate().skip(i + 1).find(|(_, r)| r.depth <= row.depth).map(|(j, _)| j - 1).unwrap_or(rows.len().saturating_sub(1))
}

/// Open a group that just accepted a drop. `onto` is its id from before the move.
pub(crate) fn reveal_drop(tree: &mut Vec<FavNode>, from: &[usize], onto: &[usize], zone: u8) {
    if zone != 1 {
        return;
    }
    let Some(id) = adjust(onto, from) else { return };
    set_open(tree, &id, true);
}

/// Path of a node that was not inside `removed`, after `removed` is gone.
fn adjust(path: &[usize], removed: &[usize]) -> Option<Vec<usize>> {
    if !removed.is_empty() && path.starts_with(removed) {
        return None;
    }
    let mut out = path.to_vec();
    let n = removed.len().saturating_sub(1);
    if !removed.is_empty() && path.len() > n && removed[..n] == path[..n] && removed[n] < path[n] {
        out[n] -= 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Vec<FavNode> {
        tree_from_flat(&["C:\\WORK\tWork".into(), "D:\\Photos".into()])
    }

    #[test]
    fn flat_list_migrates_into_the_root_and_back() {
        let tree = work();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "Work");
        assert!(!tree[0].file);
        assert_eq!(flat_favs(&tree), vec!["C:\\WORK\tWork".to_string(), "D:\\Photos".to_string()]);
        assert!(contains_path(&tree, "c:\\work"));
        assert!(!contains_path(&tree, "C:\\WORK\\x"));
    }

    #[test]
    fn files_groups_and_drops_round_trip() {
        let mut tree = work();
        assert!(add_group(&mut tree, &[], "Jobs"));
        let jobs = visible_rows(&tree).iter().find(|r| r.group).unwrap().id.clone();
        assert!(add_item(&mut tree, r"C:\a\note.txt", true));
        let file = leaf_paths(&tree).last().unwrap().clone();
        assert!(apply_drop(&mut tree, &file, &jobs, 1).is_some());
        assert!(contains_path(&tree, r"C:\a\note.txt"));
        let rows = visible_rows(&tree);
        let nested = rows.iter().find(|r| r.file).unwrap();
        assert_eq!(nested.depth, 1);
        assert_eq!(nested.path, r"C:\a\note.txt");
        let values = tree_to_values(&tree);
        let back = tree_from_values(&values);
        assert_eq!(back, tree);
        assert!(back.iter().any(|n| n.is_group() && n.children.iter().any(|c| c.file)));
        assert!(apply_drop(&mut tree, &jobs, &jobs, 1).is_none());
        let child = visible_rows(&tree).iter().find(|r| r.file).unwrap().id.clone();
        assert!(apply_drop(&mut tree, &jobs, &child, 1).is_none());
    }

    #[test]
    fn reorder_matches_a_flat_remove_and_insert() {
        let mut tree = tree_from_flat(&["A".into(), "B".into(), "C".into(), "D".into()]);
        assert!(reorder_leaves(&mut tree, 0, 2));
        assert_eq!(flat_favs(&tree), vec!["B".to_string(), "C".to_string(), "A".to_string(), "D".to_string()]);
        assert!(reorder_leaves(&mut tree, 2, 0));
        assert_eq!(flat_favs(&tree), vec!["A".to_string(), "B".to_string(), "C".to_string(), "D".to_string()]);
        assert!(rename_leaf(&mut tree, 1, "Bee"));
        assert_eq!(flat_favs(&tree)[1], "B\tBee");
        assert!(remove_leaf(&mut tree, 1));
        assert_eq!(flat_favs(&tree), vec!["A".to_string(), "C".to_string(), "D".to_string()]);
    }

    #[test]
    fn collapsed_group_hides_its_children_and_junk_json_is_skipped() {
        let mut tree = Vec::new();
        add_group(&mut tree, &[], "Box");
        add_item(&mut tree, r"C:\f", true);
        let file = leaf_paths(&tree).pop().unwrap();
        apply_drop(&mut tree, &file, &[0], 1);
        assert_eq!(visible_rows(&tree).len(), 2);
        toggle_open(&mut tree, &[0]);
        assert_eq!(visible_rows(&tree).len(), 1);
        assert_eq!(drop_zone(true, 5, 28), 0);
        assert_eq!(drop_zone(true, 14, 28), 1);
        assert_eq!(drop_zone(false, 2, 28), 0);
        assert_eq!(drop_zone(true, 22, 28), 2);
        assert_eq!(drop_zone(false, 20, 28), 2);
        let junk = vec![json!("group"), json!({"name": "Box", "children": [{"path": "C:\\f", "file": true}]})];
        let parsed = tree_from_values(&junk);
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].children[0].file);
    }

    fn jobs_and_desk() -> Vec<FavNode> {
        vec![
            FavNode {
                name: "Jobs".into(),
                path: String::new(),
                file: false,
                open: true,
                children: vec![FavNode::leaf(r"C:\a", "a", false), FavNode::leaf(r"C:\b", "b", false)],
            },
            FavNode {
                name: "Desk".into(),
                path: String::new(),
                file: false,
                open: true,
                children: vec![FavNode::leaf(r"C:\c", "c", false), FavNode::leaf(r"C:\d", "d", true)],
            },
        ]
    }

    fn labels(tree: &[FavNode]) -> String {
        visible_rows(tree).into_iter().map(|r| r.name).collect::<Vec<_>>().join("/")
    }

    #[test]
    fn reorders_groups_leaves_and_moves_a_leaf_between_groups() {
        let mut tree = jobs_and_desk();
        assert_eq!(apply_drop(&mut tree, &[0], &[1], 2), Some(vec![1]), "group moves after the other group");
        assert_eq!(labels(&tree), "Desk/c/d/Jobs/a/b");
        assert!(apply_drop(&mut tree, &[1], &[0], 0).is_some(), "group moves back before the other group");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");

        assert!(apply_drop(&mut tree, &[0, 0], &[0, 1], 2).is_some(), "leaf moves after its sibling");
        assert_eq!(labels(&tree), "Jobs/b/a/Desk/c/d");
        assert!(apply_drop(&mut tree, &[0, 1], &[0, 0], 0).is_some(), "leaf moves back before its sibling");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");

        assert!(apply_drop(&mut tree, &[0, 0], &[1], 1).is_some(), "leaf appends into the other group");
        assert_eq!(labels(&tree), "Jobs/b/Desk/c/d/a");
        assert_eq!(tree[1].children.last().map(|n| n.name.as_str()), Some("a"));
        assert!(apply_drop(&mut tree, &[1, 2], &[0, 0], 0).is_some(), "leaf moves to the front of the first group");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");
        assert!(apply_drop(&mut tree, &[0, 1], &[1, 0], 2).is_some(), "leaf lands between the other group's leaves");
        assert_eq!(labels(&tree), "Jobs/a/Desk/c/b/d");
        assert_eq!(tree[1].children.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["c", "b", "d"]);
    }

    #[test]
    fn after_line_follows_an_open_group_and_a_drop_opens_it() {
        let tree = jobs_and_desk();
        let rows = visible_rows(&tree);
        assert_eq!(after_row(&rows, 0), 2, "open group: line sits under its last child");
        assert_eq!(after_row(&rows, 1), 1, "leaf: line stays on that row");
        assert_eq!(after_row(&rows, 3), 5);
        let mut closed = tree.clone();
        closed[0].open = false;
        let rows = visible_rows(&closed);
        assert_eq!(after_row(&rows, 0), 0, "collapsed group: the next row is already the sibling");

        closed[1].open = false;
        let jobs = vec![0usize];
        let desk = vec![1usize];
        assert!(apply_drop(&mut closed, &jobs, &desk, 1).is_some());
        reveal_drop(&mut closed, &jobs, &desk, 1);
        assert_eq!(closed[0].name, "Desk");
        assert!(closed[0].open, "the group that received the drop is expanded");
        assert!(closed[0].children.iter().any(|n| n.name == "Jobs"));
    }

    fn cand(path: &str, name: &str, pinned: bool, folder: bool) -> PinCand {
        PinCand { path: path.into(), name: name.into(), pinned, folder }
    }

    #[test]
    fn pinned_folders_keep_shell_order_and_drop_files() {
        let pins = pinned_folders(&[
            cand(r"D:\z", "Zed", true, true),
            cand(r"C:\file.txt", "note", true, false),
            cand(r"C:\a", "A", false, true),
            cand("", "This PC", true, true),
            cand(r"  ", "blank", true, true),
            cand(r"d:\z\", "again", true, true),
            cand(r"E:\c", "", true, true),
            cand(r"C:\", "Drive", true, false),
        ]);
        assert_eq!(pins.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), [r"D:\z", r"E:\c"]);
        assert_eq!(pins[0].name, "Zed");
        assert_eq!(pins[1].name, "c", "an empty shell name falls back to the folder name");
        assert_eq!(tidy_path(r"C:\"), r"C:\");
        assert_eq!(tidy_path("C:"), r"C:\");
        assert!(pin_drop_paths(&[(r"C:\a".into(), true), (r"c:\a\".into(), true)]).unwrap() == vec![r"C:\a".to_string()]);
        assert!(pin_drop_paths(&[(r"C:\a".into(), true), (r"C:\b.txt".into(), false)]).is_none());
        assert!(pin_drop_paths(&[(r"C:\b.txt".into(), false)]).is_none());
        assert!(pin_drop_paths(&[(String::new(), true)]).is_none());
        assert!(pin_drop_paths(&[]).is_none());
    }

    #[test]
    fn explorer_pins_lead_the_list_and_are_not_tree_nodes() {
        let tree = jobs_and_desk();
        let pins = pinned_folders(&[cand(r"D:\z", "Zed", true, true), cand(r"E:\c", "Cee", true, true)]);
        let rows = compose_side(&tree, true, &pins, "Explorer pins");
        assert!(rows[0].pins && rows[0].group && rows[0].open && rows[0].id.is_empty());
        assert_eq!(rows[0].name, "Explorer pins");
        assert_eq!((rows[1].path.as_str(), rows[1].depth, rows[1].file, rows[1].pins), (r"D:\z", 1, false, true));
        assert_eq!(rows[2].path, r"E:\c");
        let jobs = rows.iter().find(|r| r.name == "Jobs").unwrap();
        assert!(!jobs.pins && jobs.id == vec![0] && jobs.depth == 0);
        let closed = compose_side(&tree, false, &pins, "Explorer pins");
        assert!(closed.iter().filter(|r| r.pins).all(|r| r.group));
        assert_eq!(closed[1].name, "Jobs");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");
    }

    #[test]
    fn pin_rows_do_not_reorder_and_a_folder_drop_pins() {
        assert_eq!(side_act(true, false, false, false, true), SideAct::None, "a pin cannot be reordered inside the group");
        assert_eq!(side_act(true, true, false, true, false), SideAct::None, "the pins header is not dragged into favourites");
        assert_eq!(side_act(false, false, true, false, true), SideAct::None, "a file dropped on the group is refused");
        assert_eq!(side_act(false, true, false, true, true), SideAct::None, "a group is not a folder to pin");
        assert_eq!(side_act(false, false, false, false, true), SideAct::Pin);
        assert_eq!(side_act(true, false, false, false, false), SideAct::Copy);
        assert_eq!(side_act(false, false, false, false, false), SideAct::Move);
        assert_eq!(side_act(false, false, true, false, false), SideAct::Move);

        let now = Instant::now();
        let wait = Duration::from_secs(2);
        assert!(pins_due(None, now, wait));
        assert!(!pins_due(Some(now), now, wait));
        assert!(!pins_due(Some(now.checked_sub(Duration::from_millis(1999)).unwrap()), now, wait));
        assert!(pins_due(Some(now.checked_sub(Duration::from_secs(2)).unwrap()), now, wait));
    }

    #[test]
    fn copying_a_pin_into_favourites_leaves_the_original() {
        let mut tree = jobs_and_desk();
        let before = tree.clone();
        let id = copy_leaf(&mut tree, r"E:\pin\", "Pin", &[1], 1).unwrap();
        assert_eq!(id, vec![1, 2]);
        assert_eq!(tree[1].children.last().map(|n| n.path.as_str()), Some(r"E:\pin"));
        assert_eq!(tree[0], before[0], "the other group is untouched");
        assert!(copy_leaf(&mut tree, r"e:\pin", "Pin", &[0], 1).is_none(), "the same folder is not added twice");
        assert!(copy_leaf(&mut tree, r"E:\other", "O", &[1, 0], 1).is_none(), "a leaf has no inside");
        let id = copy_leaf(&mut tree, r"E:\other", "", &[0, 0], 0).unwrap();
        assert_eq!(id, vec![0, 0]);
        assert_eq!(tree[0].children[0].path, r"E:\other");
        assert_eq!(tree[0].children[1].name, "a");
    }
}
