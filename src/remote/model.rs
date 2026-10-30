//! Server identity, URL parsing, UNC form, and the rclone command shape.
//! Nothing here writes a password into a path, a config value, or an argument.

use crate::session::{FavNode, ServerLink};

pub(crate) const KINDS: [&str; 4] = ["smb", "webdav", "sftp", "ftp"];
pub(crate) const KIND_LABELS: [&str; 4] = ["SMB", "WebDAV", "SFTP", "FTP"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub(crate) kind: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) remote: String,
    pub(crate) user: String,
    /// From `user:password@host` only. Never copied onto [`ServerLink`] or a path.
    pub(crate) password: String,
    pub(crate) tls: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) port_text: String,
    pub(crate) remote: String,
    pub(crate) user: String,
    pub(crate) tls: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Built {
    pub(crate) link: ServerLink,
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    Host,
    Port,
}

pub(crate) fn needs_rclone(kind: &str) -> bool {
    let k = kind.trim().to_ascii_lowercase();
    k == "sftp" || k == "ftp"
}

/// Ask once, and only when rclone or WinFsp is missing.
pub(crate) fn should_ask_tools(asked: bool, rclone: bool, winfsp: bool) -> bool {
    !asked && (!rclone || !winfsp)
}

pub(crate) fn winget_args(id: &str) -> Vec<String> {
    ["install", "--id", id, "-e", "--disable-interactivity", "--accept-package-agreements", "--accept-source-agreements"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(crate) const WINGET_RCLONE: &str = "Rclone.Rclone";
pub(crate) const WINGET_WINFSP: &str = "WinFsp.WinFsp";

pub(crate) fn effective_port(kind: &str, port: u16, tls: bool) -> u16 {
    if port != 0 {
        return port;
    }
    match kind {
        "webdav" => {
            if tls {
                443
            } else {
                80
            }
        }
        "sftp" => 22,
        "ftp" => 21,
        _ => 445,
    }
}

pub(crate) fn mount_key(link: &ServerLink) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        link.kind.to_ascii_lowercase(),
        link.host.trim().to_ascii_lowercase(),
        effective_port(&link.kind, link.port, link.tls),
        link.user.trim().to_ascii_lowercase(),
        tidy_remote(&link.kind, &link.remote).to_ascii_lowercase()
    )
}

/// Credential Manager target. The password is not part of the name.
pub(crate) fn cred_target(link: &ServerLink) -> String {
    format!(
        "Maguro:{}:{}@{}:{}",
        link.kind.to_ascii_lowercase(),
        link.user.trim(),
        link.host.trim().to_ascii_lowercase(),
        effective_port(&link.kind, link.port, link.tls)
    )
}

pub(crate) fn tidy_remote(kind: &str, raw: &str) -> String {
    let t = raw.trim().trim_matches(|c| c == '\r' || c == '\n');
    if kind == "smb" {
        t.trim_matches(['\\', '/']).replace('/', "\\")
    } else {
        let t = t.replace('\\', "/");
        let keep_root = t.starts_with('/');
        let body = t.trim_matches('/');
        if body.is_empty() {
            String::new()
        } else if keep_root {
            format!("/{body}")
        } else {
            body.to_string()
        }
    }
}

/// `\\host\share\rest`, or the WebDAV redirector form `\\host@SSL\DavWWWRoot\path`.
pub(crate) fn unc_for(link: &ServerLink) -> String {
    let host = link.host.trim().trim_start_matches(['\\', '/']);
    let remote = tidy_remote(&link.kind, &link.remote);
    if link.kind == "webdav" {
        let port = link.port;
        let tail = remote.trim_start_matches('/').replace('/', "\\");
        let root = if link.tls {
            if port == 0 || port == 443 {
                format!(r"\\{host}@SSL\DavWWWRoot")
            } else {
                format!(r"\\{host}@SSL@{port}\DavWWWRoot")
            }
        } else if port == 0 || port == 80 {
            format!(r"\\{host}\DavWWWRoot")
        } else {
            format!(r"\\{host}@{port}\DavWWWRoot")
        };
        if tail.is_empty() {
            root
        } else {
            format!(r"{root}\{tail}")
        }
    } else if remote.is_empty() {
        format!(r"\\{host}")
    } else {
        format!(r"\\{host}\{remote}")
    }
}

/// Share root `WNetAddConnection2` accepts: `\\server\share` or `\\host@SSL\DavWWWRoot`.
pub(crate) fn share_root(unc: &str) -> String {
    let rest = unc.trim().trim_start_matches(['\\', '/']);
    let mut parts = rest.split(['\\', '/']).filter(|p| !p.is_empty());
    let server = parts.next().unwrap_or("");
    let share = parts.next().unwrap_or("");
    if share.is_empty() {
        format!(r"\\{server}")
    } else {
        format!(r"\\{server}\{share}")
    }
}

pub(crate) fn canonical_path(link: &ServerLink) -> String {
    match link.kind.as_str() {
        "smb" | "webdav" => unc_for(link),
        "sftp" | "ftp" => {
            let port = link.port;
            let host = link.host.trim();
            let user = link.user.trim();
            let remote = tidy_remote(&link.kind, &link.remote);
            let auth = if user.is_empty() { String::new() } else { format!("{user}@") };
            let port = if port == 0 { String::new() } else { format!(":{port}") };
            let path = if remote.is_empty() {
                String::new()
            } else if remote.starts_with('/') {
                remote
            } else {
                format!("/{remote}")
            };
            format!("{}://{auth}{host}{port}{path}", link.kind)
        }
        _ => String::new(),
    }
}

pub(crate) fn display_name(name: &str, link: &ServerLink) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return name.chars().take(80).collect();
    }
    let host = link.host.trim();
    let user = link.user.trim();
    if user.is_empty() {
        host.to_string()
    } else {
        format!("{user}@{host}")
    }
}

pub(crate) fn normalize(fields: &Fields) -> Result<Built, Fail> {
    let kind = fields.kind.trim().to_ascii_lowercase();
    if !KINDS.contains(&kind.as_str()) {
        return Err(Fail::Host);
    }
    let host = fields.host.trim().trim_start_matches(['\\', '/']).to_string();
    if host.is_empty() || host.contains(['\r', '\n', ' ']) {
        return Err(Fail::Host);
    }
    let port = if fields.port_text.trim().is_empty() {
        0
    } else {
        match fields.port_text.trim().parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => return Err(Fail::Port),
        }
    };
    let tls = kind == "webdav" && fields.tls;
    let remote = tidy_remote(&kind, &fields.remote);
    let user = fields.user.trim().to_string();
    if user.contains(['\r', '\n']) || remote.contains(['\r', '\n']) {
        return Err(Fail::Host);
    }
    let link = ServerLink { kind, host, port, remote, user, tls };
    let path = canonical_path(&link);
    if path.is_empty() {
        return Err(Fail::Host);
    }
    let name = display_name(&fields.name, &link);
    Ok(Built { link, name, path })
}

pub(crate) fn parse_remote_url(raw: &str) -> Option<Parsed> {
    let s = raw.trim();
    let (kind, tls, rest) = if let Some(r) = strip_scheme(s, "sftp") {
        ("sftp", false, r)
    } else if let Some(r) = strip_scheme(s, "ftp") {
        ("ftp", false, r)
    } else if let Some(r) = strip_scheme(s, "smb") {
        ("smb", false, r)
    } else if let Some(r) = strip_scheme(s, "webdavs") {
        ("webdav", true, r)
    } else if let Some(r) = strip_scheme(s, "webdav") {
        ("webdav", false, r)
    } else {
        return None;
    };
    if rest.is_empty() || rest.contains([' ', '\r', '\n']) {
        return None;
    }
    let (user, password, hostpath) = split_userinfo(rest);
    let (hostport, remote_raw) = match hostpath.find('/') {
        Some(i) => (&hostpath[..i], &hostpath[i..]),
        None => (hostpath.as_str(), ""),
    };
    if hostport.is_empty() {
        return None;
    }
    let (host, port) = split_host_port(hostport)?;
    if host.is_empty() {
        return None;
    }
    Some(Parsed {
        kind: kind.to_string(),
        host,
        port,
        remote: tidy_remote(kind, remote_raw),
        user,
        password,
        tls: kind == "webdav" && tls,
    })
}

fn strip_scheme<'a>(s: &'a str, scheme: &str) -> Option<&'a str> {
    let prefix = format!("{scheme}://");
    s.get(..prefix.len()).filter(|p| p.eq_ignore_ascii_case(&prefix))?;
    s.get(prefix.len()..)
}

fn split_userinfo(rest: &str) -> (String, String, String) {
    // The last '@' splits userinfo from the host, so a password may contain '@'.
    let Some(at) = rest.rfind('@') else {
        return (String::new(), String::new(), rest.to_string());
    };
    let (info, host) = rest.split_at(at);
    let host = host.trim_start_matches('@').to_string();
    let (user, pass) = match info.split_once(':') {
        Some((u, p)) => (u.to_string(), p.to_string()),
        None => (info.to_string(), String::new()),
    };
    (user, pass, host)
}

fn split_host_port(hostport: &str) -> Option<(String, u16)> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = if let Some(p) = tail.strip_prefix(':') {
            if p.is_empty() {
                0
            } else {
                p.parse().ok()?
            }
        } else if tail.is_empty() {
            0
        } else {
            return None;
        };
        return Some((host.to_string(), port));
    }
    if let Some((host, port)) = hostport.rsplit_once(':') {
        if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) && !host.is_empty() {
            return Some((host.to_string(), port.parse().ok()?));
        }
    }
    Some((hostport.to_string(), 0))
}

pub(crate) fn fields_from_link(link: &ServerLink, name: &str) -> Fields {
    Fields {
        kind: link.kind.clone(),
        name: name.to_string(),
        host: link.host.clone(),
        port_text: if link.port == 0 { String::new() } else { link.port.to_string() },
        remote: link.remote.clone(),
        user: link.user.clone(),
        tls: link.tls,
    }
}

pub(crate) fn fields_from_parsed(p: &Parsed) -> Fields {
    Fields {
        kind: p.kind.clone(),
        name: String::new(),
        host: p.host.clone(),
        port_text: if p.port == 0 { String::new() } else { p.port.to_string() },
        remote: p.remote.clone(),
        user: p.user.clone(),
        tls: p.tls,
    }
}

fn walk_servers(nodes: &[FavNode], out: &mut Vec<ServerLink>) {
    for n in nodes {
        if let Some(s) = &n.server {
            out.push(s.clone());
        }
        if n.is_group() {
            walk_servers(&n.children, out);
        }
    }
}

pub(crate) fn best_match(tree: &[FavNode], parsed: &Parsed) -> Option<ServerLink> {
    let mut found = Vec::new();
    walk_servers(tree, &mut found);
    let mut best: Option<(i32, ServerLink)> = None;
    for link in found {
        if !link.kind.eq_ignore_ascii_case(&parsed.kind) || !link.host.eq_ignore_ascii_case(&parsed.host) {
            continue;
        }
        if !parsed.user.is_empty() && !link.user.eq_ignore_ascii_case(&parsed.user) {
            continue;
        }
        if parsed.port != 0 && effective_port(&link.kind, link.port, link.tls) != parsed.port && link.port != parsed.port {
            continue;
        }
        if parsed.tls != link.tls && parsed.kind == "webdav" && parsed.port == 0 && link.port == 0 {
            continue;
        }
        let mut score = 1;
        if !parsed.user.is_empty() {
            score += 2;
        }
        let want = tidy_remote(&parsed.kind, &parsed.remote);
        let have = tidy_remote(&link.kind, &link.remote);
        if !want.is_empty() && want.eq_ignore_ascii_case(&have) {
            score += 4;
        }
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, link));
        }
    }
    best.map(|(_, l)| l)
}

/// A saved SMB or WebDAV server whose UNC covers `path`.
pub(crate) fn server_for_unc(tree: &[FavNode], path: &str) -> Option<ServerLink> {
    let want = path.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase();
    if !want.starts_with(r"\\") {
        return None;
    }
    let mut found = Vec::new();
    walk_servers(tree, &mut found);
    found.into_iter().find(|link| {
        if link.kind != "smb" && link.kind != "webdav" {
            return false;
        }
        let unc = unc_for(link).trim_end_matches(['\\', '/']).to_ascii_lowercase();
        want == unc || want.starts_with(&(unc.clone() + "\\"))
    })
}

pub(crate) fn still_used(tree: &[FavNode], link: &ServerLink) -> bool {
    let key = mount_key(link);
    let mut found = Vec::new();
    walk_servers(tree, &mut found);
    found.iter().any(|other| mount_key(other) == key)
}

/// Another saved server still uses this Credential Manager target.
pub(crate) fn cred_used(tree: &[FavNode], link: &ServerLink) -> bool {
    let target = cred_target(link);
    let mut found = Vec::new();
    walk_servers(tree, &mut found);
    found.iter().any(|other| cred_target(other) == target)
}

/// rclone config text. `obscured` is already obscured; the plaintext password is not accepted here.
pub(crate) fn config_body(link: &ServerLink, obscured: Option<&str>) -> String {
    let mut lines = vec!["[pg]".to_string(), format!("type = {}", link.kind), format!("host = {}", one_line(&link.host))];
    let port = effective_port(&link.kind, link.port, link.tls);
    lines.push(format!("port = {port}"));
    if !link.user.trim().is_empty() {
        lines.push(format!("user = {}", one_line(&link.user)));
    }
    if let Some(pass) = obscured.filter(|s| !s.is_empty()) {
        lines.push(format!("pass = {}", one_line(pass)));
    }
    if link.kind == "sftp" {
        // `none` skips the host-key prompt so a hidden mount cannot block on stdin.
        lines.push("known_hosts_file = none".into());
    }
    lines.join("\n") + "\n"
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], "")
}

/// Arguments for `rclone mount`. The password is not an argument; it lives in the config file.
pub(crate) fn mount_args(config: &str, link: &ServerLink, letter: char) -> Vec<String> {
    let remote = tidy_remote(&link.kind, &link.remote);
    let spec = if remote.is_empty() {
        "pg:".to_string()
    } else if remote.starts_with('/') {
        format!("pg:{remote}")
    } else {
        format!("pg:{remote}")
    };
    vec![
        "mount".into(),
        spec,
        format!("{letter}:"),
        "--config".into(),
        config.to_string(),
        "--network-mode".into(),
        "--volname".into(),
        "Maguro".into(),
        "--vfs-cache-mode".into(),
        "writes".into(),
        "--dir-cache-time".into(),
        "10s".into(),
    ]
}

pub(crate) fn args_leak(args: &[String], secret: &str) -> bool {
    !secret.is_empty() && args.iter().any(|a| a.contains(secret))
}

/// A path sits on one of the live mount roots (drive letter or directory).
pub(crate) fn covered_by_mount(path: &str, roots: &[String]) -> bool {
    let path = path.trim().replace('/', "\\");
    if path.is_empty() {
        return false;
    }
    roots.iter().any(|root| {
        let root = root.trim().trim_end_matches(['\\', '/']).replace('/', "\\");
        if root.is_empty() {
            return false;
        }
        path.eq_ignore_ascii_case(&root) || path.to_ascii_lowercase().starts_with(&(root.to_ascii_lowercase() + "\\"))
    })
}

/// Add on the panel background or a favourites group. Edit on a server leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuExtra {
    Add,
    Edit,
    None,
}

pub(crate) fn menu_extra(has_row: bool, in_tree: bool, group: bool, server: bool) -> MenuExtra {
    if !has_row {
        MenuExtra::Add
    } else if in_tree && group {
        MenuExtra::Add
    } else if in_tree && !group && server {
        MenuExtra::Edit
    } else {
        MenuExtra::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(kind: &str, host: &str, port: u16, remote: &str, user: &str, tls: bool) -> ServerLink {
        ServerLink { kind: kind.into(), host: host.into(), port, remote: remote.into(), user: user.into(), tls }
    }

    #[test]
    fn old_shape_is_not_required_and_password_never_lands_in_the_path() {
        let built = normalize(&Fields {
            kind: "sftp".into(),
            name: String::new(),
            host: "example.com".into(),
            port_text: "22".into(),
            remote: "/data".into(),
            user: "alice".into(),
            tls: false,
        })
        .unwrap();
        assert_eq!(built.name, "alice@example.com");
        assert_eq!(built.path, "sftp://alice@example.com:22/data");
        assert!(!built.path.contains("s3cret"));
        let body = config_body(&built.link, Some("obscured-token"));
        assert!(body.contains("pass = obscured-token"));
        assert!(!body.contains("s3cret-pl4in"));
        let args = mount_args(r"C:\temp\pg.conf", &built.link, 'Z');
        assert!(args.iter().any(|a| a == "--network-mode"));
        assert!(!args_leak(&args, "s3cret-pl4in"));
        assert!(!args_leak(&args, "obscured-token"));
        assert_eq!(cred_target(&built.link), "Maguro:sftp:alice@example.com:22");
        assert!(!cred_target(&built.link).contains("s3cret"));
    }

    #[test]
    fn urls_prefill_and_match_a_saved_server() {
        let sftp = parse_remote_url("sftp://alice:s3cret-pl4in@example.com:2222/data").unwrap();
        assert_eq!(sftp.user, "alice");
        assert_eq!(sftp.password, "s3cret-pl4in");
        assert_eq!(sftp.port, 2222);
        assert_eq!(sftp.remote, "/data");
        let built = normalize(&fields_from_parsed(&sftp)).unwrap();
        assert!(!built.path.contains("s3cret-pl4in"));
        assert!(parse_remote_url(r"C:\WORK\pgtest").is_none());
        assert!(parse_remote_url("ftp://files.example").unwrap().user.is_empty());

        let smb = parse_remote_url("smb://localhost/C$/WORK/pgtest/data").unwrap();
        let smb_built = normalize(&fields_from_parsed(&smb)).unwrap();
        assert_eq!(smb_built.path, r"\\localhost\C$\WORK\pgtest\data");

        let dav = parse_remote_url("webdav://localhost:8090/dav").unwrap();
        assert!(!dav.tls);
        assert_eq!(unc_for(&normalize(&fields_from_parsed(&dav)).unwrap().link), r"\\localhost@8090\DavWWWRoot\dav");
        let davs = parse_remote_url("webdavs://cloud.example/remote.php/dav").unwrap();
        assert!(davs.tls);
        assert_eq!(unc_for(&normalize(&fields_from_parsed(&davs)).unwrap().link), r"\\cloud.example@SSL\DavWWWRoot\remote.php\dav");

        let mut tree = vec![FavNode::leaf(r"C:\WORK\pgtest", "local", false)];
        tree.push(FavNode::server_leaf(&built.name, &built.path, built.link.clone()));
        let hit = best_match(&tree, &parse_remote_url("sftp://alice@example.com:2222").unwrap()).unwrap();
        assert_eq!(hit.host, "example.com");
        assert!(best_match(&tree, &parse_remote_url("ftp://example.com").unwrap()).is_none());
        assert!(server_for_unc(&tree, r"\\localhost\C$\WORK\pgtest\data\a.txt").is_none());
    }

    #[test]
    fn smb_unc_matches_a_saved_server_and_webdav_uses_the_redirector() {
        let smb = link("smb", "localhost", 0, r"C$\WORK\pgtest\data", "", false);
        assert_eq!(unc_for(&smb), r"\\localhost\C$\WORK\pgtest\data");
        assert_eq!(share_root(&unc_for(&smb)), r"\\localhost\C$");
        let mut tree = vec![FavNode::server_leaf("local", &canonical_path(&smb), smb.clone())];
        assert_eq!(server_for_unc(&tree, r"\\localhost\C$\WORK\pgtest\data\a.txt").unwrap().host, "localhost");
        let other = link("smb", "files", 0, "share", "ada", false);
        tree.push(FavNode::server_leaf("files", &canonical_path(&other), other));
        assert!(still_used(&tree, &smb));
        tree.remove(0);
        assert!(!still_used(&tree, &smb));
    }

    #[test]
    fn menu_and_tools_and_mount_coverage() {
        assert_eq!(menu_extra(false, false, false, false), MenuExtra::Add);
        assert_eq!(menu_extra(true, true, true, false), MenuExtra::Add);
        assert_eq!(menu_extra(true, true, false, true), MenuExtra::Edit);
        assert_eq!(menu_extra(true, true, false, false), MenuExtra::None);
        assert_eq!(menu_extra(true, false, true, false), MenuExtra::None);
        assert!(should_ask_tools(false, false, true));
        assert!(should_ask_tools(false, true, false));
        assert!(!should_ask_tools(true, false, false));
        assert!(!should_ask_tools(false, true, true));
        let args = winget_args(WINGET_RCLONE);
        assert!(args.iter().any(|a| a == "Rclone.Rclone"));
        assert!(winget_args(WINGET_WINFSP).iter().any(|a| a == "WinFsp.WinFsp"));
        assert!(covered_by_mount(r"Z:\pgtest\a.txt", &[r"Z:\".into()]));
        assert!(covered_by_mount(r"C:\WORK\pgtest\mnt\sftp\a", &[r"C:\WORK\pgtest\mnt\sftp".into()]));
        assert!(!covered_by_mount(r"C:\WORK\other", &[r"C:\WORK\pgtest\mnt\sftp".into()]));
        assert_eq!(normalize(&Fields { kind: "sftp".into(), name: "x".into(), host: String::new(), port_text: String::new(), remote: String::new(), user: String::new(), tls: false }), Err(Fail::Host));
        assert_eq!(normalize(&Fields { kind: "ftp".into(), name: "x".into(), host: "h".into(), port_text: "nope".into(), remote: String::new(), user: String::new(), tls: false }), Err(Fail::Port));
    }
}
