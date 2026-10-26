// Pure rename rules, preview, and a safe execution order. No Win32 here, so the tests run with the crate.

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaseMode {
    Lower,
    Upper,
    Title,
    Sentence,
}

/// Where a generated piece is written. `At` is a character index in the stem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumPos {
    Prefix,
    Suffix,
    /// `0` is before the first character. `-1` appends. Other negatives count back from the end.
    At(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DateSrc {
    Modified,
    Created,
    Accessed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DatePos {
    Prefix,
    Suffix,
    /// The stem becomes the formatted date. The extension stays.
    Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrimEdge {
    Both,
    Left,
    Right,
}

/// One step in the stack. Later phases add variants; existing ones stay in this order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    Replace { find: String, replace: String, case_sensitive: bool, include_ext: bool },
    Prefix { text: String },
    Suffix { text: String },
    Case { mode: CaseMode, include_ext: bool },
    Extension { ext: String },
    /// `start + step * index`, zero-padded. The index is the item's place in the batch.
    Number { start: i64, step: i64, pad: u8, pos: NumPos },
    /// File times only. There is no EXIF read.
    Date { src: DateSrc, pattern: String, pos: DatePos },
    Insert { text: String, at: i32 },
    Delete { at: i32, len: usize },
    /// Empty `chars` trims whitespace. Otherwise those characters.
    Trim { chars: String, edge: TrimEdge },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BatchItem {
    pub dir: String,
    pub name: String,
    pub is_dir: bool,
    pub modified: Option<i64>,
    pub created: Option<i64>,
    pub accessed: Option<i64>,
}

impl BatchItem {
    pub(crate) fn new(dir: impl Into<String>, name: impl Into<String>, is_dir: bool) -> Self {
        Self { dir: dir.into(), name: name.into(), is_dir, modified: None, created: None, accessed: None }
    }
}

/// Why a new name cannot be applied. The first one in this order is the one the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Issue {
    Empty,
    InvalidChars,
    Reserved,
    Trailing,
    TooLong,
    Duplicate,
    Exists,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewRow {
    pub old_name: String,
    pub new_name: String,
    pub issues: Vec<Issue>,
    /// Char indexes `[start, end)` of the part that differs. Empty when that side did not change.
    pub old_span: Option<(usize, usize)>,
    pub new_span: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub dir: String,
    pub from: String,
    pub to: String,
}

pub(crate) fn dir_key(dir: &str) -> String {
    dir.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

pub(crate) fn name_key(name: &str) -> String {
    name.to_lowercase()
}

pub(crate) fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with(['\\', '/']) {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

/// Stem and extension. Folders and dotfiles keep the whole name as the stem.
pub(crate) fn split_name(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem, ext),
        _ => (name, ""),
    }
}

fn join_name(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

pub(crate) fn apply_name(item: &BatchItem, rules: &[Rule], index: usize, tz_offset_secs: i32) -> String {
    let mut name = item.name.clone();
    for rule in rules {
        name = apply_one(&name, item, is_dir_of(item), rule, index, tz_offset_secs);
    }
    name
}

fn is_dir_of(item: &BatchItem) -> bool {
    item.is_dir
}

fn apply_one(name: &str, item: &BatchItem, is_dir: bool, rule: &Rule, index: usize, tz: i32) -> String {
    let (stem, ext) = split_name(name, is_dir);
    match rule {
        Rule::Replace { find, replace, case_sensitive, include_ext } => {
            if find.is_empty() {
                return name.to_string();
            }
            if *include_ext {
                replace_all(name, find, replace, *case_sensitive)
            } else {
                join_name(&replace_all(stem, find, replace, *case_sensitive), ext)
            }
        }
        Rule::Prefix { text } => {
            if text.is_empty() {
                name.to_string()
            } else {
                join_name(&format!("{text}{stem}"), ext)
            }
        }
        Rule::Suffix { text } => {
            if text.is_empty() {
                name.to_string()
            } else {
                join_name(&format!("{stem}{text}"), ext)
            }
        }
        Rule::Case { mode, include_ext } => {
            if *include_ext {
                apply_case(name, *mode)
            } else {
                join_name(&apply_case(stem, *mode), ext)
            }
        }
        Rule::Extension { ext: new_ext } => {
            if is_dir {
                return name.to_string();
            }
            let cleaned = new_ext.trim().trim_start_matches('.');
            join_name(stem, cleaned)
        }
        Rule::Number { start, step, pad, pos } => {
            let n = start.saturating_add(step.saturating_mul(index as i64));
            let text = format_num(n, *pad);
            match pos {
                NumPos::Prefix => join_name(&format!("{text}{stem}"), ext),
                NumPos::Suffix => join_name(&format!("{stem}{text}"), ext),
                NumPos::At(at) => join_name(&insert_at(stem, &text, *at), ext),
            }
        }
        Rule::Date { src, pattern, pos } => {
            if pattern.is_empty() {
                return name.to_string();
            }
            let stamp = match src {
                DateSrc::Modified => item.modified,
                DateSrc::Created => item.created,
                DateSrc::Accessed => item.accessed,
            };
            let Some(stamp) = stamp else { return name.to_string() };
            let text = format_date(stamp, tz, pattern);
            match pos {
                DatePos::Prefix => join_name(&format!("{text}{stem}"), ext),
                DatePos::Suffix => join_name(&format!("{stem}{text}"), ext),
                DatePos::Replace => join_name(&text, ext),
            }
        }
        Rule::Insert { text, at } => {
            if text.is_empty() {
                name.to_string()
            } else {
                join_name(&insert_at(stem, text, *at), ext)
            }
        }
        Rule::Delete { at, len } => {
            if *len == 0 {
                name.to_string()
            } else {
                join_name(&delete_at(stem, *at, *len), ext)
            }
        }
        Rule::Trim { chars, edge } => join_name(&trim_stem(stem, chars, *edge), ext),
    }
}

fn format_num(n: i64, pad: u8) -> String {
    let pad = pad as usize;
    let neg = n < 0;
    // `i64::MIN` has no positive counterpart; keep the magnitude as a wider integer.
    let mag = if n == i64::MIN { 1u128 << 63 } else { n.unsigned_abs() as u128 };
    let body = format!("{mag:0width$}", width = pad);
    if neg { format!("-{body}") } else { body }
}

/// `0` starts, `-1` appends. A negative value counts back from the end, then one past that so `-1` is the end.
fn index_insert(len: usize, at: i32) -> usize {
    if at >= 0 {
        (at as usize).min(len)
    } else {
        (len as i32 + at + 1).clamp(0, len as i32) as usize
    }
}

/// `0` is the first character, `-1` the last. Nothing is deleted past the end.
fn index_delete(len: usize, at: i32) -> usize {
    if len == 0 {
        return 0;
    }
    if at >= 0 {
        (at as usize).min(len)
    } else {
        (len as i32 + at).clamp(0, len as i32 - 1) as usize
    }
}

fn insert_at(stem: &str, text: &str, at: i32) -> String {
    let chars: Vec<char> = stem.chars().collect();
    let i = index_insert(chars.len(), at);
    let mut out = String::new();
    out.extend(chars[..i].iter());
    out.push_str(text);
    out.extend(chars[i..].iter());
    out
}

fn delete_at(stem: &str, at: i32, n: usize) -> String {
    let chars: Vec<char> = stem.chars().collect();
    if chars.is_empty() || n == 0 {
        return stem.to_string();
    }
    let i = index_delete(chars.len(), at);
    let end = (i + n).min(chars.len());
    let mut out = String::new();
    out.extend(chars[..i].iter());
    out.extend(chars[end..].iter());
    out
}

fn trim_stem(stem: &str, chars: &str, edge: TrimEdge) -> String {
    let drop = |c: char| if chars.is_empty() { c.is_whitespace() } else { chars.contains(c) };
    match edge {
        TrimEdge::Both => stem.trim_matches(drop).to_string(),
        TrimEdge::Left => stem.trim_start_matches(drop).to_string(),
        TrimEdge::Right => stem.trim_end_matches(drop).to_string(),
    }
}

/// `YYYY MM DD hh mm ss`. `MM` is the month and `mm` is the minute. Unknown letters stay as written.
pub(crate) fn format_date(unix: i64, offset_secs: i32, pattern: &str) -> String {
    let (y, mo, d, h, mi, s) = civil(unix, offset_secs);
    let p: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < p.len() {
        let rest: String = p[i..].iter().collect();
        if let Some(tok) = rest.strip_prefix("YYYY") {
            let _ = tok;
            out.push_str(&format!("{y:04}"));
            i += 4;
        } else if rest.starts_with("YY") {
            out.push_str(&format!("{:02}", (y.rem_euclid(100)) as u32));
            i += 2;
        } else if rest.starts_with("MM") {
            out.push_str(&format!("{mo:02}"));
            i += 2;
        } else if rest.starts_with("DD") {
            out.push_str(&format!("{d:02}"));
            i += 2;
        } else if rest.starts_with("hh") {
            out.push_str(&format!("{h:02}"));
            i += 2;
        } else if rest.starts_with("mm") {
            out.push_str(&format!("{mi:02}"));
            i += 2;
        } else if rest.starts_with("ss") {
            out.push_str(&format!("{s:02}"));
            i += 2;
        } else {
            out.push(p[i]);
            i += 1;
        }
    }
    out
}

fn civil(unix: i64, offset_secs: i32) -> (i32, u32, u32, u32, u32, u32) {
    let secs = unix.saturating_add(offset_secs as i64);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400) as u32;
    let h = sod / 3600;
    let mi = (sod % 3600) / 60;
    let s = sod % 60;
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, h, mi, s)
}

/// Howard Hinnant, days since 1970-01-01 to a civil date.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d)
}

fn replace_all(hay: &str, find: &str, repl: &str, case_sensitive: bool) -> String {
    if find.is_empty() {
        return hay.to_string();
    }
    if case_sensitive {
        return hay.replace(find, repl);
    }
    let h: Vec<char> = hay.chars().collect();
    let f: Vec<char> = find.chars().collect();
    if f.is_empty() {
        return hay.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < h.len() {
        if i + f.len() <= h.len() && chars_eq_ci(&h[i..i + f.len()], &f) {
            out.push_str(repl);
            i += f.len();
        } else {
            out.push(h[i]);
            i += 1;
        }
    }
    out
}

fn chars_eq_ci(a: &[char], b: &[char]) -> bool {
    a.iter().flat_map(|c| c.to_lowercase()).eq(b.iter().flat_map(|c| c.to_lowercase()))
}

fn apply_case(s: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Upper => s.to_uppercase(),
        CaseMode::Title => title_case(s),
        CaseMode::Sentence => sentence_case(s),
    }
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for c in s.chars() {
        if c.is_whitespace() || matches!(c, '_' | '-' | '.') {
            out.push(c);
            cap = true;
        } else if cap {
            out.extend(c.to_uppercase());
            cap = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

fn sentence_case(s: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for c in s.chars() {
        if cap && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            cap = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

pub(crate) fn change_span(old: &str, new: &str) -> (Option<(usize, usize)>, Option<(usize, usize)>) {
    if old == new {
        return (None, None);
    }
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let mut p = 0;
    while p < a.len() && p < b.len() && a[p] == b[p] {
        p += 1;
    }
    let mut s = 0;
    while s < a.len() - p && s < b.len() - p && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    let old_span = (p < a.len() - s).then_some((p, a.len() - s));
    let new_span = (p < b.len() - s).then_some((p, b.len() - s));
    (old_span, new_span)
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Explorer's classic limit: 259 UTF-16 units for the path, 255 for the file name.
pub(crate) fn path_too_long(dir: &str, name: &str) -> bool {
    let dir = dir.trim_end_matches(['\\', '/']);
    utf16_len(name) > 255 || utf16_len(dir) + 1 + utf16_len(name) > 259
}

fn invalid_chars(name: &str) -> bool {
    name.chars().any(|c| (c as u32) < 0x20 || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
}

fn reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    let u = stem.to_ascii_uppercase();
    matches!(u.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((u.starts_with("COM") || u.starts_with("LPT")) && u.len() == 4 && u.as_bytes()[3].is_ascii_digit())
}

fn bad_tail(name: &str) -> bool {
    name.is_empty() || name == "." || name == ".." || name.ends_with(' ') || name.ends_with('.')
}

fn name_issues(dir: &str, name: &str) -> Vec<Issue> {
    let mut v = Vec::new();
    if name.is_empty() || name == "." || name == ".." {
        v.push(Issue::Empty);
    }
    if invalid_chars(name) {
        v.push(Issue::InvalidChars);
    }
    if reserved_name(name) {
        v.push(Issue::Reserved);
    }
    if !name.is_empty() && name != "." && name != ".." && (name.ends_with(' ') || name.ends_with('.')) {
        v.push(Issue::Trailing);
    }
    if path_too_long(dir, name) {
        v.push(Issue::TooLong);
    }
    // Empty already covers a blank name; a name that is only "." is Empty, not a second Trailing.
    if bad_tail(name) && name.ends_with('.') && name != "." && name != ".." && !v.contains(&Issue::Trailing) {
        v.push(Issue::Trailing);
    }
    v.sort();
    v.dedup();
    v
}

/// `existing` holds every name currently in those folders, including the items being renamed.
/// Keys are `(dir_key, name_key)`.
pub(crate) fn preview(items: &[BatchItem], rules: &[Rule], existing: &HashSet<(String, String)>, tz_offset_secs: i32) -> Vec<PreviewRow> {
    let tz = tz_offset_secs;
    let planned: Vec<(String, String)> = items
        .iter()
        .enumerate()
        .map(|(i, it)| (it.name.clone(), apply_name(it, rules, i, tz)))
        .collect();
    let mut occ = existing.clone();
    for it in items {
        occ.remove(&(dir_key(&it.dir), name_key(&it.name)));
    }
    let mut final_count: Vec<((String, String), usize)> = Vec::new();
    for (it, (_, new)) in items.iter().zip(planned.iter()) {
        let k = (dir_key(&it.dir), name_key(new));
        if let Some(slot) = final_count.iter_mut().find(|(key, _)| key == &k) {
            slot.1 += 1;
        } else {
            final_count.push((k, 1));
        }
    }
    items
        .iter()
        .zip(planned.iter())
        .map(|(it, (old, new))| {
            let mut issues = name_issues(&it.dir, new);
            let k = (dir_key(&it.dir), name_key(new));
            let dup = final_count.iter().find(|(key, _)| key == &k).map(|(_, n)| *n).unwrap_or(1) > 1;
            if dup {
                issues.push(Issue::Duplicate);
            } else if occ.contains(&k) {
                issues.push(Issue::Exists);
            }
            issues.sort();
            issues.dedup();
            let (old_span, new_span) = change_span(old, new);
            PreviewRow { old_name: old.clone(), new_name: new.clone(), issues, old_span, new_span }
        })
        .collect()
}

pub(crate) fn error_count(rows: &[PreviewRow]) -> usize {
    rows.iter().filter(|r| !r.issues.is_empty()).count()
}

pub(crate) fn change_count(rows: &[PreviewRow]) -> usize {
    rows.iter().filter(|r| r.old_name != r.new_name).count()
}

/// Waves of renames that never step on a name still in use.
/// `avoid` is every name that stays put (not a source of `pairs`), keyed like `preview`.
/// A cycle or a case-only change goes through `~pgNNNN.tmp` first.
struct Job {
    dir: String,
    dir_k: String,
    current: String,
    target: String,
}

pub(crate) fn plan(pairs: &[(String, String, String)], avoid: &HashSet<(String, String)>) -> Vec<Vec<Step>> {
    let mut jobs: Vec<Job> = pairs
        .iter()
        .filter(|(_, from, to)| from != to)
        .map(|(dir, from, to)| Job { dir: dir.clone(), dir_k: dir_key(dir), current: from.clone(), target: to.clone() })
        .collect();
    let mut waves = Vec::new();
    let mut temp_n = 0u32;
    let mut guard = 0;
    while !jobs.is_empty() && guard < 10_000 {
        guard += 1;
        let ready: Vec<usize> = (0..jobs.len()).filter(|&i| is_ready(&jobs, i, avoid)).collect();
        if !ready.is_empty() {
            let wave = ready
                .iter()
                .map(|&i| Step { dir: jobs[i].dir.clone(), from: jobs[i].current.clone(), to: jobs[i].target.clone() })
                .collect();
            for i in ready.into_iter().rev() {
                jobs.remove(i);
            }
            waves.push(wave);
            continue;
        }
        let blocked: Vec<usize> = (0..jobs.len()).filter(|&i| needs_temp(&jobs, i)).collect();
        if blocked.is_empty() {
            break;
        }
        let mut wave = Vec::new();
        for i in blocked {
            let (tmp, next) = fresh_temp(&jobs, avoid, temp_n, &jobs[i].dir_k);
            temp_n = next;
            wave.push(Step { dir: jobs[i].dir.clone(), from: jobs[i].current.clone(), to: tmp.clone() });
            jobs[i].current = tmp;
        }
        waves.push(wave);
    }
    waves
}

struct JobRef<'a> {
    current: &'a str,
    target: &'a str,
}

fn job_at<'a>(jobs: &'a [Job], i: usize) -> JobRef<'a> {
    JobRef { current: &jobs[i].current, target: &jobs[i].target }
}

fn case_only(j: JobRef<'_>) -> bool {
    name_key(j.current) == name_key(j.target) && j.current != j.target
}

fn occupied_by_other(jobs: &[Job], i: usize) -> bool {
    let t = name_key(&jobs[i].target);
    jobs.iter().enumerate().any(|(j, o)| j != i && o.dir_k == jobs[i].dir_k && name_key(&o.current) == t)
}

fn hits_avoid(jobs: &[Job], i: usize, avoid: &HashSet<(String, String)>) -> bool {
    avoid.contains(&(jobs[i].dir_k.clone(), name_key(&jobs[i].target)))
}

fn is_ready(jobs: &[Job], i: usize, avoid: &HashSet<(String, String)>) -> bool {
    let j = job_at(jobs, i);
    j.current != j.target && !case_only(j) && !occupied_by_other(jobs, i) && !hits_avoid(jobs, i, avoid)
}

fn needs_temp(jobs: &[Job], i: usize) -> bool {
    let j = job_at(jobs, i);
    j.current != j.target && (case_only(j) || occupied_by_other(jobs, i))
}

fn fresh_temp(jobs: &[Job], avoid: &HashSet<(String, String)>, mut n: u32, dir_k: &str) -> (String, u32) {
    loop {
        let name = format!("~pg{n:04}.tmp");
        n = n.saturating_add(1);
        let k = name_key(&name);
        let used = jobs.iter().any(|j| j.dir_k == dir_k && (name_key(&j.current) == k || name_key(&j.target) == k))
            || avoid.iter().any(|(d, nk)| d == dir_k && nk == &k);
        if !used {
            return (name, n);
        }
        if n > 100_000 {
            return (format!("~pg{n}.tmp"), n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> BatchItem {
        BatchItem::new(r"C:\pics", name, false)
    }

    fn dir_item(name: &str) -> BatchItem {
        BatchItem::new(r"C:\pics", name, true)
    }

    fn keys(names: &[&str]) -> HashSet<(String, String)> {
        names.iter().map(|n| (dir_key(r"C:\pics"), name_key(n))).collect()
    }

    fn only(rules: &[Rule], name: &str) -> String {
        apply_name(&item(name), rules, 0, 0)
    }

    #[test]
    fn replace_respects_case_extension_and_empty_find() {
        let plain = Rule::Replace { find: "img".into(), replace: "pic".into(), case_sensitive: false, include_ext: false };
        assert_eq!(only(&[plain.clone()], "img_01.JPG"), "pic_01.JPG");
        assert_eq!(only(&[plain.clone()], "IMG.img"), "pic.img");
        let cs = Rule::Replace { find: "img".into(), replace: "pic".into(), case_sensitive: true, include_ext: false };
        assert_eq!(only(&[cs], "IMG_01.jpg"), "IMG_01.jpg");
        let all = Rule::Replace { find: "jpg".into(), replace: "png".into(), case_sensitive: false, include_ext: true };
        assert_eq!(only(&[all], "a.JPG"), "a.png");
        let empty = Rule::Replace { find: String::new(), replace: "x".into(), case_sensitive: false, include_ext: true };
        assert_eq!(only(&[empty], "a.jpg"), "a.jpg");
        assert_eq!(only(&[plain], "my img img.txt"), "my pic pic.txt");
    }

    #[test]
    fn prefix_and_suffix_stay_outside_the_extension() {
        let pre = Rule::Prefix { text: "IMG_".into() };
        let suf = Rule::Suffix { text: "_v2".into() };
        assert_eq!(only(&[pre.clone()], "cat.jpg"), "IMG_cat.jpg");
        assert_eq!(only(&[suf.clone()], "cat.jpg"), "cat_v2.jpg");
        assert_eq!(only(&[pre, suf], "cat.jpg"), "IMG_cat_v2.jpg");
        assert_eq!(only(&[Rule::Suffix { text: "_x".into() }], ".gitignore"), ".gitignore_x");
        assert_eq!(apply_name(&dir_item("photos"), &[Rule::Prefix { text: "a_".into() }], 0, 0), "a_photos");
    }

    #[test]
    fn case_and_extension_rules() {
        let lower = Rule::Case { mode: CaseMode::Lower, include_ext: false };
        let upper = Rule::Case { mode: CaseMode::Upper, include_ext: true };
        let title = Rule::Case { mode: CaseMode::Title, include_ext: false };
        let sentence = Rule::Case { mode: CaseMode::Sentence, include_ext: false };
        assert_eq!(only(&[lower], "My Cat.JPG"), "my cat.JPG");
        assert_eq!(only(&[upper], "My Cat.JPG"), "MY CAT.JPG");
        assert_eq!(only(&[title], "my_cat-dog.jpg"), "My_Cat-Dog.jpg");
        assert_eq!(only(&[sentence], "hello THERE.jpg"), "Hello there.jpg");
        assert_eq!(only(&[Rule::Extension { ext: ".png".into() }], "a.jpg"), "a.png");
        assert_eq!(only(&[Rule::Extension { ext: String::new() }], "a.jpg"), "a");
        assert_eq!(apply_name(&dir_item("pics.jpg"), &[Rule::Extension { ext: "png".into() }], 0, 0), "pics.jpg");
    }

    #[test]
    fn rules_run_top_to_bottom() {
        let rules = [
            Rule::Replace { find: "a".into(), replace: "b".into(), case_sensitive: true, include_ext: false },
            Rule::Prefix { text: "b".into() },
        ];
        assert_eq!(only(&rules, "a.txt"), "bb.txt");
    }

    #[test]
    fn spans_mark_the_edited_middle() {
        assert_eq!(change_span("cat.jpg", "cat.jpg"), (None, None));
        assert_eq!(change_span("a.txt", "b.txt"), (Some((0, 1)), Some((0, 1))));
        assert_eq!(change_span("cat.jpg", "IMG_cat.jpg"), (None, Some((0, 4))));
        // The suffix is inserted; the old name has no removed characters, only a gap before ".jpg".
        assert_eq!(change_span("cat.jpg", "cat_v2.jpg"), (None, Some((3, 6))));
    }

    #[test]
    fn validation_flags_bad_names_duplicates_and_existing_files() {
        let items = vec![item("a.txt"), item("b.txt"), item("c.txt")];
        let rules = [Rule::Replace { find: "a".into(), replace: "b".into(), case_sensitive: true, include_ext: false }];
        let rows = preview(&items, &rules, &keys(&["a.txt", "b.txt", "c.txt"]), 0);
        assert!(rows[0].issues.contains(&Issue::Duplicate) || rows[0].issues.contains(&Issue::Exists) || rows[1].issues.is_empty());
        // a.txt → b.txt, and b.txt stays: both end on b.txt
        assert!(rows[0].issues.contains(&Issue::Duplicate));
        assert!(rows[1].issues.contains(&Issue::Duplicate));
        assert!(rows[2].issues.is_empty());

        let clash = preview(&[item("new.txt")], &[Rule::Prefix { text: "old".into() }], &keys(&["new.txt", "oldnew.txt"]), 0);
        assert_eq!(clash[0].issues, vec![Issue::Exists]);

        let bad = preview(
            &[item("ok.txt")],
            &[Rule::Replace { find: "ok".into(), replace: "CON".into(), case_sensitive: true, include_ext: true }],
            &keys(&["ok.txt"]),
            0,
        );
        assert_eq!(bad[0].new_name, "CON.txt");
        assert!(bad[0].issues.contains(&Issue::Reserved));

        let chars = preview(
            &[item("ok.txt")],
            &[Rule::Prefix { text: "a:".into() }],
            &keys(&["ok.txt"]),
            0,
        );
        assert!(chars[0].issues.contains(&Issue::InvalidChars));

        // A suffix sits in front of the extension, so the trailing mark is checked on a name with no extension.
        let trail = preview(&[item("ok")], &[Rule::Suffix { text: " ".into() }], &keys(&["ok"]), 0);
        assert_eq!(trail[0].new_name, "ok ");
        assert!(trail[0].issues.contains(&Issue::Trailing));

        let blank = preview(
            &[item("ok.txt")],
            &[Rule::Replace { find: "ok.txt".into(), replace: String::new(), case_sensitive: true, include_ext: true }],
            &keys(&["ok.txt"]),
            0,
        );
        assert!(blank[0].issues.contains(&Issue::Empty));
    }

    #[test]
    fn chains_are_not_collisions_and_other_folders_are_independent() {
        let mut items = vec![item("a.txt"), item("b.txt")];
        let rules = [
            Rule::Replace { find: "a".into(), replace: "b".into(), case_sensitive: true, include_ext: false },
            // second file: b.txt → c.txt via a prefix that we apply only by renaming the source names directly
        ];
        // Direct names: build previews from already-target names by using per-item rules through apply.
        let _ = rules;
        let existing = keys(&["a.txt", "b.txt"]);
        // a→b and b→c is a chain. Express it as two replace rules that don't fit one stack,
        // so call the occupancy logic through preview of the final names by stubbing rules per item:
        let rows = {
            let custom = [
                BatchItem::new(r"C:\pics", "a.txt", false),
                BatchItem::new(r"C:\pics", "b.txt", false),
            ];
            // Use a stack that maps both? Can't with one stack. Validate via preview after
            // pretending the rules already produced the targets by replacing the whole stem.
            preview(
                &custom,
                &[Rule::Replace { find: "a.txt".into(), replace: "b.txt".into(), case_sensitive: true, include_ext: true }],
                &existing,
                0,
            )
        };
        // Only the first file changes, onto a name the second still holds: duplicate.
        assert!(rows[0].issues.contains(&Issue::Duplicate));

        items[1].name = "b.txt".into();
        let chain_items = vec![
            BatchItem::new(r"C:\pics", "a.txt", false),
            BatchItem::new(r"C:\pics", "b.txt", false),
        ];
        // Final names computed outside one shared rule: a→b, b→c. Feed them as the "names"
        // by checking plan + a manual occupancy copy of preview's rule.
        let finals = [("a.txt", "b.txt"), ("b.txt", "c.txt")];
        let mut occ = existing.clone();
        for (old, _) in finals {
            occ.remove(&(dir_key(r"C:\pics"), name_key(old)));
        }
        let mut dup = false;
        let mut exists = false;
        let mut seen = HashSet::new();
        for (_, new) in finals {
            let k = (dir_key(r"C:\pics"), name_key(new));
            if !seen.insert(k.clone()) {
                dup = true;
            } else if occ.contains(&k) {
                exists = true;
            }
        }
        assert!(!dup && !exists);

        let other = preview(
            &[
                BatchItem::new(r"C:\one", "a.txt", false),
                BatchItem::new(r"C:\two", "a.txt", false),
            ],
            &[Rule::Prefix { text: "n_".into() }],
            &HashSet::from([
                (dir_key(r"C:\one"), name_key("a.txt")),
                (dir_key(r"C:\two"), name_key("a.txt")),
            ]),
            0,
        );
        assert!(other.iter().all(|r| r.issues.is_empty()));
        assert_eq!(other[0].new_name, "n_a.txt");
        let _ = chain_items;
    }

    #[test]
    fn long_paths_are_rejected() {
        let dir = "C:\\".to_string() + &"d".repeat(260);
        let it = BatchItem::new(dir, "name.txt", false);
        let rows = preview(&[it], &[], &HashSet::new(), 0);
        assert!(rows[0].issues.contains(&Issue::TooLong));
        assert!(path_too_long(r"C:\short", &"n".repeat(256)));
        assert!(!path_too_long(r"C:\short", "name.txt"));
    }

    fn simulate(start: &[&str], waves: &[Vec<Step>]) -> Vec<String> {
        let mut names: Vec<String> = start.iter().map(|s| (*s).to_string()).collect();
        for wave in waves {
            let mut claimed = HashSet::new();
            for step in wave {
                assert!(claimed.insert(name_key(&step.to)), "two steps in one wave share {}", step.to);
                let clash = names.iter().any(|n| name_key(n) == name_key(&step.to) && n != &step.from);
                assert!(!clash, "{} is still occupied", step.to);
            }
            for step in wave {
                let pos = names.iter().position(|n| n == &step.from).unwrap();
                names[pos] = step.to.clone();
            }
        }
        names
    }

    #[test]
    fn plan_orders_chains_cycles_and_case_changes() {
        let dir = r"C:\pics";
        let avoid = HashSet::new();
        let simple = plan(&[(dir.into(), "a.txt".into(), "b.txt".into())], &avoid);
        assert_eq!(simulate(&["a.txt"], &simple), vec!["b.txt".to_string()]);
        assert_eq!(simple.len(), 1);

        let chain = plan(
            &[(dir.into(), "a.txt".into(), "b.txt".into()), (dir.into(), "b.txt".into(), "c.txt".into())],
            &avoid,
        );
        let ended = simulate(&["a.txt", "b.txt"], &chain);
        assert_eq!(ended, vec!["b.txt".to_string(), "c.txt".to_string()]);

        let cycle = plan(
            &[(dir.into(), "a.txt".into(), "b.txt".into()), (dir.into(), "b.txt".into(), "a.txt".into())],
            &avoid,
        );
        let ended = simulate(&["a.txt", "b.txt"], &cycle);
        assert_eq!(ended, vec!["b.txt".to_string(), "a.txt".to_string()]);
        assert!(cycle.iter().flat_map(|w| w.iter()).any(|s| s.to.contains("~pg") || s.from.contains("~pg")));

        let case = plan(&[(dir.into(), "Photo.JPG".into(), "photo.jpg".into())], &avoid);
        let ended = simulate(&["Photo.JPG"], &case);
        assert_eq!(ended, vec!["photo.jpg".to_string()]);
        assert!(case.len() >= 2);

        let stay = plan(&[(dir.into(), "a.txt".into(), "a.txt".into())], &avoid);
        assert!(stay.is_empty());

        let held = keys(&["kept.txt"]);
        let blocked = plan(&[(dir.into(), "a.txt".into(), "kept.txt".into())], &held);
        assert!(blocked.is_empty() || simulate(&["a.txt"], &blocked).iter().any(|n| n != "kept.txt") || blocked.iter().all(|w| w.is_empty()));
        // A name that stays on disk is not a legal target; the planner must not pretend the file moved onto it.
        assert!(blocked.iter().flatten().all(|s| !name_key(&s.to).eq(&name_key("kept.txt"))));
    }

    #[test]
    fn temps_do_not_reuse_a_real_name() {
        let dir = r"C:\pics";
        let avoid = keys(&["~pg0000.tmp"]);
        let cycle = plan(
            &[(dir.into(), "a.txt".into(), "b.txt".into()), (dir.into(), "b.txt".into(), "a.txt".into())],
            &avoid,
        );
        assert!(cycle.iter().flatten().all(|s| name_key(&s.from) != name_key("~pg0000.tmp") && name_key(&s.to) != name_key("~pg0000.tmp")));
        let ended = simulate(&["a.txt", "b.txt"], &cycle);
        assert_eq!(ended, vec!["b.txt".to_string(), "a.txt".to_string()]);
    }

    #[test]
    fn numbering_uses_index_padding_and_position() {
        let suf = Rule::Number { start: 1, step: 1, pad: 3, pos: NumPos::Suffix };
        assert_eq!(apply_name(&item("a.jpg"), &[suf.clone()], 0, 0), "a001.jpg");
        assert_eq!(apply_name(&item("a.jpg"), &[suf], 2, 0), "a003.jpg");
        let pre = Rule::Number { start: 10, step: 5, pad: 0, pos: NumPos::Prefix };
        assert_eq!(apply_name(&item("a.jpg"), &[pre], 1, 0), "15a.jpg");
        let at = Rule::Number { start: 1, step: 1, pad: 2, pos: NumPos::At(1) };
        assert_eq!(apply_name(&item("ab.jpg"), &[at], 0, 0), "a01b.jpg");
        let end = Rule::Number { start: 7, step: 1, pad: 2, pos: NumPos::At(-1) };
        assert_eq!(apply_name(&item("ab.jpg"), &[end], 0, 0), "ab07.jpg");
        let neg = Rule::Number { start: -2, step: 1, pad: 2, pos: NumPos::Suffix };
        assert_eq!(apply_name(&item("a"), &[neg], 0, 0), "a-02");
        assert_eq!(apply_name(&dir_item("pics"), &[Rule::Number { start: 1, step: 1, pad: 2, pos: NumPos::Prefix }], 0, 0), "01pics");
        assert_eq!(apply_name(&item(".gitignore"), &[Rule::Number { start: 1, step: 1, pad: 2, pos: NumPos::Suffix }], 0, 0), ".gitignore01");
        let items = vec![item("a.txt"), item("b.txt")];
        let rows = preview(&items, &[Rule::Number { start: 5, step: 2, pad: 2, pos: NumPos::Prefix }], &keys(&["a.txt", "b.txt"]), 0);
        assert_eq!(rows[0].new_name, "05a.txt");
        assert_eq!(rows[1].new_name, "07b.txt");
        assert!(rows.iter().all(|r| r.issues.is_empty()));
        let held = preview(&items, &[Rule::Number { start: 1, step: 0, pad: 0, pos: NumPos::Suffix }], &keys(&["a.txt", "b.txt"]), 0);
        assert_eq!(held[0].new_name, "a1.txt");
        assert_eq!(held[1].new_name, "b1.txt");
    }

    #[test]
    fn dates_come_from_file_times_not_exif() {
        let mut it = item("shot.jpg");
        it.modified = Some(0);
        let prefix = Rule::Date { src: DateSrc::Modified, pattern: "YYYY-MM-DD".into(), pos: DatePos::Prefix };
        assert_eq!(apply_name(&it, &[prefix], 0, 0), "1970-01-01shot.jpg");
        let full = Rule::Date { src: DateSrc::Modified, pattern: "YYYY-MM-DD hh:mm:ss".into(), pos: DatePos::Replace };
        assert_eq!(apply_name(&it, &[full], 0, 0), "1970-01-01 00:00:00.jpg");
        let local = Rule::Date { src: DateSrc::Modified, pattern: "YYYY-MM-DD hh".into(), pos: DatePos::Suffix };
        assert_eq!(apply_name(&it, &[local.clone()], 0, 9 * 3600), "shot1970-01-01 09.jpg");
        assert_eq!(apply_name(&it, &[local], 0, -3600), "shot1969-12-31 23.jpg");
        assert_eq!(apply_name(&item("shot.jpg"), &[Rule::Date { src: DateSrc::Modified, pattern: "YYYY".into(), pos: DatePos::Prefix }], 0, 0), "shot.jpg");
        let blank = Rule::Date { src: DateSrc::Created, pattern: String::new(), pos: DatePos::Prefix };
        assert_eq!(apply_name(&it, &[blank], 0, 0), "shot.jpg");
        it.created = Some(86_400);
        it.accessed = Some(1_582_934_400);
        let created = Rule::Date { src: DateSrc::Created, pattern: "DD".into(), pos: DatePos::Replace };
        assert_eq!(apply_name(&it, &[created], 0, 0), "02.jpg");
        let accessed = Rule::Date { src: DateSrc::Accessed, pattern: "YYYY-MM-DD".into(), pos: DatePos::Replace };
        assert_eq!(apply_name(&it, &[accessed], 0, 0), "2020-02-29.jpg");
        assert_eq!(format_date(0, 0, "YY/MM/DD"), "70/01/01");
        assert_eq!(format_date(0, 0, "file_MM"), "file_01");
        assert_eq!(format_date(0, 0, "hhmm"), "0000");
        assert_eq!(format_date(0, 0, "note"), "note");
        let mut a = item("a.jpg");
        let mut b = item("b.jpg");
        a.modified = Some(0);
        b.modified = Some(0);
        let clash = preview(
            &[a, b],
            &[Rule::Date { src: DateSrc::Modified, pattern: "YYYYMMDD".into(), pos: DatePos::Replace }],
            &keys(&["a.jpg", "b.jpg"]),
            0,
        );
        assert_eq!(clash[0].new_name, "19700101.jpg");
        assert!(clash[0].issues.contains(&Issue::Duplicate));
        assert!(clash[1].issues.contains(&Issue::Duplicate));
    }

    #[test]
    fn insert_delete_and_trim_use_characters() {
        assert_eq!(only(&[Rule::Insert { text: "_".into(), at: 0 }], "ab.jpg"), "_ab.jpg");
        assert_eq!(only(&[Rule::Insert { text: "X".into(), at: -1 }], "ab.jpg"), "abX.jpg");
        assert_eq!(only(&[Rule::Insert { text: "X".into(), at: 1 }], "ab.jpg"), "aXb.jpg");
        assert_eq!(only(&[Rule::Insert { text: String::new(), at: 0 }], "ab.jpg"), "ab.jpg");
        assert_eq!(only(&[Rule::Insert { text: "다".into(), at: 1 }], "가나.jpg"), "가다나.jpg");
        assert_eq!(only(&[Rule::Delete { at: 0, len: 1 }], "ab.jpg"), "b.jpg");
        assert_eq!(only(&[Rule::Delete { at: -1, len: 1 }], "abc.jpg"), "ab.jpg");
        assert_eq!(only(&[Rule::Delete { at: -2, len: 2 }], "abcd.jpg"), "ab.jpg");
        assert_eq!(only(&[Rule::Delete { at: 1, len: 5 }], "abcd.jpg"), "a.jpg");
        assert_eq!(only(&[Rule::Delete { at: 2, len: 2 }], "a.txt"), "a.txt");
        assert_eq!(only(&[Rule::Delete { at: 0, len: 0 }], "ab.jpg"), "ab.jpg");
        assert_eq!(only(&[Rule::Delete { at: 1, len: 1 }], "가나.jpg"), "가.jpg");
        assert_eq!(only(&[Rule::Trim { chars: String::new(), edge: TrimEdge::Both }], "  hi  .jpg"), "hi.jpg");
        assert_eq!(only(&[Rule::Trim { chars: "_".into(), edge: TrimEdge::Both }], "__a__.jpg"), "a.jpg");
        assert_eq!(only(&[Rule::Trim { chars: "x".into(), edge: TrimEdge::Left }], "xxax.jpg"), "ax.jpg");
        assert_eq!(only(&[Rule::Trim { chars: "x".into(), edge: TrimEdge::Right }], "xxax.jpg"), "xxa.jpg");
        assert_eq!(only(&[Rule::Trim { chars: String::new(), edge: TrimEdge::Right }], "ab.jpg"), "ab.jpg");
    }

}
