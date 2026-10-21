// Favourites are a tree of groups and leaves. Leaves are folders or files.
// The flat `favs` list is the leaves, depth-first, so the star, the settings
// page, and an old `fav=` line keep working. Groups live only in the tree.

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
    apply_drop(tree, &src, &dst, zone)
}

/// `zone`: 0 before `onto`, 1 into `onto` when it is a group, 2 after `onto`.
pub(crate) fn apply_drop(tree: &mut Vec<FavNode>, from: &[usize], onto: &[usize], zone: u8) -> bool {
    if from.is_empty() || onto.is_empty() || from == onto || onto.starts_with(from) {
        return false;
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
    let Some(node) = take_node(tree, from) else { return false };
    let parent = adjust(&parent, from).unwrap_or(parent);
    insert_child(tree, &parent, index, node).is_some()
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
        assert!(apply_drop(&mut tree, &file, &jobs, 1));
        assert!(contains_path(&tree, r"C:\a\note.txt"));
        let rows = visible_rows(&tree);
        let nested = rows.iter().find(|r| r.file).unwrap();
        assert_eq!(nested.depth, 1);
        assert_eq!(nested.path, r"C:\a\note.txt");
        let values = tree_to_values(&tree);
        let back = tree_from_values(&values);
        assert_eq!(back, tree);
        assert!(back.iter().any(|n| n.is_group() && n.children.iter().any(|c| c.file)));
        assert!(!apply_drop(&mut tree, &jobs, &jobs, 1));
        let child = visible_rows(&tree).iter().find(|r| r.file).unwrap().id.clone();
        assert!(!apply_drop(&mut tree, &jobs, &child, 1));
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
        let junk = vec![json!("group"), json!({"name": "Box", "children": [{"path": "C:\\f", "file": true}]})];
        let parsed = tree_from_values(&junk);
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].children[0].file);
    }
}
