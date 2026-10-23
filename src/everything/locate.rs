// Where Everything.exe might be, and how to describe it. Registry and PATH reads happen later, on demand.

pub(crate) const CLASS_14: &str = "EVERYTHING_TASKBAR_NOTIFICATION";
pub(crate) const CLASS_15A: &str = "EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)";
pub(crate) const DOWNLOADS: &str = "https://www.voidtools.com/downloads/";

pub(crate) fn window_classes() -> [&'static str; 2] {
    [CLASS_14, CLASS_15A]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) revision: u32,
    pub(crate) build: u32,
}

impl Version {
    pub(crate) fn from_parts(major: u32, minor: u32, revision: u32, build: u32) -> Option<Self> {
        if major == 0 || major > 9 {
            None
        } else {
            Some(Version { major, minor, revision, build })
        }
    }

    pub(crate) fn label(self) -> String {
        format!("{}.{}.{}.{}", self.major, self.minor, self.revision, self.build)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    NotInstalled,
    InstalledStopped,
    Running { version: Option<Version> },
}

pub(crate) fn classify(installed: bool, running: bool, version: Option<Version>) -> Presence {
    if running {
        Presence::Running { version }
    } else if installed {
        Presence::InstalledStopped
    } else {
        Presence::NotInstalled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallPlan {
    Winget,
    Browser,
}

pub(crate) fn install_plan(winget: bool) -> InstallPlan {
    if winget { InstallPlan::Winget } else { InstallPlan::Browser }
}

/// Uninstall-key fields. A match is voidtools Everything, not some other product with the word in its name.
pub(crate) fn exe_from_uninstall(display_name: &str, publisher: &str, install_location: &str, display_icon: &str, uninstall: &str) -> Option<String> {
    if !looks_like_everything(display_name, publisher, display_icon, uninstall, install_location) {
        return None;
    }
    extract_everything_exe(display_icon)
        .or_else(|| sibling_everything(uninstall))
        .or_else(|| exe_in_dir(install_location))
        .or_else(|| extract_everything_exe(uninstall))
}

pub(crate) fn install_candidates(program_files: &str, program_files_x86: &str, path_env: &str, extra: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for e in extra {
        push_unique(&mut out, e);
    }
    let mut roots = Vec::new();
    for r in [program_files, program_files_x86, r"C:\Program Files", r"C:\Program Files (x86)"] {
        let r = r.trim().trim_end_matches(['\\', '/']);
        if !r.is_empty() {
            push_unique(&mut roots, r);
        }
    }
    for root in roots {
        push_unique(&mut out, &format!(r"{root}\Everything\Everything.exe"));
        push_unique(&mut out, &format!(r"{root}\Everything 1.5a\Everything.exe"));
    }
    for dir in path_env.split(';') {
        let dir = dir.trim().trim_matches('"').trim_end_matches(['\\', '/']);
        if dir.is_empty() || dir == "." {
            continue;
        }
        push_unique(&mut out, &format!(r"{dir}\Everything.exe"));
    }
    out
}

pub(crate) fn first_present(cands: &[String], exists: impl Fn(&str) -> bool) -> Option<String> {
    cands.iter().find(|c| exists(c)).cloned()
}

fn looks_like_everything(name: &str, publisher: &str, icon: &str, uninstall: &str, location: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    let name_ok = name == "everything" || name.starts_with("everything ");
    if !name_ok {
        return false;
    }
    if publisher.to_ascii_lowercase().contains("voidtools") {
        return true;
    }
    let blob = format!("{icon}\n{uninstall}\n{location}").to_ascii_lowercase();
    blob.contains("voidtools") || blob.contains("everything.exe") || blob.contains("\\everything") || blob.contains("/everything")
}

fn extract_everything_exe(s: &str) -> Option<String> {
    let unquoted = unquote(s);
    let lower = unquoted.to_ascii_lowercase();
    let idx = lower.find("everything.exe")?;
    let path = unquoted[..idx + "everything.exe".len()].trim().trim_matches('"');
    drive_path(path)
}

fn sibling_everything(uninstall: &str) -> Option<String> {
    let unquoted = unquote(uninstall);
    let lower = unquoted.to_ascii_lowercase();
    if !lower.ends_with("uninstall.exe") {
        return None;
    }
    let dir = unquoted.rsplit_once(['\\', '/'])?.0;
    if !dir.to_ascii_lowercase().contains("everything") {
        return None;
    }
    Some(format!(r"{dir}\Everything.exe"))
}

fn exe_in_dir(dir: &str) -> Option<String> {
    let dir = dir.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    if dir.is_empty() {
        return None;
    }
    Some(format!(r"{dir}\Everything.exe"))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        s
    };
    s.split(',').next().unwrap_or(s).trim().trim_matches('"').to_string()
}

fn drive_path(path: &str) -> Option<String> {
    let path = path.trim().trim_matches('"');
    if let Some(i) = path.rfind(":\\") {
        let start = i.saturating_sub(1);
        if path.as_bytes().get(start).is_some_and(|c| c.is_ascii_alphabetic()) {
            return Some(path[start..].to_string());
        }
    }
    if let Some(i) = path.find("\\\\") {
        return Some(path[i..].to_string());
    }
    None
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
    fn classes_cover_1_4_and_the_1_5a_instance() {
        assert_eq!(window_classes(), [CLASS_14, "EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)"]);
    }

    #[test]
    fn presence_prefers_a_running_instance_and_versions_need_a_major() {
        let v = Version::from_parts(1, 4, 1, 1024).unwrap();
        assert_eq!(v.label(), "1.4.1.1024");
        assert!(Version::from_parts(0, 0, 0, 0).is_none());
        assert!(Version::from_parts(99, 0, 0, 0).is_none());
        assert_eq!(classify(false, false, None), Presence::NotInstalled);
        assert_eq!(classify(true, false, None), Presence::InstalledStopped);
        assert_eq!(classify(false, true, Some(v)), Presence::Running { version: Some(v) });
        assert_eq!(install_plan(true), InstallPlan::Winget);
        assert_eq!(install_plan(false), InstallPlan::Browser);
        assert!(DOWNLOADS.starts_with("https://www.voidtools.com/"));
    }

    #[test]
    fn uninstall_fields_point_at_everything_exe() {
        assert_eq!(
            exe_from_uninstall("Everything", "voidtools", "", r"C:\Program Files\Everything\Everything.exe,0", "").as_deref(),
            Some(r"C:\Program Files\Everything\Everything.exe")
        );
        assert_eq!(
            exe_from_uninstall("Everything 1.5a", "voidtools", r"C:\Program Files\Everything 1.5a\", "", "").as_deref(),
            Some(r"C:\Program Files\Everything 1.5a\Everything.exe")
        );
        assert_eq!(
            exe_from_uninstall("Everything", "", "", "", r#""C:\Program Files\Everything\Uninstall.exe""#).as_deref(),
            Some(r"C:\Program Files\Everything\Everything.exe")
        );
        assert!(exe_from_uninstall("Everything Search Tool", "Acme", "", "", "").is_none());
        assert!(exe_from_uninstall("Not Everything", "voidtools", r"C:\Everything", "", "").is_none());
    }

    #[test]
    fn candidates_try_known_paths_then_path_and_skip_duplicates() {
        let extra = vec![r"D:\Apps\Everything.exe".into()];
        let c = install_candidates(r"C:\Program Files", r"C:\Program Files (x86)", r"D:\bin;.", &extra);
        assert_eq!(c[0], r"D:\Apps\Everything.exe");
        assert!(c.iter().any(|p| p == r"C:\Program Files\Everything\Everything.exe"));
        assert!(c.iter().any(|p| p == r"C:\Program Files\Everything 1.5a\Everything.exe"));
        assert!(c.iter().any(|p| p == r"C:\Program Files (x86)\Everything\Everything.exe"));
        assert!(c.iter().any(|p| p == r"D:\bin\Everything.exe"));
        assert_eq!(c.iter().filter(|p| p.eq_ignore_ascii_case(r"C:\Program Files\Everything\Everything.exe")).count(), 1);
        let picked = first_present(&c, |p| p.starts_with(r"C:\Program Files\Everything\"));
        assert_eq!(picked.as_deref(), Some(r"C:\Program Files\Everything\Everything.exe"));
        assert!(first_present(&c, |_| false).is_none());
    }
}
