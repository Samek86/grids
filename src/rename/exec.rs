// IFileOperation on a worker thread, so the window stays up and Windows draws the progress.
// Waves from `plan` run in order. A later wave that fails rolls the finished waves back.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use serde::{Deserialize, Serialize};

use crate::imports::*;

use super::engine::Step;

pub(crate) const WM_RENAME_DONE: u32 = WM_APP + 81;

enum Outcome {
    Ok,
    Cancel,
    Failed(String),
}

struct Done {
    outcome: Outcome,
}

static DONE: Mutex<Option<Done>> = Mutex::new(None);

const MAX_UNDO: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    dir: String,
    now: String,
    was: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Batch {
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Disk {
    batches: Vec<Batch>,
}

static HISTORY: Mutex<Vec<Batch>> = Mutex::new(Vec::new());
static LOADED: Once = Once::new();

pub(crate) fn undo_count() -> usize {
    ensure_history();
    HISTORY.lock().map(|g| g.len()).unwrap_or(0)
}

pub(crate) fn last_undo() -> Option<Vec<(String, String, String)>> {
    ensure_history();
    HISTORY.lock().ok().and_then(|g| g.last().map(pairs_of))
}

pub(crate) fn push_undo(pairs: Vec<(String, String, String)>) {
    if pairs.is_empty() {
        return;
    }
    ensure_history();
    let Some(snap) = HISTORY.lock().ok().map(|mut g| {
        push_batch(&mut g, Batch { entries: entries_of(&pairs) });
        g.clone()
    }) else {
        return;
    };
    save_history(&undo_path(), &snap);
}

pub(crate) fn pop_undo() {
    ensure_history();
    let Some(snap) = HISTORY.lock().ok().map(|mut g| {
        g.pop();
        g.clone()
    }) else {
        return;
    };
    save_history(&undo_path(), &snap);
}

fn entries_of(pairs: &[(String, String, String)]) -> Vec<Entry> {
    pairs.iter().map(|(dir, now, was)| Entry { dir: dir.clone(), now: now.clone(), was: was.clone() }).collect()
}

fn pairs_of(batch: &Batch) -> Vec<(String, String, String)> {
    batch.entries.iter().map(|e| (e.dir.clone(), e.now.clone(), e.was.clone())).collect()
}

fn push_batch(hist: &mut Vec<Batch>, batch: Batch) {
    if batch.entries.is_empty() {
        return;
    }
    hist.push(batch);
    while hist.len() > MAX_UNDO {
        hist.remove(0);
    }
}

fn undo_path() -> PathBuf {
    crate::settings::store::config_dir().join("rename-undo.json")
}

fn ensure_history() {
    LOADED.call_once(|| {
        let loaded = load_history(&undo_path());
        if let Ok(mut g) = HISTORY.lock() {
            *g = loaded;
        }
    });
}

fn load_history(path: &Path) -> Vec<Batch> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    serde_json::from_str::<Disk>(&text).map(|d| d.batches).unwrap_or_default()
}

fn save_history(path: &Path, hist: &[Batch]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string_pretty(&Disk { batches: hist.to_vec() }) {
        let _ = std::fs::write(path, body);
    }
}

pub(crate) struct Finished {
    pub ok: bool,
    pub cancel: bool,
    pub err: String,
}

pub(crate) fn take_done() -> Option<Finished> {
    let done = DONE.lock().ok().and_then(|mut g| g.take())?;
    let (ok, cancel, err) = match done.outcome {
        Outcome::Ok => (true, false, String::new()),
        Outcome::Cancel => (false, true, String::new()),
        Outcome::Failed(e) => (false, false, e),
    };
    Some(Finished { ok, cancel, err })
}

pub(crate) unsafe fn start(notify: HWND, waves: Vec<Vec<Step>>) {
    // HWND is a raw pointer. Pass the address as an integer so the worker can post back.
    let notify = notify.0 as isize;
    let job = std::thread::Builder::new().name("panegrid-rename".into()).spawn(move || {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let outcome = run_waves(&waves);
        if com {
            CoUninitialize();
        }
        if let Ok(mut g) = DONE.lock() {
            *g = Some(Done { outcome });
        }
        let notify = HWND(notify as *mut _);
        if !notify.0.is_null() && IsWindow(Some(notify)).as_bool() {
            let _ = PostMessageW(Some(notify), WM_RENAME_DONE, WPARAM(0), LPARAM(0));
        }
    });
    if let Ok(job) = job {
        crate::JOBS.with_borrow_mut(|j| {
            j.retain(|h| !h.is_finished());
            j.push(job);
        });
    } else if notify != 0 {
        if let Ok(mut g) = DONE.lock() {
            *g = Some(Done { outcome: Outcome::Failed(String::new()) });
        }
        let _ = PostMessageW(Some(HWND(notify as *mut _)), WM_RENAME_DONE, WPARAM(0), LPARAM(0));
    }
}

fn run_waves(waves: &[Vec<Step>]) -> Outcome {
    let mut done: Vec<Vec<Step>> = Vec::new();
    for wave in waves {
        if wave.is_empty() {
            continue;
        }
        match unsafe { run_wave(wave) } {
            Wave::Ok => done.push(wave.clone()),
            Wave::Cancel => {
                rollback(&done);
                return Outcome::Cancel;
            }
            Wave::Failed(e) => {
                rollback(&done);
                return Outcome::Failed(e);
            }
        }
    }
    Outcome::Ok
}

enum Wave {
    Ok,
    Cancel,
    Failed(String),
}

unsafe fn run_wave(wave: &[Step]) -> Wave {
    let op: IFileOperation = match CoCreateInstance(&FileOperation, None, CLSCTX_ALL) {
        Ok(op) => op,
        Err(e) => return Wave::Failed(e.message().to_string()),
    };
    // No confirmation: the preview already blocked invalid names. Errors come back here
    // instead of a second dialog. The progress window still shows.
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOFX_ADDUNDORECORD;
    if let Err(e) = op.SetOperationFlags(flags) {
        return Wave::Failed(e.message().to_string());
    }
    for step in wave {
        let path = super::engine::join_path(&step.dir, &step.from);
        let item: IShellItem = match SHCreateItemFromParsingName(&HSTRING::from(&path), None) {
            Ok(it) => it,
            Err(e) => return Wave::Failed(e.message().to_string()),
        };
        let new_name = HSTRING::from(&step.to);
        if let Err(e) = op.RenameItem(&item, PCWSTR(new_name.as_ptr()), None) {
            return Wave::Failed(e.message().to_string());
        }
    }
    if let Err(e) = op.PerformOperations() {
        if cancelled(&e) {
            return Wave::Cancel;
        }
        return Wave::Failed(e.message().to_string());
    }
    if op.GetAnyOperationsAborted().unwrap_or(BOOL(0)).as_bool() {
        return Wave::Cancel;
    }
    Wave::Ok
}

fn cancelled(e: &windows::core::Error) -> bool {
    e.code() == COPYENGINE_E_USER_CANCELLED || e.code() == ERROR_CANCELLED.to_hresult()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_caps_at_twenty_and_roundtrips() {
        let mut hist = Vec::new();
        for i in 0..25 {
            push_batch(&mut hist, Batch { entries: vec![Entry { dir: r"C:\pics".into(), now: format!("n{i}"), was: format!("w{i}") }] });
        }
        assert_eq!(hist.len(), 20);
        assert_eq!(hist[0].entries[0].now, "n5");
        assert_eq!(hist[19].entries[0].now, "n24");
        let folder = std::env::temp_dir().join(format!("panegrid-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let path = folder.join("rename-undo.json");
        save_history(&path, &hist);
        assert_eq!(load_history(&path), hist);
        let _ = std::fs::remove_dir_all(&folder);
        push_batch(&mut hist, Batch { entries: Vec::new() });
        assert_eq!(hist.len(), 20);
    }
}

fn rollback(done: &[Vec<Step>]) {
    for wave in done.iter().rev() {
        let rev: Vec<Step> = wave
            .iter()
            .rev()
            .map(|s| Step { dir: s.dir.clone(), from: s.to.clone(), to: s.from.clone() })
            .collect();
        if !rev.is_empty() {
            let _ = unsafe { run_wave(&rev) };
        }
    }
}
