// Saved rule stacks. Files live in `%AppData%\Panegrid\rename-presets`, never in panegrid.json.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::engine::Rule;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Preset {
    pub name: String,
    pub rules: Vec<Rule>,
    pub files: bool,
    pub folders: bool,
    pub extensions: String,
}

pub(crate) fn dir() -> PathBuf {
    crate::settings::store::config_dir().join("rename-presets")
}

pub(crate) fn list(folder: &Path) -> Vec<Preset> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(preset) = serde_json::from_str::<Preset>(&text) else { continue };
        if preset.name.trim().is_empty() {
            continue;
        }
        out.push(preset);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub(crate) fn save(folder: &Path, preset: &Preset) -> Result<(), String> {
    let name = preset.name.trim();
    if name.is_empty() {
        return Err("empty".into());
    }
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let stored = Preset { name: name.to_string(), ..preset.clone() };
    let body = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
    std::fs::write(path_for(folder, name), body).map_err(|e| e.to_string())
}

pub(crate) fn delete(folder: &Path, name: &str) -> Result<(), String> {
    let path = path_for(folder, name.trim());
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn path_for(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{}.json", fnv(name)))
}

fn fnv(name: &str) -> String {
    let mut h = 0xcbf29ce484222325u64;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rename::engine::{self, CaseMode, DatePos, DateSrc, NumPos, TrimEdge};

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("panegrid-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn presets_roundtrip_every_rule_and_ignore_junk() {
        let folder = scratch();
        let preset = Preset {
            name: "사진 정리".into(),
            files: true,
            folders: false,
            extensions: "jpg, png".into(),
            rules: vec![
                Rule::Replace { find: "img".into(), replace: "pic".into(), case_sensitive: false, include_ext: false },
                Rule::Number { start: 1, step: 2, pad: 3, pos: NumPos::Suffix },
                Rule::Date { src: DateSrc::Modified, pattern: "YYYY-MM-DD".into(), pos: DatePos::Prefix },
                Rule::Insert { text: "_".into(), at: -1 },
                Rule::Delete { at: 0, len: 1 },
                Rule::Trim { chars: String::new(), edge: TrimEdge::Both },
                Rule::Case { mode: CaseMode::Lower, include_ext: true },
                Rule::Regex { pattern: r"(\d+)".into(), replace: "n$1".into(), case_sensitive: true, include_ext: false },
            ],
        };
        save(&folder, &preset).unwrap();
        std::fs::write(folder.join("not-a-preset.txt"), "nope").unwrap();
        std::fs::write(folder.join("bad.json"), "{").unwrap();
        let loaded = list(&folder);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], Preset { name: preset.name.clone(), ..preset.clone() });
        let filter = engine::Filter { files: true, folders: false, extensions: engine::parse_extensions(&loaded[0].extensions) };
        assert_eq!(filter.extensions, vec!["jpg".to_string(), "png".to_string()]);
        delete(&folder, &preset.name).unwrap();
        assert!(list(&folder).is_empty());
        delete(&folder, &preset.name).unwrap();
        let _ = std::fs::remove_dir_all(&folder);
    }
}
