// The string Everything itself searches. Folder scope is a quoted path prefix.
// The rest of the text is Everything's own syntax (`ext:`, `size:`, `dm:`, `regex:`, `path:`).
// A leading `all:` or `everywhere:` searches every indexed drive. `;` outside quotes is OR, as in the pane filter.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Folder,
    Everywhere,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchText {
    pub(crate) scope: Scope,
    pub(crate) text: String,
}

pub(crate) fn forces_everywhere(raw: &str) -> bool {
    strip_scope_prefix(raw).0
}

/// The query with a leading scope prefix removed, when one was there.
pub(crate) fn without_scope_prefix(raw: &str) -> Option<String> {
    let (forced, rest) = strip_scope_prefix(raw);
    if forced { Some(rest.trim().to_string()) } else { None }
}

pub(crate) fn search_text(folder: &str, raw: &str, scope_all: bool, show_hidden: bool) -> Option<SearchText> {
    let (forced, terms) = strip_scope_prefix(raw);
    let terms = terms.trim();
    if terms.is_empty() {
        return None;
    }
    let scope = if forced || scope_all { Scope::Everywhere } else { Scope::Folder };
    let terms = semicolons_to_or(terms);
    let mut text = String::new();
    if scope == Scope::Folder {
        let folder = normalize_folder(folder);
        if folder.is_empty() {
            return None;
        }
        text.push('"');
        text.push_str(&folder);
        text.push('\\');
        text.push('"');
        text.push(' ');
    }
    text.push_str(&terms);
    if !show_hidden && !mentions_attrib(&terms) {
        text.push_str(" !attrib:H !attrib:S");
    }
    Some(SearchText { scope, text })
}

fn strip_scope_prefix(raw: &str) -> (bool, &str) {
    let t = raw.trim_start();
    for p in ["all:", "everywhere:"] {
        // get(): a byte cut inside a non-ASCII character (Korean, Japanese) is None, not a panic.
        if t.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)) {
            return (true, &t[p.len()..]);
        }
    }
    (false, t)
}

fn normalize_folder(folder: &str) -> String {
    folder.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_string()
}

fn semicolons_to_or(s: &str) -> String {
    let mut out = String::new();
    let mut quote = false;
    for c in s.chars() {
        if c == '"' {
            quote = !quote;
            out.push(c);
        } else if c == ';' && !quote {
            out.push('|');
        } else {
            out.push(c);
        }
    }
    out
}

fn mentions_attrib(s: &str) -> bool {
    s.to_ascii_lowercase().contains("attrib:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_scope_quotes_the_path_and_passes_everything_syntax() {
        let s = search_text(r"C:\Work\", "ext:pdf size:>1mb dm:today", false, true).unwrap();
        assert_eq!(s.scope, Scope::Folder);
        assert_eq!(s.text, r#""C:\Work\" ext:pdf size:>1mb dm:today"#);
        let hidden = search_text(r"C:/Work/", "regex:foo.*", false, false).unwrap();
        assert_eq!(hidden.text, r#""C:\Work\" regex:foo.* !attrib:H !attrib:S"#);
        let spaced = search_text(r#" "C:\Program Files\Work" "#, "path:notes", false, true).unwrap();
        assert_eq!(spaced.text, r#""C:\Program Files\Work\" path:notes"#);
    }

    #[test]
    fn everywhere_scope_prefix_and_semicolon_or() {
        let all = search_text(r"C:\Work", "all: *.dll", false, true).unwrap();
        assert_eq!(all.scope, Scope::Everywhere);
        assert_eq!(all.text, "*.dll");
        assert!(forces_everywhere("  Everywhere: report"));
        assert!(!forces_everywhere("가나다라 보고서"));
        assert!(!forces_everywhere("日本語のファイル名を探す"));
        assert!(forces_everywhere("all:日本"));
        assert!(search_text(r"C:\Work", "회의록", false, true).is_some());
        assert_eq!(without_scope_prefix("all: report").as_deref(), Some("report"));
        assert!(without_scope_prefix("report").is_none());
        let pill = search_text(r"C:\Work", "*.pdf;*.txt", true, true).unwrap();
        assert_eq!(pill.scope, Scope::Everywhere);
        assert_eq!(pill.text, "*.pdf|*.txt");
        let quoted = search_text(r"C:\Work", r#"regex:"a;b";ext:pdf"#, false, true).unwrap();
        assert_eq!(quoted.text, r#""C:\Work\" regex:"a;b"|ext:pdf"#);
    }

    #[test]
    fn an_attrib_term_is_left_alone_and_an_empty_query_is_refused() {
        let s = search_text(r"C:\Work", "ext:pdf attrib:H", false, false).unwrap();
        assert_eq!(s.text, r#""C:\Work\" ext:pdf attrib:H"#);
        assert!(search_text(r"C:\Work", "   ", false, true).is_none());
        assert!(search_text(r"C:\Work", "all:", false, true).is_none());
        assert!(search_text("", "report", false, true).is_none());
    }
}
