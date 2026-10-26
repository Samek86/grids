// Pure WinMerge rules: where the program might be, which panes and files to compare,
// and the command line. Registry and CreateProcess stay out of this file.

pub(crate) const DOWNLOAD: &str = "https://winmerge.org/";
pub(crate) const EXE_NAME: &str = "WinMergeU.exe";
/// HKLM and HKCU, native and WOW6432Node.
pub(crate) const REG_SUB: &str = r"Software\Thingamahoochie\WinMerge";
pub(crate) const REG_VALUE: &str = "Executable";
pub(crate) const APP_PATHS: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\WinMergeU.exe";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Where {
    /// The Settings path exists (or is a folder that contains WinMergeU.exe).
    Custom(String),
    /// The Settings path does not exist. `detected` is an auto-detected exe, if one does.
    CustomMissing { custom: String, detected: Option<String> },
    Detected(String),
    NotInstalled,
}

impl Where {
    pub(crate) fn exe(&self) -> Option<&str> {
        match self {
            Where::Custom(p) | Where::Detected(p) => Some(p),
            Where::CustomMissing { detected, .. } => detected.as_deref(),
            Where::NotInstalled => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Folders,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Side {
    pub(crate) path: String,
    pub(crate) desc: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Hidden,
    Command,
    Submenu(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MenuPlan {
    pub(crate) folders: Shape,
    pub(crate) two_files: bool,
    pub(crate) same_name: Shape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FolderTarget {
    Ready { left: String, right_index: usize, right: String },
    NotAFolder,
    NeedAnother,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FilePick {
    None,
    One(String),
    Two(String, String),
}

/// Unquote a registry or dialog path. A trailing `,0` icon index is dropped.
pub(crate) fn clean_exe(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let s = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        s
    };
    let s = s.split(',').next().unwrap_or(s).trim().trim_matches('"');
    (!s.is_empty()).then(|| s.to_string())
}

/// A path that already names an exe stays. A directory becomes `Dir\WinMergeU.exe`.
pub(crate) fn as_exe(path: &str) -> String {
    let p = path.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    if p.is_empty() {
        return String::new();
    }
    if p.to_ascii_lowercase().ends_with(".exe") {
        p.to_string()
    } else {
        format!(r"{p}\{EXE_NAME}")
    }
}

/// Registry values, then App Paths, then Program Files, then PATH. Duplicates are dropped.
pub(crate) fn install_candidates(reg: &[String], app_paths: &[String], program_files: &str, program_files_x86: &str, path_env: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in reg.iter().chain(app_paths) {
        push_exe(&mut out, p);
    }
    let mut roots = Vec::new();
    for r in [program_files, program_files_x86, r"C:\Program Files", r"C:\Program Files (x86)"] {
        let r = r.trim().trim_end_matches(['\\', '/']);
        if !r.is_empty() {
            push_unique(&mut roots, r);
        }
    }
    for root in roots {
        push_exe(&mut out, &format!(r"{root}\WinMerge\{EXE_NAME}"));
    }
    for dir in path_env.split(';') {
        let dir = dir.trim().trim_matches('"').trim_end_matches(['\\', '/']);
        if dir.is_empty() || dir == "." {
            continue;
        }
        push_exe(&mut out, &format!(r"{dir}\{EXE_NAME}"));
    }
    out
}

/// Custom path wins when that file exists. Otherwise the first candidate that exists.
pub(crate) fn resolve(custom: &str, candidates: &[String], exists: impl Fn(&str) -> bool) -> Where {
    let custom_raw = clean_exe(custom).unwrap_or_default();
    if !custom_raw.is_empty() {
        let exe = as_exe(&custom_raw);
        if !exe.is_empty() && exists(&exe) {
            return Where::Custom(exe);
        }
        let detected = candidates.iter().find(|c| exists(c)).cloned();
        return Where::CustomMissing { custom: custom_raw, detected };
    }
    match candidates.iter().find(|c| exists(c)) {
        Some(p) => Where::Detected(p.clone()),
        None => Where::NotInstalled,
    }
}

/// Visible filesystem folders, in pane order: `(index, path)`.
pub(crate) fn folder_panes(paths: &[String], visible: usize) -> Vec<(usize, String)> {
    paths.iter().take(visible).enumerate().filter(|(_, p)| is_folder_path(p)).map(|(i, p)| (i, p.clone())).collect()
}

pub(crate) fn is_folder_path(path: &str) -> bool {
    path.get(1..3) == Some(":\\") || path.starts_with(r"\\")
}

/// Other folder panes, starting with the one after `active` and wrapping.
pub(crate) fn others_after(active: usize, panes: &[(usize, String)]) -> Vec<usize> {
    let Some(pos) = panes.iter().position(|(i, _)| *i == active) else {
        return Vec::new();
    };
    let n = panes.len();
    (1..n).map(|k| panes[(pos + k) % n].0).collect()
}

pub(crate) fn folder_target(active: usize, panes: &[(usize, String)], chosen: Option<usize>) -> FolderTarget {
    let Some((_, left)) = panes.iter().find(|(i, _)| *i == active) else {
        return FolderTarget::NotAFolder;
    };
    let others = others_after(active, panes);
    if others.is_empty() {
        return FolderTarget::NeedAnother;
    }
    let idx = match chosen {
        Some(i) if others.contains(&i) => i,
        Some(_) => return FolderTarget::NeedAnother,
        None => others[0],
    };
    let right = panes.iter().find(|(i, _)| *i == idx).map(|(_, p)| p.clone()).unwrap_or_default();
    FolderTarget::Ready { left: left.clone(), right_index: idx, right }
}

/// `selected_files` counts files. `selected_total` counts every selected item.
pub(crate) fn menu_plan(active_is_folder: bool, others: &[usize], selected_files: usize, selected_total: usize) -> MenuPlan {
    let folders = shape_of(active_is_folder && !others.is_empty(), others);
    let two_files = selected_total == 2 && selected_files == 2;
    let same_name = shape_of(selected_total == 1 && selected_files == 1 && !others.is_empty(), others);
    MenuPlan { folders, two_files, same_name }
}

pub(crate) fn picked_files(items: &[(String, bool)]) -> FilePick {
    let files: Vec<&str> = items.iter().filter(|(_, file)| *file).map(|(p, _)| p.as_str()).collect();
    if items.len() == 2 && files.len() == 2 {
        FilePick::Two(files[0].to_string(), files[1].to_string())
    } else if items.len() == 1 && files.len() == 1 {
        FilePick::One(files[0].to_string())
    } else {
        FilePick::None
    }
}

pub(crate) fn file_label(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    t.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

/// Last path component. A drive root stays `C:\`.
pub(crate) fn folder_label(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.as_bytes().get(1) == Some(&b':') {
        return format!(r"{t}\");
    }
    t.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

pub(crate) fn same_named(file: &str, folder: &str) -> String {
    let name = file_label(file);
    let folder = folder.trim_end_matches(['\\', '/']);
    if folder.is_empty() {
        name
    } else {
        format!(r"{folder}\{name}")
    }
}

pub(crate) fn sides_folders(left: &str, right: &str) -> (Side, Side) {
    (Side { desc: folder_label(left), path: left.to_string() }, Side { desc: folder_label(right), path: right.to_string() })
}

pub(crate) fn sides_files(left: &str, right: &str) -> (Side, Side) {
    (Side { desc: file_label(left), path: left.to_string() }, Side { desc: file_label(right), path: right.to_string() })
}

/// `/u` keeps the paths out of WinMerge's recent list. `/r` is folders only.
/// `/dl` and `/dr` are the pane folder names (or the file names, for two selected files).
pub(crate) fn arg_list(mode: Mode, left: &Side, right: &Side) -> Vec<String> {
    let mut args = vec!["/u".to_string()];
    if mode == Mode::Folders {
        args.push("/r".into());
    }
    args.push("/dl".into());
    args.push(left.desc.clone());
    args.push("/dr".into());
    args.push(right.desc.clone());
    args.push(left.path.clone());
    args.push(right.path.clone());
    args
}

/// Quote one argument the way `CommandLineToArgvW` reads it. A trailing `\` is doubled
/// so it does not escape the closing quote (`C:\` would otherwise become `C:`).
pub(crate) fn quote_arg(s: &str) -> String {
    let mut out = String::from("\"");
    let mut bs = 0usize;
    for c in s.chars() {
        match c {
            '\\' => {
                out.push('\\');
                bs += 1;
            }
            '"' => {
                for _ in 0..=bs {
                    out.push('\\');
                }
                out.push('"');
                bs = 0;
            }
            c => {
                out.push(c);
                bs = 0;
            }
        }
    }
    for _ in 0..bs {
        out.push('\\');
    }
    out.push('"');
    out
}

pub(crate) fn command_params(mode: Mode, left: &Side, right: &Side) -> String {
    arg_list(mode, left, right)
        .into_iter()
        .map(|s| if is_switch(&s) { s } else { quote_arg(&s) })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_switch(s: &str) -> bool {
    matches!(s, "/u" | "/r" | "/dl" | "/dr")
}

fn shape_of(show: bool, others: &[usize]) -> Shape {
    if !show {
        Shape::Hidden
    } else if others.len() == 1 {
        Shape::Command
    } else {
        Shape::Submenu(others.to_vec())
    }
}

fn push_exe(out: &mut Vec<String>, raw: &str) {
    let Some(clean) = clean_exe(raw) else { return };
    let exe = as_exe(&clean);
    if !exe.is_empty() {
        push_unique(out, &exe);
    }
}

fn push_unique(out: &mut Vec<String>, path: &str) {
    let path = path.trim().trim_matches('"');
    if path.is_empty() || out.iter().any(|e| e.eq_ignore_ascii_case(path)) {
        return;
    }
    out.push(path.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_match_the_winmerge_installer() {
        assert_eq!(REG_SUB, r"Software\Thingamahoochie\WinMerge");
        assert_eq!(REG_VALUE, "Executable");
        assert!(APP_PATHS.ends_with(r"App Paths\WinMergeU.exe"));
        assert_eq!(EXE_NAME, "WinMergeU.exe");
        assert_eq!(DOWNLOAD, "https://winmerge.org/");
    }

    #[test]
    fn clean_exe_unquotes_and_drops_an_icon_index() {
        assert_eq!(clean_exe(r#""C:\Program Files\WinMerge\WinMergeU.exe""#).as_deref(), Some(r"C:\Program Files\WinMerge\WinMergeU.exe"));
        assert_eq!(clean_exe(r"C:\Program Files\WinMerge\WinMergeU.exe,0").as_deref(), Some(r"C:\Program Files\WinMerge\WinMergeU.exe"));
        assert_eq!(as_exe(r"C:\Program Files\WinMerge\"), r"C:\Program Files\WinMerge\WinMergeU.exe");
        assert_eq!(as_exe(r"D:\Apps\WinMergeU.exe"), r"D:\Apps\WinMergeU.exe");
        assert!(clean_exe("  ").is_none());
        assert!(as_exe("").is_empty());
    }

    #[test]
    fn candidates_follow_registry_app_paths_program_files_then_path() {
        let reg = vec![r"C:\Reg\WinMergeU.exe".into(), r"C:\RegDir".into()];
        let apps = vec![r#""D:\App Paths\WinMergeU.exe""#.into()];
        let c = install_candidates(&reg, &apps, r"C:\Program Files", r"C:\Program Files (x86)", r"D:\bin;.;");
        assert_eq!(c[0], r"C:\Reg\WinMergeU.exe");
        assert_eq!(c[1], r"C:\RegDir\WinMergeU.exe");
        assert_eq!(c[2], r"D:\App Paths\WinMergeU.exe");
        let pf = c.iter().position(|p| p == r"C:\Program Files\WinMerge\WinMergeU.exe").unwrap();
        let app = c.iter().position(|p| p == r"D:\App Paths\WinMergeU.exe").unwrap();
        let path = c.iter().position(|p| p == r"D:\bin\WinMergeU.exe").unwrap();
        assert!(app < pf && pf < path);
        assert!(c.iter().any(|p| p == r"C:\Program Files (x86)\WinMerge\WinMergeU.exe"));
        assert_eq!(c.iter().filter(|p| p.eq_ignore_ascii_case(r"C:\Program Files\WinMerge\WinMergeU.exe")).count(), 1);
        assert!(!c.iter().any(|p| p.ends_with(r".\WinMergeU.exe")));
    }

    #[test]
    fn resolve_prefers_a_custom_path_and_falls_back_when_it_is_missing() {
        let cands = vec![r"C:\Program Files\WinMerge\WinMergeU.exe".into()];
        let there = |p: &str| p.starts_with(r"C:\Program Files") || p.starts_with(r"D:\Apps");
        assert_eq!(resolve("", &cands, there), Where::Detected(cands[0].clone()));
        assert_eq!(resolve(r"D:\Apps\WinMergeU.exe", &cands, there), Where::Custom(r"D:\Apps\WinMergeU.exe".into()));
        assert_eq!(
            resolve(r"D:\Apps\WinMerge\", &cands, |p| p == r"D:\Apps\WinMerge\WinMergeU.exe"),
            Where::Custom(r"D:\Apps\WinMerge\WinMergeU.exe".into())
        );
        match resolve(r"D:\missing\WinMergeU.exe", &cands, there) {
            Where::CustomMissing { custom, detected: Some(p) } => {
                assert_eq!(custom, r"D:\missing\WinMergeU.exe");
                assert_eq!(p, cands[0]);
                assert_eq!(Where::CustomMissing { custom, detected: Some(p.clone()) }.exe(), Some(cands[0].as_str()));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(resolve(r"D:\missing\WinMergeU.exe", &cands, |_| false), Where::CustomMissing { custom: r"D:\missing\WinMergeU.exe".into(), detected: None });
        assert_eq!(resolve("", &cands, |_| false), Where::NotInstalled);
        assert!(Where::NotInstalled.exe().is_none());
    }

    #[test]
    fn next_pane_wraps_and_a_choice_must_be_one_of_the_others() {
        let panes = vec![(0usize, r"C:\A".into()), (1, r"D:\B".into()), (3, r"E:\C".into())];
        assert_eq!(others_after(0, &panes), vec![1, 3]);
        assert_eq!(others_after(1, &panes), vec![3, 0]);
        assert_eq!(others_after(3, &panes), vec![0, 1]);
        assert!(others_after(2, &panes).is_empty());
        match folder_target(0, &panes, None) {
            FolderTarget::Ready { left, right_index, right } => {
                assert_eq!((left.as_str(), right_index, right.as_str()), (r"C:\A", 1, r"D:\B"));
            }
            other => panic!("{other:?}"),
        }
        match folder_target(3, &panes, Some(1)) {
            FolderTarget::Ready { right_index: 1, right, .. } => assert_eq!(right, r"D:\B"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(folder_target(0, &panes, Some(9)), FolderTarget::NeedAnother));
        assert!(matches!(folder_target(0, &[(0, r"C:\Only".into())], None), FolderTarget::NeedAnother));
        assert!(matches!(folder_target(4, &panes, None), FolderTarget::NotAFolder));
        let visible = folder_panes(&[r"C:\A".into(), "::{this-pc}".into(), r"\\srv\share".into(), String::new()], 3);
        assert_eq!(visible, vec![(0, r"C:\A".into()), (2, r"\\srv\share".into())]);
    }

    #[test]
    fn menu_plan_is_a_command_for_one_other_pane_and_a_submenu_for_more() {
        let one = menu_plan(true, &[1], 0, 0);
        assert_eq!(one.folders, Shape::Command);
        assert!(!one.two_files);
        assert_eq!(one.same_name, Shape::Hidden);

        let many = menu_plan(true, &[1, 2], 1, 1);
        assert_eq!(many.folders, Shape::Submenu(vec![1, 2]));
        assert_eq!(many.same_name, Shape::Submenu(vec![1, 2]));
        assert!(!many.two_files);

        let files = menu_plan(true, &[1], 2, 2);
        assert!(files.two_files);
        assert_eq!(files.same_name, Shape::Hidden);
        assert_eq!(files.folders, Shape::Command);

        let lone = menu_plan(false, &[], 2, 3);
        assert_eq!(lone.folders, Shape::Hidden);
        assert!(!lone.two_files); // a folder in the selection is not two files
        assert_eq!(picked_files(&[(r"C:\a.txt".into(), true), (r"C:\b.txt".into(), true)]), FilePick::Two(r"C:\a.txt".into(), r"C:\b.txt".into()));
        assert_eq!(picked_files(&[(r"C:\a.txt".into(), true)]), FilePick::One(r"C:\a.txt".into()));
        assert_eq!(picked_files(&[(r"C:\a.txt".into(), true), (r"C:\dir".into(), false)]), FilePick::None);
    }

    #[test]
    fn labels_and_the_same_named_file_in_the_other_pane() {
        assert_eq!(folder_label(r"C:\Users\Docs\"), "Docs");
        assert_eq!(folder_label(r"C:\"), r"C:\");
        assert_eq!(folder_label(r"\\srv\share\Photos"), "Photos");
        assert_eq!(file_label(r"D:\B\note.txt"), "note.txt");
        assert_eq!(same_named(r"C:\A\note.txt", r"D:\B\"), r"D:\B\note.txt");
        assert_eq!(same_named(r"C:\A\note.txt", r"C:\"), r"C:\note.txt");
        assert_eq!(same_named(r"C:\A\note.txt", r"\\srv\share"), r"\\srv\share\note.txt");
        let (l, r) = sides_folders(r"C:\Docs", r"D:\Backup");
        assert_eq!((l.desc.as_str(), r.desc.as_str()), ("Docs", "Backup"));
    }

    #[test]
    fn command_line_is_recursive_for_folders_and_quotes_a_drive_root() {
        let (l, r) = sides_folders(r"C:\Docs", r"D:\Backup");
        let folders = command_params(Mode::Folders, &l, &r);
        assert_eq!(folders, r#"/u /r /dl "Docs" /dr "Backup" "C:\Docs" "D:\Backup""#);
        assert!(arg_list(Mode::Folders, &l, &r).contains(&"/r".to_string()));
        let (a, b) = sides_files(r"C:\Docs\a.txt", r"C:\Docs\b.txt");
        let files = command_params(Mode::Files, &a, &b);
        assert_eq!(files, r#"/u /dl "a.txt" /dr "b.txt" "C:\Docs\a.txt" "C:\Docs\b.txt""#);
        assert!(!files.contains(" /r "));
        let (root, work) = sides_folders(r"C:\", r"D:\Work");
        assert_eq!(command_params(Mode::Folders, &root, &work), r#"/u /r /dl "C:\\" /dr "Work" "C:\\" "D:\Work""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(""), r#""""#);
    }
}
