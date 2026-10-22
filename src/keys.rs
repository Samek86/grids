// One table for every app shortcut. The settings page rebinds rows in this table,
// and keydown reads it, so a change applies on the next key without a restart.
// Chords are "Ctrl+Shift+T". A digit-range command stores "Ctrl+1 … Ctrl+9"
// and matches 1 through 9 with those modifiers.

use std::cell::RefCell;

use serde_json::{Map, Value};

use crate::tr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Chord {
    pub vk: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Any main-row digit 1–9. `vk` is VK_1 (0x31).
    pub digits: bool,
}

struct Cmd {
    id: &'static str,
    cat: [&'static str; 3],
    act: [&'static str; 3],
    vk: u16,
    ctrl: bool,
    shift: bool,
    alt: bool,
    /// Pressing a digit rebinds the whole 1–9 range (Go to pane).
    range: bool,
}

impl Cmd {
    fn chord(&self) -> Chord {
        Chord { vk: self.vk, ctrl: self.ctrl, shift: self.shift, alt: self.alt, digits: self.range }
    }
}

const fn cmd(id: &'static str, cat: [&'static str; 3], act: [&'static str; 3], vk: u16, ctrl: bool, shift: bool, alt: bool, range: bool) -> Cmd {
    Cmd { id, cat, act, vk, ctrl, shift, alt, range }
}

const fn s(en: &'static str, ko: &'static str, ja: &'static str) -> [&'static str; 3] {
    [en, ko, ja]
}

const TAB: [&str; 3] = s("Tab", "탭", "タブ");
const PANE: [&str; 3] = s("Pane", "패널", "ペイン");
const MOVE: [&str; 3] = s("Move", "이동", "移動");
const VIEW: [&str; 3] = s("View", "보기", "表示");
const FILE: [&str; 3] = s("File", "파일", "ファイル");
const SESS: [&str; 3] = s("Session", "세션", "セッション");
const FAV: [&str; 3] = s("Favourites", "즐겨찾기", "お気に入り");
const HELP: [&str; 3] = s("Help", "도움말", "ヘルプ");
const WIN: [&str; 3] = s("Window", "창", "ウィンドウ");

const CMDS: &[Cmd] = &[
    cmd("tab.new", TAB, s("New tab", "새 탭", "新しいタブ"), 0x54, true, false, false, false),
    cmd("tab.close", TAB, s("Close tab", "탭 닫기", "タブを閉じる"), 0x57, true, false, false, false),
    cmd("tab.next", TAB, s("Next tab", "다음 탭", "次のタブ"), 0x09, true, false, false, false),
    cmd("tab.prev", TAB, s("Previous tab", "이전 탭", "前のタブ"), 0x09, true, true, false, false),
    cmd("tab.reopen", TAB, s("Reopen closed tab", "닫은 탭 다시 열기", "閉じたタブを開き直す"), 0x54, true, true, false, false),
    cmd("pane.next", PANE, s("Next pane", "다음 패널", "次のペイン"), 0x22, true, false, false, false),
    cmd("pane.prev", PANE, s("Previous pane", "이전 패널", "前のペイン"), 0x21, true, false, false, false),
    cmd("pane.goto", PANE, s("Go to pane 1–9", "1–9번 패널로 이동", "ペイン 1〜9 へ移動"), 0x31, true, false, false, true),
    cmd("pane.left", MOVE, s("Pane to the left", "왼쪽 패널", "左のペイン"), 0x25, false, true, true, false),
    cmd("pane.right", MOVE, s("Pane to the right", "오른쪽 패널", "右のペイン"), 0x27, false, true, true, false),
    cmd("pane.up", MOVE, s("Pane above", "위 패널", "上のペイン"), 0x26, false, true, true, false),
    cmd("pane.down", MOVE, s("Pane below", "아래 패널", "下のペイン"), 0x28, false, true, true, false),
    cmd("nav.back", MOVE, s("Back", "뒤로", "戻る"), 0x25, false, false, true, false),
    cmd("nav.fwd", MOVE, s("Forward", "앞으로", "進む"), 0x27, false, false, true, false),
    cmd("nav.up", MOVE, s("Parent folder", "상위 폴더", "上のフォルダー"), 0x26, false, false, true, false),
    cmd("nav.recent", MOVE, s("Recent folders", "최근 폴더", "最近のフォルダー"), 0x28, false, false, true, false),
    cmd("nav.addr", MOVE, s("Address bar", "주소창", "アドレス バー"), 0x4C, true, false, false, false),
    cmd("nav.addr2", MOVE, s("Address bar", "주소창", "アドレス バー"), 0x44, false, false, true, false),
    cmd("nav.addr3", MOVE, s("Address bar", "주소창", "アドレス バー"), 0x73, false, false, false, false),
    cmd("view.filter", VIEW, s("Filter", "필터", "フィルター"), 0x46, true, false, false, false),
    cmd("view.peek", VIEW, s("Quick look", "빠른 미리보기", "クイックルック"), 0x72, false, false, false, false),
    cmd("focus.next", VIEW, s("Next focus", "다음 포커스", "次のフォーカス"), 0x75, false, false, false, false),
    cmd("focus.prev", VIEW, s("Previous focus", "이전 포커스", "前のフォーカス"), 0x75, false, true, false, false),
    cmd("view.hidden", VIEW, s("Show hidden files", "숨김 파일 표시", "隠しファイルを表示"), 0x48, true, false, false, false),
    cmd("view.panel", VIEW, s("Show the left panel", "왼쪽 패널 표시", "左のパネルを表示"), 0x45, true, true, false, false),
    cmd("view.zoom", VIEW, s("Maximise / restore the pane", "패널 크게 보기 / 복원", "ペインを最大化 / 元に戻す"), 0x4D, true, true, false, false),
    cmd("file.path", FILE, s("Copy full path", "전체 경로 복사", "フルパスをコピー"), 0x43, true, true, false, false),
    cmd("file.sel", FILE, s("Select by pattern", "패턴으로 선택", "パターンで選択"), 0x6B, false, false, false, false),
    cmd("file.sel2", FILE, s("Select by pattern", "패턴으로 선택", "パターンで選択"), 0x41, true, true, false, false),
    cmd("file.desel", FILE, s("Deselect by pattern", "패턴으로 선택 해제", "パターンで選択解除"), 0x6D, false, false, false, false),
    cmd("file.invert", FILE, s("Invert selection", "선택 반전", "選択を反転"), 0x6A, false, false, false, false),
    cmd("file.new", FILE, s("New text file", "새 텍스트 문서", "新しいテキスト ファイル"), 0x4E, true, false, true, false),
    cmd("file.dir", FILE, s("New folder", "새 폴더", "新しいフォルダー"), 0x4E, true, true, false, false),
    cmd("sess.save", SESS, s("Save session", "세션 저장", "セッションを保存"), 0x53, true, false, false, false),
    cmd("fav.list", FAV, s("Favourites list", "즐겨찾기 목록", "お気に入り一覧"), 0x42, true, false, false, false),
    cmd("app.settings", HELP, s("Settings", "설정", "設定"), 0xBC, true, false, false, false),
    cmd("app.help", HELP, s("Shortcut help", "단축키 도움말", "ショートカットのヘルプ"), 0x70, false, false, false, false),
    cmd("app.keys", HELP, s("Shortcut list", "단축키 목록", "ショートカット一覧"), 0xBF, true, false, false, false),
    cmd("win.new", WIN, s("New window", "새 창", "新しいウィンドウ"), 0x4E, true, false, false, false),
];

struct KeyState {
    binds: Vec<Chord>,
    extra: Map<String, Value>,
    capture: Option<usize>,
    swallow: u16,
    modal: bool,
}

impl KeyState {
    fn fresh() -> Self {
        KeyState { binds: CMDS.iter().map(Cmd::chord).collect(), extra: Map::new(), capture: None, swallow: 0, modal: false }
    }
}

thread_local! {
    static STATE: RefCell<KeyState> = RefCell::new(KeyState::fresh());
}

fn with<R>(f: impl FnOnce(&mut KeyState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub(crate) fn len() -> usize {
    CMDS.len()
}

pub(crate) fn index(id: &str) -> Option<usize> {
    CMDS.iter().position(|c| c.id == id)
}

pub(crate) fn id_of(i: usize) -> &'static str {
    CMDS.get(i).map(|c| c.id).unwrap_or("")
}

pub(crate) fn is_command(id: &str) -> bool {
    index(id).is_some()
}

fn label(t: [&'static str; 3]) -> String {
    tr(t[0], t[1], t[2]).to_string()
}

pub(crate) fn action(i: usize) -> String {
    CMDS.get(i).map(|c| label(c.act)).unwrap_or_default()
}

pub(crate) fn chord_text(i: usize) -> String {
    with(|s| s.binds.get(i).copied().map(|c| format_chord(&c)).unwrap_or_default())
}

pub(crate) fn row(i: usize) -> (String, String, String) {
    let Some(c) = CMDS.get(i) else { return (String::new(), String::new(), String::new()) };
    (label(c.cat), label(c.act), chord_text(i))
}

pub(crate) fn is_custom(i: usize) -> bool {
    with(|s| CMDS.get(i).is_some_and(|c| s.binds.get(i).copied() != Some(c.chord())))
}

pub(crate) fn default_chord(i: usize) -> Option<Chord> {
    CMDS.get(i).map(Cmd::chord)
}

pub(crate) fn same(i: usize, c: &Chord) -> bool {
    with(|s| s.binds.get(i).copied() == Some(*c))
}

pub(crate) fn begin(i: usize) {
    if i < CMDS.len() {
        with(|s| s.capture = Some(i));
    }
}

pub(crate) fn cancel() {
    with(|s| s.capture = None);
}

pub(crate) fn capturing() -> bool {
    with(|s| s.capture.is_some())
}

pub(crate) fn capture_row() -> Option<usize> {
    with(|s| s.capture)
}

pub(crate) fn set_modal(on: bool) {
    with(|s| s.modal = on);
}

pub(crate) fn modal() -> bool {
    with(|s| s.modal)
}

pub(crate) fn swallow(vk: u16) {
    with(|s| s.swallow = vk);
}

pub(crate) fn swallows(vk: u16) -> bool {
    vk != 0 && with(|s| s.swallow == vk)
}

pub(crate) fn clear_swallow(vk: u16) {
    with(|s| {
        if s.swallow == vk {
            s.swallow = 0;
        }
    });
}

fn overlaps(a: Chord, b: Chord) -> bool {
    if a.ctrl != b.ctrl || a.shift != b.shift || a.alt != b.alt {
        return false;
    }
    match (a.digits, b.digits) {
        (true, true) => true,
        (true, false) => (0x31..=0x39).contains(&b.vk),
        (false, true) => (0x31..=0x39).contains(&a.vk),
        (false, false) => a.vk == b.vk,
    }
}

pub(crate) fn conflict(row: usize, chord: Chord) -> Option<usize> {
    with(|s| s.binds.iter().enumerate().find(|(i, c)| *i != row && overlaps(**c, chord)).map(|(i, _)| i))
}

/// `chord` becomes row's binding. The previous owner of that chord, if any, receives row's old one.
pub(crate) fn rebind(row: usize, chord: Chord) {
    with(|s| {
        if row >= s.binds.len() || s.binds[row] == chord {
            return;
        }
        let old = s.binds[row];
        if let Some(other) = s.binds.iter().enumerate().find(|(i, c)| *i != row && overlaps(**c, chord)).map(|(i, _)| i) {
            s.binds[row] = chord;
            s.binds[other] = old;
        } else {
            s.binds[row] = chord;
        }
    });
}

pub(crate) fn reset_all() {
    with(|s| {
        s.binds = CMDS.iter().map(Cmd::chord).collect();
        s.capture = None;
    });
}

pub(crate) enum Offer {
    Ignore,
    Cancel,
    Same,
    Bind { row: usize, chord: Chord, other: Option<usize> },
}

fn modifier(vk: u16) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 | 0x5B | 0x5C)
}

pub(crate) fn offer(vk: u16, ctrl: bool, shift: bool, alt: bool) -> Offer {
    let Some(row) = with(|s| s.capture) else { return Offer::Ignore };
    if modifier(vk) {
        return Offer::Ignore;
    }
    if vk == 0x1B && !ctrl && !shift && !alt {
        with(|s| s.capture = None);
        return Offer::Cancel;
    }
    let mut chord = Chord { vk, ctrl, shift, alt, digits: false };
    if CMDS.get(row).is_some_and(|c| c.range) && (0x31..=0x39).contains(&vk) {
        chord.vk = 0x31;
        chord.digits = true;
    }
    with(|s| s.capture = None);
    if with(|s| s.binds.get(row).copied() == Some(chord)) {
        return Offer::Same;
    }
    let other = conflict(row, chord);
    Offer::Bind { row, chord, other }
}

pub(crate) fn lookup(vk: u16, ctrl: bool, shift: bool, alt: bool) -> Option<usize> {
    with(|s| {
        let exact = s.binds.iter().enumerate().find(|(_, c)| !c.digits && c.vk == vk && c.ctrl == ctrl && c.shift == shift && c.alt == alt);
        if let Some((i, _)) = exact {
            return Some(i);
        }
        if !(0x31..=0x39).contains(&vk) {
            return None;
        }
        s.binds.iter().enumerate().find(|(_, c)| c.digits && c.ctrl == ctrl && c.shift == shift && c.alt == alt).map(|(i, _)| i)
    })
}

fn untangle(binds: &mut [Chord]) {
    for i in 0..binds.len() {
        let clash = binds.iter().take(i).any(|c| overlaps(*c, binds[i]));
        if !clash {
            continue;
        }
        let def = CMDS[i].chord();
        if binds.iter().take(i).any(|c| overlaps(*c, def)) {
            continue;
        }
        binds[i] = def;
    }
}

pub(crate) fn load_bindings(map: &Map<String, Value>) {
    let mut binds: Vec<Chord> = CMDS.iter().map(Cmd::chord).collect();
    let mut extra = Map::new();
    for (k, v) in map {
        match index(k) {
            Some(i) => {
                if let Some(c) = v.as_str().and_then(parse_chord) {
                    binds[i] = c;
                }
            }
            None => {
                extra.insert(k.clone(), v.clone());
            }
        }
    }
    untangle(&mut binds);
    with(|s| {
        s.binds = binds;
        s.extra = extra;
        s.capture = None;
    });
}

pub(crate) fn bindings_json() -> Map<String, Value> {
    with(|s| {
        let mut m = s.extra.clone();
        for (i, c) in CMDS.iter().enumerate() {
            m.remove(c.id);
            if s.binds.get(i).copied().is_some_and(|b| b != c.chord()) {
                if let Some(b) = s.binds.get(i) {
                    m.insert(c.id.to_string(), Value::String(format_chord(b)));
                }
            }
        }
        m
    })
}

pub(crate) fn format_chord(c: &Chord) -> String {
    if c.digits {
        let mut one = *c;
        one.digits = false;
        one.vk = 0x31;
        let mut nine = one;
        nine.vk = 0x39;
        return format!("{} … {}", format_one(&one), format_one(&nine));
    }
    format_one(c)
}

fn format_one(c: &Chord) -> String {
    let mut p = Vec::new();
    if c.ctrl {
        p.push("Ctrl".to_string());
    }
    if c.alt {
        p.push("Alt".to_string());
    }
    if c.shift {
        p.push("Shift".to_string());
    }
    p.push(key_name(c.vk));
    p.join("+")
}

fn key_name(vk: u16) -> String {
    match vk {
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x08 => "Back".into(),
        0x2E => "Del".into(),
        0x30..=0x39 => char::from(vk as u8).to_string(),
        0x41..=0x5A => char::from(vk as u8).to_string(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        0x6A => "Num*".into(),
        0x6B => "Num+".into(),
        0x6D => "Num-".into(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0xBC => ",".into(),
        0xBF => "/".into(),
        _ => format!("VK_{vk:02X}"),
    }
}

pub(crate) fn parse_chord(s: &str) -> Option<Chord> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (head, digits) = if let Some((l, _)) = s.split_once('…') {
        (l.trim(), true)
    } else if let Some((l, _)) = s.split_once("...") {
        (l.trim(), true)
    } else {
        (s, false)
    };
    let mut parts: Vec<&str> = head.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    // "Num+" keeps its trailing plus; every other '+' separates a modifier.
    let plus = head.trim_end().ends_with('+') && parts.last().is_some_and(|p| p.eq_ignore_ascii_case("num"));
    let key = if plus { format!("{}+", parts.pop()?) } else { parts.pop()?.to_string() };
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    for p in parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            _ => return None,
        }
    }
    let vk = parse_key(&key)?;
    let mut c = Chord { vk, ctrl, shift, alt, digits: false };
    if digits {
        if !(0x31..=0x39).contains(&vk) {
            return None;
        }
        c.vk = 0x31;
        c.digits = true;
    }
    Some(c)
}

fn parse_key(k: &str) -> Option<u16> {
    match k {
        "←" => return Some(0x25),
        "→" => return Some(0x27),
        "↑" => return Some(0x26),
        "↓" => return Some(0x28),
        "," => return Some(0xBC),
        "/" => return Some(0xBF),
        _ => {}
    }
    let low = k.to_ascii_lowercase();
    match low.as_str() {
        "tab" => Some(0x09),
        "enter" | "return" => Some(0x0D),
        "esc" | "escape" => Some(0x1B),
        "space" => Some(0x20),
        "pageup" | "pgup" | "prior" => Some(0x21),
        "pagedown" | "pgdn" | "next" => Some(0x22),
        "left" => Some(0x25),
        "up" => Some(0x26),
        "right" => Some(0x27),
        "down" => Some(0x28),
        "back" | "backspace" => Some(0x08),
        "del" | "delete" => Some(0x2E),
        "comma" => Some(0xBC),
        "slash" | "/" => Some(0xBF),
        _ => parse_key_more(&low, k),
    }
}

fn parse_key_more(low: &str, raw: &str) -> Option<u16> {
    if let Some(rest) = low.strip_prefix("vk_") {
        return u16::from_str_radix(rest, 16).ok();
    }
    match low {
        "num*" => return Some(0x6A),
        "num+" => return Some(0x6B),
        "num-" => return Some(0x6D),
        _ => {}
    }
    if let Some(rest) = low.strip_prefix("num") {
        let n: u16 = rest.parse().ok()?;
        return (n <= 9).then_some(0x60 + n);
    }
    if let Some(rest) = low.strip_prefix('f') {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            let n: u16 = rest.parse().ok()?;
            return (1..=24).contains(&n).then_some(0x70 + n - 1);
        }
    }
    let mut chars = raw.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_uppercase() as u16);
    }
    if c.is_ascii_digit() {
        return Some(c as u16);
    }
    None
}

const EXPLORER: [&str; 3] = [
    "F2, Del, Ctrl+C/X/V, F5\tSame as Explorer\n\n",
    "F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n",
    "F2, Del, Ctrl+C/X/V, F5\tエクスプローラーと同じ\n\n",
];

const NOTES_EN: &str = "\
Layout button: click for an even grid, press and drag for rows per column (arrow keys + Enter work too)\n\
⇄ button: open the selected folder (or the current one) in another pane, with this pane's columns; or copy / move the selected files to another pane's folder (up to 10,000)\n\
Save (Ctrl+S): overwrite the current session file; green dot = unsaved layout changes (not folder moves)\n\
Load: pick a saved session by its layout preview\n\
Double-click a splitter: split evenly\n\
Arguments: grids.exe C:\\a D:\\b ... or grids.exe work.grids\n\
If Grids is already running, a new window opens in that process.";

const NOTES_KO: &str = "\
레이아웃 버튼: 클릭 = 균등 배치, 누른 채 끌기 = 열마다 칸 수 (방향키+Enter도 가능)\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기, 열 순서·폭·정렬도 함께. 또는 선택한 파일을 다른 패널 폴더로 복사·이동 (최대 10,000개)\n\
저장 (Ctrl+S): 지금 세션 파일에 바로 덮어쓰기, 초록 점 = 저장 안 된 배치 변경(폴더 이동은 제외)\n\
불러오기: 저장한 세션을 배치 미리보기로 보고 한 번 클릭\n\
스플리터 더블클릭: 균등 분할\n\
실행 인자: grids.exe C:\\a D:\\b ... 또는 grids.exe 작업.grids\n\
이미 실행 중이면 그 프로세스에 새 창이 열립니다.";

const NOTES_JA: &str = "\
レイアウト ボタン: クリックで均等配置、押したままドラッグで列ごとの行数（方向キー+Enter も可）\n\
⇄ ボタン: 選択したフォルダー（なければ現在のフォルダー）を別ペインで開く。列の順序・幅・並べ替えも引き継ぐ。または選択したファイルを別ペインのフォルダーへコピー / 移動（最大 10,000 個）\n\
保存 (Ctrl+S): 現在のセッション ファイルに上書き。緑の点 = 未保存のレイアウト変更（フォルダー移動は含まない）\n\
読み込み: 保存したセッションをレイアウトプレビューで見てクリック\n\
スプリッターをダブルクリック: 均等に分割\n\
引数: grids.exe C:\\a D:\\b ... または grids.exe work.grids\n\
すでに起動している場合は、そのプロセスに新しいウィンドウを開きます。";

pub(crate) fn help_text() -> String {
    let mut s = String::new();
    for i in 0..CMDS.len() {
        s.push_str(&chord_text(i));
        s.push('\t');
        s.push_str(&action(i));
        s.push('\n');
    }
    s.push_str(tr(EXPLORER[0], EXPLORER[1], EXPLORER[2]));
    s.push_str(tr(NOTES_EN, NOTES_KO, NOTES_JA));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        load_bindings(&Map::new());
    }

    #[test]
    fn defaults_round_trip_and_match_the_old_chords() {
        fresh();
        let want = [
            ("tab.new", "Ctrl+T"),
            ("tab.close", "Ctrl+W"),
            ("tab.next", "Ctrl+Tab"),
            ("tab.prev", "Ctrl+Shift+Tab"),
            ("tab.reopen", "Ctrl+Shift+T"),
            ("pane.next", "Ctrl+PageDown"),
            ("pane.prev", "Ctrl+PageUp"),
            ("pane.goto", "Ctrl+1 … Ctrl+9"),
            ("pane.left", "Alt+Shift+←"),
            ("pane.right", "Alt+Shift+→"),
            ("nav.back", "Alt+←"),
            ("nav.up", "Alt+↑"),
            ("nav.addr", "Ctrl+L"),
            ("nav.addr2", "Alt+D"),
            ("nav.addr3", "F4"),
            ("file.sel", "Num+"),
            ("file.sel2", "Ctrl+Shift+A"),
            ("file.desel", "Num-"),
            ("file.invert", "Num*"),
            ("file.new", "Ctrl+Alt+N"),
            ("file.dir", "Ctrl+Shift+N"),
            ("app.settings", "Ctrl+,"),
            ("app.help", "F1"),
            ("app.keys", "Ctrl+/"),
            ("view.zoom", "Ctrl+Shift+M"),
            ("view.panel", "Ctrl+Shift+E"),
            ("focus.next", "F6"),
            ("focus.prev", "Shift+F6"),
            ("view.peek", "F3"),
            ("win.new", "Ctrl+N"),
        ];
        for (id, text) in want {
            let i = index(id).unwrap();
            assert_eq!(chord_text(i), text, "{id}");
            assert_eq!(parse_chord(text).unwrap(), default_chord(i).unwrap());
            assert_eq!(format_chord(&parse_chord(&text.to_ascii_lowercase()).unwrap()), text);
        }
        assert!(parse_chord("").is_none());
        assert!(parse_chord("Ctrl+Nope").is_none());
        assert_eq!(lookup(b'T' as u16, true, false, false), index("tab.new"));
        assert_eq!(lookup(b'T' as u16, true, true, false), index("tab.reopen"));
        assert_eq!(lookup(b'5' as u16, true, false, false), index("pane.goto"));
        assert_eq!(lookup(b'0' as u16, true, false, false), None);
        assert_eq!(lookup(b'5' as u16, true, true, false), None);
        assert_eq!(lookup(0x25, false, true, true), index("pane.left"));
        assert_eq!(lookup(0x25, false, false, true), index("nav.back"));
        assert_eq!(lookup(b'N' as u16, true, false, false), index("win.new"));
        assert_eq!(lookup(b'N' as u16, true, false, true), index("file.new"));
        assert_eq!(lookup(b'N' as u16, true, true, false), index("file.dir"));
    }

    #[test]
    fn rebind_conflict_swap_reset_and_unknown_keys() {
        fresh();
        let tab = index("tab.new").unwrap();
        let help = index("app.help").unwrap();
        let k = parse_chord("Ctrl+K").unwrap();
        assert_eq!(conflict(tab, k), None);
        rebind(tab, k);
        assert_eq!(lookup(b'K' as u16, true, false, false), Some(tab));
        assert_eq!(lookup(b'T' as u16, true, false, false), None);
        assert!(is_custom(tab));
        // Help takes Ctrl+T, which is free, then we point New tab back at Ctrl+T and Help receives Ctrl+K.
        rebind(help, parse_chord("Ctrl+T").unwrap());
        rebind(tab, parse_chord("Ctrl+T").unwrap());
        assert_eq!(lookup(b'T' as u16, true, false, false), Some(tab));
        assert_eq!(lookup(b'K' as u16, true, false, false), Some(help));
        assert_eq!(lookup(0x70, false, false, false), None);
        assert_eq!(chord_text(help), "Ctrl+K");

        let mut map = Map::new();
        map.insert("tab.new".into(), serde_json::json!("Ctrl+K"));
        map.insert("tab.close".into(), serde_json::json!("Ctrl+K"));
        map.insert("copy".into(), serde_json::json!("Ctrl+C"));
        map.insert("later".into(), serde_json::json!({"keep": true}));
        load_bindings(&map);
        assert_eq!(chord_text(tab), "Ctrl+K");
        assert_eq!(chord_text(index("tab.close").unwrap()), "Ctrl+W");
        let saved = bindings_json();
        assert_eq!(saved["tab.new"], "Ctrl+K");
        assert!(saved.get("tab.close").is_none());
        assert_eq!(saved["copy"], "Ctrl+C");
        assert_eq!(saved["later"]["keep"], true);
        reset_all();
        let cleared = bindings_json();
        assert!(cleared.get("tab.new").is_none());
        assert_eq!(cleared["copy"], "Ctrl+C");
        assert_eq!(lookup(b'T' as u16, true, false, false), Some(tab));

        begin(tab);
        assert!(capturing());
        assert!(matches!(offer(0x11, true, false, false), Offer::Ignore));
        assert!(matches!(offer(0x1B, false, false, false), Offer::Cancel));
        assert!(!capturing());
        begin(index("pane.goto").unwrap());
        match offer(b'5' as u16, true, true, false) {
            Offer::Bind { chord, other, .. } => {
                assert!(chord.digits);
                assert_eq!(format_chord(&chord), "Ctrl+Shift+1 … Ctrl+Shift+9");
                assert_eq!(other, None);
            }
            _ => panic!("digit rebind of go-to-pane should stay a range"),
        }
        fresh();
    }
}
