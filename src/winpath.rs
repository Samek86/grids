//! Path helpers that must not panic on a multibyte folder name.
//!
//! `\\?\` is only added once the UTF-16 length reaches the classic MAX_PATH edge.
//! A short name such as `F:\CAC\サンプル` stays as it is: the prefix changes Win32 parsing.

/// `&s[..n]` panics when `n` lands inside a character. This returns `None` instead.
pub(crate) fn split_prefix(s: &str, n: usize) -> Option<&str> {
    s.get(..n)
}

/// Wide length of `path`, not its UTF-8 byte length.
pub(crate) fn utf16_len(path: &str) -> usize {
    path.encode_utf16().count()
}

/// Prefix `\\?\` or `\\?\UNC\` only when the path is long enough to need it.
pub(crate) fn win_long_path(path: &str) -> String {
    if path.starts_with(r"\\?\") {
        return path.to_string();
    }
    // 248 leaves room for a trailing NUL inside the 260-unit classic limit.
    if utf16_len(path) < 248 {
        return path.to_string();
    }
    if let Some(rest) = path.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{rest}")
    } else {
        format!(r"\\?\{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_prefix_is_not_sliced_or_rewritten() {
        assert_eq!(split_prefix("サンプル", 2), None);
        assert_eq!(split_prefix("サンプル", 3), Some("サ"));
        let folder = r"F:\CAC\サンプル";
        assert!(utf16_len(folder) < 248);
        assert_eq!(win_long_path(folder), folder);
        assert_eq!(win_long_path(r"\\?\C:\already"), r"\\?\C:\already");
    }

    #[test]
    fn long_paths_gain_the_prefix_once() {
        let long = format!(r"C:\{}", "a".repeat(260));
        let got = win_long_path(&long);
        assert!(got.starts_with(r"\\?\C:\"), "{got}");
        assert!(!got[4..].contains(r"\\?\"));
        let unc = format!(r"\\server\share\{}", "b".repeat(260));
        let got = win_long_path(&unc);
        assert!(got.starts_with(r"\\?\UNC\server\share\"), "{got}");
    }
}
