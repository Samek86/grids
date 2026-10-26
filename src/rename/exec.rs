// IFileOperation on a worker thread, so the window stays up and Windows draws the progress.
// Waves from `plan` run in order. A later wave that fails rolls the finished waves back.

use std::sync::Mutex;

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
static LAST_UNDO: Mutex<Option<Vec<(String, String, String)>>> = Mutex::new(None);

pub(crate) fn last_undo() -> Option<Vec<(String, String, String)>> {
    LAST_UNDO.lock().ok().and_then(|g| g.clone())
}

pub(crate) fn clear_undo() {
    if let Ok(mut g) = LAST_UNDO.lock() {
        *g = None;
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

/// `undo` is what a later Undo should run: `(dir, name-after, name-before)`.
/// `record` stores it. An undo pass passes `record = false` and clears the last batch.
pub(crate) unsafe fn start(notify: HWND, waves: Vec<Vec<Step>>, undo: Vec<(String, String, String)>, record: bool) {
    // HWND is a raw pointer. Pass the address as an integer so the worker can post back.
    let notify = notify.0 as isize;
    let job = std::thread::Builder::new().name("panegrid-rename".into()).spawn(move || {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let outcome = run_waves(&waves);
        if com {
            CoUninitialize();
        }
        let undo = match &outcome {
            Outcome::Ok if record => undo,
            Outcome::Ok => {
                clear_undo();
                Vec::new()
            }
            _ => Vec::new(),
        };
        if matches!(outcome, Outcome::Ok) && record {
            if let Ok(mut g) = LAST_UNDO.lock() {
                *g = Some(undo.clone());
            }
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
