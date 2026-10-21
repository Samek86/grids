// Grids: multi-pane file manager. Shell tree on the left, up to 4 columns of up to 4 Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, Once};
use std::time::Instant;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS, SFGAO_FOLDER};
use windows::Win32::System::Threading::{OpenProcess, INFINITE, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{AccessibleObjectFromPoint, IAccessible, ROLE_SYSTEM_LIST};
use windows::Win32::UI::Controls::Dialogs::{ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
use windows::Win32::UI::Controls::{
    SetWindowTheme, BeginBufferedPaint, BufferedPaintInit, EndBufferedPaint, BPBF_TOPDOWNDIB, DRAWITEMSTRUCT, ODS_SELECTED, TOOLTIPS_CLASSW,
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDIS_HOT, CDIS_SELECTED, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT,
    HDF_CENTER, HDF_RIGHT, HDF_SORTDOWN, HDF_SORTUP, HDITEMW, HDI_FORMAT, HDI_TEXT, HDM_GETITEMW, HDN_BEGINTRACKW, HDN_ITEMCHANGINGW, HDN_TRACKW, HDI_WIDTH, NMHEADERW, LVCF_MINWIDTH, LVCOLUMNW, LVM_GETCOLUMNW, LVM_SETCOLUMNW,
    LVM_GETCOLUMNWIDTH, LVM_SETCOLUMNWIDTH, NMCUSTOMDRAW, NMHDR, NMLVCUSTOMDRAW, NM_CUSTOMDRAW,
    TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_UPDATETIPTEXTW, TTTOOLINFOW, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::Common::{COMDLG_FILTERSPEC, ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

mod settings;
mod store;

const MAXP: usize = 16;
const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
// child ids inside a pane
const ID_BACK: i32 = 101;
const ID_FWD: i32 = 102;
const ID_UP: i32 = 103;
const ID_EDIT: i32 = 104;
const ID_OPEN_IN: i32 = 105;
const ID_FILTER: i32 = 106;
const ID_FAV: i32 = 107; // ☆+ / ★: add or remove the pane's folder from favourites
// top toolbar buttons (children of the main window)
const ID_TB_LAYOUT: i32 = 201;
const ID_TB_SAVE: i32 = 202;
const ID_TB_LOAD: i32 = 204;
const ID_TB_THEME: i32 = 205;
const ID_TB_FAV: i32 = 206;
const ID_TB_VIEW: i32 = 203;
const ID_TB_SET: i32 = 207; // gear: settings window
// menu ids
const ID_HIDDEN: usize = 1100;
const ID_KEYS: usize = 1101;
const ID_DBLUP: usize = 1103;
const ID_LANG_AUTO: usize = 1110;
const ID_LANG_EN: usize = 1111;
const ID_LANG_KO: usize = 1112;
const ID_LANG_JA: usize = 1113;
const ID_SETTINGS: usize = 1114;
const ID_NEWFILE: usize = 1115;
const ID_HELP: usize = 1105;
const ID_ABOUT: usize = 1106;
const HOME: &str = "https://github.com/efww/grids";
const ID_ACCENT: usize = 1400; // + preset index
const ID_ACCENT_PICK: usize = 1410;
const EM_SETSEL: u32 = 0xB1;
const EM_GETSEL: u32 = 0xB0;
const EM_SCROLLCARET: u32 = 0xB7;
const EN_CHANGE: u32 = 0x0300;
const TIMER_STATUS: usize = 1;
const TIMER_FILTER: usize = 2;
const TIMER_REVEAL: usize = 3;
const TIMER_SETTLE: usize = 4; // on a pane: narrow column widths going back on after a folder change
const TIMER_SAVE: usize = 5; // debounced grids.json write
const SAVE_SOON_MS: u32 = 1500;
const SAVE_LATEST_MS: u128 = 30_000;
/// Hidden tabs that still hold Explorer windows. Older hidden tabs keep only their saved grid.
const PARK_CACHE: usize = 1;
const TIMER_NEWF: usize = 6; // on a pane: select and rename a folder just created
const TIMER_NEWFILE: usize = 7; // on a pane: select and rename a file just created
// tree-view messages (the sidebar's SysTreeView32)
const TVM_GETNEXTITEM: u32 = 0x110A;
const TVM_ENSUREVISIBLE: u32 = 0x1114;
const TVM_GETITEMRECT: u32 = 0x1104;
const TVGN_CARET: usize = 9;
const TVM_SETBKCOLOR: u32 = 0x111D;
const TVM_GETTOOLTIPS: u32 = 0x1119;
// list-view messages (the Explorer list inside each pane)
const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_GETHEADER: u32 = 0x101F;
const LVM_GETTOOLTIPS: u32 = 0x104E;
const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
const LVS_EX_AUTOSIZECOLUMNS: usize = 0x1000_0000;
const LVS_EX_COLUMNSNAPPOINTS: usize = 0x4000_0000;
const HDM_GETITEMCOUNT: u32 = 0x1200;
const LVM_SETTEXTCOLOR: u32 = 0x1024;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_SETSELECTEDCOLUMN: u32 = 0x108C;
const LVM_GETSELECTEDCOLUMN: u32 = 0x10AE;
const TVM_SETTEXTCOLOR: u32 = 0x111E;
const COPY_ID: usize = 0x4752_4944; // "GRID": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

// Themes, 0xRRGGBB. Light: an ivory frame around white plates; dark: charcoal. Each theme has its own
// selection colours (same names, tuned to the ground): pastels on light, muted mid tones on dark.
struct Palette {
    bg: u32,   // frame: window, toolbar, gaps, status line
    card: u32, // plates and lists
    line: u32,
    pill: u32, // fields
    text: u32,
    sub: u32,
    accents: [(&'static str, u32); 6],
}

const LIGHT: Palette = Palette {
    bg: 0xF3F1EC,
    card: 0xFFFFFF,
    line: 0xE4E0D8,
    pill: 0xF4F2EE,
    text: 0x2A2825,
    sub: 0x7C776F,
    accents: [("하늘", 0x8DB5E8), ("그레이", 0xC5C5C5), ("라벤더", 0xA99BE0), ("세이지", 0x8CC5A8), ("피치", 0xEFAE92), ("로즈", 0xE7A1B5)],
};

const DARK: Palette = Palette {
    bg: 0x1C1D20,
    card: 0x26282C,
    line: 0x36393E,
    pill: 0x303338,
    text: 0xE6E8EB,
    sub: 0x9AA0A8,
    accents: [("하늘", 0x4C6FA8), ("그레이", 0x4B4B4B), ("라벤더", 0x6A5AA8), ("세이지", 0x3E8676), ("피치", 0xA8624A), ("로즈", 0xA65A78)],
};

static DARK_ON: AtomicBool = AtomicBool::new(false);
// Resolved UI language: 0 English, 1 Korean, 2 Japanese. The preference (including Auto) lives on the session.
static UI: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    Auto,
    En,
    Ko,
    Ja,
}

impl Lang {
    fn from_ini(v: &str) -> Self {
        match v.trim() {
            "en" => Lang::En,
            "ko" => Lang::Ko, // also the old on/off value
            "ja" => Lang::Ja,
            _ => Lang::Auto,
        }
    }

    fn ini(self) -> &'static str {
        match self {
            Lang::Auto => "auto",
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::Ja => "ja",
        }
    }

    /// Auto follows the Windows UI language: Korean, Japanese, otherwise English.
    fn resolve(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::Ko => 1,
            Lang::Ja => 2,
            Lang::Auto => {
                let prim = unsafe { GetUserDefaultUILanguage() } & 0x3FF;
                if prim == 0x12 {
                    1
                } else if prim == 0x11 {
                    2
                } else {
                    0
                }
            }
        }
    }
}

fn ui() -> u8 {
    UI.load(Ordering::Relaxed)
}

/// UI text. Explorer's own strings (column names, shell menus) stay with Windows.
fn tr(en: &'static str, ko: &'static str, ja: &'static str) -> &'static str {
    match ui() {
        1 => ko,
        2 => ja,
        _ => en,
    }
}

fn ui_face() -> &'static str {
    match ui() {
        1 => "Malgun Gothic",
        2 => "Yu Gothic UI",
        _ => "Segoe UI",
    }
}

const ACCENT_EN: [&str; 6] = ["Sky", "Gray", "Lavender", "Sage", "Peach", "Rose"];
const ACCENT_JA: [&str; 6] = ["空", "グレー", "ラベンダー", "セージ", "ピーチ", "ローズ"];

fn pal() -> &'static Palette {
    if DARK_ON.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

const INK: u32 = 0x2A2825; // dark text on light selection colours
const GREEN: u32 = 0x1FA463; // unsaved layout changes
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

// Text/icons on a coloured fill: near-black on light colours, white on dark ones (black accent -> white text).
fn on_color(bg: u32) -> u32 {
    let l = 0.2126 * (bg >> 16 & 0xFF) as f32 + 0.7152 * (bg >> 8 & 0xFF) as f32 + 0.0722 * (bg & 0xFF) as f32;
    if l > 160.0 {
        INK
    } else {
        0xFFFFFF
    }
}
// Segoe Fluent Icons / MDL2 Assets code points
const ICON_SEARCH: &str = "\u{E721}";
const ICON_FILTER: &str = "\u{E71C}";
const ICON_LOCK: &str = "\u{E72E}";
const ICON_CLOSE: &str = "\u{E711}";
const ICON_PLUS: &str = "\u{E710}";
/// Tab stripe colours, 0xRRGGBB. Chosen so a thin bar and a light tint read on both themes.
const TAB_COLORS: [(u32, &str, &str, &str); 8] = [
    (0xE15A4A, "Red", "빨강", "赤"),
    (0xE8943A, "Orange", "주황", "オレンジ"),
    (0xC49212, "Yellow", "노랑", "黄"),
    (0x3EAE6E, "Green", "초록", "緑"),
    (0x2A9D9A, "Teal", "청록", "ティール"),
    (0x3D7EE0, "Blue", "파랑", "青"),
    (0x8A5CD0, "Purple", "보라", "紫"),
    (0xD45A8C, "Pink", "분홍", "ピンク"),
];
const CLOSED_KEEP: usize = 10;
const ID_TAB_EDIT: i32 = 208;

fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_TB_LAYOUT => "\u{E8A9}",
        ID_TB_SAVE => "\u{E74E}",
        ID_TB_LOAD => "\u{E838}",
        ID_TB_FAV => "\u{E734}",
        ID_TB_SET => "\u{E713}",
        ID_TB_THEME if DARK_ON.load(Ordering::Relaxed) => "\u{E706}", // sun: back to light
        ID_TB_THEME => "\u{E708}",                                     // moon: go dark
        _ => "\u{E890}",
    }
}

const KEYS_EN: &str = "Ctrl+T\tNew tab\n\
Ctrl+W\tClose the tab (a locked tab stays)\n\
Ctrl+Tab / Ctrl+Shift+Tab\tNext / previous tab\n\
Ctrl+Shift+T\tReopen the last closed tab\n\
Ctrl+PageDown / Ctrl+PageUp\tNext / previous pane\n\
Ctrl+1 … Ctrl+9\tGo to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes)\n\
Alt+Shift+← / → / ↑ / ↓\tPane to the left / right / above / below\n\
Alt+← / Alt+→\tBack / forward\n\
Alt+↑\tParent folder\n\
Alt+↓\tRecent folders (all panes, newest first)\n\
Ctrl+L, Alt+D, F4\tAddress bar: also .. or a subfolder name, a file to open, or a command such as cmd, wt, powershell (runs in this pane's folder)\n\
Ctrl+B\tFavourites list (↑/↓ to choose, Enter to open, Del to remove)\n\
Ctrl+Shift+M\tMaximize / restore the active pane (or double-click its header)\n\
Ctrl+Shift+C\tCopy the full path of the selected items (or of the current folder)\n\
Ctrl+F\tFilter (*.pdf;*.txt or part of a name; folders always show)\n\
Enter / Esc\tApply / cancel the address or filter\n\
Ctrl+H\tShow hidden files\n\
Ctrl+Alt+N\tNew text file (then rename)\n\
Ctrl+Shift+N\tNew folder (then rename)\n\
Ctrl+,\tSettings\n\
F1\tHelp (opens the README on GitHub)\n\
F2, Del, Ctrl+C/X/V, F5\tSame as Explorer\n\n\
Layout button: click for an even grid, press and drag for rows per column (arrow keys + Enter work too)\n\
⇄ button: open the selected folder (or the current one) in another pane, with this pane's columns; or copy / move the selected files to another pane's folder (up to 10,000)\n\
Save (Ctrl+S): overwrite the current session file; green dot = unsaved layout changes (not folder moves)\n\
Load: pick a saved session by its layout preview\n\
Double-click a splitter: split evenly\n\
Arguments: grids.exe C:\\a D:\\b ... or grids.exe work.grids\n\
If Grids is already running, they open in that window.";

const KEYS_KO: &str = "Ctrl+T\t새 탭\n\
Ctrl+W\t탭 닫기 (잠긴 탭은 그대로)\n\
Ctrl+Tab / Ctrl+Shift+Tab\t다음 / 이전 탭\n\
Ctrl+Shift+T\t마지막으로 닫은 탭 다시 열기\n\
Ctrl+PageDown / Ctrl+PageUp\t다음 / 이전 패널\n\
Ctrl+1 … Ctrl+9\t1~9번 패널로 이동 (패널 머리줄 왼쪽의 번호, 아주 좁은 패널에서는 생략)\n\
Alt+Shift+← / → / ↑ / ↓\t왼쪽·오른쪽·위·아래 패널로 이동\n\
Alt+← / Alt+→\t뒤로 / 앞으로\n\
Alt+↑\t상위 폴더\n\
Alt+↓\t최근 폴더 (모든 패널, 최근 순)\n\
Ctrl+L, Alt+D, F4\t주소창: .. 이나 하위 폴더 이름, 열 파일, cmd·wt·powershell 같은 명령도 가능(이 패널 폴더에서 실행)\n\
Ctrl+B\t즐겨찾기 목록 (↑/↓ 고르기, Enter 열기, Del 빼기)\n\
Ctrl+Shift+M\t선택한 패널 크게 보기 / 복원 (머리줄 더블클릭도 가능)\n\
Ctrl+Shift+C\t선택한 항목(없으면 현재 폴더)의 전체 경로 복사\n\
Ctrl+F\t필터 (*.pdf;*.txt 또는 이름 일부, 폴더는 항상 표시)\n\
Enter / Esc\t주소창·필터 적용 / 취소\n\
Ctrl+H\t숨김 파일 표시\n\
Ctrl+Alt+N\t새 텍스트 문서 (이어서 이름 바꾸기)\n\
Ctrl+Shift+N\t새 폴더 (이어서 이름 바꾸기)\n\
Ctrl+,\t설정\n\
F1\t도움말 (GitHub README 열기)\n\
F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n\
레이아웃 버튼: 클릭 = 균등 배치, 누른 채 끌기 = 열마다 칸 수 (방향키+Enter도 가능)\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기, 열 순서·폭·정렬도 함께. 또는 선택한 파일을 다른 패널 폴더로 복사·이동 (최대 10,000개)\n\
저장 (Ctrl+S): 지금 세션 파일에 바로 덮어쓰기, 초록 점 = 저장 안 된 배치 변경(폴더 이동은 제외)\n\
불러오기: 저장한 세션을 배치 미리보기로 보고 한 번 클릭\n\
스플리터 더블클릭: 균등 분할\n\
실행 인자: grids.exe C:\\a D:\\b ... 또는 grids.exe 작업.grids\n\
이미 실행 중이면 그 창에서 엽니다.";

const KEYS_JA: &str = "Ctrl+T\t新しいタブ\n\
Ctrl+W\tタブを閉じる（ロックしたタブはそのまま）\n\
Ctrl+Tab / Ctrl+Shift+Tab\t次 / 前のタブ\n\
Ctrl+Shift+T\t最後に閉じたタブを開き直す\n\
Ctrl+PageDown / Ctrl+PageUp\t次 / 前のペイン\n\
Ctrl+1 … Ctrl+9\tペイン 1〜9 へ移動（各ペインヘッダー左の番号。非常に狭いペインでは非表示）\n\
Alt+Shift+← / → / ↑ / ↓\t左 / 右 / 上 / 下のペイン\n\
Alt+← / Alt+→\t戻る / 進む\n\
Alt+↑\t上のフォルダー\n\
Alt+↓\t最近のフォルダー（すべてのペイン、新しい順）\n\
Ctrl+L, Alt+D, F4\tアドレスバー: .. やサブフォルダー名、開くファイル、cmd・wt・powershell などのコマンド（このペインのフォルダーで実行）\n\
Ctrl+B\tお気に入り一覧（↑/↓ で選択、Enter で開く、Del で削除）\n\
Ctrl+Shift+M\tアクティブなペインを最大化 / 元に戻す（ヘッダーのダブルクリックでも可）\n\
Ctrl+Shift+C\t選択項目（なければ現在のフォルダー）のフルパスをコピー\n\
Ctrl+F\tフィルター（*.pdf;*.txt または名前の一部。フォルダーは常に表示）\n\
Enter / Esc\tアドレスまたはフィルターを適用 / キャンセル\n\
Ctrl+H\t隠しファイルを表示\n\
Ctrl+Alt+N\t新しいテキスト ファイル（続けて名前を変更）\n\
Ctrl+Shift+N\t新しいフォルダー（続けて名前を変更）\n\
Ctrl+,\t設定\n\
F1\tヘルプ（GitHub の README を開く）\n\
F2, Del, Ctrl+C/X/V, F5\tエクスプローラーと同じ\n\n\
レイアウト ボタン: クリックで均等配置、押したままドラッグで列ごとの行数（方向キー+Enter も可）\n\
⇄ ボタン: 選択したフォルダー（なければ現在のフォルダー）を別ペインで開く。列の順序・幅・並べ替えも引き継ぐ。または選択したファイルを別ペインのフォルダーへコピー / 移動（最大 10,000 個）\n\
保存 (Ctrl+S): 現在のセッション ファイルに上書き。緑の点 = 未保存のレイアウト変更（フォルダー移動は含まない）\n\
読み込み: 保存したセッションをレイアウトプレビューで見てクリック\n\
スプリッターをダブルクリック: 均等に分割\n\
引数: grids.exe C:\\a D:\\b ... または grids.exe work.grids\n\
すでに起動している場合は、そのウィンドウで開きます。";

// ---------- session (settings live in grids.json; named sessions stay .grids files in the ini shape) ----------

/// One tab: a whole pane grid (layout, folders, view, columns, filter). The active tab's grid
/// also lives on the session fields the rest of the window already reads.
#[derive(Debug, Clone, PartialEq)]
struct Tab {
    name: String,
    locked: bool,
    /// Stripe colour 0xRRGGBB. None draws the tab in the theme colours.
    color: Option<u32>,
    heights: Vec<usize>,
    colr: Vec<f64>,
    rowr: Vec<Vec<f64>>,
    active: usize,
    paths: Vec<String>,
    views: Vec<(i32, i32)>,
    cols: Vec<String>,
    filters: Vec<String>,
}

fn pad_filters(mut f: Vec<String>) -> Vec<String> {
    f.resize(MAXP, String::new());
    f.truncate(MAXP);
    f
}

impl Tab {
    fn from_live(s: &Session, name: String, locked: bool, filters: Vec<String>) -> Self {
        Tab {
            name,
            locked,
            color: None,
            heights: s.heights.clone(),
            colr: s.colr.clone(),
            rowr: s.rowr.clone(),
            active: s.active,
            paths: s.paths.clone(),
            views: s.views.clone(),
            cols: s.cols.clone(),
            filters: pad_filters(filters),
        }
    }
}

/// The next default name: 1, then 2, skipping numbers already used.
fn next_tab_name(tabs: &[Tab]) -> String {
    let mut n = 1u32;
    loop {
        let s = n.to_string();
        if !tabs.iter().any(|t| t.name == s) {
            return s;
        }
        n += 1;
        if n > 10_000 {
            return s;
        }
    }
}

/// A copied tab keeps a numbered name as the next free number, and any other name as "Name (2)".
fn duplicate_name(tabs: &[Tab], src: &str) -> String {
    if !src.is_empty() && src.chars().all(|c| c.is_ascii_digit()) {
        return next_tab_name(tabs);
    }
    let mut n = 2u32;
    loop {
        let s = format!("{src} ({n})");
        if !tabs.iter().any(|t| t.name == s) {
            return s;
        }
        n += 1;
        if n > 1000 {
            return s;
        }
    }
}

/// A locked tab, and the last remaining tab, stay open.
fn can_close_tab(tabs: &[Tab], i: usize) -> bool {
    tabs.len() > 1 && tabs.get(i).is_some_and(|t| !t.locked)
}

/// Indexes closed by "close other tabs": every unlocked tab except `keep`, low to high.
fn close_other_indexes(tabs: &[Tab], keep: usize) -> Vec<usize> {
    tabs.iter().enumerate().filter(|(i, t)| *i != keep && !t.locked).map(|(i, _)| i).collect()
}

fn reorder_vec<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= v.len() || to >= v.len() {
        return;
    }
    let item = v.remove(from);
    v.insert(to, item);
}

/// Where the active index lands after a tab moves from `from` to `to`.
fn moved_active(active: usize, from: usize, to: usize) -> usize {
    if active == from {
        return to;
    }
    if from < active && to >= active {
        active - 1
    } else if from > active && to <= active {
        active + 1
    } else {
        active
    }
}

#[derive(Debug, Clone)]
struct Session {
    win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    side: i32,             // sidebar width in px
    heights: Vec<usize>,   // panes per column, left to right (1..=4 columns of 1..=4)
    colr: Vec<f64>,        // column width ratios
    rowr: Vec<Vec<f64>>,   // row height ratios, per column
    active: usize,
    paths: Vec<String>,     // parsing names per pane slot (numbered down each column, left to right)
    views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    accent: u32,            // selection colour 0xRRGGBB, light theme
    accent_dark: u32,       // selection colour, dark theme
    dark: bool,
    dblup: bool,            // double-click on empty space goes to the parent folder
    file: String,           // named session (.grids) the layout was last loaded from / saved to
    favs: Vec<String>,      // favourite folders (parsing names), app-wide
    recent: Vec<String>,    // recently visited folders from every pane, newest first, app-wide
    lang: Lang,             // UI language preference; Auto follows Windows
    single: bool,           // a second launch hands its arguments to the running window
    cols: Vec<String>,      // per pane: columns and sort of its view (cols_text)
    theme: u8,              // 0 follow Windows, 1 light, 2 dark
    font: u8,               // 0 default, 1 large, 2 larger
    bar: bool,              // toolbar
    stat: bool,             // status line
    view_new: i32,          // FOLDERVIEWMODE for a pane that has none yet (4 details, 3 list, 1 icons)
    start_new: u8,          // 0 This PC, 1 Desktop, 2 start_path
    start_path: String,
    ext: bool,              // show file extensions (the Windows Explorer setting)
    tree_on: bool,          // left folder tree
    edge: bool,             // accent border on the active pane
    confirm: bool,          // ask before delete
    del: u8,                // 0 recycle bin, 1 permanent
    new_name: String,       // empty = the name for the display language
    dropm: u8,              // 0 Windows rules, 1 always copy, 2 always ask (the ⇄ menu)
    boot: u8,               // 0 restore last layout, 1 open boot_file, 2 default 2×2
    boot_file: String,
    autosave: bool,         // write the named session on exit
    dot: bool,              // green unsaved mark on the save button
    tabs: Vec<Tab>,         // every tab's grid; the active one's grid is also the fields above
    tab: usize,             // which tab is on screen
}

impl Default for Session {
    fn default() -> Self {
        let mut s = Session {
            win: None,
            side: 0,
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            accent: 0x0067C0, // Windows Settings blue; a chosen swatch replaces it
            accent_dark: 0x4CC2FF,
            dark: false,
            dblup: true,
            file: String::new(),
            favs: Vec::new(),
            recent: Vec::new(),
            lang: Lang::Auto,
            single: true,
            cols: vec![String::new(); MAXP],
            theme: 0,
            font: 0,
            bar: true,
            stat: true,
            view_new: 4,
            start_new: 0,
            start_path: String::new(),
            ext: true,
            tree_on: true,
            edge: true,
            confirm: true,
            del: 0,
            new_name: String::new(),
            dropm: 0,
            boot: 0,
            boot_file: String::new(),
            autosave: false,
            dot: true,
            tabs: Vec::new(),
            tab: 0,
        };
        s.anchor();
        s
    }
}

impl PartialEq for Session {
    fn eq(&self, o: &Self) -> bool {
        self.win == o.win
            && self.side == o.side
            && self.heights == o.heights
            && self.colr == o.colr
            && self.rowr == o.rowr
            && self.active == o.active
            && self.paths == o.paths
            && self.views == o.views
            && self.accent == o.accent
            && self.accent_dark == o.accent_dark
            && self.dark == o.dark
            && self.dblup == o.dblup
            && self.file == o.file
            && self.favs == o.favs
            && self.recent == o.recent
            && self.lang == o.lang
            && self.single == o.single
            && self.cols == o.cols
            && self.theme == o.theme
            && self.font == o.font
            && self.bar == o.bar
            && self.stat == o.stat
            && self.view_new == o.view_new
            && self.start_new == o.start_new
            && self.start_path == o.start_path
            && self.ext == o.ext
            && self.tree_on == o.tree_on
            && self.edge == o.edge
            && self.confirm == o.confirm
            && self.del == o.del
            && self.new_name == o.new_name
            && self.dropm == o.dropm
            && self.boot == o.boot
            && self.boot_file == o.boot_file
            && self.autosave == o.autosave
            && self.dot == o.dot
            && self.tab == o.tab
            && self.view_tabs() == o.view_tabs()
    }
}

/// Favourite entries are `path` or `path<tab>display name`. The path is what gets opened.
fn fav_path(f: &str) -> &str {
    f.split('\t').next().unwrap_or(f)
}

fn fav_label(f: &str) -> String {
    match f.split_once('\t') {
        Some((_, n)) if !n.is_empty() => n.to_string(),
        _ => short_name(fav_path(f)),
    }
}

fn even(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

fn nums<T: std::str::FromStr>(v: &str) -> Vec<T> {
    v.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

fn join<T: Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

// Ratios from a hand-edited ini can be anything: wrong length, zero, negative -> fall back to equal.
fn fix(r: Vec<f64>, n: usize) -> Vec<f64> {
    if r.len() != n || r.iter().any(|x| !(*x > 0.0)) {
        return even(n);
    }
    let sum: f64 = r.iter().sum();
    r.iter().map(|x| x / sum).collect()
}

// "2 × 2" for an even grid, "1 | 3" when the columns differ.
fn layout_label(h: &[usize]) -> String {
    match h.iter().all(|&n| n == h[0]) {
        true => format!("{} × {}", h[0], h.len()),
        false => h.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | "),
    }
}

impl Session {
    /// Tabs as saved: the active tab's grid is the live fields, its name and lock stay.
    fn view_tabs(&self) -> Vec<Tab> {
        let mut tabs = if self.tabs.is_empty() {
            vec![Tab::from_live(self, "1".into(), false, vec![String::new(); MAXP])]
        } else {
            self.tabs.clone()
        };
        let i = self.tab.min(tabs.len() - 1);
        let name = tabs[i].name.clone();
        let locked = tabs[i].locked;
        let color = tabs[i].color;
        let filters = tabs[i].filters.clone();
        tabs[i] = Tab::from_live(self, name, locked, filters);
        tabs[i].color = color;
        tabs
    }

    /// Write the live grid into the active tab. An empty list becomes one tab named 1.
    fn anchor(&mut self) {
        if self.tabs.is_empty() {
            self.tabs.push(Tab::from_live(self, "1".into(), false, vec![String::new(); MAXP]));
            self.tab = 0;
            return;
        }
        if self.tab >= self.tabs.len() {
            self.tab = 0;
        }
        let name = self.tabs[self.tab].name.clone();
        let locked = self.tabs[self.tab].locked;
        let color = self.tabs[self.tab].color;
        let filters = pad_filters(std::mem::take(&mut self.tabs[self.tab].filters));
        self.tabs[self.tab] = Tab::from_live(self, name, locked, filters);
        self.tabs[self.tab].color = color;
    }

    /// Copy the active tab onto the live grid fields.
    fn publish(&mut self) {
        if self.tabs.is_empty() {
            self.anchor();
            return;
        }
        self.tab = self.tab.min(self.tabs.len() - 1);
        let t = self.tabs[self.tab].clone();
        self.heights = t.heights;
        self.colr = t.colr;
        self.rowr = t.rowr;
        let n = self.heights.iter().sum::<usize>().max(1);
        self.active = t.active.min(n - 1);
        self.paths = t.paths;
        self.views = t.views;
        self.cols = t.cols;
    }

    fn accent_now(&self) -> u32 {
        if self.dark {
            self.accent_dark
        } else {
            self.accent
        }
    }

    fn panes(&self) -> usize {
        self.heights.iter().sum()
    }

    /// New layout with even splits.
    fn set_heights(&mut self, h: Vec<usize>) {
        self.colr = even(h.len());
        self.rowr = h.iter().map(|&n| even(n)).collect();
        self.heights = h;
        self.active = self.active.min(self.panes() - 1);
    }

    fn parse(text: &str) -> Session {
        let mut s = Session::default();
        s.tabs.clear();
        let mut rowr: Vec<Vec<f64>> = Vec::new();
        let (mut saw_theme, mut saw_dark) = (false, false);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                "side" => s.side = v.parse().unwrap_or(0),
                "cols" => {
                    let h: Vec<usize> = nums::<usize>(v).into_iter().take(4).map(|n| n.clamp(1, 4)).collect();
                    if !h.is_empty() {
                        s.heights = h;
                    }
                }
                // older ini: even rows x cols grid
                "grid" => {
                    if let [r, c] = nums::<usize>(v)[..] {
                        s.heights = vec![r.clamp(1, 4); c.clamp(1, 4)];
                    }
                }
                "colr" => s.colr = nums(v),
                "rowr" => rowr = v.split('|').map(nums).collect(),
                "active" => s.active = v.parse().unwrap_or(0),
                "accent" => s.accent = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).unwrap_or(s.accent) & 0xFFFFFF,
                "accent_dark" => s.accent_dark = u32::from_str_radix(v.trim(), 16).unwrap_or(s.accent_dark) & 0xFFFFFF,
                "dark" => {
                    s.dark = v.trim() == "1";
                    saw_dark = true;
                }
                "theme" => {
                    s.theme = match v.trim() {
                        "light" => 1,
                        "dark" => 2,
                        "auto" => 0,
                        _ => s.theme,
                    };
                    saw_theme = true;
                }
                "font" => s.font = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "bar" => s.bar = v.trim() != "0",
                "stat" => s.stat = v.trim() != "0",
                "viewnew" => s.view_new = match v.trim() {
                    "3" | "list" => 3,
                    "1" | "icon" => 1,
                    _ => 4,
                },
                "start" => s.start_new = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "startp" => s.start_path = v.trim().to_string(),
                "ext" => s.ext = v.trim() != "0",
                "tree" => s.tree_on = v.trim() != "0",
                "edge" => s.edge = v.trim() != "0",
                "confirm" => s.confirm = v.trim() != "0",
                "del" => s.del = if v.trim() == "1" { 1 } else { 0 },
                "newname" => s.new_name = v.trim().to_string(),
                "drop" => s.dropm = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "boot" => s.boot = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "bootf" => s.boot_file = v.trim().to_string(),
                "autosave" => s.autosave = v.trim() == "1",
                "dot" => s.dot = v.trim() != "0",
                "dblup" => s.dblup = v.trim() != "0",
                "lang" => s.lang = Lang::from_ini(v),
                "single" => s.single = v.trim() != "0",
                "session" => s.file = v.trim().to_string(),
                "tabs" => {
                    if let Some((tabs, active)) = tabs_from_json(v) {
                        s.tabs = tabs;
                        s.tab = active;
                    }
                }
                "fav" if !v.trim().is_empty() => s.favs.push(v.trim().to_string()),
                "recent" if !v.trim().is_empty() && s.recent.len() < RECENT => s.recent.push(v.trim().to_string()),
                _ => {
                    if let Some(i) = slot('p') {
                        s.paths[i] = v.to_string();
                    } else if let (Some(i), [m, z]) = (slot('v'), &nums::<i32>(v)[..]) {
                        s.views[i] = (*m, *z);
                    } else if let Some(i) = slot('c') {
                        s.cols[i] = v.trim().to_string();
                    }
                }
            }
        }
        s.colr = fix(std::mem::take(&mut s.colr), s.heights.len());
        // one group per column; an older single group applies to every column
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        if s.active >= s.panes() {
            s.active = 0;
        }
        // An older ini has dark= and no theme=. That stays an explicit light or dark choice.
        if !saw_theme && saw_dark {
            s.theme = if s.dark { 2 } else { 1 };
        }
        if s.tabs.is_empty() {
            s.anchor();
        } else {
            s.publish();
        }
        s
    }

    fn to_ini(&self) -> String {
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        let rowr = self.rowr.iter().map(|r| join(r)).collect::<Vec<_>>().join("|");
        o += &format!("side={}\ncols={}\n", self.side, join(&self.heights));
        o += &format!("colr={}\nrowr={rowr}\nactive={}\n", join(&self.colr), self.active);
        o += &format!("accent={:06X}\naccent_dark={:06X}\ndark={}\n", self.accent, self.accent_dark, self.dark as u8);
        let theme = ["auto", "light", "dark"][self.theme.min(2) as usize];
        o += &format!("theme={theme}\nfont={}\nbar={}\nstat={}\n", self.font, self.bar as u8, self.stat as u8);
        o += &format!("viewnew={}\nstart={}\n", self.view_new, self.start_new);
        if !self.start_path.is_empty() {
            o += &format!("startp={}\n", self.start_path);
        }
        o += &format!("ext={}\ntree={}\nedge={}\n", self.ext as u8, self.tree_on as u8, self.edge as u8);
        o += &format!("confirm={}\ndel={}\n", self.confirm as u8, self.del);
        if !self.new_name.is_empty() {
            o += &format!("newname={}\n", self.new_name);
        }
        o += &format!("drop={}\nboot={}\n", self.dropm, self.boot);
        if !self.boot_file.is_empty() {
            o += &format!("bootf={}\n", self.boot_file);
        }
        o += &format!("autosave={}\ndot={}\n", self.autosave as u8, self.dot as u8);
        o += &format!("dblup={}\nlang={}\nsingle={}\n", self.dblup as u8, self.lang.ini(), self.single as u8);
        if !self.file.is_empty() {
            o += &format!("session={}\n", self.file);
        }
        for f in &self.favs {
            o += &format!("fav={f}\n");
        }
        for r in &self.recent {
            o += &format!("recent={r}\n");
        }
        for i in 0..MAXP {
            if !self.paths[i].is_empty() {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
            if !self.cols[i].is_empty() {
                o += &format!("c{i}={}\n", self.cols[i]);
            }
        }
        let tabs = self.view_tabs();
        let active = self.tab.min(tabs.len() - 1);
        o += &format!("tabs={}\n", tabs_json(&tabs, active));
        o
    }

    fn is_fav(&self, path: &str) -> bool {
        !path.is_empty() && self.favs.iter().any(|f| fav_path(f).eq_ignore_ascii_case(path))
    }

    fn default_view(&self) -> (i32, i32) {
        match self.view_new {
            3 => (3, 16),
            1 => (1, 96),
            _ => (4, 16),
        }
    }

    /// What goes into a named session file: no window position, no pointer to itself, no favourites or recent folders.
    fn snapshot(&self) -> String {
        let mut c = self.clone();
        c.anchor();
        c.win = None;
        c.file.clear();
        c.favs.clear();
        c.recent.clear();
        c.to_ini()
    }

    /// Takes every tab from a named session; window position and sidebar width stay as they are.
    fn take_layout(&mut self, o: &Session) {
        (self.heights, self.colr, self.rowr) = (o.heights.clone(), o.colr.clone(), o.rowr.clone());
        (self.active, self.paths, self.views, self.cols) = (o.active, o.paths.clone(), o.views.clone(), o.cols.clone());
        self.tabs = o.view_tabs();
        self.tab = o.tab.min(self.tabs.len().saturating_sub(1));
    }
}

/// Named-session text for one tab: its grid only.
fn tab_session_text(s: &Session, i: usize) -> Option<String> {
    let t = s.view_tabs().get(i)?.clone();
    let mut one = s.clone();
    one.tabs = vec![t];
    one.tab = 0;
    one.publish();
    Some(one.snapshot())
}

fn parse_tab_color(v: &str) -> Option<u32> {
    let t = v.trim().trim_start_matches('#');
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(t, 16).ok().map(|n| n & 0xFFFFFF)
}

fn pane_json(t: &Tab) -> Vec<Value> {
    let mut panes = Vec::new();
    for i in 0..MAXP {
        let filter = t.filters.get(i).cloned().unwrap_or_default();
        if t.paths[i].is_empty() && t.views[i] == (0, 0) && t.cols[i].is_empty() && filter.is_empty() {
            continue;
        }
        let mut p = serde_json::Map::new();
        p.insert("index".into(), Value::from(i));
        if !t.paths[i].is_empty() {
            p.insert("path".into(), Value::String(t.paths[i].clone()));
        }
        if t.views[i].0 != 0 {
            p.insert("view".into(), Value::Array(vec![Value::from(t.views[i].0), Value::from(t.views[i].1)]));
        }
        if !t.cols[i].is_empty() {
            p.insert("columns".into(), Value::String(t.cols[i].clone()));
        }
        if !filter.is_empty() {
            p.insert("filter".into(), Value::String(filter));
        }
        panes.push(Value::Object(p));
    }
    panes
}

fn tabs_json(tabs: &[Tab], active: usize) -> String {
    let v: Vec<Value> = tabs.iter().enumerate().map(|(i, t)| {
        let mut obj = serde_json::json!({
            "name": t.name,
            "locked": t.locked,
            "active": i == active,
            "layout": { "columns": t.heights, "colr": t.colr, "rowr": t.rowr, "active": t.active },
            "panes": pane_json(t),
        });
        if let Some(c) = t.color {
            if let Some(m) = obj.as_object_mut() {
                m.insert("color".into(), Value::String(format!("{c:06X}")));
            }
        }
        obj
    }).collect();
    serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
}

/// A tab object needs a name, a layout, panes, or a lock. Anything else is left for a newer build.
fn tab_from_value(v: &Value) -> Option<Tab> {
    let obj = v.as_object()?;
    if !obj.contains_key("name") && !obj.contains_key("layout") && !obj.contains_key("panes") && !obj.contains_key("locked") {
        return None;
    }
    let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    let locked = obj.get("locked").and_then(|n| n.as_bool()).unwrap_or(false);
    let color = obj.get("color").and_then(|c| c.as_str()).and_then(parse_tab_color);
    let mut heights = vec![2, 2];
    let mut colr = Vec::new();
    let mut rowr: Vec<Vec<f64>> = Vec::new();
    let mut active = 0usize;
    if let Some(l) = obj.get("layout").and_then(|l| l.as_object()) {
        if let Some(c) = l.get("columns").and_then(|c| c.as_array()) {
            let h: Vec<usize> = c.iter().filter_map(|n| n.as_u64()).map(|n| (n as usize).clamp(1, 4)).take(4).collect();
            if !h.is_empty() {
                heights = h;
            }
        }
        if let Some(c) = l.get("colr").and_then(|c| c.as_array()) {
            colr = c.iter().filter_map(|n| n.as_f64()).collect();
        }
        if let Some(rows) = l.get("rowr").and_then(|c| c.as_array()) {
            rowr = rows.iter().map(|r| r.as_array().map(|a| a.iter().filter_map(|n| n.as_f64()).collect()).unwrap_or_default()).collect();
        }
        active = l.get("active").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    }
    colr = fix(colr, heights.len());
    let group = |c: usize| if rowr.len() == heights.len() { rowr.get(c) } else { rowr.first() };
    let rowr = heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
    let n: usize = heights.iter().sum();
    if active >= n {
        active = 0;
    }
    let mut paths = vec![String::new(); MAXP];
    let mut views = vec![(0i32, 0i32); MAXP];
    let mut cols = vec![String::new(); MAXP];
    let mut filters = vec![String::new(); MAXP];
    if let Some(panes) = obj.get("panes").and_then(|p| p.as_array()) {
        for p in panes {
            let Some(o) = p.as_object() else { continue };
            let i = o.get("index").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
            if i >= MAXP {
                continue;
            }
            if let Some(path) = o.get("path").and_then(|x| x.as_str()) {
                paths[i] = path.to_string();
            }
            if let Some(view) = o.get("view").and_then(|x| x.as_array()) {
                let m = view.first().and_then(|n| n.as_i64()).unwrap_or(0) as i32;
                let z = view.get(1).and_then(|n| n.as_i64()).unwrap_or(0) as i32;
                if m != 0 {
                    views[i] = (m, z);
                }
            }
            if let Some(c) = o.get("columns").and_then(|x| x.as_str()) {
                cols[i] = c.to_string();
            }
            if let Some(f) = o.get("filter").and_then(|x| x.as_str()) {
                filters[i] = f.to_string();
            }
        }
    }
    let name = if name.is_empty() { "1".into() } else { name };
    Some(Tab { name, locked, color, heights, colr, rowr, active, paths, views, cols, filters })
}

fn tabs_from_json(v: &str) -> Option<(Vec<Tab>, usize)> {
    let values: Vec<Value> = serde_json::from_str(v).ok()?;
    let mut tabs = Vec::new();
    let mut active = None;
    for val in &values {
        let Some(t) = tab_from_value(val) else { continue };
        if val.get("active").and_then(|a| a.as_bool()) == Some(true) && active.is_none() {
            active = Some(tabs.len());
        }
        tabs.push(t);
    }
    if tabs.is_empty() {
        return None;
    }
    let active = active.unwrap_or(0).min(tabs.len() - 1);
    Some((tabs, active))
}

/// Put `values` on the session. Invalid entries are skipped; none left means one tab from the live grid.
fn adopt_tabs(s: &mut Session, values: &[Value]) {
    let mut tabs = Vec::new();
    let mut active = None;
    for v in values {
        let Some(t) = tab_from_value(v) else { continue };
        if v.get("active").and_then(|a| a.as_bool()) == Some(true) && active.is_none() {
            active = Some(tabs.len());
        }
        tabs.push(t);
    }
    if tabs.is_empty() {
        s.tabs.clear();
        s.tab = 0;
        s.anchor();
        return;
    }
    s.tabs = tabs;
    s.tab = active.unwrap_or(0).min(s.tabs.len() - 1);
    s.publish();
}

/// Layout changes worth a save prompt: grid, split ratios, view modes, which tab is open. Folder changes don't count.
fn layout_differs(cur: &Session, base: &Session) -> bool {
    let near = |x: &[f64], y: &[f64]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| (a - b).abs() < 0.005);
    let grid = |c: &Session, b: &Session| {
        c.heights != b.heights
            || !near(&c.colr, &b.colr)
            || c.rowr.len() != b.rowr.len()
            || c.rowr.iter().zip(&b.rowr).any(|(a, b)| !near(a, b))
            || (0..c.panes()).any(|i| b.views[i].0 != 0 && c.views[i] != b.views[i])
            || (0..c.panes()).any(|i| !b.cols[i].is_empty() && c.cols[i] != b.cols[i])
    };
    if grid(cur, base) || cur.tab != base.tab || cur.tabs.len() != base.tabs.len() {
        return true;
    }
    cur.tabs.iter().zip(&base.tabs).enumerate().any(|(i, (a, b))| {
        if i == cur.tab {
            return false;
        }
        let n = a.heights.iter().sum::<usize>().min(MAXP);
        a.heights != b.heights
            || !near(&a.colr, &b.colr)
            || a.rowr.len() != b.rowr.len()
            || a.rowr.iter().zip(&b.rowr).any(|(x, y)| !near(x, y))
            || (0..n).any(|k| b.views.get(k).is_some_and(|v| v.0 != 0) && a.views.get(k) != b.views.get(k))
            || (0..n).any(|k| b.cols.get(k).is_some_and(|c| !c.is_empty()) && a.cols.get(k) != b.cols.get(k))
    })
}

// Drives, folders and shares; not shell places like This PC or the Recycle Bin, whose columns are their own.
fn is_fs(path: &str) -> bool {
    path.get(1..3) == Some(":\\") || path.starts_with(r"\\")
}

const RECENT: usize = 20;

// A pane moving from `old` to `new` puts both at the front of the recent list. Coming back to the folder it
// already holds (session restore, theme/language rebuild, refresh) is no visit; neither is a fresh pane or a shell place.
fn visit(list: &mut Vec<String>, old: &str, new: &str) {
    if old.is_empty() || old.eq_ignore_ascii_case(new) || !is_fs(new) {
        return;
    }
    for p in [old, new].into_iter().filter(|p| is_fs(p)) {
        list.retain(|x| !x.eq_ignore_ascii_case(p));
        list.insert(0, p.to_string());
    }
    list.truncate(RECENT);
}

// The recent menu of a pane: everything but the folder it shows.
fn recent_items(list: &[String], cur: &str) -> Vec<String> {
    list.iter().filter(|p| !p.eq_ignore_ascii_case(cur)).cloned().collect()
}

// A view's columns: visible ones in order with their widths, then the sort columns, each property as
// "format GUID/property id": "GUID/pid=width,...;GUID/pid=direction,...".
fn cols_text(cols: &[(PROPERTYKEY, u32)], sort: &[(PROPERTYKEY, i32)]) -> String {
    let key = |k: &PROPERTYKEY| format!("{:?}/{}", k.fmtid, k.pid);
    let cols: Vec<String> = cols.iter().map(|(k, w)| format!("{}={w}", key(k))).collect();
    let sort: Vec<String> = sort.iter().map(|(k, d)| format!("{}={d}", key(k))).collect();
    format!("{};{}", cols.join(","), sort.join(","))
}

fn parse_cols(t: &str) -> (Vec<(PROPERTYKEY, u32)>, Vec<(PROPERTYKEY, i32)>) {
    fn items<T: std::str::FromStr>(s: &str) -> Vec<(PROPERTYKEY, T)> {
        let item = |e: &str| {
            let (k, v) = e.split_once('=')?;
            let (g, pid) = k.split_once('/')?;
            Some((PROPERTYKEY { fmtid: GUID::try_from(g).ok()?, pid: pid.parse().ok()? }, v.parse().ok()?))
        };
        s.split(',').filter_map(item).collect()
    }
    let (cols, sort) = t.split_once(';').unwrap_or((t, ""));
    (items(cols), items(sort))
}

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
fn resolve(p: String) -> String {
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
}

fn is_session_file(p: &Path) -> bool {
    // .qdr: sessions saved before the rename
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("grids") || x.eq_ignore_ascii_case("qdr"))
}

// A lone session-file argument means "load this session".
fn session_arg(args: &[String]) -> Option<&str> {
    match args {
        [f] if is_session_file(Path::new(f)) => Some(f),
        _ => None,
    }
}

// `grids.exe A B C` opens the paths in pane order, growing the grid if needed.
fn apply_args(s: &mut Session, args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    let n = args.len().min(MAXP);
    for (i, p) in args.into_iter().take(MAXP).enumerate() {
        s.paths[i] = p;
    }
    if n > s.panes() {
        let c = (n as f64).sqrt().ceil() as usize;
        s.set_heights(vec![n.div_ceil(c); c]);
    }
    s.active = 0;
    s.anchor();
}

fn sessions_dir() -> PathBuf {
    store::exe_dir().join("sessions")
}

fn human(b: u64) -> String {
    let (mut v, mut k) = (b as f64, 0);
    while v >= 1024.0 && k < 4 {
        v /= 1024.0;
        k += 1;
    }
    match k {
        0 => format!("{b} B"),
        _ => format!("{v:.1} {}", ["", "KB", "MB", "GB", "TB"][k]),
    }
}

/// "pdf; *.txt" -> "*pdf*;*.txt" for PathMatchSpec: plain words match anywhere in the name.
fn filter_spec(text: &str) -> String {
    let parts = text.split(';').map(str::trim).filter(|t| !t.is_empty());
    let parts = parts.map(|t| if t.contains(['*', '?']) { t.to_string() } else { format!("*{t}*") });
    parts.collect::<Vec<_>>().join(";")
}

// ---------- grid geometry ----------

/// Splits `len` px starting at `start` into cells by `ratios`, with `gap` px splitters between them.
fn cells(start: i32, len: i32, ratios: &[f64], gap: i32) -> Vec<(i32, i32)> {
    let avail = (len - gap * (ratios.len() as i32 - 1)).max(0) as f64;
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(ratios.len());
    for (i, r) in ratios.iter().enumerate() {
        let a = start + (acc * avail).round() as i32 + i as i32 * gap;
        acc += r;
        out.push((a, start + (acc * avail).round() as i32 + i as i32 * gap));
    }
    out
}

/// Moves the boundary after cell `k` to `pos`, keeping both neighbours at least `min` px.
fn drag_ratio(r: &mut [f64], k: usize, pos: i32, start: i32, len: i32, gap: i32, min: i32) {
    let avail = (len - gap * (r.len() as i32 - 1)).max(1) as f64;
    let before: f64 = r[..k].iter().sum();
    let pair = r[k] + r[k + 1];
    let m = (min as f64 / avail).min(pair / 2.0);
    let t = ((pos - start - k as i32 * gap) as f64 / avail - before).clamp(m, pair - m);
    (r[k], r[k + 1]) = (t, pair - t);
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Side,
    Col(usize),        // boundary after column k
    Row(usize, usize), // (column, boundary after row k in it)
}

// Main window: toolbar on top, status line at the bottom, plates (sidebar + grid) in between.
// The gaps between plates are the splitters.
struct Geo {
    w: i32,
    h: i32,   // client height
    top: i32, // toolbar bottom = plates top
    bot: i32, // plates bottom = status line top
    pad: i32,
    gap: i32,
    side: i32, // sidebar plate width
    gx: i32,   // grid left
    xs: Vec<(i32, i32)>,
    ys: Vec<Vec<(i32, i32)>>, // per column
    zoom: bool, // one pane fills the grid: no splitters
}

fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap) = (s(8), s(8));
    let side = if a.s.tree_on { a.s.side.clamp(s(120), (rc.right - 2 * pad - gap - s(240)).max(s(120))) } else { 0 };
    let gx = if a.s.tree_on { pad + side + gap } else { pad };
    let top = (if a.s.bar { s(34) } else { 0 }) + s(32); // toolbar, then the tab strip
    let bot = if a.s.stat { (rc.bottom - s(24)).max(top) } else { rc.bottom };
    Geo {
        xs: cells(gx, rc.right - pad - gx, &a.s.colr, gap),
        ys: a.s.rowr.iter().map(|r| cells(top, bot - top, r, gap)).collect(),
        w: rc.right,
        h: rc.bottom,
        top,
        bot,
        pad,
        gap,
        side,
        gx,
        zoom: a.zoom.is_some(),
    }
}

fn side_rect(g: &Geo) -> RECT {
    RECT { left: g.pad, top: g.top, right: g.pad + g.side, bottom: g.bot }
}

fn status_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.bot, right: g.w, bottom: g.h }
}

/// Pane slot -> (column, row): panes are numbered down each column, left to right.
fn slot(h: &[usize], mut i: usize) -> (usize, usize) {
    for (c, &n) in h.iter().enumerate() {
        if i < n {
            return (c, i);
        }
        i -= n;
    }
    (h.len() - 1, h[h.len() - 1] - 1)
}

fn cell_rect(g: &Geo, h: &[usize], i: usize) -> RECT {
    let (c, r) = slot(h, i);
    let ((x0, x1), (y0, y1)) = (g.xs[c], g.ys[c][r]);
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; the Explorer view below.
struct PaneGeo {
    num: RECT,      // pane number (Ctrl+N); zero width when the pane is too narrow
    btn: [RECT; 4], // back, forward, up, open-in
    addr: RECT,
    filt: RECT,
    fav: RECT,
    view: RECT,
}

fn pane_geo(w: i32, h: i32, dpi: i32) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(30), s(22), s(24), s(4));
    let y = (bar - bh) / 2;
    let r = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let fav = r(w - pad - b, b);
    let fw = s(150).min((w - 2 * pad - 5 * b) / 3).max(0);
    let filt = r(fav.left - s(2) - fw, fw);
    let open = r(filt.left - s(2) - b, b);
    // pane number at the far left, only while the address edit (pill minus its s(8)/s(6) insets) keeps s(32)
    let nw = s(18);
    let nw = if open.left - s(4) - (pad + 3 * b + s(4) + nw) >= s(8) + s(6) + s(32) { nw } else { 0 };
    let x = pad + nw;
    let addr_x = x + 3 * b + s(4);
    PaneGeo {
        num: r(pad, nw),
        btn: [r(x, b), r(x + b, b), r(x + 2 * b, b), open],
        addr: r(addr_x, open.left - s(4) - addr_x),
        filt,
        fav,
        // 2px in, so the active ring stays visible around the list
        view: RECT { left: s(2), top: bar, right: w - s(2), bottom: h - s(2) },
    }
}

fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y < g.top || y >= g.bot {
        return None;
    }
    if x >= g.pad + g.side && x < g.gx {
        return Some(Hit::Side);
    }
    if x < g.gx || g.zoom {
        return None;
    }
    let between = |v: &[(i32, i32)], p: i32| v.windows(2).position(|w| p >= w[0].1 && p < w[1].0);
    if let Some(k) = between(&g.xs, x) {
        return Some(Hit::Col(k));
    }
    // row gaps belong to one column: rows no longer line up across columns
    let c = g.xs.iter().position(|&(a, b)| x >= a && x < b)?;
    between(&g.ys[c], y).map(|k| Hit::Row(c, k))
}

// ---------- app state ----------
// Single UI thread. Rule: never call into Win32/COM that can re-enter our window procs while inside app().

struct Pane {
    hwnd: HWND,
    eb: IExplorerBrowser,
    cookie: u32,
    disp: String,
    pending: Option<String>, // columns to put on when the next folder shows (session restore, open in)
    settle: Option<(String, u8)>, // columns just put on, and passes left to hold narrow widths (TIMER_SETTLE)
}

struct App {
    s: Session,
    main: HWND,
    tree: Option<INameSpaceTreeControl>,
    tree_hwnd: HWND,
    tree_input: bool, // a click/key reached the tree since its last selection change
    font: HFONT,
    icon_font: HFONT,
    fh: i32, // UI font line height in px
    dpi: i32,
    panes: Vec<Option<Pane>>,
    drag: Option<Hit>,
    sizing: bool, // inside the window's own move/size loop
    status: String,
    status_key: String,
    status_base: (String, Option<(u64, bool, bool)>, String), // text before / after the selection size, (file bytes, has folders, partial)
    view_menu: HMENU,
    base: Option<Session>, // last loaded/saved (or startup) state; differs -> green dot
    dirty: bool,
    small_font: HFONT,
    load: HWND, // load popup while open
    load_closed: Option<Instant>,
    load_hot: Option<usize>, // card index, or LOAD_SAVE_AS / LOAD_FOLDER
    fav_popup: HWND,
    fav_closed: Option<Instant>,
    fav_hot: Option<(usize, bool)>, // (row, on its ✕)
    fav_prev: HWND,                 // focus before the popup opened; Esc goes back there
    cards: Vec<(PathBuf, Session, String)>,
    hover: HWND, // owner-drawn button under the mouse
    tip: HWND,
    bars: [HWND; 2], // splitter guide bars (vertical, horizontal)
    grid: HWND,      // layout picker popup while open
    reveal_tries: u8, // sidebar reveal passes left
    grid_preview: Vec<usize>,       // layout shown in the picker
    grid_drag: Option<Vec<usize>>, // press-and-drag in progress
    grid_preset: Option<usize>,    // preset tile under the mouse
    grid_closed: Option<Instant>,
    zoom: Option<usize>, // pane maximized over the grid (runtime only, never saved)
    note: Option<(String, Instant)>, // status line confirmation and when it was set (runtime only)
    drop_hover: HWND,                         // pane whose top bar something openable is being dragged over
    drop_open: Option<(HWND, usize, IShellItem)>, // folder dropped on a top bar, opened by WM_DROP_OPEN
    park: Vec<Vec<Option<Pane>>>,             // hidden browsers, parallel to s.tabs; empty once evicted
    park_lru: Vec<usize>,                     // parked tabs that still have browsers, oldest first
    save_since: Option<Instant>,              // first unsaved change in this debounce window
    closed: Vec<Tab>,                         // recently closed tabs, newest first (not saved)
    tabs_hwnd: HWND,
    tab_edit: HWND,
    tab_hot: Option<usize>,
    tab_hot_close: bool,
    tab_hot_plus: bool,
    tab_drag: Option<usize>,
    tab_press: Option<(usize, i32, i32, bool)>, // tab, x, y, press landed on its close box
    tab_renaming: Option<usize>,
    tab_rename_skip: bool,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        tree_input: false,
        font: HFONT::default(),
        icon_font: HFONT::default(),
        fh: 16,
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        sizing: false,
        status: String::new(),
        status_key: String::new(),
        status_base: Default::default(),
        view_menu: HMENU::default(),
        base: None,
        dirty: false,
        small_font: HFONT::default(),
        load: HWND::default(),
        load_closed: None,
        load_hot: None,
        fav_popup: HWND::default(),
        fav_closed: None,
        fav_hot: None,
        fav_prev: HWND::default(),
        cards: Vec::new(),
        hover: HWND::default(),
        tip: HWND::default(),
        bars: [HWND::default(); 2],
        grid: HWND::default(),
        reveal_tries: 0,
        grid_preview: Vec::new(),
        grid_drag: None,
        grid_preset: None,
        grid_closed: None,
        zoom: None,
        note: None,
        drop_hover: HWND::default(),
        drop_open: None,
        park: Vec::new(),
        park_lru: Vec::new(),
        save_since: None,
        closed: Vec::new(),
        tabs_hwnd: HWND::default(),
        tab_edit: HWND::default(),
        tab_hot: None,
        tab_hot_close: false,
        tab_hot_plus: false,
        tab_drag: None,
        tab_press: None,
        tab_renaming: None,
        tab_rename_skip: false,
    });
}

fn no_panes() -> Vec<Option<Pane>> {
    (0..MAXP).map(|_| None).collect()
}

// Per-pane filter spec. Not in APP: the shell view may call IncludeObject from its own thread.
static FILTERS: Mutex<[String; MAXP]> = Mutex::new([const { String::new() }; MAXP]);

// Folder created by Ctrl+Shift+N, waiting for the view to list it so it can be selected and renamed.
struct PendingNew {
    pane: usize,
    name: String,
    left: u8,
}
static PENDING_NEW: Mutex<Option<PendingNew>> = Mutex::new(None);

/// "New folder", then "New folder (2)", "New folder (3)", … The first free name wins.
fn fresh_folder(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let name = format!("{base} ({n})");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

fn filter_of(i: usize) -> String {
    FILTERS.lock().map(|f| f[i].clone()).unwrap_or_default()
}

fn app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

/// Pane that commands act on. A tab strip would return the active tab's pane from here.
fn active_pane() -> usize {
    app(|a| a.s.active)
}

fn eb_of(i: usize) -> Option<IExplorerBrowser> {
    app(|a| a.panes.get(i)?.as_ref().map(|p| p.eb.clone()))
}

unsafe fn view_of<T: Interface>(i: usize) -> Option<T> {
    eb_of(i)?.GetCurrentView::<T>().ok()
}

fn pane_of(h: HWND) -> Option<usize> {
    app(|a| {
        a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h || unsafe { IsChild(p.hwnd, h).as_bool() }))
    })
}

fn pane_ctl(i: usize, id: i32) -> Option<HWND> {
    let h = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd))?;
    unsafe { GetDlgItem(Some(h), id).ok() }
}

unsafe fn text_of(h: HWND) -> String {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..n])
}

// scroll an unfocused, untouched address bar to its end, so a cut-off path shows the current folder
unsafe fn show_path_end(e: HWND, disp: &str) {
    let (mut s, mut en) = (0u32, 0u32);
    SendMessageW(e, EM_GETSEL, Some(WPARAM(&mut s as *mut u32 as usize)), Some(LPARAM(&mut en as *mut u32 as isize)));
    if GetFocus() == e || s != en || text_of(e) != disp {
        return;
    }
    let len = GetWindowTextLengthW(e) as usize;
    // back to the start first: drops any scroll left from when the box was 0 px wide
    for at in [0, len] {
        SendMessageW(e, EM_SETSEL, Some(WPARAM(at)), Some(LPARAM(at as isize)));
        SendMessageW(e, EM_SCROLLCARET, None, None);
    }
}

// ---------- fonts ----------

fn logfont(face: &str, px: i32) -> LOGFONTW {
    let mut lf = LOGFONTW { lfHeight: -px, lfCharSet: DEFAULT_CHARSET, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    lf
}

// Windows' own UI font at this DPI (no custom fonts: they only reached our chrome and cost paint time),
// plus the MDL2 icon font for the buttons. Pushed to every edit; also used on DPI change.
unsafe fn apply_font() {
    let dpi = app(|a| a.dpi);
    let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let sys = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi as u32);
    let mut lf = if sys.is_ok() { ncm.lfMessageFont } else { logfont("Segoe UI", 12 * dpi / 96) };
    lf.lfFaceName = logfont(ui_face(), 0).lfFaceName; // Segoe UI / Malgun Gothic / Yu Gothic UI
    let scale = match app(|a| a.s.font) {
        1 => 125,
        2 => 150,
        _ => 100,
    };
    lf.lfHeight = lf.lfHeight * scale / 100;
    let font = CreateFontIndirectW(&lf);
    let small = CreateFontIndirectW(&LOGFONTW { lfHeight: lf.lfHeight * 85 / 100, ..lf });
    let icon_font = CreateFontIndirectW(&logfont("Segoe MDL2 Assets", 14 * dpi / 96));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    let (old, old_icon, panes, edit) = app(|a| {
        a.fh = tm.tmHeight;
        let old_small = std::mem::replace(&mut a.small_font, small);
        if !old_small.is_invalid() {
            unsafe { _ = DeleteObject(old_small.into()) };
        }
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        let mut panes: Vec<HWND> = a.panes.iter().flatten().map(|p| p.hwnd).collect();
        for set in &a.park {
            panes.extend(set.iter().flatten().map(|p| p.hwnd));
        }
        (old, old_icon, panes, a.tab_edit)
    });
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
    for p in panes {
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p), id) {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
    for f in [old, old_icon].into_iter().filter(|f| !f.is_invalid()) {
        _ = DeleteObject(f.into());
    }
    relayout();
    redraw_all();
}

// ---------- drawing (plain GDI: square shapes need no anti-aliasing, and fills are cheap) ----------

fn cref(rgb: u32) -> COLORREF {
    COLORREF((rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF))
}

unsafe fn fill(dc: HDC, r: RECT, rgb: u32) {
    SetDCBrushColor(dc, cref(rgb));
    FillRect(dc, &r, HBRUSH(GetStockObject(DC_BRUSH).0));
}

unsafe fn frame(dc: HDC, r: RECT, w: i32, rgb: u32) {
    let sides = [RECT { bottom: r.top + w, ..r }, RECT { top: r.bottom - w, ..r }, RECT { right: r.left + w, ..r }, RECT { left: r.right - w, ..r }];
    for side in sides {
        fill(dc, side, rgb);
    }
}

unsafe fn text(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, align: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // an empty Vec's dangling pointer crashes DrawTextW even with length 0
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
    SelectObject(dc, old);
}

// Paints only the invalid area into an off-screen buffer and blits once (no flicker mid-drag).
unsafe fn buffered(h: HWND, draw: impl FnOnce(HDC, RECT)) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let mut mem = HDC::default();
    let pb = BeginBufferedPaint(hdc, &ps.rcPaint, BPBF_TOPDOWNDIB, None, &mut mem);
    if pb == 0 {
        draw(hdc, rc);
    } else {
        draw(mem, rc);
        _ = EndBufferedPaint(pb, true);
    }
    _ = EndPaint(h, &ps);
}

// Pane header colours (background, text/icons): tinted with the selection colour while active.
// The address and filter fields keep their own colours either way.
fn header_colors(pane: HWND) -> (u32, u32) {
    app(|a| {
        let active = a.panes.get(a.s.active).and_then(|p| p.as_ref()).is_some_and(|p| p.hwnd == pane);
        if active || a.drop_hover == pane {
            (a.s.accent_now(), on_color(a.s.accent_now()))
        } else {
            (pal().card, pal().text)
        }
    })
}

unsafe fn draw_button(d: &DRAWITEMSTRUCT) {
    let (hover, dpi, font, icon_font, main) = app(|a| (a.hover, a.dpi, a.font, a.icon_font, a.main));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_main = parent == main;
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = hover == d.hwndItem;
    let (bg, ink) = if on_main { (pal().bg, pal().text) } else { header_colors(parent) };
    let tinted = bg != pal().bg && bg != pal().card;
    fill(dc, rc, bg);
    if pressed || hot {
        let t = match (tinted, pressed) {
            (true, true) => 0.3,
            (true, false) => 0.18,
            (false, true) => 0.13,
            (false, false) => 0.07,
        };
        fill(dc, rc, blend(bg, ink, t));
    }
    let fg = if tinted || hot || pressed { ink } else { pal().sub };
    if id == ID_FAV {
        // ★ filled (gray, or the header ink when tinted) in a favourite folder, otherwise ☆ with a small +
        let fav = app(|a| a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == parent)).is_some_and(|i| a.s.is_fav(&a.s.paths[i])));
        if fav {
            return draw_star(dc, rc, if tinted { ink } else { pal().sub }, Star::Filled, dpi);
        }
        // ☆ exactly where ★ goes, with a + at its exact centre
        return draw_star(dc, rc, fg, Star::Plus, dpi);
    }
    if !on_main {
        return text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
    }
    let s = |v: i32| v * dpi / 96;
    if id == ID_TB_LAYOUT {
        // the one label in the chrome: the current grid
        let icon = RECT { left: rc.left + s(4), right: rc.left + s(22), ..rc };
        text(dc, icon, glyph(id), icon_font, fg, DT_CENTER);
        return text(dc, RECT { left: icon.right + s(4), ..rc }, &text_of(d.hwndItem), font, pal().text, DT_LEFT);
    }
    // icon only, names live in the tooltips; save turns green with a dot while the layout differs
    if id == ID_TB_FAV {
        return draw_star(dc, rc, fg, Star::Outline, dpi);
    }
    let dirty = id == ID_TB_SAVE && app(|a| a.dirty);
    text(dc, rc, glyph(id), icon_font, if dirty { GREEN } else { fg }, DT_CENTER);
    if dirty {
        fill(dc, RECT { left: rc.right - s(10), top: rc.top + s(4), right: rc.right - s(4), bottom: rc.top + s(10) }, GREEN);
    }
}

// A chubby five-point star (inner radius 0.56 of the outer one) with softly rounded tips, as 0..=255
// coverage per pixel of an n×n box. Font stars are too sharp, so it's computed: 4×4 samples per pixel.
#[derive(Clone, Copy, PartialEq)]
enum Star {
    Outline,
    Filled,
    Plus, // outline with a + in its exact centre (add to favourites)
}

fn star_coverage(n: i32, kind: Star) -> Vec<u8> {
    let filled = kind == Star::Filled;
    let (c, r_out) = (n as f32 / 2.0, n as f32 / 2.0 - 0.5);
    let pt = |k: i32, r: f32| {
        let a = (-90.0 + 36.0 * k as f32).to_radians();
        (c + r * a.cos(), c + r * a.sin())
    };
    let poly: Vec<(f32, f32)> = (0..10).map(|k| pt(k, if k % 2 == 0 { r_out } else { r_out * 0.56 })).collect();
    let (stroke, round) = ((n as f32 * 0.11).max(1.2), r_out * 0.17);
    // each tip is replaced by a circle tangent to both of its edges
    let tips: Vec<((f32, f32), f32)> = (0..5)
        .map(|i| {
            let (t, v1, v2) = (poly[2 * i], poly[(2 * i + 9) % 10], poly[2 * i + 1]);
            let (a, b) = ((v1.0 - t.0, v1.1 - t.1), (v2.0 - t.0, v2.1 - t.1));
            let half = ((a.0 * b.0 + a.1 * b.1) / (a.0.hypot(a.1) * b.0.hypot(b.1))).acos() / 2.0; // half the tip angle
            let d = round / half.sin();
            let (ux, uy) = ((t.0 - c) / r_out, (t.1 - c) / r_out);
            ((t.0 - ux * d, t.1 - uy * d), d)
        })
        .collect();
    let inside_dist = |x: f32, y: f32| -> Option<f32> {
        for (i, &((cx, cy), d)) in tips.iter().enumerate() {
            let (ux, uy) = ((poly[2 * i].0 - c) / r_out, (poly[2 * i].1 - c) / r_out);
            if (x - cx) * ux + (y - cy) * uy > 0.0 && (x - poly[2 * i].0).hypot(y - poly[2 * i].1) < 2.0 * d {
                let e = round - (x - cx).hypot(y - cy);
                return (e >= 0.0).then_some(e);
            }
        }
        let mut inside = false;
        let mut edge = f32::MAX;
        for k in 0..10 {
            let ((x1, y1), (x2, y2)) = (poly[k], poly[(k + 1) % 10]);
            if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
                inside = !inside;
            }
            let (dx, dy) = (x2 - x1, y2 - y1);
            let t = (((x - x1) * dx + (y - y1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            edge = edge.min((x - x1 - t * dx).hypot(y - y1 - t * dy));
        }
        inside.then_some(edge)
    };
    let mut out = vec![0u8; (n * n) as usize];
    for py in 0..n {
        for px in 0..n {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, y) = (px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0);
                    hit += inside_dist(x, y).is_some_and(|e| filled || e < stroke) as u32;
                }
            }
            out[(py * n + px) as usize] = (hit * 255 / 16) as u8;
        }
    }
    if kind == Star::Plus {
        // whole pixels, with the same parity as n so the + is centred exactly on the star's centre,
        // and arms short enough to float in the hollow without touching the outline
        let mut t = (n / 12).max(1);
        if (n - t) % 2 != 0 {
            t += 1;
        }
        let hollow = r_out * 0.516 - stroke; // centre to the inside of the outline, at its nearest
        let len = t + 2 * ((hollow - 0.4 - t as f32 / 2.0) as i32).max(1);
        let (a, b) = ((n - t) / 2, (n - len) / 2);
        for i in b..b + len {
            for j in a..a + t {
                out[(j * n + i) as usize] = 255;
                out[(i * n + j) as usize] = 255;
            }
        }
    }
    out
}

unsafe fn draw_star(dc: HDC, rc: RECT, rgb: u32, kind: Star, dpi: i32) {
    let n = 15 * dpi / 96;
    let cov = star_coverage(n, kind);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: n,
            biHeight: -n,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
    let px = std::slice::from_raw_parts_mut(bits as *mut u8, (n * n * 4) as usize);
    for (i, &a) in cov.iter().enumerate() {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8; // premultiplied BGRA
        px[i * 4..i * 4 + 4].copy_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    let mem = CreateCompatibleDC(Some(dc));
    let old = SelectObject(mem, bmp.into());
    let (x, y) = ((rc.left + rc.right - n) / 2, (rc.top + rc.bottom - n) / 2);
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    _ = AlphaBlend(dc, x, y, n, n, mem, 0, 0, n, n, blend);
    SelectObject(mem, old);
    _ = DeleteDC(mem);
    _ = DeleteObject(bmp.into());
}

// Repaints the save icon and puts the session name / unsaved state into its tooltip.
unsafe fn refresh_save_button() {
    let (main, tip, file, dirty) = app(|a| (a.main, a.tip, a.s.file.clone(), a.dirty));
    let Ok(b) = GetDlgItem(Some(main), ID_TB_SAVE) else { return };
    _ = InvalidateRect(Some(b), None, false);
    let mut hint = match (ui(), Path::new(&file).file_stem()) {
        (_, None) => tr("Save session (Ctrl+S)", "세션 저장 (Ctrl+S)", "セッションを保存 (Ctrl+S)").into(),
        (1, Some(n)) => format!("'{}'에 저장 (Ctrl+S)", n.to_string_lossy()),
        (2, Some(n)) => format!("'{}' に保存 (Ctrl+S)", n.to_string_lossy()),
        (_, Some(n)) => format!("Save to '{}' (Ctrl+S)", n.to_string_lossy()),
    };
    if dirty {
        hint += tr(" · unsaved layout changes", " · 저장 안 된 배치 변경", " · 未保存のレイアウト変更");
    }
    let hint = HSTRING::from(hint);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND,
        hwnd: main,
        uId: b.0 as usize,
        lpszText: PWSTR(hint.as_ptr() as *mut u16),
        ..Default::default()
    };
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&ti as *const _ as isize)));
}

// Polled with the status timer: view modes are only observable by asking the views.
unsafe fn check_dirty() {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
    let changed = app(|a| {
        if let Some(b) = a.base.as_mut() {
            for i in 0..a.s.panes() {
                if b.cols[i].is_empty() {
                    b.cols[i] = a.s.cols[i].clone();
                }
            }
        }
        let d = a.s.dot && a.base.as_ref().is_some_and(|b| layout_differs(&a.s, b));
        std::mem::replace(&mut a.dirty, d) != d
    });
    if changed {
        refresh_save_button();
    }
}

// Hover highlight for owner-drawn buttons (they have no hot state of their own).
unsafe fn set_hover(h: HWND) {
    let old = app(|a| std::mem::replace(&mut a.hover, h));
    if old == h {
        return;
    }
    for w in [old, h].into_iter().filter(|w| !w.0.is_null()) {
        _ = InvalidateRect(Some(w), None, false);
    }
    if !h.0.is_null() {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

unsafe fn is_icon_button(h: HWND) -> bool {
    let mut c = [0u16; 16];
    let n = GetClassNameW(h, &mut c) as usize;
    String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Button")
        && GetWindowLongPtrW(h, GWL_STYLE) as i32 & 0xF == BS_OWNERDRAW
}

unsafe fn add_tip(btn: HWND, tip: &str) {
    tip_text(btn, tip, TTM_ADDTOOLW);
}

// TTM_ADDTOOLW to add a control's tooltip, TTM_UPDATETIPTEXTW to change its text.
unsafe fn tip_text(btn: HWND, tip: &str, msg: u32) {
    let tip = HSTRING::from(tip);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: GetParent(btn).unwrap_or_default(),
        uId: btn.0 as usize,
        lpszText: PWSTR(tip.as_ptr() as *mut u16),
        ..Default::default()
    };
    SendMessageW(app(|a| a.tip), msg, None, Some(LPARAM(&ti as *const _ as isize)));
}

unsafe fn pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as _));
    s
}

unsafe fn name(pidl: *const ITEMIDLIST, kind: SIGDN) -> String {
    SHGetNameFromIDList(pidl, kind).map(|p| pwstr(p)).unwrap_or_default()
}

// ---------- navigation ----------

unsafe fn navigate(i: usize, path: &str) -> bool {
    let Some(eb) = eb_of(i) else { return false };
    let mut pidl = std::ptr::null_mut();
    if SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, 0, None).is_err() {
        return false;
    }
    let ok = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

unsafe fn browse(i: usize, flags: u32) {
    if let Some(eb) = eb_of(i) {
        _ = eb.BrowseToIDList(std::ptr::null(), flags);
    }
}

unsafe fn browse_item(i: usize, si: &IShellItem) {
    let Some(eb) = eb_of(i) else { return };
    if let Ok(pidl) = SHGetIDListFromObject(si) {
        _ = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl as _));
    }
}

unsafe fn focus_view(i: usize) {
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32);
        if let Ok(h) = sv.GetWindow() {
            _ = SetFocus(Some(h));
        }
    }
}

unsafe fn focus_ctl(i: usize, id: i32) -> bool {
    if let Some(e) = pane_ctl(i, id) {
        _ = SetFocus(Some(e));
        SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    true
}

// Address bar input as (a path, a program and its arguments); either may be missing. Unpaired quotes: nothing.
fn addr_parts(raw: &str) -> Option<(Option<&str>, Option<(&str, &str)>)> {
    let quotes = raw.matches('"').count();
    if quotes % 2 == 1 {
        return None;
    }
    let path = match quotes {
        0 => Some(raw),
        2 if raw.len() > 1 && raw.starts_with('"') && raw.ends_with('"') => Some(&raw[1..raw.len() - 1]),
        _ => None,
    };
    let cmd = if let Some(q) = raw.strip_prefix('"') {
        let (prog, rest) = q.split_once('"')?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| (prog, rest.trim()))
    } else {
        let (prog, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
        (!prog.contains(['\\', '/', ':', '"'])).then(|| (prog, rest.trim()))
    };
    Some((path, cmd.filter(|(prog, _)| !prog.is_empty())))
}

fn is_rel(p: &str) -> bool {
    !p.contains(':') && !p.starts_with(['\\', '/'])
}

unsafe fn addr_reset(i: usize, e: HWND) {
    let disp = app(|x| x.panes[i].as_ref().map(|p| p.disp.clone())).unwrap_or_default();
    _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
    focus_view(i);
    show_path_end(e, &disp);
}

// Enter in the address bar: a folder (relative to this pane's) is browsed to, a file opened, else a command
// (cmd, wt, powershell ...) runs in this pane's folder.
unsafe fn go_edit(i: usize) {
    let Some(e) = pane_ctl(i, ID_EDIT) else { return };
    let raw = text_of(e);
    let Some((path, cmd)) = addr_parts(raw.trim()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    // none inside a zip or in a shell place; metadata (not GetFileAttributes) also takes a bare \\server\share
    let cur = app(|a| a.s.paths[i].clone());
    let cur = (is_fs(&cur) && std::fs::metadata(&cur).is_ok_and(|m| m.is_dir())).then_some(cur);
    let dir = cur.as_deref().map(HSTRING::from);
    let run = |mut sei: SHELLEXECUTEINFOW| {
        sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.lpDirectory = dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
        sei.nShow = SW_SHOWNORMAL.0;
        ShellExecuteExW(&mut sei).is_ok()
    };
    // Some(true) = browsed, Some(false) = opened or ran
    let done = 'go: {
        if let Some(p) = path {
            // std::path::absolute is GetFullPathNameW: folds `..` and `.`
            let full = cur.as_ref().filter(|_| is_rel(p)).and_then(|c| std::path::absolute(Path::new(c).join(p)).ok());
            let target = full.filter(|f| std::fs::metadata(f).is_ok()).map_or(p.to_string(), |f| f.to_string_lossy().into_owned());
            let (mut pidl, mut sfgao) = (std::ptr::null_mut(), 0u32);
            if SHParseDisplayName(&HSTRING::from(target), None, &mut pidl, SFGAO_FOLDER.0, Some(&mut sfgao)).is_ok() {
                let r = if sfgao & SFGAO_FOLDER.0 != 0 {
                    eb_of(i).is_some_and(|eb| eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok()).then_some(true)
                } else {
                    run(SHELLEXECUTEINFOW { fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI, lpIDList: pidl as _, ..Default::default() }).then_some(false)
                };
                CoTaskMemFree(Some(pidl as _));
                break 'go r;
            }
        }
        let Some((prog, args)) = cmd else { break 'go None };
        // wt opens in its own default folder unless told; lpDirectory makes `.` this pane's
        let wt = prog.eq_ignore_ascii_case("wt") || prog.eq_ignore_ascii_case("wt.exe");
        let args = if wt && args.is_empty() && cur.is_some() { "-d ." } else { args };
        let (prog, args) = (HSTRING::from(prog), HSTRING::from(args));
        run(SHELLEXECUTEINFOW { fMask: SEE_MASK_FLAG_NO_UI, lpFile: PCWSTR(prog.as_ptr()), lpParameters: PCWSTR(args.as_ptr()), ..Default::default() }).then_some(false)
    };
    match done {
        Some(true) => focus_view(i),
        Some(false) => addr_reset(i, e),
        None => _ = MessageBeep(MB_ICONWARNING),
    }
}

// The selected folder (or, with none selected, the folder on display) of `src` opens in `dst`.
unsafe fn open_in(src: usize, dst: usize) {
    let Some(fv) = view_of::<IFolderView2>(src) else { return };
    let is_dir = |it: &IShellItem| it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
    let sel = fv.GetSelection(false).and_then(|a| a.GetItemAt(0)).ok().filter(is_dir);
    if let Some(item) = sel.or_else(|| fv.GetFolder::<IShellItem>().ok()) {
        // the target takes this pane's columns too (order, widths, sort): set one up, copy it around
        save_view(src);
        app(|a| {
            let cols = a.s.cols[src].clone();
            if let Some(p) = a.panes[dst].as_mut() {
                p.pending = Some(cols);
            }
        });
        browse_item(dst, &item);
        set_active(dst);
        focus_view(dst);
    }
}

unsafe fn open_in_menu(h: HWND, src: usize) {
    // (pane, shown name, folder it held when the menu opened)
    let items = app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.panes());
        let live = live.filter(|(k, _)| *k != src);
        live.filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone(), a.s.paths[k].clone()))).collect::<Vec<_>>()
    });
    let arr = view_of::<IFolderView2>(src).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Open the selected folder (or this one) in:", "선택한 폴더(없으면 현재 폴더)를 열 패널:", "選択したフォルダー（なければ現在のフォルダー）を開くペイン:")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (k, disp, _) in &items {
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("{} {}    {disp}", tr("Pane", "패널", "ペイン"), k + 1)));
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    match &picked {
        Ok(srcs) => {
            let n = srcs.len();
            let titles = [
                (false, 101, tr("Copy {n} selected items to", "선택한 {n}개 복사", "選択した {n} 個をコピー").replace("{n}", &n.to_string())),
                (true, 201, tr("Move {n} selected items to", "선택한 {n}개 이동", "選択した {n} 個を移動").replace("{n}", &n.to_string())),
            ];
            let dropm = app(|a| a.s.dropm);
            for (mv, base, title) in titles {
                if mv && dropm == 1 {
                    continue; // always copy: the move submenu stays hidden
                }
                let Ok(sub) = CreatePopupMenu() else { continue };
                for (k, disp, path) in &items {
                    let ok = is_fs(path) && send_mode(srcs, path, mv).is_some();
                    let flags = if ok { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, base + k, &HSTRING::from(format!("{} {}    {disp}", tr("Pane", "패널", "ペイン"), k + 1)));
                }
                _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(title));
            }
        }
        Err(why) => {
            let text = match why {
                Pick::TooMany => tr("Copy / move: more than 10,000 selected items — use Ctrl+C / Ctrl+V", "복사·이동: 선택이 10,000개를 넘습니다 — Ctrl+C / Ctrl+V를 쓰세요", "コピー / 移動: 選択が 10,000 個を超えています — Ctrl+C / Ctrl+V を使ってください"),
                Pick::Unreadable => tr("Copy / move: some selected items aren't regular files", "복사·이동: 일반 파일이 아닌 항목이 섞여 있습니다", "コピー / 移動: 通常のファイルでない項目が含まれています"),
                Pick::Empty => tr("Copy / move: select files first", "복사·이동: 먼저 파일을 고르세요", "コピー / 移動: 先にファイルを選択してください"),
            };
            _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(text));
        }
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(src, ID_OPEN_IN) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    match (pick as usize, picked) {
        (0, _) => {}
        (p @ 1..=100, _) => open_in(src, p - 1),
        (p, Ok(srcs)) => {
            // only to the folder the menu showed, and only the whole selection it counted
            let (mv, k) = (p > 200, (p - 1) % 100);
            let shown = items.iter().find(|(i, ..)| *i == k).map(|(.., path)| path.clone());
            let sent = shown.filter(|s| *s == app(|a| a.s.paths[k].clone())).and_then(|dst| {
                let rename = send_mode(&srcs, &dst, mv)?;
                Some(start_copy(srcs, dst, rename, mv))
            });
            if sent != Some(true) {
                _ = MessageBeep(MB_ICONWARNING);
            }
        }
        _ => {}
    }
}

// Alt+↓ or right-click on ← / →: folders recently visited in any pane, opened in this one.
unsafe fn recent_menu(i: usize) {
    let Some(h) = app(|a| a.panes[i].as_ref().map(|p| p.hwnd)) else { return };
    let items = app(|a| recent_items(&a.s.recent, &a.s.paths[i]));
    let Ok(menu) = CreatePopupMenu() else { return };
    if items.is_empty() {
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("No recent folders yet", "최근 폴더가 아직 없습니다", "最近のフォルダーはまだありません")));
    }
    for (k, p) in items.iter().enumerate() {
        // "name<tab>parent"; drive roots and share roots show whole
        let path = std::path::Path::new(p);
        let (leaf, parent) = match (path.file_name(), path.parent()) {
            (Some(n), Some(d)) => (n.to_string_lossy().into_owned(), d.display().to_string()),
            _ => (p.clone(), String::new()),
        };
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("{leaf}\t{parent}").replace('&', "&&")));
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(i, ID_BACK) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 && !navigate(i, &items[pick as usize - 1]) {
        _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
    }
    focus_view(i);
}

unsafe fn apply_filter(i: usize) {
    let Some(f) = pane_ctl(i, ID_FILTER) else { return };
    let spec = filter_spec(&text_of(f));
    let changed = FILTERS.lock().map(|mut fl| std::mem::replace(&mut fl[i], spec.clone()) != spec).unwrap_or(false);
    if let (true, Some(sv)) = (changed, view_of::<IShellView>(i)) {
        _ = sv.Refresh(); // re-enumerates, asking IncludeObject again
    }
    if let (true, Some(pane)) = (changed, app(|a| a.panes[i].as_ref().map(|p| p.hwnd))) {
        _ = InvalidateRect(Some(pane), None, false); // filter badge on/off
    }
    if changed {
        note_save();
    }
}

thread_local! {
    static KEEP_VIEWS: Cell<bool> = const { Cell::new(false) };
}

// Pane-sticky view mode and columns (order, widths, sort): remember them before leaving a folder,
// re-apply after arriving. Columns seen in shell places (This PC ...) are theirs, not the pane's.
unsafe fn save_view(i: usize) {
    if KEEP_VIEWS.with(|c| c.get()) {
        return;
    }
    if app(|a| a.panes[i].as_ref().is_none_or(|p| p.pending.is_some() || p.settle.is_some())) {
        return; // columns are about to be put on, or still settling: don't overwrite them
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    free_min_widths(view_list(&fv)); // a list made after the folder change gets it here
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    if fv.GetViewModeAndIconSize(&mut m, &mut z).is_ok() {
        app(|a| a.s.views[i] = (m.0, z));
    }
    if let Some(c) = read_cols(&fv, view_list(&fv)).filter(|_| app(|a| is_fs(&a.s.paths[i]))) {
        app(|a| a.s.cols[i] = c);
    }
}

// Column names in on-screen order, from the pane's list header: dragging a column to a new place
// changes only the header's order, which the view's own column list doesn't follow.
unsafe fn header_names(lv: HWND) -> Vec<String> {
    let n = list_titles(lv).len();
    let mut order = vec![0i32; n];
    SendMessageW(lv, LVM_GETCOLUMNORDERARRAY, Some(WPARAM(n)), Some(LPARAM(order.as_mut_ptr() as isize)));
    let titles = list_titles(lv);
    order.iter().filter_map(|&k| titles.get(k as usize).cloned()).collect()
}

// Column titles of a list, by column index.
unsafe fn list_titles(lv: HWND) -> Vec<String> {
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    let n = SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize;
    (0..n)
        .map(|k| {
            let mut text = [0u16; 128];
            let mut item = HDITEMW { mask: HDI_TEXT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
            SendMessageW(header, HDM_GETITEMW, Some(WPARAM(k)), Some(LPARAM(&mut item as *mut _ as isize)));
            String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(0)])
        })
        .collect()
}

// The list inside a view. During a folder change the old view's list is still around, so it is
// found from the view itself, not by looking through the pane.
unsafe fn view_list(fv: &IFolderView2) -> HWND {
    let view = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
    FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default()
}

// Explorer puts a floor under some columns (Size stops at 90 px) so numbers never get cut, and puts it
// back whenever a column changes (through an interface, not messages we could catch). In a pane the
// user decides: the floor is lowered to a sliver right before each resize, a drag on the header or a
// width set on the list.
unsafe fn free_min_widths(lv: HWND) {
    if lv.0.is_null() {
        return;
    }
    let mut was = 0usize;
    let first = !GetWindowSubclass(lv, Some(list_floor_proc), 3, Some(&mut was as *mut _)).as_bool();
    _ = SetWindowSubclass(lv, Some(list_floor_proc), 3, 0);
    if first {
        // the view puts its floor back on the first width change a list sees, and only then: spend that
        // change here (a pixel wider and back) so the user's first drag already goes below it
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
            let w = SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0;
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w + 1)));
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w)));
        }
    }
    // and no snapping to the floor or ideal width while dragging, no automatic sizing: the user's width
    let snap = LVS_EX_COLUMNSNAPPOINTS | LVS_EX_AUTOSIZECOLUMNS;
    if SendMessageW(lv, LVM_GETEXTENDEDLISTVIEWSTYLE, None, None).0 as usize & snap != 0 {
        SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(snap)), Some(LPARAM(0)));
    }
}

unsafe fn lower_floors(lv: HWND) {
    let floor = 20 * GetDpiForWindow(lv) as i32 / 96;
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
        let mut col = LVCOLUMNW { mask: LVCF_MINWIDTH, ..Default::default() };
        if SendMessageW(lv, LVM_GETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&mut col as *mut _ as isize))).0 != 0 && col.cxMin > floor {
            let lower = LVCOLUMNW { mask: LVCF_MINWIDTH, cxMin: floor, ..Default::default() };
            SendMessageW(lv, LVM_SETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&lower as *const _ as isize)));
        }
    }
}

unsafe extern "system" fn list_floor_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let resize = match msg {
        LVM_SETCOLUMNWIDTH => true,
        WM_NOTIFY => matches!((*(lp.0 as *const NMHDR)).code, HDN_BEGINTRACKW | HDN_TRACKW | HDN_ITEMCHANGINGW),
        _ => false,
    };
    if resize {
        lower_floors(h);
    }
    // the view stops a column coming down from above its floor at the floor (through its own channel,
    // not a message): the narrowing asked for goes on again right after, from the floor, where it isn't stopped
    if msg == WM_NOTIFY && (*(lp.0 as *const NMHDR)).code == HDN_ITEMCHANGINGW {
        let nm = &*(lp.0 as *const NMHEADERW);
        if !nm.pitem.is_null() && (*nm.pitem).mask.0 & HDI_WIDTH.0 != 0 {
            let floor = 20 * GetDpiForWindow(h) as i32 / 96; // never all the way to nothing
            let (col, want) = (nm.iItem, (*nm.pitem).cxy.max(floor));
            let now = SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col as usize)), None).0 as i32;
            let ask = if want < now { (col as i64) << 32 | want as i64 } else { -1 };
            NARROWING.store(ask, Ordering::Relaxed); // a newer change (or a widening) cancels an older one
            if ask >= 0 {
                _ = PostMessageW(Some(h), WM_NARROW_AGAIN, WPARAM(col as usize), LPARAM(want as isize));
            }
        }
    }
    if msg == WM_NARROW_AGAIN {
        let (col, want) = (wp.0, lp.0 as i32);
        if NARROWING.load(Ordering::Relaxed) == ((col as i64) << 32 | want as i64)
            && SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col)), None).0 as i32 != want // stopped at the floor, or let through to nothing
        {
            NARROWING.store(-1, Ordering::Relaxed);
            SendMessageW(h, LVM_SETCOLUMNWIDTH, Some(WPARAM(col)), Some(LPARAM(want as isize)));
        }
        return LRESULT(0);
    }
    DefSubclassProc(h, msg, wp, lp)
}

const WM_NARROW_AGAIN: u32 = WM_APP + 41;
const WM_DROP_OPEN: u32 = WM_APP + 42;
static NARROWING: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // column << 32 | width


// The view's visible columns with widths, put in on-screen order by name, then its sort columns.
// Widths come from the list: one narrowed below the view's floor is only there (the view still says 90).
unsafe fn read_cols(fv: &IFolderView2, lv: HWND) -> Option<String> {
    let cm: IColumnManager = fv.cast().ok()?;
    let (titles, shown) = (list_titles(lv), header_names(lv));
    let mut keys = vec![PROPERTYKEY::default(); cm.GetColumnCount(CM_ENUM_VISIBLE).ok()? as usize];
    cm.GetColumns(CM_ENUM_VISIBLE, &mut keys).ok()?;
    let info = |k: &PROPERTYKEY| {
        let mask = (CM_MASK_WIDTH.0 | CM_MASK_NAME.0) as u32;
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: mask, ..Default::default() };
        _ = cm.GetColumnInfo(k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        let on_list = titles.iter().position(|t| *t == name).map(|c| SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 as u32);
        (*k, on_list.filter(|&w| w > 0).unwrap_or(ci.uWidth), name)
    };
    let mut left: Vec<_> = keys.iter().map(info).collect();
    let mut cols = Vec::new();
    for name in &shown {
        if let Some(at) = left.iter().position(|c| c.2 == *name) {
            cols.push(left.remove(at));
        }
    }
    cols.extend(left); // any the header didn't name keep the view's order
    let cols: Vec<_> = cols.into_iter().map(|(k, w, _)| (k, w)).collect();
    let mut sort = vec![SORTCOLUMN::default(); fv.GetSortColumnCount().ok()?.max(0) as usize];
    fv.GetSortColumns(&mut sort).ok()?;
    Some(cols_text(&cols, &sort.iter().map(|c| (c.propkey, c.direction.0)).collect::<Vec<_>>()))
}

// The view keeps its own floor (Size: 90 px) under widths set through it, also when it builds a new
// folder's columns: narrower ones go straight onto the list, which lets them through (list_floor_proc).
unsafe fn put_widths(fv: &IFolderView2, text: &str, lv: HWND) {
    let Ok(cm) = fv.cast::<IColumnManager>() else { return };
    let titles = list_titles(lv);
    for (k, w) in parse_cols(text).0 {
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_NAME.0 as u32, ..Default::default() };
        _ = cm.GetColumnInfo(&k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        if let Some(c) = titles.iter().position(|t| *t == name) {
            if SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 != w as isize {
                SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w as isize)));
            }
        }
    }
}

unsafe fn apply_cols(fv: &IFolderView2, text: &str, lv: HWND) {
    let (cols, sort) = parse_cols(text);
    if let (false, Ok(cm)) = (cols.is_empty(), fv.cast::<IColumnManager>()) {
        if cm.SetColumns(&cols.iter().map(|c| c.0).collect::<Vec<_>>()).is_ok() {
            for (k, w) in &cols {
                let ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_WIDTH.0 as u32, uWidth: *w, ..Default::default() };
                _ = cm.SetColumnInfo(k, &ci);
            }
            put_widths(fv, text, lv);
        }
    }
    if !sort.is_empty() {
        _ = fv.SetSortColumns(&sort.iter().map(|&(propkey, d)| SORTCOLUMN { propkey, direction: SORTDIRECTION(d) }).collect::<Vec<_>>());
    }
}

// A few passes after a folder change (50, 150, 400, 1000 ms) put narrow widths back as the view builds
// its columns at its own floor; then the pane's columns are read from the view again.
unsafe fn settle_cols(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SETTLE);
    let Some(i) = pane_of(h) else { return };
    let Some((cols, left)) = app(|a| a.panes[i].as_ref().and_then(|p| p.settle.clone())) else { return };
    if let Some(fv) = view_of::<IFolderView2>(i) {
        let lv = view_list(&fv); // the new folder's list exists by now
        free_min_widths(lv);
        put_widths(&fv, &cols, lv);
    }
    let left = left - 1;
    app(|a| {
        if let Some(p) = a.panes[i].as_mut() {
            p.settle = (left > 0).then(|| (cols, left));
        }
    });
    if left > 0 {
        SetTimer(Some(h), TIMER_SETTLE, [1000, 400, 150][left as usize - 1], None);
    } else {
        note_save();
    }
}

unsafe fn on_nav(i: usize, pidl: *const ITEMIDLIST) {
    let parse = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let disp = name(pidl, SIGDN_DESKTOPABSOLUTEEDITING);
    let Some((h, eb, view, cols)) = app(|a| {
        let fs = is_fs(&parse);
        let old = std::mem::replace(&mut a.s.paths[i], parse.clone());
        visit(&mut a.s.recent, &old, &a.s.paths[i]);
        let (v, own) = (a.s.views[i], a.s.cols[i].clone());
        a.panes[i].as_mut().map(|p| {
            p.disp = disp.clone();
            // waiting columns (session, open in) first, else the pane's own; none in shell places
            let cols = p.pending.take().unwrap_or(own);
            (p.hwnd, p.eb.clone(), v, Some(cols).filter(|c| fs && !c.is_empty()))
        })
    }) else {
        return;
    };
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
        show_path_end(e, &disp);
    }
    if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
        _ = InvalidateRect(Some(b), None, false);
    }
    if let Ok(fv) = eb.GetCurrentView::<IFolderView2>() {
        if view.0 != 0 {
            _ = fv.SetViewModeAndIconSize(FOLDERVIEWMODE(view.0), view.1);
        }
        let lv = view_list(&fv);
        free_min_widths(lv);
        if let Some(cols) = cols {
            apply_cols(&fv, &cols, lv);
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.settle = Some((cols, 4));
                }
            });
            SetTimer(Some(h), TIMER_SETTLE, 50, None);
        }
        // No full-row select: a drag that starts in the size/date cells draws a selection box
        // (and selects the names it covers) instead of grabbing that row's file.
        _ = fv.SetCurrentFolderFlags(FWF_FULLROWSELECT.0 as u32, 0);
    }
    if DARK_ON.load(Ordering::Relaxed) {
        dark_list(h);
    }
    if app(|a| a.s.active == i) {
        if let Ok(item) = SHCreateItemFromIDList::<IShellItem>(pidl) {
            sync_tree(&item);
        }
    }
    hook_ctx(i);
    note_save();
}

#[implement(IExplorerBrowserEvents)]
struct Events(usize);

impl IExplorerBrowserEvents_Impl for Events_Impl {
    fn OnNavigationPending(&self, _: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe { save_view(self.0) };
        Ok(())
    }
    fn OnViewCreated(&self, _: Ref<IShellView>) -> Result<()> {
        Ok(())
    }
    fn OnNavigationComplete(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe { on_nav(self.0, pidl) };
        Ok(())
    }
    fn OnNavigationFailed(&self, _: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// Site of each Explorer control. It asks the site for ICommDlgBrowser, whose IncludeObject is the name filter.
#[implement(IServiceProvider, ICommDlgBrowser)]
struct Host(usize);

impl IServiceProvider_Impl for Host_Impl {
    fn QueryService(&self, service: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        unsafe {
            *ppv = std::ptr::null_mut();
            // SID_SExplorerBrowserFrame is defined as IID_ICommDlgBrowser.
            if *service == ICommDlgBrowser::IID {
                return self.to_interface::<ICommDlgBrowser>().query(riid, ppv).ok();
            }
        }
        Err(E_NOINTERFACE.into())
    }
}

impl ICommDlgBrowser_Impl for Host_Impl {
    fn OnDefaultCommand(&self, _: Ref<IShellView>) -> Result<()> {
        Err(S_FALSE.into()) // let the view do its default action (open)
    }
    fn OnStateChange(&self, _: Ref<IShellView>, change: u32) -> Result<()> {
        if change == CDBOSC_SELCHANGE {
            SEL_REV[self.0].fetch_add(1, Ordering::Relaxed); // same count, other items: the status line still updates
        }
        Ok(())
    }
    fn IncludeObject(&self, sv: Ref<IShellView>, pidl: *const ITEMIDLIST) -> Result<()> {
        let spec = filter_of(self.0);
        if spec.is_empty() {
            return Ok(());
        }
        unsafe {
            let sf: IShellFolder = sv.ok()?.cast::<IFolderView>()?.GetFolder()?;
            let mut attr = SFGAO_FOLDER.0;
            _ = sf.GetAttributesOf(&[pidl], &mut attr);
            if attr & SFGAO_FOLDER.0 != 0 {
                return Ok(()); // folders stay, so a filtered pane can still be navigated
            }
            let mut sr = STRRET::default();
            let mut name = PWSTR::null();
            sf.GetDisplayNameOf(pidl, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0), &mut sr)?;
            StrRetToStrW(&mut sr, Some(pidl), &mut name)?;
            let hit = PathMatchSpecW(PCWSTR(name.0), &HSTRING::from(spec)).as_bool();
            CoTaskMemFree(Some(name.0 as _));
            if hit {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        }
    }
}

// A folder dropped on a pane's top bar (its fields and buttons too: OLE walks up to the pane) opens there.
// Never MOVE: a shell source deletes what it moved. LINK or COPY leaves the source alone.
fn drop_pick(allowed: DROPEFFECT) -> DROPEFFECT {
    [DROPEFFECT_LINK, DROPEFFECT_COPY].into_iter().find(|e| allowed.0 & e.0 != 0).unwrap_or(DROPEFFECT_NONE)
}

unsafe fn drop_items(d: &Ref<IDataObject>) -> Option<IShellItemArray> {
    let arr: IShellItemArray = SHCreateShellItemArrayFromDataObject(d.as_ref()?).ok()?;
    (arr.GetCount().unwrap_or(0) > 0).then_some(arr)
}

#[implement(IDropTarget)]
struct PaneDrop {
    slot: usize,
    hwnd: HWND,
    ok: Cell<bool>, // the data being dragged holds shell items
    helper: Option<IDropTargetHelper>,
}

impl PaneDrop {
    // sets the effect this drop would get, and tints the header only while it isn't NONE
    unsafe fn pick(&self, effect: *mut DROPEFFECT) {
        *effect = if self.ok.get() { drop_pick(*effect) } else { DROPEFFECT_NONE };
        self.hover(*effect != DROPEFFECT_NONE);
    }
    unsafe fn hover(&self, on: bool) {
        let h = self.hwnd;
        let changed = app(|a| {
            let was = a.drop_hover == h;
            if on {
                a.drop_hover = h;
            } else if was {
                a.drop_hover = HWND::default();
            }
            was != on
        });
        if changed {
            repaint_header(h);
        }
    }
    // the folder (or a file's folder) is queued, not opened: a drag from this pane's own list is still running
    unsafe fn queue(&self, d: &Ref<IDataObject>) -> bool {
        let Some(item) = drop_items(d).and_then(|a| a.GetItemAt(0).ok()) else { return false };
        let is_dir = item.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
        let Some(folder) = (if is_dir { Some(item) } else { item.GetParent().ok() }) else { return false };
        _ = app(|a| a.drop_open.replace((self.hwnd, self.slot, folder)));
        if PostMessageW(Some(self.hwnd), WM_DROP_OPEN, WPARAM(0), LPARAM(0)).is_ok() {
            return true;
        }
        _ = app(|a| a.drop_open.take());
        false
    }
}

impl IDropTarget_Impl for PaneDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.ok.set(drop_items(&d).is_some());
            self.pick(effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragEnter(self.hwnd, d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.pick(effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragOver(&POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            self.hover(false);
            if let Some(hp) = &self.helper {
                _ = hp.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let allowed = *effect;
            *effect = DROPEFFECT_NONE;
            self.hover(false);
            let eff = drop_pick(allowed);
            if eff != DROPEFFECT_NONE && self.queue(&d) {
                *effect = eff;
            }
            if let Some(hp) = &self.helper {
                _ = hp.Drop(d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
}

// ---------- shell tree (sidebar) ----------

#[implement(INameSpaceTreeControlEvents)]
struct TreeEvents;

impl INameSpaceTreeControlEvents_Impl for TreeEvents_Impl {
    fn OnItemClick(&self, psi: Ref<IShellItem>, hit: u32, click: u32) -> Result<()> {
        // Re-clicking the already selected folder fires no selection change; navigate here instead.
        let on_item = hit & NSTCEHT_ONITEM.0 as u32 != 0 && click & NSTCECT_LBUTTON.0 as u32 != 0;
        if let (true, Some(si), Some(t)) = (on_item, psi.as_ref(), app(|a| a.tree.clone())) {
            if unsafe { t.GetItemState(si, NSTCIS_SELECTED.0 as u32) }.unwrap_or(0) != 0 {
                unsafe { browse_item(app(|a| a.s.active), si) };
            }
        }
        Err(S_FALSE.into()) // let the tree do its default handling
    }
    fn OnSelectionChanged(&self, sel: Ref<IShellItemArray>) -> Result<()> {
        // The tree selects its root on its own at startup; only follow selections right after user input.
        // (Focus is no good here: on a click the selection changes before the tree takes focus.)
        let user = app(|a| std::mem::take(&mut a.tree_input));
        if let Some(item) = sel.as_ref().filter(|_| user).and_then(|s| unsafe { s.GetItemAt(0) }.ok()) {
            unsafe { browse_item(app(|a| a.s.active), &item) };
        }
        Ok(())
    }
    fn OnPropertyItemCommit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemStateChanging(&self, _: Ref<IShellItem>, _: u32, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemStateChanged(&self, _: Ref<IShellItem>, _: u32, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnKeyboardInput(&self, _: u32, _: WPARAM, _: LPARAM) -> Result<()> {
        Err(S_FALSE.into())
    }
    fn OnBeforeExpand(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnAfterExpand(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeginLabelEdit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnEndLabelEdit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetToolTip(&self, _: Ref<IShellItem>, _: PWSTR, _: i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeItemDelete(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemAdded(&self, _: Ref<IShellItem>, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemDeleted(&self, _: Ref<IShellItem>, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeContextMenu(&self, _: Ref<IShellItem>, _: *const GUID, _: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnAfterContextMenu(&self, psi: Ref<IShellItem>, pcmin: Ref<IContextMenu>, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        let (Some(item), Some(inner)) = (psi.as_ref(), pcmin.as_ref()) else {
            return Err(E_NOTIMPL.into());
        };
        if !shell_folder(item) {
            return Err(E_NOTIMPL.into());
        }
        let path = shell_parse(item);
        if path.is_empty() {
            return Err(E_NOTIMPL.into());
        }
        let menu: IContextMenu = FavCtx { inner: inner.clone(), path, cmd: Cell::new(u32::MAX), item: Cell::new(usize::MAX), popup: Cell::new(0) }.into();
        unsafe {
            *ppv = std::ptr::null_mut();
            if menu.query(riid, ppv).is_ok() && !(*ppv).is_null() {
                return Ok(());
            }
            *ppv = std::ptr::null_mut();
        }
        // keep the tree's own menu if it asked for an interface we don't wrap
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeStateImageChange(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetDefaultIconIndex(&self, _: Ref<IShellItem>, _: *mut i32, _: *mut i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

unsafe fn create_tree(main: HWND) -> Result<(INameSpaceTreeControl, HWND)> {
    let t: INameSpaceTreeControl = CoCreateInstance(&NamespaceTreeControl, None, CLSCTX_INPROC_SERVER)?;
    let style = NSTCS_HASEXPANDOS.0
        | NSTCS_ROOTHASEXPANDO.0
        | NSTCS_FADEINOUTEXPANDOS.0
        | NSTCS_NOINFOTIP.0
        | NSTCS_ALLOWJUNCTIONS.0
        | NSTCS_SHOWSELECTIONALWAYS.0
        | NSTCS_FULLROWSELECT.0
        | NSTCS_HORIZONTALSCROLL.0; // a real scrollbar, so a deep folder can be scrolled into view (AUTOHSCROLL has none)
    t.Initialize(main, &RECT::default(), style as u32)?;
    let root: IShellItem = SHGetKnownFolderItem(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)?;
    t.AppendRoot(&root, SHCONTF_FOLDERS.0 as u32, NSTCRS_EXPANDED.0 as u32, None)?;
    let ev: INameSpaceTreeControlEvents = TreeEvents.into();
    t.TreeAdvise(&ev)?;
    let h = t.cast::<IOleWindow>()?.GetWindow()?;
    Ok((t, h))
}

// ---------- panes ----------

unsafe fn child(parent: HWND, class: PCWSTR, text: PCWSTR, style: u32, ex: WINDOW_EX_STYLE, id: i32, font: HFONT) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style);
    let menu = HMENU(id as isize as _);
    let h = CreateWindowExW(ex, class, text, style, 0, 0, 0, 0, Some(parent), Some(menu), None, None).unwrap_or_default();
    SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    h
}

unsafe fn create_pane(i: usize) -> Result<Pane> {
    let (main, font) = app(|a| (a.main, a.font));
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    let buttons = [
        (ID_BACK, tr("Back", "뒤로", "戻る"), tr("Back  (Alt+←) · right-click: recent folders", "뒤로  (Alt+←) · 오른쪽 클릭: 최근 폴더", "戻る  (Alt+←) · 右クリック: 最近のフォルダー")),
        (ID_FWD, tr("Forward", "앞으로", "進む"), tr("Forward  (Alt+→) · right-click: recent folders", "앞으로  (Alt+→) · 오른쪽 클릭: 최근 폴더", "進む  (Alt+→) · 右クリック: 最近のフォルダー")),
        (ID_UP, tr("Parent folder", "상위 폴더", "上のフォルダー"), tr("Parent folder  (Alt+↑)", "상위 폴더  (Alt+↑)", "上のフォルダー  (Alt+↑)")),
        (ID_OPEN_IN, tr("Open in another pane", "다른 패널에서 열기", "別のペインで開く"), tr("Open in / copy or move to another pane", "다른 패널에서 열기 / 복사·이동", "別のペインで開く / コピー・移動")),
        (ID_FAV, tr("Favourite", "즐겨찾기", "お気に入り"), tr("Add to favourites (filled star: remove)", "즐겨찾기에 추가 (채워진 별: 빼기)", "お気に入りに追加（塗りつぶしの星: 削除）")),
    ];
    for (id, name, tip) in buttons {
        add_tip(child(h, w!("BUTTON"), PCWSTR(HSTRING::from(name).as_ptr()), btn, plain, id, font), tip);
    }
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    // Folder suggestions as you type, like Explorer's address bar; folders only, since navigate() only browses folders
    _ = SHAutoComplete(edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_AUTOAPPEND_FORCE_OFF);
    child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);

    let eb: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
    let site: IServiceProvider = Host(i).into();
    IUnknown_SetSite(&eb, &site)?; // before Initialize, so the view picks up our ICommDlgBrowser
    let fs = FOLDERSETTINGS { ViewMode: FVM_DETAILS.0 as u32, fFlags: FWF_NOWEBVIEW.0 as u32 };
    eb.Initialize(h, &RECT::default(), Some(&fs))?;
    eb.SetOptions(EBO_NOBORDER)?;
    let ev: IExplorerBrowserEvents = Events(i).into();
    let cookie = eb.Advise(&ev)?;
    let helper = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok(); // drag image over the bar
    let dt: IDropTarget = PaneDrop { slot: i, hwnd: h, ok: Cell::new(false), helper }.into();
    _ = RegisterDragDrop(h, &dt);
    let pending = Some(app(|a| a.s.cols[i].clone())); // the session's, for the first folder
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new(), pending, settle: None })
}

unsafe fn destroy_pane(i: usize) {
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    save_view(i);
    let (p, queued) = app(|a| {
        let p = a.panes[i].take();
        if p.as_ref().is_some_and(|p| p.hwnd == a.drop_hover) {
            a.drop_hover = HWND::default();
        }
        let queued = if a.drop_open.as_ref().is_some_and(|d| d.1 == i) { a.drop_open.take() } else { None };
        (p, queued)
    });
    if let Some(p) = p {
        _ = RevokeDragDrop(p.hwnd);
        _ = p.eb.Unadvise(p.cookie);
        _ = IUnknown_SetSite(&p.eb, None);
        _ = p.eb.Destroy();
        _ = DestroyWindow(p.hwnd);
    }
    drop(queued);
    if let Ok(mut f) = FILTERS.lock() {
        f[i].clear();
    }
}

// Only the panes visible in the current grid exist; hidden slots keep just their path.
unsafe fn sync_panes() {
    let (main, n) = app(|a| (a.main, a.s.panes()));
    if main.0.is_null() {
        return;
    }
    for i in 0..MAXP {
        let has = app(|a| a.panes[i].is_some());
        if i < n && !has {
            let Ok(p) = create_pane(i) else { continue };
            app(|a| a.panes[i] = Some(p));
            let path = app(|a| a.s.paths[i].clone());
            if path.is_empty() || !navigate(i, &path) {
                let start = app(|a| {
                    if a.s.views[i].0 == 0 {
                        a.s.views[i] = a.s.default_view();
                    }
                    start_folder(&a.s)
                });
                navigate(i, &start);
            }
        } else if i >= n && has {
            destroy_pane(i);
        }
    }
}

// Moves a batch of windows at once (one repaint, no copied bits).
unsafe fn move_all(moves: &[(HWND, RECT)], eb: Option<(&IExplorerBrowser, RECT)>) {
    let Ok(mut dwp) = BeginDeferWindowPos(moves.len() as i32 + 1) else { return };
    for (h, r) in moves.iter().filter(|(h, _)| !h.0.is_null()) {
        match DeferWindowPos(dwp, *h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, MOVE) {
            Ok(d) => dwp = d,
            Err(_) => return,
        }
    }
    if let Some((eb, r)) = eb {
        _ = eb.SetRect(Some(&mut dwp as *mut HDWP), r);
    }
    _ = EndDeferWindowPos(dwp);
}

unsafe fn layout_pane(h: HWND) {
    let Some((eb, dpi, disp)) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| (p.eb.clone(), a.dpi, p.disp.clone()))) else {
        return;
    };
    let fh = app(|a| a.fh) + 2;
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let s = |v: i32| v * dpi / 96;
    let pg = pane_geo(rc.right, rc.bottom, dpi);
    // borderless edit, vertically centred inside its pill
    let inside = |p: RECT, l: i32, r: i32| {
        let top = p.top + (p.bottom - p.top - fh) / 2;
        RECT { left: p.left + l, top, right: (p.right - r).max(p.left + l), bottom: top + fh }
    };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    let mut moves: Vec<(HWND, RECT)> = [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].iter().zip(pg.btn).map(|(&id, r)| (ctl(id), r)).collect();
    moves.push((ctl(ID_EDIT), inside(pg.addr, s(8), s(6))));
    moves.push((ctl(ID_FILTER), inside(pg.filt, s(24), s(6))));
    moves.push((ctl(ID_FAV), pg.fav));
    move_all(&moves, Some((&eb, pg.view)));
    show_path_end(ctl(ID_EDIT), &disp);
}

unsafe fn relayout() {
    sync_panes();
    let Some((main, moves, label, shown, show_bar, show_tree, tree_h)) = app(|a| {
        if a.main.0.is_null() {
            return None;
        }
        if a.zoom.is_some_and(|z| z != a.s.active || z >= a.s.panes()) {
            a.zoom = None;
        }
        let g = geo(a);
        let s = |v: i32| v * a.dpi / 96;
        let sr = side_rect(&g);
        let tree = RECT { left: sr.left + s(2), top: sr.top + s(4), right: sr.right - s(2), bottom: sr.bottom - s(2) };
        let mut moves = vec![(a.tree_hwnd, tree)];
        let mut x = g.pad;
        let tb = |id: i32| unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
        for (id, w) in [(ID_TB_LAYOUT, s(70)), (ID_TB_SAVE, s(32)), (ID_TB_LOAD, s(32)), (ID_TB_VIEW, s(32)), (ID_TB_FAV, s(32))] {
            moves.push((tb(id), RECT { left: x, top: s(5), right: x + w, bottom: s(5) + s(24) }));
            x += w + s(4);
        }
        let right = g.w - g.pad; // theme, then the settings gear, at the far right
        moves.push((tb(ID_TB_THEME), RECT { left: right - s(32), top: s(5), right, bottom: s(5) + s(24) }));
        let gear = right - s(32) - s(4) - s(32);
        moves.push((tb(ID_TB_SET), RECT { left: gear, top: s(5), right: gear + s(32), bottom: s(5) + s(24) }));
        let show_bar = a.s.bar;
        let show_tree = a.s.tree_on;
        let tree_h = a.tree_hwnd;
        let strip_top = if show_bar { s(34) } else { 0 };
        moves.push((a.tabs_hwnd, RECT { left: 0, top: strip_top, right: g.w, bottom: strip_top + s(32) }));
        let full = RECT { left: g.gx, top: g.top, right: g.w - g.pad, bottom: g.bot };
        let mut shown = Vec::new();
        for (i, p) in a.panes.iter().enumerate().take(a.s.panes()) {
            if let Some(p) = p {
                moves.push((p.hwnd, if a.zoom == Some(i) { full } else { cell_rect(&g, &a.s.heights, i) }));
                shown.push((p.hwnd, a.zoom.is_none_or(|z| z == i)));
            }
        }
        Some((a.main, moves, layout_label(&a.s.heights), shown, show_bar, show_tree, tree_h))
    }) else {
        return;
    };
    move_all(&moves, None);
    for id in [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_VIEW, ID_TB_FAV, ID_TB_THEME, ID_TB_SET] {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            _ = ShowWindow(b, if show_bar { SW_SHOW } else { SW_HIDE });
        }
    }
    if !tree_h.0.is_null() {
        _ = ShowWindow(tree_h, if show_tree { SW_SHOW } else { SW_HIDE });
    }
    let strip = app(|a| a.tabs_hwnd);
    if !strip.0.is_null() {
        _ = ShowWindow(strip, SW_SHOW);
    }
    for (h, on) in shown {
        _ = ShowWindow(h, if on { SW_SHOW } else { SW_HIDE });
    }
    start_reveal(main); // sidebar size may have changed: keep its selection in view
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        if text_of(b) != label {
            _ = SetWindowTextW(b, &HSTRING::from(label));
        }
    }
    _ = InvalidateRect(Some(main), None, true);
    // Paint now: WM_PAINT is only generated once the message queue is empty, which never happens
    // while the mouse keeps moving, so mid-drag the views would keep showing the previous frame.
    _ = RedrawWindow(Some(main), None, None, RDW_UPDATENOW | RDW_ALLCHILDREN);
}

// Splitter drag shows translucent guide bars where the boundaries will land; the panes move once, on release.
unsafe fn show_bars() {
    let Some((main, bars, rects)) = app(|a| {
        let d = a.drag?;
        let g = geo(a);
        let t = 3 * a.dpi / 96;
        let vbar = |x: i32| RECT { left: x - t / 2, top: g.top, right: x - t / 2 + t, bottom: g.bot };
        let hbar = |(x0, x1): (i32, i32), y: i32| RECT { left: x0, top: y - t / 2, right: x1, bottom: y - t / 2 + t };
        let rects = match d {
            Hit::Side => [Some(vbar(g.pad + g.side + g.gap / 2)), None],
            Hit::Col(k) => [Some(vbar(g.xs[k].1 + g.gap / 2)), None],
            Hit::Row(c, k) => [None, Some(hbar(g.xs[c], g.ys[c][k].1 + g.gap / 2))],
        };
        Some((a.main, a.bars, rects))
    }) else {
        return;
    };
    for (b, r) in bars.into_iter().zip(rects) {
        match r {
            Some(r) => {
                let mut p = POINT { x: r.left, y: r.top };
                _ = ClientToScreen(main, &mut p);
                _ = SetWindowPos(b, None, p.x, p.y, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            None => _ = ShowWindow(b, SW_HIDE),
        }
    }
}

extern "system" fn bar_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

unsafe fn end_drag() {
    let Some(bars) = app(|a| a.drag.take().map(|_| a.bars)) else { return };
    for b in bars {
        _ = ShowWindow(b, SW_HIDE);
    }
    relayout();
    redraw_all();
}

// Full repaint of everything once a drag/resize/layout change is over, so nothing stale survives.
unsafe fn redraw_all() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = RedrawWindow(Some(main), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

unsafe fn set_active(i: usize) {
    let (old, hs) = app(|a| {
        let old = std::mem::replace(&mut a.s.active, i);
        (old, [old, i].map(|k| a.panes[k].as_ref().map(|p| p.hwnd)))
    });
    if old != i {
        if app(|a| a.zoom.take()).is_some() {
            relayout();
            redraw_all();
        }
        for h in hs.into_iter().flatten() {
            repaint_header(h);
        }
        if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
            sync_tree(&item);
        }
    }
}

unsafe fn repaint_header(pane: HWND) {
    _ = InvalidateRect(Some(pane), None, false);
    for id in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN, ID_FAV, ID_EDIT, ID_FILTER] {
        if let Ok(c) = GetDlgItem(Some(pane), id) {
            _ = InvalidateRect(Some(c), None, true);
        }
    }
}

// The sidebar follows the active pane: expand down to its folder and select it.
// (Programmatic selection doesn't navigate: OnSelectionChanged only follows user input.)
unsafe fn sync_tree(item: &IShellItem) {
    let Some((t, main, th)) = app(|a| a.tree.clone().map(|t| (t, a.main, a.tree_hwnd))) else { return };
    // the tree scrolls on its own when it selects; hide that and paint once, after the reveal
    let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).ok();
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    _ = t.EnsureItemVisible(item);
    _ = t.SetItemState(item, NSTCIS_SELECTED.0 as u32, NSTCIS_SELECTED.0 as u32);
    reveal_tree_selection(); // turns redraw back on
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
        _ = InvalidateRect(Some(tv), None, true);
    }
    start_reveal(main); // and again while the tree finishes expanding
}

// Scrolls the tree so the selected folder is on screen: vertically, and sideways when deep nesting
// pushes its name past a narrow sidebar.
unsafe fn reveal_tree_selection() {
    let (th, dpi) = app(|a| (a.tree_hwnd, a.dpi));
    let Ok(tv) = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()) else { return };
    let item = SendMessageW(tv, TVM_GETNEXTITEM, Some(WPARAM(TVGN_CARET)), None).0;
    if item == 0 {
        return;
    }
    let label = || {
        let mut r = RECT::default();
        *(&mut r as *mut RECT as *mut isize) = item; // TVM_GETITEMRECT takes the item in the RECT
        let ok = SendMessageW(tv, TVM_GETITEMRECT, Some(WPARAM(1)), Some(LPARAM(&mut r as *mut _ as isize))).0 != 0;
        ok.then_some(r)
    };
    let mut c = RECT::default();
    _ = GetClientRect(tv, &mut c);
    if c.right == 0 {
        return; // not laid out yet; relayout asks again
    }
    let margin = 16 * dpi / 96;
    // all the scrolling happens unseen, then one repaint if anything moved
    let pos = || (GetScrollPos(tv, SB_HORZ), GetScrollPos(tv, SB_VERT));
    let before = pos();
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    SendMessageW(tv, TVM_ENSUREVISIBLE, None, Some(LPARAM(item)));
    // sideways, a line at a time (avoids guessing the tree's scroll units): the selected folder's name
    // wins over the roots on the left, which may get cut off. One direction only, so a name wider than
    // the sidebar can't bounce between the two rules.
    let left = label().is_some_and(|r| r.left < margin);
    for _ in 0..300 {
        match label() {
            Some(r) if left && r.left < margin && GetScrollPos(tv, SB_HORZ) > 0 => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None), // SB_LINELEFT
            Some(r) if !left && r.right > c.right && r.left > margin => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(1)), None), // SB_LINERIGHT
            _ => break,
        }
    }
    if !left && label().is_some_and(|r| r.left < margin) {
        SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None); // overshot: back one, so the next pass is a no-op
    }
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
    if pos() != before {
        _ = InvalidateRect(Some(tv), None, true);
    }
}

// The tree expands down to a new folder asynchronously, so the reveal runs a few times as it settles.
unsafe fn start_reveal(main: HWND) {
    app(|a| a.reveal_tries = 3);
    SetTimer(Some(main), TIMER_REVEAL, 120, None);
}

unsafe fn set_accent(c: u32) {
    let (bars, panes) = app(|a| {
        if a.s.dark {
            a.s.accent_dark = c;
        } else {
            a.s.accent = c;
        }
        (a.bars, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>())
    });
    let old = SetClassLongPtrW(bars[0], GCLP_HBRBACKGROUND, CreateSolidBrush(cref(c)).0 as isize);
    if old != 0 {
        _ = DeleteObject(HGDIOBJ(old as _));
    }
    for p in panes {
        repaint_header(p); // all of them: inactive panes show the accent in their filter badge
    }
    settings::redraw();
}

// The hosted Explorer list ignores the app-wide dark mode, so its (new, per folder) list view is dyed here.
unsafe fn dark_list(pane: HWND) {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        let class = String::from_utf16_lossy(&c[..n]);
        if class == "SysListView32" || class == "SHELLDLL_DefView" {
            unsafe { (*(out.0 as *mut Vec<(String, HWND)>)).push((class, h)) };
        }
        true.into()
    }
    let mut found: Vec<(String, HWND)> = Vec::new();
    _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut found as *mut _ as isize));
    // the view paints the name column itself (custom draw, light colours); recolour it on the way back
    for (_, v) in found.iter().filter(|(c, _)| c == "SHELLDLL_DefView") {
        _ = SetWindowSubclass(*v, Some(dark_view_proc), 1, 0);
    }
    let lists: Vec<HWND> = found.into_iter().filter(|(c, _)| c == "SysListView32").map(|(_, h)| h).collect();
    let (card, text) = (cref(pal().card).0 as isize, cref(pal().text).0 as isize);
    for lv in lists {
        _ = SetWindowTheme(lv, w!("DarkMode_Explorer"), PCWSTR::null()); // also darkens its scrollbars
        // the sorted-column tint stays white in dark mode
        _ = SetWindowSubclass(lv, Some(dark_list_proc), 2, 0);
        SendMessageW(lv, LVM_SETSELECTEDCOLUMN, Some(WPARAM(usize::MAX)), None);
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            _ = SetWindowTheme(header, w!("DarkMode_ItemsView"), PCWSTR::null());
        }
        // the info tip over a file, and the full name over a truncated one
        let tip = HWND(SendMessageW(lv, LVM_GETTOOLTIPS, None, None).0 as _);
        if !tip.0.is_null() {
            _ = SetWindowTheme(tip, w!("DarkMode_Explorer"), PCWSTR::null());
        }
        _ = InvalidateRect(Some(lv), None, true);
    }
}

// The view re-sets the sorted column through an internal interface (no window message to catch),
// so it is cleared right before each paint instead. The column header is the list's child, so its
// draw notifications come through here too.
unsafe extern "system" fn dark_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let dark = DARK_ON.load(Ordering::Relaxed);
    if msg == WM_PAINT && dark && DefSubclassProc(h, LVM_GETSELECTEDCOLUMN, WPARAM(0), LPARAM(0)).0 as i32 != -1 {
        DefSubclassProc(h, LVM_SETSELECTEDCOLUMN, WPARAM(usize::MAX), LPARAM(0));
    }
    if msg == WM_NOTIFY && dark && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &*(lp.0 as *const NMCUSTOMDRAW);
        if cd.hdr.hwndFrom.0 as isize == DefSubclassProc(h, LVM_GETHEADER, WPARAM(0), LPARAM(0)).0 {
            if cd.dwDrawStage == CDDS_PREPAINT {
                return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
            }
            if cd.dwDrawStage == CDDS_ITEMPREPAINT {
                draw_header_item(cd);
                return LRESULT(CDRF_SKIPDEFAULT as isize);
            }
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// The column header's theme draws its labels in the light theme's dark grey whatever colour it is
// given, so in dark mode each column is drawn here: flat background, label, divider, sort chevron.
unsafe fn draw_header_item(cd: &NMCUSTOMDRAW) {
    let (dc, rc, hdr) = (cd.hdc, cd.rc, cd.hdr.hwndFrom);
    let s = |v: i32| v * GetDpiForWindow(hdr) as i32 / 96;
    let hot = cd.uItemState.0 & (CDIS_HOT.0 | CDIS_SELECTED.0) != 0;
    fill(dc, rc, if hot { blend(pal().card, pal().text, 0.08) } else { pal().card });
    fill(dc, RECT { left: rc.right - 1, ..rc }, pal().line);
    let mut text = [0u16; 260];
    let mut item = HDITEMW { mask: HDI_TEXT | HDI_FORMAT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
    SendMessageW(hdr, HDM_GETITEMW, Some(WPARAM(cd.dwItemSpec)), Some(LPARAM(&mut item as *mut _ as isize)));
    let n = text.iter().position(|&c| c == 0).unwrap_or(0);
    if n > 0 {
        let align = if item.fmt.0 & HDF_RIGHT.0 != 0 {
            DT_RIGHT
        } else if item.fmt.0 & HDF_CENTER.0 != 0 {
            DT_CENTER
        } else {
            DT_LEFT
        };
        let mut r = RECT { left: rc.left + s(7), right: rc.right - s(7), ..rc };
        let font = SelectObject(dc, HGDIOBJ(SendMessageW(hdr, WM_GETFONT, None, None).0 as _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, cref(pal().sub));
        DrawTextW(dc, &mut text[..n], &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
    }
    // sort chevron along the top edge, centred, as Windows 11 draws it
    let up = item.fmt.0 & HDF_SORTUP.0 != 0;
    if up || item.fmt.0 & HDF_SORTDOWN.0 != 0 {
        let (cx, t, arm) = ((rc.left + rc.right) / 2, s(1).max(1), s(4));
        for k in 0..=arm {
            let y = rc.top + s(1) + if up { k } else { arm - k };
            fill(dc, RECT { left: cx - k, top: y, right: cx - k + t, bottom: y + t }, pal().sub);
            fill(dc, RECT { left: cx + k, top: y, right: cx + k + t, bottom: y + t }, pal().sub);
        }
    }
}

unsafe extern "system" fn dark_view_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_NOTIFY && DARK_ON.load(Ordering::Relaxed) && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &mut *(lp.0 as *mut NMLVCUSTOMDRAW);
        let stage = cd.nmcd.dwDrawStage.0;
        if stage == CDDS_PREPAINT.0 {
            return LRESULT(r.0 | CDRF_NOTIFYITEMDRAW as isize);
        }
        if stage & CDDS_ITEMPREPAINT.0 == CDDS_ITEMPREPAINT.0 {
            (cd.clrText, cd.clrTextBk) = (cref(pal().text), cref(pal().card));
            if stage & CDDS_SUBITEM.0 == 0 {
                return LRESULT(r.0 | CDRF_NOTIFYSUBITEMDRAW as isize);
            }
        }
    }
    r
}

// Undocumented but long-stable uxtheme exports (Windows 10 1903+): app-wide dark mode, which is what
// makes menus, scrollbars and the hosted Explorer views follow the theme.
unsafe fn uxtheme(ordinal: usize) -> FARPROC {
    let m = LoadLibraryW(w!("uxtheme.dll")).ok()?;
    GetProcAddress(m, PCSTR(ordinal as *const u8))
}

/// AppsUseLightTheme = 0 means Windows is in dark app mode.
unsafe fn windows_dark() -> bool {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD};
    let mut key = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), None, KEY_READ, &mut key).is_err() {
        return false;
    }
    let mut data = [0u8; 4];
    let mut n = 4u32;
    let mut typ = REG_DWORD;
    let ok = RegQueryValueExW(key, w!("AppsUseLightTheme"), None, Some(&mut typ), Some(data.as_mut_ptr()), Some(&mut n));
    _ = RegCloseKey(key);
    ok.is_ok() && u32::from_le_bytes(data) == 0
}

fn resolve_dark(theme: u8) -> bool {
    match theme {
        2 => true,
        1 => false,
        _ => unsafe { windows_dark() },
    }
}

unsafe fn start_folder(s: &Session) -> String {
    match s.start_new {
        1 => {
            let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) else { return THIS_PC.into() };
            let path = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            if path.is_empty() { THIS_PC.into() } else { path }
        }
        2 if !s.start_path.is_empty() => s.start_path.clone(),
        _ => THIS_PC.into(),
    }
}

/// Show or hide extensions for the hosted Explorer views. This is Windows' own HideFileExt value.
unsafe fn apply_ext(show: bool) {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD};
    let mut key = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"), None, KEY_SET_VALUE, &mut key).is_err() {
        return;
    }
    let data = [(!show) as u8, 0, 0, 0];
    _ = RegSetValueExW(key, w!("HideFileExt"), None, REG_DWORD, Some(&data));
    _ = RegCloseKey(key);
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    for i in 0..MAXP {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
}

// Applies the current theme everywhere; panes are rebuilt so their Explorer views pick it up.
unsafe fn apply_theme() {
    app(|a| a.s.dark = resolve_dark(a.s.theme));
    let (main, dark, tree, accent, tip) = app(|a| (a.main, a.s.dark, a.tree_hwnd, a.s.accent_now(), a.tip));
    // tooltips: dark on dark, back to the default on light (these windows outlive the theme switch)
    let tip_theme = |t: HWND| {
        if !t.0.is_null() {
            _ = SetWindowTheme(t, if dark { w!("DarkMode_Explorer") } else { PCWSTR::null() }, PCWSTR::null());
        }
    };
    tip_theme(tip);
    DARK_ON.store(dark, Ordering::Relaxed);
    if let Some(f) = uxtheme(135) {
        let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_preferred_app_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
    }
    if let Some(f) = uxtheme(136) {
        let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
        flush_menu_themes();
    }
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(main, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4); // immersive dark title bar
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    if let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) {
        _ = SetWindowTheme(tv, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, PCWSTR::null());
        let colour = |c: u32| LPARAM(if dark { cref(c).0 as isize } else { -1 }); // -1: system default
        SendMessageW(tv, TVM_SETBKCOLOR, None, Some(colour(pal().card)));
        SendMessageW(tv, TVM_SETTEXTCOLOR, None, Some(colour(pal().text)));
        tip_theme(HWND(SendMessageW(tv, TVM_GETTOOLTIPS, None, None).0 as _));
    }
    set_accent(accent);
    // Every tab's Explorer windows follow the theme. Their folders, filters and columns are already on the tab.
    remember_tabs();
    let filters = app(|a| a.s.tabs.get(a.s.tab).map(|t| t.filters.clone()).unwrap_or_default());
    destroy_every_pane();
    put_filters(&filters);
    app(|a| {
        let n = a.s.tabs.len().max(1);
        a.park = (0..n).map(|_| no_panes()).collect();
        a.park_lru.clear();
    });
    relayout();
    sync_filter_edits();
    redraw_all();
    settings::refresh();
}

unsafe fn pick_accent() {
    let (main, cur) = app(|a| (a.main, a.s.accent_now()));
    let mut custom = [COLORREF(0xFFFFFF); 16];
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: main,
        rgbResult: cref(cur),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    if ChooseColorW(&mut cc).as_bool() {
        set_accent(cref(cc.rgbResult.0).0); // COLORREF is BGR; cref swaps back
    }
}

// Bumped on every navigation event and pane teardown, so a click pair spanning a folder change is dropped.
static NAV_GEN: AtomicUsize = AtomicUsize::new(0);

fn gen() -> usize {
    NAV_GEN.load(Ordering::SeqCst)
}

#[derive(Clone, Copy, PartialEq)]
struct Click {
    win: isize,
    gen: usize,
    time: u32,
    x: i32,
    y: i32,
}

thread_local! {
    static LAST_CLICK: Cell<Option<Click>> = const { Cell::new(None) };
}

// Windows 11's list (DirectUIHWND) has no CS_DBLCLKS, so a double-click is two background presses in a row,
// within the system's double-click time and box. The pair is consumed: a third click doesn't go up again.
fn next_click(prev: Option<Click>, now: Click, bg: bool, ms: u32, cx: i32, cy: i32) -> (Option<Click>, bool) {
    if !bg {
        return (None, false);
    }
    match prev {
        Some(p)
            if p.win == now.win
                && p.gen == now.gen
                && now.time.wrapping_sub(p.time) <= ms
                && (now.x - p.x).abs() <= cx / 2
                && (now.y - p.y).abs() <= cy / 2 =>
        {
            (None, true)
        }
        _ => (Some(now), false),
    }
}

// Only the list itself is background; rows, group headers, collapse buttons, column headers are not.
fn bg_role(role: u32) -> bool {
    role == ROLE_SYSTEM_LIST
}

// The generation is read before the probe and again after it: a navigation during the probe voids it.
fn bg_probe(gen: impl Fn() -> usize, probe: impl FnOnce() -> bool) -> (usize, bool) {
    let g0 = gen();
    let b = probe();
    (g0, b && gen() == g0)
}

// After dispatch: same view window, nothing selected, and no navigation since the click. The generation is
// read last, after the queries that could run navigation callbacks.
fn confirm(cand: (usize, isize), query: impl FnOnce() -> (Option<isize>, Option<u32>), gen: impl Fn() -> usize) -> bool {
    let (view, sel) = query();
    view == Some(cand.1) && sel == Some(0) && gen() == cand.0
}

// Same-thread window, so WM_GETOBJECT runs inline; call outside app().
unsafe fn on_background(m: &MSG) -> bool {
    if WindowFromPoint(m.pt) != m.hwnd {
        return false;
    }
    let mut acc: Option<IAccessible> = None;
    let mut child = VARIANT::default();
    if AccessibleObjectFromPoint(m.pt, &mut acc, &mut child).is_err() {
        return false;
    }
    let bg = acc.and_then(|a| a.get_accRole(&child).ok()).is_some_and(|mut role| {
        let r = &role.Anonymous.Anonymous;
        let bg = r.vt == VT_I4 && bg_role(r.Anonymous.lVal as u32);
        _ = VariantClear(&mut role);
        bg
    });
    _ = VariantClear(&mut child);
    bg
}

// A double-click on empty space in a list goes up a folder: (pane, generation, view window) when this press
// completes a background pair.
unsafe fn up_candidate(m: &MSG) -> Option<(usize, usize, isize)> {
    if !matches!(m.message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) || !app(|a| a.s.dblup) {
        return None;
    }
    let i = pane_of(m.hwnd)?;
    let svh = Cell::new(HWND::default());
    let (g0, bg) = bg_probe(gen, || {
        let Some(h) = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()) else {
            return false;
        };
        svh.set(h);
        let mut c = [0u16; 32];
        let n = GetClassNameW(m.hwnd, &mut c) as usize;
        let list = matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32");
        list && (h == m.hwnd || IsChild(h, m.hwnd).as_bool())
            && m.wParam.0 & (MK_CONTROL.0 | MK_SHIFT.0) as usize == 0
            && on_background(m)
    });
    let now = Click { win: m.hwnd.0 as isize, gen: g0, time: m.time, x: m.pt.x, y: m.pt.y };
    let (cx, cy) = (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK));
    let (next, up) = next_click(LAST_CLICK.get(), now, bg, GetDoubleClickTime(), cx, cy);
    LAST_CLICK.set(next);
    up.then(|| (i, g0, svh.get().0 as isize))
}

unsafe fn cycle(d: i32) {
    let (a, n) = app(|a| (a.s.active as i32, a.s.panes() as i32));
    let i = (a + d).rem_euclid(n) as usize;
    set_active(i);
    focus_view(i);
}

/// The slot of the nearest cell wholly to one side of `cur` (dx/dy = -1, 0 or 1) that overlaps it across that axis.
fn neighbour(cells: &[(usize, RECT)], cur: usize, dx: i32, dy: i32) -> Option<usize> {
    let &(_, c) = cells.iter().find(|(s, _)| *s == cur)?;
    let mid = |r: &RECT| if dx != 0 { r.top + r.bottom } else { r.left + r.right };
    cells
        .iter()
        .filter(|(s, _)| *s != cur)
        .filter_map(|&(s, r)| {
            let gap = match (dx, dy) {
                (-1, _) => c.left - r.right,
                (1, _) => r.left - c.right,
                (_, -1) => c.top - r.bottom,
                _ => r.top - c.bottom,
            };
            let overlap = if dx != 0 { r.top.max(c.top) < r.bottom.min(c.bottom) } else { r.left.max(c.left) < r.right.min(c.right) };
            (gap >= 0 && overlap).then_some((gap, (mid(&r) - mid(&c)).abs(), s))
        })
        .min()
        .map(|(_, _, s)| s)
}

// Like Windows Terminal's Alt+arrow or tmux's prefix+arrow: move to the pane on screen in that direction.
unsafe fn move_pane(dx: i32, dy: i32) {
    let (cells, cur) = app(|a| {
        let g = geo(a);
        let cells = (0..a.s.panes()).filter(|&i| a.panes[i].is_some()).map(|i| (i, cell_rect(&g, &a.s.heights, i))).collect::<Vec<_>>();
        (cells, a.s.active)
    });
    if let Some(j) = neighbour(&cells, cur, dx, dy) {
        set_active(j);
        focus_view(j);
    }
}

// Like tmux's pane zoom: the active pane takes the whole grid until toggled back or another pane is picked.
unsafe fn toggle_zoom() {
    if app(|a| a.drag.is_some() || a.s.panes() <= 1) {
        return;
    }
    let i = app(|a| {
        a.zoom = if a.zoom.is_some() { None } else { Some(a.s.active) };
        a.s.active
    });
    relayout();
    redraw_all();
    focus_view(i);
}

unsafe fn set_layout(h: Vec<usize>) {
    app(|a| {
        a.zoom = None;
        a.s.set_heights(h);
    });
    relayout();
    redraw_all();
}

// Same switch as Explorer's "Hidden items" (global shell setting).
unsafe fn hidden() -> bool {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 & 1 != 0
}

unsafe fn toggle_hidden() {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 ^= 1;
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, true);
    for i in 0..MAXP {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
    settings::redraw();
}

// ---------- sessions ----------

thread_local! {
    static LAST_SAVED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Writes every tab (name, lock, order, folders, filters, columns) and the window.
/// The startup mode is stored beside that list; it does not gate the write.
unsafe fn save_session() {
    remember_tabs();
    let main = app(|a| a.main);
    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if !main.0.is_null() {
        _ = GetWindowPlacement(main, &mut wp);
    }
    let text = app(|a| {
        if !main.0.is_null() {
            let r = wp.rcNormalPosition;
            let max = (wp.showCmd == SW_SHOWMAXIMIZED.0 as u32) as i32;
            a.s.win = Some([r.left, r.top, r.right, r.bottom, max]);
        }
        store::sync_text(&a.s)
    });
    let same = LAST_SAVED.with(|l| l.borrow().as_str() == text);
    if same || store::write_settings(&text).is_ok() {
        if !same {
            LAST_SAVED.with(|l| *l.borrow_mut() = text);
        }
        app(|a| a.save_since = None);
    }
}

/// Coalesce tab, folder and window edits into one write, and never wait longer than SAVE_LATEST_MS.
unsafe fn note_save() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    let flush = app(|a| {
        let now = Instant::now();
        let late = a.save_since.is_some_and(|t| now.duration_since(t).as_millis() >= SAVE_LATEST_MS);
        if late {
            a.save_since = None;
            true
        } else {
            if a.save_since.is_none() {
                a.save_since = Some(now);
            }
            false
        }
    });
    if flush {
        _ = KillTimer(Some(main), TIMER_SAVE);
        save_session();
    } else {
        SetTimer(Some(main), TIMER_SAVE, SAVE_SOON_MS, None);
    }
}

// Save button / Ctrl+S: overwrite the current session file; the first time, ask for a name.
unsafe fn save_current() {
    let file = app(|a| a.s.file.clone());
    if file.is_empty() {
        _ = save_session_as();
    } else {
        write_session(Path::new(&file));
    }
}

unsafe fn write_session(path: &Path) {
    remember_tabs();
    if std::fs::write(path, app(|a| a.s.snapshot())).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    app(|a| {
        a.s.file = path.to_string_lossy().into_owned();
        (a.base, a.dirty) = (Some(a.s.clone()), false);
    });
    refresh_save_button();
}

// The named-session save dialog: naming, overwrite prompts, and the sessions folder.
unsafe fn ask_session_path(title: &str) -> Result<String> {
    let main = app(|a| a.main);
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
    let kind = HSTRING::from(tr("Grids session (*.grids)", "Grids 세션 (*.grids)", "Grids セッション (*.grids)"));
    dlg.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(kind.as_ptr()), pszSpec: w!("*.grids") }])?;
    dlg.SetDefaultExtension(w!("grids"))?;
    dlg.SetTitle(&HSTRING::from(title))?;
    if let Ok(f) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.to_string_lossy().as_ref()), None) {
        dlg.SetFolder(&f)?;
    }
    dlg.Show(Some(main))?; // Err on cancel
    Ok(pwstr(dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?))
}

unsafe fn save_session_as() -> Result<()> {
    let path = ask_session_path(tr("Save session", "세션 저장", "セッションを保存"))?;
    write_session(Path::new(&path));
    Ok(())
}

/// Writes one tab's grid to a named session. The window's own session file stays as it is.
unsafe fn save_tab_as_session(i: usize) {
    remember_tabs();
    let Some(text) = app(|a| tab_session_text(&a.s, i)) else { return };
    let Ok(path) = ask_session_path(tr("Save this tab as session", "이 탭을 세션으로 저장", "このタブをセッションとして保存")) else { return };
    if std::fs::write(Path::new(&path), text).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

unsafe fn load_session_file(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let other = Session::parse(&text);
    let mut s = app(|a| a.s.clone());
    s.take_layout(&other);
    s.file = path.to_string_lossy().into_owned();
    install_grids(s);
    app(|a| (a.base, a.dirty) = (Some(a.s.clone()), false));
    refresh_save_button();
    relayout();
    sync_filter_edits();
    redraw_all();
    focus_view(active_pane());
}

// Paths from the command line or a second instance: a session file loads, folders fill panes from the active one on.
unsafe fn open_args(args: Vec<String>) {
    if let Some(f) = session_arg(&args) {
        return load_session_file(Path::new(f));
    }
    if app(|x| x.zoom.take()).is_some() {
        relayout();
        redraw_all();
    }
    let (a, n) = app(|x| (x.s.active, x.s.panes()));
    for (k, p) in args.iter().take(n).enumerate() {
        navigate((a + k) % n, p);
    }
    focus_view(a);
}

// Window of a running copy of this same exe. A portable.txt copy keeps its own grids.json.
unsafe fn running_instance() -> Option<HWND> {
    let me = std::env::current_exe().ok()?.to_string_lossy().to_lowercase();
    let mut h = None;
    loop {
        let w = FindWindowExW(None, h, w!("GridsMain"), PCWSTR::null()).ok()?;
        h = Some(w);
        let mut pid = 0;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
        let (mut buf, mut n) = ([0u16; 1024], 1024u32);
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        _ = CloseHandle(p);
        if ok && String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() == me {
            return Some(w);
        }
    }
}

#[repr(C)]
struct CopyData {
    dw: usize,
    cb: u32,
    lp: *const core::ffi::c_void,
}

// ---------- status bar ----------

// Bumped by each pane's selection-change notification; part of the status key.
static SEL_REV: [AtomicUsize; MAXP] = [const { AtomicUsize::new(0) }; MAXP];

// The selection split into summed file bytes and folders left to the size worker.
// partial: something wasn't counted (over 10k items, no file system path, unreadable, a drive root).
#[derive(Debug, PartialEq)]
struct Sel {
    file_bytes: u64,
    dirs: Vec<String>,
    partial: bool,
}

// n: the selection's count (None = it couldn't be read), path(k): item k's file system path.
fn collect_sel(n: Option<u32>, path: impl Fn(u32) -> Option<String>) -> Sel {
    let Some(n) = n else { return Sel { file_bytes: 0, dirs: Vec::new(), partial: true } };
    // ponytail: sums at most 10k selected items; bigger selections are marked partial
    let mut s = Sel { file_bytes: 0, dirs: Vec::new(), partial: n > 10_000 };
    for k in 0..n.min(10_000) {
        let Some(p) = path(k) else {
            s.partial = true;
            continue;
        };
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() && Path::new(&p).parent().is_some() => s.dirs.push(p),
            Ok(m) if m.is_file() => s.file_bytes += m.len(),
            // a drive or share root (its size is the free-space part), or unreadable
            _ => s.partial = true,
        }
    }
    s
}

unsafe fn selected_size(fv: &IFolderView2) -> Sel {
    let arr = fv.GetSelection(false).ok();
    let n = arr.as_ref().and_then(|a| a.GetCount().ok());
    collect_sel(n, |k| Some(pwstr(arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?)))
}

// The " (…)" after "n selected". dirs: the folder total so far (bytes, done, partial), None = not started.
fn size_text(file_bytes: u64, has_dirs: bool, dirs: Option<(u64, bool, bool)>, collect_partial: bool) -> String {
    let (b, done, partial) = match dirs {
        _ if !has_dirs => (0, true, false),
        Some(d) => d,
        None => (0, false, false),
    };
    let (bytes, partial) = (file_bytes + b, partial || collect_partial);
    if !done {
        return match bytes {
            0 => tr(" (calculating…)", " (계산 중…)", " (計算中…)").into(),
            b => format!(" ({}…)", human(b)),
        };
    }
    match (bytes, partial) {
        (0, false) if !has_dirs => String::new(),
        (0, true) if !has_dirs => tr(" (size unknown)", " (크기 모름)", " (サイズ不明)").into(),
        (b, true) => format!(" ({}, {})", human(b), tr("partial", "일부만", "一部のみ")),
        (b, false) => format!(" ({})", human(b)),
    }
}

// Folder sizes are summed by one worker thread. gen changes on every new target or cancel;
// the worker drops results of an older gen and stops walking once DIR_GEN moves on.
#[derive(Default)]
struct SizeJob {
    gen: usize,
    target: Vec<String>,
    bytes: u64,
    done: bool,
    partial: bool,
}

impl SizeJob {
    // True = a new walk is due. busy (a copy runs) or no folders cancel; the same target keeps
    // running, or keeps its result.
    fn want(&mut self, dirs: &[String], busy: bool) -> bool {
        if busy || dirs.is_empty() {
            (self.gen, self.done) = (self.gen + 1, false);
            self.target.clear();
            return false;
        }
        if self.target == dirs {
            return false;
        }
        *self = SizeJob { gen: self.gen + 1, target: dirs.to_vec(), ..Default::default() };
        true
    }

    fn publish(&mut self, gen: usize, bytes: u64, done: bool, partial: bool) -> bool {
        if self.gen != gen {
            return false;
        }
        (self.bytes, self.done, self.partial) = (bytes, done, partial);
        true
    }
}

static DIR_SIZE: Mutex<SizeJob> = Mutex::new(SizeJob { gen: 0, target: Vec::new(), bytes: 0, done: false, partial: false });
static DIR_WAKE: Condvar = Condvar::new();
static DIR_GEN: AtomicUsize = AtomicUsize::new(0); // copy of DIR_SIZE.gen, read without the lock

fn size_job() -> std::sync::MutexGuard<'static, SizeJob> {
    DIR_SIZE.lock().unwrap_or_else(|e| e.into_inner())
}

fn want_size(dirs: &[String], busy: bool) {
    let mut j = size_job();
    if j.want(dirs, busy) {
        static WORKER: Once = Once::new();
        WORKER.call_once(|| _ = std::thread::Builder::new().spawn(size_worker));
        DIR_WAKE.notify_one();
    }
    DIR_GEN.store(j.gen, Ordering::Relaxed);
}

// Lives until the process ends. The lock is never held across file system calls.
fn size_worker() {
    let mut seen = 0;
    loop {
        let (gen, target) = {
            let mut j = size_job();
            while j.target.is_empty() || j.gen == seen {
                j = DIR_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.gen, j.target.clone())
        };
        seen = gen;
        let stop = || DIR_GEN.load(Ordering::Relaxed) != gen;
        // a copy only pauses the walk; the status line cancels it when it sees the copy
        let paused = || COPYING.load(Ordering::Relaxed) > 0;
        let (mut total, mut partial) = (0, false);
        let finished = target.iter().all(|root| {
            let (base, was) = (total, partial);
            let mut progress = |b, p| _ = size_job().publish(gen, base + b, false, was || p);
            dir_bytes(Path::new(root), &stop, &paused, &mut progress).map(|(b, p)| (total, partial) = (total + b, partial || p)).is_some()
        });
        if finished {
            size_job().publish(gen, total, true, partial);
        }
    }
}

// Bytes under `root`, not following links or junctions. Unreadable parts set the flag and are skipped.
// None = `stop` turned true. Checked before each directory read and each entry, also while `paused`;
// a single read_dir/metadata call that hangs (a dead share) still has to return first.
fn dir_bytes(root: &Path, stop: &dyn Fn() -> bool, paused: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64, bool)) -> Option<(u64, bool)> {
    let go = || {
        while paused() && !stop() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        !stop()
    };
    match std::fs::symlink_metadata(root) {
        Ok(m) if m.is_dir() => {}
        Ok(m) => return Some((if m.is_file() { m.len() } else { 0 }, false)),
        Err(_) => return Some((0, true)),
    }
    let (mut sum, mut partial, mut n, mut last) = (0, false, 0usize, Instant::now());
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if !go() {
            return None;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            partial = true;
            continue;
        };
        for e in rd {
            if !go() {
                return None;
            }
            let Ok(e) = e else {
                partial = true;
                continue;
            };
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => match e.metadata() {
                    Ok(m) => sum += m.len(),
                    Err(_) => partial = true,
                },
                Ok(_) => {} // a link or junction
                Err(_) => partial = true,
            }
            n += 1;
            if n % 2000 == 0 || last.elapsed().as_millis() >= 200 {
                progress(sum, partial);
                last = Instant::now();
            }
        }
    }
    Some((sum, partial))
}

// F5 in a file list: the shell reloads it, and selected folders are summed again.
fn resum_sizes() {
    want_size(&[], true);
    app(|a| a.status_key.clear());
}

// Polled by a 300ms timer; the expensive parts only run when the active pane's state changed.
// Other ticks only merge the folder size worker's progress into the stored text.
unsafe fn update_status() {
    let (i, path, main) = app(|a| (a.s.active, a.s.paths[a.s.active].clone(), a.main));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let sel = fv.ItemCount(SVGIO_SELECTION).unwrap_or(0);
    let spec = filter_of(i);
    // while a background copy runs, the disk is left alone (size and free space come back after)
    let busy = COPYING.load(Ordering::Relaxed) > 0;
    let zoom = app(|a| a.zoom);
    let rev = SEL_REV[i].load(Ordering::Relaxed);
    let key = format!("{i}|{path}|{n}|{sel}|{}|{spec}|{busy}|{zoom:?}|{rev}", fv.GetFocusedItem().unwrap_or(-1));
    if app(|a| a.status_key != key) {
        status_base(i, &path, main, &fv, n, sel, &spec, busy, zoom);
        app(|a| a.status_key = key);
    }
    let job = {
        let j = size_job();
        (!j.target.is_empty()).then_some((j.bytes, j.done, j.partial))
    };
    let Some(rc) = app(|a| {
        let (head, size, tail) = &a.status_base;
        let size = size.map(|(f, d, p)| size_text(f, d, job, p)).unwrap_or_default();
        // a short confirmation (path copied) leads for 3 s: the line's end gets cut when it's too long
        a.note = a.note.take().filter(|(_, t)| t.elapsed().as_secs() < 3);
        let note = a.note.as_ref().map(|(s, _)| format!("{s}     ·     ")).unwrap_or_default();
        let status = format!("{note}{head}{size}{tail}");
        (a.status != status).then(|| {
            a.status = status;
            status_rect(&geo(a))
        })
    }) else {
        return;
    };
    _ = InvalidateRect(Some(main), Some(&rc), true);
}

// The status text around the selection size, the window title, and a new folder size job.
unsafe fn status_base(i: usize, path: &str, main: HWND, fv: &IFolderView2, n: i32, sel: i32, spec: &str, busy: bool, zoom: Option<usize>) {
    let title = window_title(&app(|a| a.panes[i].as_ref().map(|p| p.disp.clone()).unwrap_or_default()));
    if text_of(main) != title {
        _ = SetWindowTextW(main, &HSTRING::from(title));
    }
    let mut parts = vec![match ui() {
        1 => format!("항목 {n}개"),
        2 => format!("項目 {n} 個"),
        _ => format!("{n} items"),
    }];
    let mut size = None;
    let mut dirs = Vec::new();
    if sel > 0 {
        if !busy {
            let s = selected_size(fv);
            size = Some((s.file_bytes, !s.dirs.is_empty(), s.partial));
            dirs = s.dirs;
        }
        parts.push(match ui() {
            1 => format!("{sel}개 선택"),
            2 => format!("{sel} 個を選択"),
            _ => format!("{sel} selected"),
        });
    }
    want_size(&dirs, busy);
    let head = parts.join("     ·     ");
    let mut parts = vec![String::new()];
    if !spec.is_empty() {
        parts.push(format!("{} {spec}", tr("Filter", "필터", "フィルター")));
    }
    let drive = path.get(1..2) == Some(":");
    if busy {
        parts.push(tr("Copying / moving…", "복사·이동 중…", "コピー / 移動中…").into());
    } else if drive || path.starts_with(r"\\") {
        // ponytail: a dead network share can stall this call once per change
        let (mut free, mut total) = (0, 0);
        if GetDiskFreeSpaceExW(&HSTRING::from(path),Some(&mut free), Some(&mut total), None).is_ok() {
            let label = if drive { &path[..2] } else { tr("Network", "네트워크", "ネットワーク") };
            parts.push(format!("{label} {} {} / {}", tr("free", "여유", "空き"), human(free), human(total)));
        }
    }
    if let Some(z) = zoom {
        let z = z + 1;
        parts.push(match ui() {
            1 => format!("패널 {z} 크게 보는 중 · Ctrl+Shift+M 또는 머리줄 더블클릭으로 복원"),
            2 => format!("ペイン {z} を最大化中 · Ctrl+Shift+M またはヘッダーのダブルクリックで戻す"),
            _ => format!("Pane {z} maximized · Ctrl+Shift+M or double-click its header to restore"),
        });
    }
    app(|a| a.status_base = (head, size, parts.join("     ·     ")));
}

// ---------- tabs ----------
// The strip is its own child so splitter hit-testing stays below it. Every hidden tab keeps its
// grid on the session. Only the most recently hidden tab also keeps its Explorer windows.

#[derive(Clone, Copy, PartialEq)]
enum TabHit {
    Tab(usize),
    Close(usize),
    Plus,
    Empty,
}

struct TabLay {
    body: Vec<RECT>,
    close: Vec<Option<RECT>>,
    plus: RECT,
}

fn tab_lay(width: i32, n: usize, locked: &[bool], dpi: i32) -> TabLay {
    let s = |v: i32| v * dpi / 96;
    let n = n.max(1);
    let (pad, gap, plus_w, max_w, min_w) = (s(6), s(2), s(28), s(90), s(28));
    let mut w = max_w;
    let used = |w: i32| n as i32 * w + gap * (n as i32 - 1) + plus_w + pad * 2;
    if used(w) > width {
        w = ((width - pad * 2 - plus_w - gap * (n as i32 - 1)) / n as i32).max(min_w);
    }
    let y = s(4);
    let h = s(26);
    let mut x = pad;
    let mut body = Vec::new();
    let mut close = Vec::new();
    for i in 0..n {
        let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
        let cw = s(16);
        let c = if locked.get(i).copied().unwrap_or(false) {
            None
        } else {
            Some(RECT {
                left: r.right - s(4) - cw,
                top: r.top + (h - cw) / 2,
                right: r.right - s(4),
                bottom: r.top + (h - cw) / 2 + cw,
            })
        };
        body.push(r);
        close.push(c);
        x += w + gap;
    }
    TabLay { body, close, plus: RECT { left: x, top: y, right: x + plus_w, bottom: y + h } }
}

fn in_rect(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn tab_hit(lay: &TabLay, x: i32, y: i32) -> TabHit {
    if in_rect(lay.plus, x, y) {
        return TabHit::Plus;
    }
    for (i, r) in lay.body.iter().enumerate() {
        if in_rect(*r, x, y) {
            if lay.close[i].is_some_and(|c| in_rect(c, x, y)) {
                return TabHit::Close(i);
            }
            return TabHit::Tab(i);
        }
    }
    TabHit::Empty
}

fn put_filters(f: &[String]) {
    if let Ok(mut g) = FILTERS.lock() {
        for i in 0..MAXP {
            g[i] = f.get(i).cloned().unwrap_or_default();
        }
    }
}

fn fit_park(a: &mut App) {
    let n = a.s.tabs.len();
    if a.park.len() < n {
        a.park.resize_with(n, no_panes);
    } else if a.park.len() > n {
        a.park.truncate(n);
    }
    a.park_lru.retain(|&i| i < a.park.len());
}

fn lru_note(a: &mut App, i: usize) {
    a.park_lru.retain(|&x| x != i);
    a.park_lru.push(i);
}

fn lru_forget(a: &mut App, i: usize) {
    a.park_lru.retain(|&x| x != i);
}

fn lru_insert_shift(a: &mut App, at: usize) {
    for x in &mut a.park_lru {
        if *x >= at {
            *x += 1;
        }
    }
}

fn lru_remove_shift(a: &mut App, at: usize) {
    a.park_lru.retain(|&x| x != at);
    for x in &mut a.park_lru {
        if *x > at {
            *x -= 1;
        }
    }
}

fn park_has_browsers(set: &[Option<Pane>]) -> bool {
    set.iter().any(|p| p.is_some())
}

/// Drop hidden Explorer windows past PARK_CACHE. Their folders, columns and filters stay on the tab.
unsafe fn evict_park() {
    loop {
        let victim = app(|a| {
            let live = a.park.iter().filter(|s| park_has_browsers(s)).count();
            if live <= PARK_CACHE {
                return None;
            }
            a.park_lru.iter().copied().find(|&i| a.park.get(i).is_some_and(|s| park_has_browsers(s))).or_else(|| {
                a.park.iter().position(|s| park_has_browsers(s))
            })
        });
        let Some(i) = victim else { break };
        let set = app(|a| {
            lru_forget(a, i);
            if i < a.park.len() {
                std::mem::replace(&mut a.park[i], no_panes())
            } else {
                no_panes()
            }
        });
        destroy_set(set);
    }
}

unsafe fn pull_filters() -> Vec<String> {
    let (n, stored) = app(|a| {
        let stored = a.s.tabs.get(a.s.tab).map(|t| t.filters.clone()).unwrap_or_default();
        (a.s.panes(), stored)
    });
    let mut f = pad_filters(stored);
    for i in 0..n {
        if let Some(e) = pane_ctl(i, ID_FILTER) {
            f[i] = filter_spec(&text_of(e));
        }
    }
    put_filters(&f);
    f
}

/// Fold the live grid and filters into the active tab so a switch or a save keeps them.
unsafe fn remember_tabs() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        save_view(i);
    }
    let filters = pull_filters();
    app(|a| {
        if a.s.tab < a.s.tabs.len() {
            a.s.tabs[a.s.tab].filters = pad_filters(filters);
        }
        a.s.anchor();
    });
}

unsafe fn detach_live() {
    for i in 0..MAXP {
        let got = app(|a| {
            a.panes[i].as_mut().map(|p| {
                let eb = p.eb.clone();
                let cookie = std::mem::take(&mut p.cookie);
                (eb, cookie, p.hwnd)
            })
        });
        if let Some((eb, cookie, hwnd)) = got {
            if cookie != 0 {
                _ = eb.Unadvise(cookie);
            }
            _ = IUnknown_SetSite(&eb, None);
            _ = KillTimer(Some(hwnd), TIMER_SETTLE);
            _ = KillTimer(Some(hwnd), TIMER_FILTER);
            _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

unsafe fn attach_live() {
    for i in 0..MAXP {
        let eb = app(|a| a.panes[i].as_ref().map(|p| p.eb.clone()));
        let Some(eb) = eb else { continue };
        let site: IServiceProvider = Host(i).into();
        _ = IUnknown_SetSite(&eb, &site);
        let ev: IExplorerBrowserEvents = Events(i).into();
        if let Ok(c) = eb.Advise(&ev) {
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.cookie = c;
                }
            });
        }
    }
}

unsafe fn sync_filter_edits() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        let spec = filter_of(i);
        if let Some(e) = pane_ctl(i, ID_FILTER) {
            if text_of(e) != spec {
                _ = SetWindowTextW(e, &HSTRING::from(spec.as_str()));
            }
        }
    }
}

unsafe fn destroy_set(set: Vec<Option<Pane>>) {
    if set.iter().all(|p| p.is_none()) {
        return;
    }
    let (back, keep) = app(|a| {
        let keep = FILTERS.lock().map(|f| f.clone()).unwrap_or_else(|_| [const { String::new() }; MAXP]);
        (std::mem::replace(&mut a.panes, set), keep)
    });
    // These browsers belong to a tab that is not on screen. Their grid is already on that tab.
    KEEP_VIEWS.with(|c| c.set(true));
    for i in 0..MAXP {
        destroy_pane(i);
    }
    KEEP_VIEWS.with(|c| c.set(false));
    app(|a| a.panes = back);
    put_filters(&keep);
}

unsafe fn destroy_every_pane() {
    for i in 0..MAXP {
        destroy_pane(i);
    }
    let sets = app(|a| a.park.drain(..).collect::<Vec<_>>());
    for set in sets {
        destroy_set(set);
    }
}

fn tabs_invalidate() {
    let h = app(|a| a.tabs_hwnd);
    if !h.0.is_null() {
        unsafe { _ = InvalidateRect(Some(h), None, false) };
    }
}

unsafe fn switch_tab(j: usize) {
    let (cur, n) = app(|a| (a.s.tab, a.s.tabs.len()));
    if j >= n || j == cur {
        return;
    }
    end_rename(true);
    remember_tabs();
    detach_live();
    let filters = app(|a| {
        fit_park(a);
        let live = std::mem::replace(&mut a.panes, no_panes());
        a.park[a.s.tab] = live;
        lru_note(a, a.s.tab);
        a.panes = std::mem::take(&mut a.park[j]);
        if a.panes.len() != MAXP {
            a.panes = no_panes();
        }
        lru_forget(a, j);
        a.s.tab = j;
        a.s.publish();
        a.zoom = None;
        a.s.tabs[j].filters.clone()
    });
    put_filters(&filters);
    attach_live();
    relayout();
    sync_filter_edits();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    evict_park();
    note_save();
}

unsafe fn switch_rel(d: i32) {
    let (i, n) = app(|a| (a.s.tab as i32, a.s.tabs.len() as i32));
    if n <= 0 {
        return;
    }
    switch_tab((i + d).rem_euclid(n) as usize);
}

unsafe fn new_tab() {
    end_rename(true);
    remember_tabs();
    detach_live();
    app(|a| {
        fit_park(a);
        let live = std::mem::replace(&mut a.panes, no_panes());
        a.park[a.s.tab] = live;
        lru_note(a, a.s.tab);
        let fresh = Session {
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            cols: vec![String::new(); MAXP],
            ..a.s.clone()
        };
        let name = next_tab_name(&a.s.tabs);
        a.s.tabs.push(Tab::from_live(&fresh, name, false, vec![String::new(); MAXP]));
        a.park.push(no_panes());
        a.s.tab = a.s.tabs.len() - 1;
        a.s.publish();
        a.zoom = None;
    });
    put_filters(&vec![String::new(); MAXP]);
    attach_live();
    relayout();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    evict_park();
    note_save();
}

unsafe fn duplicate_tab(i: usize) {
    end_rename(true);
    remember_tabs();
    let j = app(|a| {
        if i >= a.s.tabs.len() {
            return None;
        }
        fit_park(a);
        let mut t = a.s.tabs[i].clone();
        t.name = duplicate_name(&a.s.tabs, &t.name);
        t.locked = false;
        let j = (i + 1).min(a.s.tabs.len());
        a.s.tabs.insert(j, t);
        a.park.insert(j, no_panes());
        lru_insert_shift(a, j);
        if j <= a.s.tab {
            a.s.tab += 1;
        }
        Some(j)
    });
    if let Some(j) = j {
        switch_tab(j);
    }
}

unsafe fn close_tab(i: usize) {
    end_rename(true);
    if !app(|a| can_close_tab(&a.s.tabs, i)) {
        return;
    }
    let active = app(|a| a.s.tab);
    if i == active {
        remember_tabs();
        detach_live();
    }
    let (doomed, filters, switched) = app(|a| {
        fit_park(a);
        let tab = a.s.tabs.remove(i);
        let mut parked = a.park.remove(i);
        lru_remove_shift(a, i);
        a.closed.insert(0, tab);
        a.closed.truncate(CLOSED_KEEP);
        let was = a.s.tab;
        let switched = i == was;
        if switched {
            let j = i.min(a.s.tabs.len() - 1);
            a.s.tab = j;
            let next = std::mem::replace(&mut a.park[j], no_panes());
            lru_forget(a, j);
            parked = std::mem::replace(&mut a.panes, next);
            a.s.publish();
        } else if i < was {
            a.s.tab = was - 1;
        }
        let filters = a.s.tabs[a.s.tab].filters.clone();
        (parked, filters, switched)
    });
    destroy_set(doomed);
    if switched {
        put_filters(&filters);
        attach_live();
        relayout();
        sync_filter_edits();
        redraw_all();
        focus_view(active_pane());
    }
    tabs_invalidate();
    note_save();
}

unsafe fn close_other_tabs(keep: usize) {
    let victims = app(|a| close_other_indexes(&a.s.tabs, keep));
    for i in victims.into_iter().rev() {
        close_tab(i);
    }
}

unsafe fn reopen_closed_at(k: usize) {
    end_rename(true);
    let j = app(|a| {
        if k >= a.closed.len() {
            return None;
        }
        let t = a.closed.remove(k);
        fit_park(a);
        let j = (a.s.tab + 1).min(a.s.tabs.len());
        a.s.tabs.insert(j, t);
        a.park.insert(j, no_panes());
        lru_insert_shift(a, j);
        Some(j)
    });
    if let Some(j) = j {
        switch_tab(j);
    }
}

unsafe fn reorder_tab(from: usize, to: usize) {
    if from == to {
        return;
    }
    app(|a| {
        if from >= a.s.tabs.len() || to >= a.s.tabs.len() {
            return;
        }
        fit_park(a);
        reorder_vec(&mut a.s.tabs, from, to);
        reorder_vec(&mut a.park, from, to);
        a.park_lru = a.park_lru.iter().copied().map(|i| moved_active(i, from, to)).collect();
        a.s.tab = moved_active(a.s.tab, from, to);
    });
    tabs_invalidate();
    note_save();
}

unsafe fn end_rename(save: bool) {
    let (edit, which) = app(|a| {
        a.tab_rename_skip = !save;
        (a.tab_edit, a.tab_renaming.take())
    });
    if edit.0.is_null() || which.is_none() {
        return;
    }
    if save {
        if let Some(i) = which {
            let name = text_of(edit);
            app(|a| {
                if let Some(t) = a.s.tabs.get_mut(i) {
                    let name = name.trim();
                    if !name.is_empty() {
                        t.name = name.chars().take(80).collect();
                    }
                }
            });
        }
    }
    _ = ShowWindow(edit, SW_HIDE);
    tabs_invalidate();
    if save {
        note_save();
    }
}

unsafe fn begin_rename(i: usize) {
    let (edit, font, dpi, name, lay_w) = app(|a| {
        let mut rc = RECT::default();
        _ = GetClientRect(a.tabs_hwnd, &mut rc);
        let name = a.s.tabs.get(i).map(|t| t.name.clone()).unwrap_or_default();
        (a.tab_edit, a.font, a.dpi, name, rc.right)
    });
    if edit.0.is_null() || app(|a| i >= a.s.tabs.len()) {
        return;
    }
    let locked: Vec<bool> = app(|a| a.s.tabs.iter().map(|t| t.locked).collect());
    let lay = tab_lay(lay_w, locked.len(), &locked, dpi);
    let Some(r) = lay.body.get(i).copied() else { return };
    let s = |v: i32| v * dpi / 96;
    app(|a| a.tab_renaming = Some(i));
    _ = SetWindowTextW(edit, &HSTRING::from(name.as_str()));
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    _ = SetWindowPos(edit, None, r.left + s(6), r.top + s(3), (r.right - r.left - s(28)).max(s(24)), r.bottom - r.top - s(6), SWP_NOZORDER | SWP_SHOWWINDOW);
    _ = SetFocus(Some(edit));
    let len = GetWindowTextLengthW(edit) as usize;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(len as isize)));
}

unsafe fn set_tab_color(i: usize, color: Option<u32>) {
    app(|a| {
        if let Some(t) = a.s.tabs.get_mut(i) {
            t.color = color;
        }
    });
    tabs_invalidate();
    note_save();
}

unsafe fn tab_menu(i: usize, x: i32, y: i32) {
    let (hwnd, locked, others, color) = app(|a| {
        let tab = a.s.tabs.get(i);
        let locked = tab.is_some_and(|t| t.locked);
        let others = !close_other_indexes(&a.s.tabs, i).is_empty();
        (a.tabs_hwnd, locked, others, tab.and_then(|t| t.color))
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, text: &str, on: bool| {
        let flags = if on { MF_STRING } else { MF_STRING | MF_GRAYED };
        _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
    };
    add(1, tr("Rename tab", "이름 변경", "タブ名の変更"), true);
    add(2, if locked { tr("Unlock tab", "탭 잠금 해제", "タブのロックを解除") } else { tr("Lock tab", "탭 잠금", "タブをロック") }, true);
    add(3, tr("Duplicate tab", "탭 복제", "タブを複製"), true);
    add(4, tr("Close other tabs", "다른 탭 모두 닫기", "他のタブをすべて閉じる"), others);
    add(5, tr("Save this tab as session", "이 탭을 세션으로 저장", "このタブをセッションとして保存"), true);
    let Ok(colors) = CreatePopupMenu() else {
        _ = DestroyMenu(menu);
        return;
    };
    let mark = |menu: HMENU, id: usize, on: bool| {
        if on {
            _ = CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | MF_CHECKED).0);
        }
    };
    _ = AppendMenuW(colors, MF_STRING, 19, &HSTRING::from(tr("None", "없음", "なし")));
    mark(colors, 19, color.is_none());
    for (k, &(c, en, ko, ja)) in TAB_COLORS.iter().enumerate() {
        let id = 20 + k;
        _ = AppendMenuW(colors, MF_STRING, id, &HSTRING::from(tr(en, ko, ja)));
        mark(colors, id, color == Some(c));
    }
    _ = AppendMenuW(menu, MF_POPUP, colors.0 as usize, &HSTRING::from(tr("Tab color", "탭 색 지정", "タブの色")));
    let mut pt = POINT { x, y };
    _ = ClientToScreen(hwnd, &mut pt);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, hwnd, None).0 as usize;
    _ = DestroyMenu(menu);
    match pick {
        1 => begin_rename(i),
        2 => {
            app(|a| {
                if let Some(t) = a.s.tabs.get_mut(i) {
                    t.locked ^= true;
                }
            });
            tabs_invalidate();
            note_save();
        }
        3 => duplicate_tab(i),
        4 => close_other_tabs(i),
        5 => save_tab_as_session(i),
        19 => set_tab_color(i, None),
        n if (20..20 + TAB_COLORS.len()).contains(&n) => set_tab_color(i, Some(TAB_COLORS[n - 20].0)),
        _ => {}
    }
}

unsafe fn install_grids(s: Session) {
    destroy_every_pane();
    let filters = s.tabs.get(s.tab.min(s.tabs.len().saturating_sub(1))).map(|t| t.filters.clone()).unwrap_or_default();
    let n = s.tabs.len().max(1);
    put_filters(&filters);
    app(|a| {
        a.s = s;
        a.park = (0..n).map(|_| no_panes()).collect();
        a.park_lru.clear();
        a.zoom = None;
    });
}

fn current_tab_lay() -> TabLay {
    app(|a| {
        let mut rc = RECT::default();
        unsafe { _ = GetClientRect(a.tabs_hwnd, &mut rc) };
        let locked: Vec<bool> = a.s.tabs.iter().map(|t| t.locked).collect();
        tab_lay(rc.right.max(1), locked.len().max(1), &locked, a.dpi)
    })
}

unsafe fn paint_tabs(h: HWND) {
    let (active, names, locked, colors, hot, hot_close, hot_plus, dpi, font, icon, accent) = app(|a| {
        (
            a.s.tab,
            a.s.tabs.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            a.s.tabs.iter().map(|t| t.locked).collect::<Vec<_>>(),
            a.s.tabs.iter().map(|t| t.color).collect::<Vec<_>>(),
            a.tab_hot,
            a.tab_hot_close,
            a.tab_hot_plus,
            a.dpi,
            a.font,
            a.icon_font,
            a.s.accent_now(),
        )
    });
    let lay = current_tab_lay();
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        fill(dc, rc, pal().bg);
        fill(dc, RECT { top: rc.bottom - 1, ..rc }, pal().line);
        for (i, r) in lay.body.iter().enumerate() {
            let on = i == active;
            let over = hot == Some(i);
            let col = colors.get(i).copied().flatten();
            let bg = match (on, over, col) {
                (true, _, Some(c)) => blend(pal().card, c, 0.18),
                (true, _, None) => pal().card,
                (_, true, Some(c)) => blend(blend(pal().card, pal().bg, 0.45), c, 0.16),
                (_, true, None) => blend(pal().card, pal().bg, 0.45),
                (_, _, Some(c)) => blend(pal().bg, c, 0.18),
                _ => pal().bg,
            };
            fill(dc, *r, bg);
            if !on {
                frame(dc, *r, 1, pal().line);
            }
            if on {
                fill(dc, RECT { bottom: r.top + s(2), ..*r }, col.unwrap_or(accent));
                fill(dc, RECT { top: r.bottom - 1, bottom: r.bottom, ..*r }, pal().card);
            }
            if let Some(c) = col {
                let inset = if on { 0 } else { 1 };
                fill(dc, RECT { left: r.left + inset, top: r.top + inset, right: r.left + inset + s(4), bottom: r.bottom - inset }, c);
            }
            let lock = locked.get(i).copied().unwrap_or(false);
            let nudge = if col.is_some() { s(5) } else { 0 };
            let mut label = *r;
            if lock {
                let ir = RECT { left: r.left + nudge + s(4), top: r.top, right: r.left + nudge + s(20), bottom: r.bottom };
                text(dc, ir, ICON_LOCK, icon, if on { pal().text } else { pal().sub }, DT_CENTER);
                label.left = ir.right;
            } else {
                label.left += s(8) + nudge;
            }
            if let Some(c) = lay.close[i] {
                label.right = c.left;
                let ink = if hot_close && hot == Some(i) { accent } else { pal().sub };
                text(dc, c, ICON_CLOSE, icon, ink, DT_CENTER);
            } else {
                label.right -= s(8);
            }
            text(dc, label, names.get(i).map(|n| n.as_str()).unwrap_or(""), font, if on { pal().text } else { pal().sub }, DT_LEFT);
        }
        let plus_bg = if hot_plus { blend(pal().card, pal().bg, 0.45) } else { pal().bg };
        fill(dc, lay.plus, plus_bg);
        text(dc, lay.plus, ICON_PLUS, icon, pal().text, DT_CENTER);
    });
}

unsafe fn tab_mouse(x: i32, y: i32) -> TabHit {
    tab_hit(&current_tab_lay(), x, y)
}

extern "system" fn tabs_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_tabs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hit = tab_mouse(x, y);
                let (hot, close, plus) = match hit {
                    TabHit::Tab(i) => (Some(i), false, false),
                    TabHit::Close(i) => (Some(i), true, false),
                    TabHit::Plus => (None, false, true),
                    TabHit::Empty => (None, false, false),
                };
                let (from, paint) = app(|a| {
                    if let Some((pi, px, py, _)) = a.tab_press {
                        if a.tab_drag.is_none() && (x - px).abs() + (y - py).abs() > (4 * a.dpi / 96).max(4) {
                            a.tab_drag = Some(pi);
                        }
                    }
                    let from = a.tab_drag;
                    let same = a.tab_hot == hot && a.tab_hot_close == close && a.tab_hot_plus == plus;
                    if !same {
                        a.tab_hot = hot;
                        a.tab_hot_close = close;
                        a.tab_hot_plus = plus;
                    }
                    (from, !same)
                });
                if let Some(from) = from {
                    let lay = current_tab_lay();
                    let to = lay.body.iter().position(|r| x < (r.left + r.right) / 2).unwrap_or(lay.body.len().saturating_sub(1));
                    if to != from && to < app(|a| a.s.tabs.len()) {
                        reorder_tab(from, to);
                        app(|a| a.tab_drag = Some(to));
                    }
                }
                if paint || from.is_some() {
                    tabs_invalidate();
                }
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut t);
            }
            WM_MOUSELEAVE => {
                app(|a| {
                    a.tab_hot = None;
                    a.tab_hot_close = false;
                    a.tab_hot_plus = false;
                });
                tabs_invalidate();
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                end_rename(true);
                match tab_mouse(x, y) {
                    TabHit::Tab(i) | TabHit::Close(i) => {
                        let on_close = matches!(tab_mouse(x, y), TabHit::Close(_));
                        app(|a| a.tab_press = Some((i, x, y, on_close)));
                        SetCapture(h);
                        if !on_close {
                            switch_tab(i);
                        }
                    }
                    TabHit::Plus => new_tab(),
                    TabHit::Empty => {}
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let press = app(|a| a.tab_press.take());
                let dragged = app(|a| a.tab_drag.take());
                _ = ReleaseCapture();
                if let Some((i, _, _, on_close)) = press {
                    if dragged.is_none() && on_close {
                        if let TabHit::Close(j) = tab_mouse(x, y) {
                            if j == i {
                                close_tab(i);
                            }
                        }
                    }
                }
            }
            WM_LBUTTONDBLCLK => {
                if let TabHit::Tab(i) = tab_mouse(xy(lp).0, xy(lp).1) {
                    begin_rename(i);
                }
            }
            WM_MBUTTONUP => {
                if let TabHit::Tab(i) | TabHit::Close(i) = tab_mouse(xy(lp).0, xy(lp).1) {
                    close_tab(i);
                }
            }
            WM_RBUTTONUP => {
                let (x, y) = xy(lp);
                let i = match tab_mouse(x, y) {
                    TabHit::Tab(i) | TabHit::Close(i) => i,
                    _ => app(|a| a.s.tab),
                };
                tab_menu(i, x, y);
            }
            WM_CAPTURECHANGED => {
                app(|a| {
                    a.tab_press = None;
                    a.tab_drag = None;
                });
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().card));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                (ID_TAB_EDIT, EN_KILLFOCUS) => {
                    let skip = app(|a| a.tab_rename_skip);
                    if !skip {
                        end_rename(true);
                    } else {
                        app(|a| a.tab_rename_skip = false);
                    }
                }
                _ => {}
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- painting ----------

// Ground, the sidebar plate and the status line. Pane plates are child windows (paint_pane).
unsafe fn paint_main(h: HWND) {
    let (g, status, font, dpi) = app(|a| (geo(a), a.status.clone(), a.font, a.dpi));
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().bg);
        if g.side > 0 {
            let side = side_rect(&g);
            fill(dc, side, pal().card);
            frame(dc, side, 1, pal().line);
        }
        if g.bot < g.h {
            let st = status_rect(&g);
            text(dc, RECT { left: g.pad + 4 * dpi / 96, ..st }, &status, font, pal().sub, DT_LEFT);
        }
    });
}

// One white plate per pane: a single accent border when active, flat fields in the top bar.
unsafe fn paint_pane(h: HWND) {
    let (active, i, dpi, icon_font, small_font, accent) = app(|a| {
        let i = a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h));
        // a drop target-to-be looks active too
        let active = a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h) || a.drop_hover == h;
        (active, i, a.dpi, a.icon_font, a.small_font, a.s.accent_now())
    });
    let filtered = i.is_some_and(|i| FILTERS.lock().is_ok_and(|f| !f[i].is_empty()));
    let (head, ink) = header_colors(h);
    let focus = GetFocus();
    let focused = |id: i32| GetDlgItem(Some(h), id).is_ok_and(|e| e == focus);
    let (addr_on, filt_on) = (focused(ID_EDIT), focused(ID_FILTER));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let pg = pane_geo(rc.right, rc.bottom, dpi);
        fill(dc, rc, pal().card);
        // active: header bar in the selection colour, same colour as the border
        fill(dc, RECT { bottom: pg.view.top, ..rc }, head);
        let edge = app(|a| a.s.edge);
        if active && edge {
            frame(dc, rc, s(2), accent);
        } else {
            frame(dc, rc, 1, pal().line);
            fill(dc, RECT { left: pg.view.left, top: pg.view.top - 1, right: pg.view.right, bottom: pg.view.top }, pal().line);
        }
        if let Some(i) = i.filter(|_| pg.num.right > pg.num.left) {
            text(dc, pg.num, &(i + 1).to_string(), small_font, if active { ink } else { pal().sub }, DT_CENTER);
        }
        for (p, on) in [(pg.addr, addr_on), (pg.filt, filt_on)] {
            fill(dc, p, pal().pill);
            if on {
                frame(dc, p, 1, if active { ink } else { accent });
            }
        }
        if filtered {
            // filter applied: accent chip left of the edit (which starts at filt.left + s(24))
            let chip =
                RECT { left: pg.filt.left + s(3), top: pg.filt.top + s(3), right: pg.filt.left + s(21), bottom: pg.filt.bottom - s(3) };
            fill(dc, chip, accent);
            text(dc, chip, ICON_FILTER, icon_font, on_color(accent), DT_CENTER);
        } else {
            let icon = RECT { left: pg.filt.left + s(4), right: pg.filt.left + s(22), ..pg.filt };
            text(dc, icon, ICON_SEARCH, icon_font, pal().sub, DT_CENTER);
        }
    });
}

// ---------- layout picker: click = even grid, press and drag across columns = rows per column, presets ----------

const PRESETS: [&[usize]; 6] = [&[1, 2], &[1, 3], &[2, 1], &[3, 1], &[1, 1, 2], &[2, 3]];

struct PickGeo {
    w: i32,
    h: i32,
    pad: i32,
    cell: i32,
    gap: i32,
    label: RECT,
    presets: [RECT; 6],
}

fn pick_geo(dpi: i32) -> PickGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, cell, gap) = (s(12), s(24), s(6));
    let gw = 4 * cell + 3 * gap;
    let label = RECT { left: pad, top: pad + gw, right: pad + gw, bottom: pad + gw + s(26) };
    let (pw, ph) = ((gw - 2 * gap) / 3, s(24));
    let presets = std::array::from_fn(|i| {
        let (x, y) = (pad + (i as i32 % 3) * (pw + gap), label.bottom + (i as i32 / 3) * (ph + gap));
        RECT { left: x, top: y, right: x + pw, bottom: y + ph }
    });
    PickGeo { w: 2 * pad + gw, h: label.bottom + 2 * ph + gap + pad, pad, cell, gap, label, presets }
}

fn pick_cell(g: &PickGeo, r: usize, c: usize) -> RECT {
    let (x, y) = (g.pad + c as i32 * (g.cell + g.gap), g.pad + r as i32 * (g.cell + g.gap));
    RECT { left: x, top: y, right: x + g.cell, bottom: y + g.cell }
}

/// (row, column) under the point; `clamp` keeps a drag that leaves the cells on the nearest one.
fn pick_at(g: &PickGeo, x: i32, y: i32, clamp: bool) -> Option<(usize, usize)> {
    let span = 4 * g.cell + 3 * g.gap;
    let inside = (g.pad..g.pad + span).contains(&x) && (g.pad..g.pad + span).contains(&y);
    let at = |v: i32| ((v - g.pad).max(0) / (g.cell + g.gap)).min(3) as usize;
    (inside || clamp).then(|| (at(y), at(x)))
}

/// Dragging: columns follow the cursor; the column under it takes the cursor's row.
fn drag_heights(mut h: Vec<usize>, r: usize, c: usize) -> Vec<usize> {
    h.resize(c + 1, r + 1);
    h[c] = r + 1;
    h
}

unsafe fn open_grid() {
    let (main, dpi, open, closed, cur) = app(|a| (a.main, a.dpi, a.grid, a.grid_closed, a.s.heights.clone()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        _ = GetWindowRect(b, &mut rc);
    }
    let g = pick_geo(dpi);
    app(|a| (a.grid_preview, a.grid_drag, a.grid_preset) = (cur, None, None));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsPicker"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.grid = h);
}

unsafe fn paint_grid(h: HWND) {
    let (preview, cur, hot, dpi, font, accent) =
        app(|a| (a.grid_preview.clone(), a.s.heights.clone(), a.grid_preset, a.dpi, a.font, a.s.accent_now()));
    let soft = blend(accent, pal().card, 0.8);
    let g = pick_geo(dpi);
    let on = |v: &[usize], r: usize, c: usize| c < v.len() && r < v[c];
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        for r in 0..4 {
            for c in 0..4 {
                let col = if on(&preview, r, c) {
                    accent
                } else if on(&cur, r, c) {
                    soft
                } else {
                    pal().pill
                };
                fill(dc, pick_cell(&g, r, c), col);
            }
        }
        let label = if preview.iter().all(|&n| n == preview[0]) {
            match ui() {
                1 => format!("{}행 × {}열", preview[0], preview.len()),
                2 => format!("{} 行 × {} 列", preview[0], preview.len()),
                _ => format!("{} × {}", preview[0], preview.len()),
            }
        } else {
            format!("{}  {}", tr("Per column", "열마다", "列ごと"), layout_label(&preview))
        };
        text(dc, g.label, &label, font, pal().text, DT_CENTER);
        // preset thumbnails: the panes of each layout on a tile
        for (i, (p, t)) in PRESETS.iter().zip(g.presets).enumerate() {
            let (tile, pane) = if hot == Some(i) { (soft, accent) } else { (pal().pill, blend(pal().card, pal().text, 0.25)) };
            fill(dc, t, tile);
            let inner = RECT { left: t.left + 3, top: t.top + 3, right: t.right - 3, bottom: t.bottom - 3 };
            for (c, (x0, x1)) in cells(inner.left, inner.right - inner.left, &even(p.len()), 2).into_iter().enumerate() {
                for (y0, y1) in cells(inner.top, inner.bottom - inner.top, &even(p[c]), 2) {
                    fill(dc, RECT { left: x0, top: y0, right: x1, bottom: y1 }, pane);
                }
            }
        }
    });
}

extern "system" fn grid_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = pick_geo(app(|a| a.dpi));
        let show = |v: Vec<usize>, preset: Option<usize>| {
            let changed = app(|a| {
                let changed = a.grid_preview != v || a.grid_preset != preset;
                (a.grid_preview, a.grid_preset) = (v, preset);
                changed
            });
            if changed {
                _ = InvalidateRect(Some(h), None, false);
            }
        };
        let apply = |v: Vec<usize>| {
            _ = DestroyWindow(h);
            set_layout(v);
        };
        match msg {
            WM_PAINT => paint_grid(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some((r, c)) = pick_at(&g, x, y, false) {
                    SetCapture(h);
                    let v = vec![r + 1; c + 1];
                    app(|a| a.grid_drag = Some(v.clone()));
                    show(v, None);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| a.grid_drag.clone()) {
                    if let Some((r, c)) = pick_at(&g, x, y, true) {
                        let v = drag_heights(d, r, c);
                        app(|a| a.grid_drag = Some(v.clone()));
                        show(v, None);
                    }
                } else if let Some((r, c)) = pick_at(&g, x, y, false) {
                    show(vec![r + 1; c + 1], None);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    show(PRESETS[i].to_vec(), Some(i));
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                if let Some(v) = app(|a| a.grid_drag.take()) {
                    _ = ReleaseCapture();
                    apply(v);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    apply(PRESETS[i].to_vec());
                }
            }
            WM_KEYDOWN => {
                // keyboard picks even grids: arrows move the corner, Enter applies
                let v = app(|a| a.grid_preview.clone());
                let (r, c) = (v[v.len() - 1] - 1, v.len() - 1);
                let (r, c) = match VIRTUAL_KEY(wp.0 as u16) {
                    VK_LEFT => (r, c.saturating_sub(1)),
                    VK_RIGHT => (r, (c + 1).min(3)),
                    VK_UP => (r.saturating_sub(1), c),
                    VK_DOWN => ((r + 1).min(3), c),
                    VK_RETURN => {
                        apply(v);
                        return LRESULT(0);
                    }
                    VK_ESCAPE => {
                        _ = DestroyWindow(h);
                        return LRESULT(0);
                    }
                    _ => (r, c),
                };
                show(vec![r + 1; c + 1], None);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.grid, a.grid_drag, a.grid_closed) = (HWND::default(), None, Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- load popup: saved sessions as cards with a to-scale layout preview ----------

const LOAD_SAVE_AS: usize = 1000;
const LOAD_FOLDER: usize = 1001;
const LOAD_MAX: usize = 9;

struct LoadGeo {
    w: i32,
    h: i32,
    cards: Vec<(RECT, RECT)>, // (card, thumbnail)
    save_as: RECT,
    folder: RECT,
}

fn load_geo(n: usize, dpi: i32) -> LoadGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, cw, th, ch) = (s(12), s(10), s(140), s(84), s(84 + 44));
    let rows = n.div_ceil(3).max(1) as i32;
    let w = 2 * pad + 3 * cw + 2 * gap;
    let cards = (0..n as i32)
        .map(|k| {
            let (x, y) = (pad + (k % 3) * (cw + gap), pad + (k / 3) * (ch + gap));
            let card = RECT { left: x, top: y, right: x + cw, bottom: y + ch };
            (card, RECT { left: x + s(6), top: y + s(6), right: x + cw - s(6), bottom: y + s(6) + th - s(12) })
        })
        .collect();
    let fy = pad + rows * (ch + gap);
    let link = |x0: i32, x1: i32| RECT { left: x0, top: fy, right: x1, bottom: fy + s(30) };
    LoadGeo { w, h: fy + s(30) + s(8), cards, save_as: link(pad, w / 2), folder: link(w / 2, w - pad) }
}

fn short_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr("This PC", "내 PC", "PC").into();
    }
    let t = path.trim_end_matches('\\');
    t.rsplit('\\').next().filter(|x| !x.starts_with("::")).unwrap_or(t).to_string()
}

// Title bar, taskbar and Alt+Tab follow the active pane's folder.
fn window_title(disp: &str) -> String {
    if disp.is_empty() { "Grids".into() } else { format!("{} - Grids", short_name(disp)) }
}

fn ago(t: std::time::SystemTime) -> String {
    let m = t.elapsed().map(|d| d.as_secs() / 60).unwrap_or(0);
    match m {
        0 => tr("just now", "방금", "たった今").into(),
        1..=59 => format!("{m}{}", tr(" min ago", "분 전", " 分前")),
        60..=1439 => format!("{}{}", m / 60, tr(" h ago", "시간 전", " 時間前")),
        _ => format!("{}{}", m / 1440, tr(" d ago", "일 전", " 日前")),
    }
}

// The layout drawn to scale, each pane labelled with its folder name.
unsafe fn draw_thumb(dc: HDC, r: RECT, s: &Session, font: HFONT) {
    fill(dc, r, pal().bg);
    let xs = cells(r.left + 2, r.right - r.left - 4, &s.colr, 2);
    let mut i = 0;
    for (c, &(x0, x1)) in xs.iter().enumerate() {
        for (y0, y1) in cells(r.top + 2, r.bottom - r.top - 4, &s.rowr[c], 2) {
            let pane = RECT { left: x0, top: y0, right: x1, bottom: y1 };
            fill(dc, pane, pal().card);
            frame(dc, pane, 1, pal().line);
            if x1 - x0 > 24 && y1 - y0 > 12 {
                let inner = RECT { left: x0 + 3, right: x1 - 3, ..pane };
                text(dc, inner, &short_name(&s.paths[i]), font, pal().sub, DT_CENTER);
            }
            i += 1;
        }
    }
}

unsafe fn open_load() {
    let (main, dpi, open, closed) = app(|a| (a.main, a.dpi, a.load, a.load_closed));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let entries = std::fs::read_dir(sessions_dir()).into_iter().flatten().flatten();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .map(|e| e.path())
        .filter(|p| is_session_file(p))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0)); // most recent first
    let cards: Vec<_> = files
        .into_iter()
        .take(LOAD_MAX)
        .filter_map(|(t, p)| Some((p.clone(), Session::parse(&std::fs::read_to_string(&p).ok()?), ago(t))))
        .collect();
    let g = load_geo(cards.len(), dpi);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LOAD) {
        _ = GetWindowRect(b, &mut rc);
    }
    app(|a| (a.cards, a.load_hot) = (cards, None));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsLoad"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.load = h);
}

unsafe fn paint_load(h: HWND) {
    let (cards, hot, cur, dpi, font, small, accent) = app(|a| {
        let cards: Vec<_> = a.cards.iter().map(|(p, s, t)| (p.clone(), s.clone(), t.clone())).collect();
        (cards, a.load_hot, a.s.file.clone(), a.dpi, a.font, a.small_font, a.s.accent_now())
    });
    let g = load_geo(cards.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if cards.is_empty() {
            let r = RECT { bottom: g.save_as.top, ..rc };
            text(dc, r, tr("No saved sessions", "저장된 세션이 없습니다", "保存されたセッションはありません"), font, pal().sub, DT_CENTER);
        }
        for (k, ((path, sess, when), (card, thumb))) in cards.iter().zip(&g.cards).enumerate() {
            if hot == Some(k) {
                fill(dc, *card, pal().pill);
            }
            draw_thumb(dc, *thumb, sess, small);
            if path.to_string_lossy() == cur.as_str() {
                frame(dc, *thumb, s(2), accent); // the session you're in
            }
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            let line = |top: i32, hgt: i32| RECT { left: card.left + s(6), right: card.right - s(6), top, bottom: top + hgt };
            text(dc, line(thumb.bottom + s(4), s(20)), &name, font, pal().text, DT_LEFT);
            let meta = format!("{}  ·  {}", layout_label(&sess.heights), when);
            text(dc, line(thumb.bottom + s(22), s(16)), &meta, small, pal().sub, DT_LEFT);
        }
        fill(dc, RECT { left: s(12), top: g.save_as.top - 1, right: rc.right - s(12), bottom: g.save_as.top }, pal().line);
        let link = |r: RECT, label: &str, k: usize, align| {
            text(dc, r, label, font, if hot == Some(k) { pal().text } else { accent }, align);
        };
        link(g.save_as, tr("+ Save as new", "+ 새 이름으로 저장", "+ 新しい名前で保存"), LOAD_SAVE_AS, DT_LEFT);
        link(g.folder, tr("Open sessions folder", "세션 폴더 열기", "セッション フォルダーを開く"), LOAD_FOLDER, DT_RIGHT);
    });
}

fn load_hit(g: &LoadGeo, x: i32, y: i32) -> Option<usize> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if inside(&g.save_as) {
        return Some(LOAD_SAVE_AS);
    }
    if inside(&g.folder) {
        return Some(LOAD_FOLDER);
    }
    g.cards.iter().position(|(c, _)| inside(c))
}

extern "system" fn load_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = app(|a| load_geo(a.cards.len(), a.dpi));
        match msg {
            WM_PAINT => paint_load(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = load_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.load_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let Some(k) = load_hit(&g, x, y) else { return LRESULT(0) };
                let file = app(|a| a.cards.get(k).map(|c| c.0.clone()));
                _ = DestroyWindow(h);
                match k {
                    LOAD_SAVE_AS => _ = save_session_as(),
                    LOAD_FOLDER => {
                        let d = sessions_dir();
                        _ = std::fs::create_dir_all(&d);
                        navigate(app(|a| a.s.active), &d.to_string_lossy());
                    }
                    _ => {
                        if let Some(f) = file {
                            load_session_file(&f);
                        }
                    }
                }
            }
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => _ = DestroyWindow(h),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.load, a.load_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- favourites: ☆+ in each pane header, the ★ list in the toolbar ----------

unsafe fn repaint_fav_buttons() {
    for h in app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()) {
        if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
            _ = InvalidateRect(Some(b), None, false);
        }
    }
}

unsafe fn set_favs(paths: &[String], add: bool) {
    if paths.is_empty() {
        return;
    }
    app(|a| {
        for path in paths {
            if path.is_empty() {
                continue;
            }
            let on = a.s.is_fav(path);
            if add && !on {
                a.s.favs.push(path.clone());
            } else if !add && on {
                a.s.favs.retain(|f| !fav_path(f).eq_ignore_ascii_case(path));
            }
        }
    });
    repaint_fav_buttons();
    // The Ctrl+B list reads the same favs. Resize it if it is already open.
    let (pop, dpi, n) = app(|a| {
        a.fav_hot = None;
        (a.fav_popup, a.dpi, a.s.favs.len())
    });
    if !pop.0.is_null() && unsafe { IsWindow(Some(pop)).as_bool() } {
        let g = fav_geo(n, dpi);
        unsafe {
            _ = SetWindowPos(pop, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            _ = InvalidateRect(Some(pop), None, false);
        }
    }
    settings::refresh();
}

unsafe fn toggle_fav(i: usize) {
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    set_favs(std::slice::from_ref(&path), !app(|a| a.s.is_fav(&path)));
}

/// Every path already a favourite → the menu removes them; otherwise it adds the rest.
fn fav_removes(paths: &[&str], is_fav: impl Fn(&str) -> bool) -> bool {
    !paths.is_empty() && paths.iter().all(|p| is_fav(p))
}

fn fav_menu_label(paths: &[String]) -> &'static str {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    if fav_removes(&refs, |p| app(|a| a.s.is_fav(p))) {
        tr("Remove from favourites", "즐겨찾기에서 빼기", "お気に入りから削除")
    } else {
        tr("Add to favourites", "즐겨찾기에 추가", "お気に入りに追加")
    }
}

/// Menu index the favourites command sits above: the properties item, or the last item.
fn fav_anchor(properties: Option<u32>, count: i32) -> u32 {
    if count <= 0 {
        0
    } else {
        properties.filter(|&i| (i as i32) < count).unwrap_or(count as u32 - 1)
    }
}

unsafe fn cmd_verb(cm: &IContextMenu, offset: usize) -> String {
    let mut buf = [0u16; 80];
    if cm.GetCommandString(offset, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_err() {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Index of the "properties" verb. Shell command ids are `id_first + offset`; separators and submenus have no id.
unsafe fn fav_anchor_pos(menu: HMENU, cm: &IContextMenu, id_first: u32, id_last: u32, start: u32) -> u32 {
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return 0;
    }
    let mut properties = None;
    for i in start.min(n as u32)..n as u32 {
        if menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == u32::MAX || id < id_first || id >= id_last {
            continue;
        }
        if cmd_verb(cm, (id - id_first) as usize).trim().eq_ignore_ascii_case("properties") {
            properties = Some(i);
            break;
        }
    }
    fav_anchor(properties, n)
}

/// Separators often have a zero id, so GetMenuItemID cannot tell them from a command. MFT_SEPARATOR can.
unsafe fn menu_sep(menu: HMENU, pos: u32) -> bool {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE,
        ..Default::default()
    };
    GetMenuItemInfoW(menu, pos, true, &mut info).is_ok() && info.fType.0 & MFT_SEPARATOR.0 != 0
}

unsafe fn menu_index(menu: HMENU, id: usize) -> Option<u32> {
    let n = GetMenuItemCount(Some(menu)).max(0);
    (0..n).find(|&i| GetMenuItemID(menu, i) as usize == id).map(|i| i as u32)
}

/// Exactly one separator above the favourites command, and none between it and Properties.
/// Run after the shell menu is filled (and again on WM_INITMENUPOPUP, once merged items exist).
unsafe fn tidy_fav_item(menu: HMENU, id: usize) {
    if id == usize::MAX {
        return;
    }
    loop {
        let Some(fav) = menu_index(menu, id) else { return };
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        if fav + 1 >= n || !menu_sep(menu, fav + 1) {
            break;
        }
        if DeleteMenu(menu, fav + 1, MF_BYPOSITION).is_err() {
            break;
        }
    }
    loop {
        let Some(fav) = menu_index(menu, id) else { return };
        if fav < 2 || !menu_sep(menu, fav - 1) || !menu_sep(menu, fav - 2) {
            break;
        }
        if DeleteMenu(menu, fav - 1, MF_BYPOSITION).is_err() {
            break;
        }
    }
    let Some(fav) = menu_index(menu, id) else { return };
    if fav > 0 && !menu_sep(menu, fav - 1) {
        _ = InsertMenuW(menu, fav, MF_SEPARATOR | MF_BYPOSITION, 0, PCWSTR::null());
    }
}

/// Favourites command at `at` (directly above Properties, or the last item). The separator is decided afterwards.
unsafe fn insert_fav(menu: HMENU, at: u32, id: usize, label: &str) {
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let label = HSTRING::from(label);
    if n == 0 {
        _ = AppendMenuW(menu, MF_STRING, id, &label);
        return;
    }
    _ = InsertMenuW(menu, at.min(n - 1), MF_STRING | MF_BYPOSITION, id, &label);
    tidy_fav_item(menu, id);
}

fn wnd_class(h: HWND) -> String {
    let mut c = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut c) } as usize;
    String::from_utf16_lossy(&c[..n])
}

// The real Explorer menu is built by the shell. These are the windows that show it; a header
// or an in-place rename box is left alone.
fn ctx_surface(h: HWND) -> bool {
    matches!(wnd_class(h).as_str(), "SHELLDLL_DefView" | "DirectUIHWND" | "SysListView32")
}

const CTX_SUB: usize = 4;
const ID_CTX_FAV: u32 = 0x8000; // outside the id range handed to IContextMenu::QueryContextMenu

struct MenuBridge {
    cm3: Option<IContextMenu3>,
    cm2: Option<IContextMenu2>,
}

thread_local! {
    static MENU_BRIDGE: RefCell<Option<MenuBridge>> = const { RefCell::new(None) };
    static CTX_MENU: Cell<isize> = const { Cell::new(0) }; // HMENU we inserted into; submenus are left alone
}

static CTX_OPEN: AtomicBool = AtomicBool::new(false);

struct ClearBridge;
impl Drop for ClearBridge {
    fn drop(&mut self) {
        MENU_BRIDGE.with(|b| *b.borrow_mut() = None);
        CTX_MENU.with(|m| m.set(0));
        CTX_OPEN.store(false, Ordering::Release);
    }
}

unsafe fn hook_ctx_hwnd(h: HWND, i: usize) {
    if !h.0.is_null() && ctx_surface(h) {
        _ = SetWindowSubclass(h, Some(ctx_proc), CTX_SUB, i);
    }
}

unsafe fn hook_ctx(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let Ok(view) = sv.GetWindow() else { return };
    hook_ctx_hwnd(view, i);
    let mut after = None;
    loop {
        let Ok(c) = FindWindowExW(Some(view), after, PCWSTR::null(), PCWSTR::null()) else { break };
        if c.0.is_null() {
            break;
        }
        hook_ctx_hwnd(c, i);
        let mut inner = None;
        loop {
            let Ok(g) = FindWindowExW(Some(c), inner, PCWSTR::null(), PCWSTR::null()) else { break };
            if g.0.is_null() {
                break;
            }
            hook_ctx_hwnd(g, i);
            inner = Some(g);
        }
        after = Some(c);
    }
}

#[repr(C)]
struct LvHit {
    pt: POINT,
    flags: u32,
    item: i32,
    sub: i32,
    group: i32,
}

// Right-click on a row the view hasn't selected yet: select it, unless it is already part of
// a multi-selection (Explorer keeps that selection).
unsafe fn select_clicked(i: usize, screen: POINT) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let lv = view_list(&fv);
    if lv.0.is_null() {
        return;
    }
    let mut pt = screen;
    if !ScreenToClient(lv, &mut pt).as_bool() {
        return;
    }
    let mut hit = LvHit { pt, flags: 0, item: -1, sub: 0, group: 0 };
    let idx = SendMessageW(lv, 0x1012, None, Some(LPARAM(&mut hit as *mut _ as isize))).0 as i32; // LVM_HITTEST
    if idx < 0 {
        return;
    }
    let Ok(pidl) = fv.Item(idx) else { return };
    let selected = fv.GetSelectionState(pidl).unwrap_or(0) & SVSI_SELECT.0 as u32 != 0;
    CoTaskMemFree(Some(pidl as _));
    if !selected {
        _ = fv.SelectItem(idx, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0) as u32);
    }
}

unsafe fn selection_pidls(i: usize) -> Vec<*mut ITEMIDLIST> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(en) = fv.Items::<IEnumIDList>(SVGIO_SELECTION) else { return Vec::new() };
    let mut out = Vec::new();
    loop {
        let mut slot = [std::ptr::null_mut()];
        let mut n = 0u32;
        let hr = en.Next(&mut slot, Some(&mut n));
        if !hr.is_ok() || n == 0 || slot[0].is_null() || out.len() == 2000 {
            break;
        }
        out.push(slot[0]);
    }
    out
}

unsafe fn selected_folders(i: usize) -> Vec<String> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(arr) = fv.GetSelection(false) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(500);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        if it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0) {
            if let Ok(p) = it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
                let p = pwstr(p);
                if !p.is_empty() {
                    out.push(p);
                }
            }
        }
    }
    out
}

unsafe fn forward_menu(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    MENU_BRIDGE.with(|b| {
        let b = b.borrow();
        let b = b.as_ref()?;
        unsafe {
            if let Some(cm3) = &b.cm3 {
                let mut lr = LRESULT(0);
                if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut lr)).is_ok() {
                    return Some(lr);
                }
            }
            if let Some(cm2) = &b.cm2 {
                if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                    return Some(LRESULT(0));
                }
            }
        }
        None
    })
}

// Shell menu for the pane. Add/Remove favourites sits directly above Properties when the selection has a folder.
// Returns false when the shell menu could not be built, so the view shows its own.
unsafe fn show_ctx(i: usize, pt: POINT) -> bool {
    if CTX_OPEN.swap(true, Ordering::AcqRel) {
        return true;
    }
    let _open = ClearBridge;
    let Some(sv) = view_of::<IShellView>(i) else { return false };
    let owner = sv.GetWindow().unwrap_or_else(|_| app(|a| a.main));
    if owner.0.is_null() {
        return false;
    }
    select_clicked(i, pt);
    let folders = selected_folders(i);
    let pidls = selection_pidls(i);
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let cm = if refs.is_empty() {
        sv.GetItemObject::<IContextMenu>(SVGIO_BACKGROUND).ok()
    } else if let Some(folder) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellFolder>().ok()) {
        folder.GetUIObjectOf::<IContextMenu>(owner, &refs, None).ok().or_else(|| sv.GetItemObject::<IContextMenu>(SVGIO_SELECTION).ok())
    } else {
        sv.GetItemObject::<IContextMenu>(SVGIO_SELECTION).ok()
    };
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    let Some(cm) = cm else { return false };
    let Ok(menu) = CreatePopupMenu() else { return false };
    let mut flags = CMF_EXPLORE | CMF_CANRENAME;
    if !refs.is_empty() {
        flags |= CMF_ITEMMENU;
    }
    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
        flags |= CMF_EXTENDEDVERBS;
    }
    let hr = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags);
    if !hr.is_ok() {
        _ = DestroyMenu(menu);
        return false;
    }
    if !folders.is_empty() {
        let at = fav_anchor_pos(menu, &cm, 1, ID_CTX_FAV, 0);
        insert_fav(menu, at, ID_CTX_FAV as usize, fav_menu_label(&folders));
    }
    MENU_BRIDGE.with(|b| {
        *b.borrow_mut() = Some(MenuBridge { cm3: cm.cast().ok(), cm2: cm.cast().ok() });
    });
    CTX_MENU.with(|m| m.set(menu.0 as isize));
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, owner, None).0;
    MENU_BRIDGE.with(|b| *b.borrow_mut() = None);
    if cmd == ID_CTX_FAV as i32 {
        let remove = app(|a| folders.iter().all(|p| a.s.is_fav(p)));
        set_favs(&folders, !remove);
    } else if cmd > 0 {
        let mut ici = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: owner,
            lpVerb: PCSTR((cmd as u32 - 1) as usize as *const u8),
            nShow: SW_SHOWNORMAL.0 as i32,
            ..Default::default()
        };
        if GetKeyState(VK_SHIFT.0 as i32) < 0 {
            ici.fMask |= CMIC_MASK_SHIFT_DOWN;
        }
        if GetKeyState(VK_CONTROL.0 as i32) < 0 {
            ici.fMask |= CMIC_MASK_CONTROL_DOWN;
        }
        _ = cm.InvokeCommand(&ici);
    }
    _ = DestroyMenu(menu);
    true
}

unsafe extern "system" fn ctx_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, data: usize) -> LRESULT {
    if msg == WM_INITMENUPOPUP {
        // The shell may insert its own separator while the menu opens. Collapse after that.
        let lr = forward_menu(msg, wp, lp);
        let popup = HMENU(wp.0 as _);
        if CTX_MENU.with(|m| m.get() != 0 && m.get() == popup.0 as isize) {
            tidy_fav_item(popup, ID_CTX_FAV as usize);
        }
        if let Some(lr) = lr {
            return lr;
        }
    } else if matches!(msg, WM_UNINITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        if let Some(lr) = forward_menu(msg, wp, lp) {
            return lr;
        }
    }
    if msg == WM_PARENTNOTIFY && (wp.0 & 0xFFFF) == WM_CREATE as usize {
        hook_ctx_hwnd(HWND(lp.0 as _), data);
    }
    if msg == WM_CONTEXTMENU {
        let pt = if lp.0 == -1 {
            let mut rc = RECT::default();
            let _ = GetWindowRect(h, &mut rc);
            POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 }
        } else {
            POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 }
        };
        let under = if lp.0 == -1 { h } else { WindowFromPoint(pt) };
        if ctx_surface(under) && show_ctx(data, pt) {
            return LRESULT(0);
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// Sidebar menu: the tree builds the shell menu, then asks us for a replacement. Ours forwards
// every shell command and puts the favourites item directly above Properties.
#[implement(IContextMenu, IContextMenu2, IContextMenu3)]
struct FavCtx {
    inner: IContextMenu,
    path: String,
    cmd: Cell<u32>,
    item: Cell<usize>, // menu id of the favourites command
    popup: Cell<isize>, // HMENU that command was inserted into
}

impl IContextMenu_Impl for FavCtx_Impl {
    fn QueryContextMenu(&self, hmenu: HMENU, indexmenu: u32, idcmdfirst: u32, idcmdlast: u32, uflags: u32) -> HRESULT {
        unsafe {
            let hr = self.inner.QueryContextMenu(hmenu, indexmenu, idcmdfirst, idcmdlast, uflags);
            if !hr.is_ok() || uflags & CMF_DEFAULTONLY != 0 {
                return hr;
            }
            let used = (hr.0 as u32) & 0xFFFF;
            // Offset 0 looks like a null verb, so an empty shell menu gets the next slot.
            let offset = if used == 0 { 1 } else { used };
            if idcmdfirst.saturating_add(offset) >= idcmdlast {
                return hr;
            }
            self.cmd.set(offset);
            let item = (idcmdfirst + offset) as usize;
            self.item.set(item);
            self.popup.set(hmenu.0 as isize);
            let at = fav_anchor_pos(hmenu, &self.inner, idcmdfirst, idcmdlast.saturating_add(1), indexmenu);
            insert_fav(hmenu, at, item, fav_menu_label(std::slice::from_ref(&self.path)));
            HRESULT((offset + 1) as i32)
        }
    }
    fn InvokeCommand(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        unsafe {
            if !pici.is_null() && verb_offset(pici) == Some(self.cmd.get()) {
                let remove = app(|a| a.s.is_fav(&self.path));
                set_favs(std::slice::from_ref(&self.path), !remove);
                return Ok(());
            }
            self.inner.InvokeCommand(pici)
        }
    }
    fn GetCommandString(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        if idcmd == self.cmd.get() as usize {
            return Ok(());
        }
        unsafe { self.inner.GetCommandString(idcmd, utype, Some(preserved), pszname, cchmax) }
    }
}

impl IContextMenu2_Impl for FavCtx_Impl {
    fn HandleMenuMsg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<()> {
        unsafe {
            let result = if let Ok(m) = self.inner.cast::<IContextMenu2>() { m.HandleMenuMsg(umsg, wparam, lparam) } else { Ok(()) };
            if umsg == WM_INITMENUPOPUP {
                tidy_fav_popup(self.popup.get(), self.item.get(), HMENU(wparam.0 as _));
            }
            result
        }
    }
}

impl IContextMenu3_Impl for FavCtx_Impl {
    fn HandleMenuMsg2(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        unsafe {
            let result = if let Ok(m) = self.inner.cast::<IContextMenu3>() {
                m.HandleMenuMsg2(umsg, wparam, lparam, Some(plresult))
            } else if let Ok(m) = self.inner.cast::<IContextMenu2>() {
                m.HandleMenuMsg(umsg, wparam, lparam)
            } else {
                Ok(())
            };
            if umsg == WM_INITMENUPOPUP {
                tidy_fav_popup(self.popup.get(), self.item.get(), HMENU(wparam.0 as _));
            }
            result
        }
    }
}

unsafe fn tidy_fav_popup(popup: isize, item: usize, menu: HMENU) {
    if popup != 0 && popup == menu.0 as isize {
        tidy_fav_item(menu, item);
    }
}

fn verb_offset(ici: *const CMINVOKECOMMANDINFO) -> Option<u32> {
    unsafe {
        let v = (*ici).lpVerb.0 as usize;
        if v > 0 && v < 0x10000 {
            return Some(v as u32);
        }
        if (*ici).cbSize >= size_of::<CMINVOKECOMMANDINFOEX>() as u32 {
            let w = (*(ici as *const CMINVOKECOMMANDINFOEX)).lpVerbW.0 as usize;
            if w > 0 && w < 0x10000 {
                return Some(w as u32);
            }
        }
        None
    }
}

fn shell_folder(it: &IShellItem) -> bool {
    unsafe { it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0) }
}

fn shell_parse(it: &IShellItem) -> String {
    unsafe { it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).map(|p| pwstr(p)).unwrap_or_default() }
}

struct FavGeo {
    w: i32,
    h: i32,
    rows: Vec<(RECT, RECT)>, // (row, remove ✕)
}

fn fav_geo(n: usize, dpi: i32) -> FavGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, rh, w) = (s(6), s(40), s(380));
    let rows = (0..n as i32)
        .map(|k| {
            let row = RECT { left: pad, top: pad + k * rh, right: w - pad, bottom: pad + (k + 1) * rh };
            (row, RECT { left: row.right - s(32), ..row })
        })
        .collect();
    FavGeo { w, h: 2 * pad + n.max(1) as i32 * rh, rows }
}

unsafe fn open_favs() {
    let (main, dpi, open, closed, n) = app(|a| (a.main, a.dpi, a.fav_popup, a.fav_closed, a.s.favs.len()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let g = fav_geo(n, dpi);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_FAV) {
        _ = GetWindowRect(b, &mut rc);
    }
    let prev = GetFocus();
    app(|a| (a.fav_hot, a.fav_prev) = (None, prev));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsFav"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.fav_popup = h);
}

unsafe fn paint_favs(h: HWND) {
    let (favs, hot, dpi, font, small, icon_font) = app(|a| (a.s.favs.clone(), a.fav_hot, a.dpi, a.font, a.small_font, a.icon_font));
    let g = fav_geo(favs.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if favs.is_empty() {
            text(dc, rc, tr("No favourites yet · add one with ☆+ in a pane header", "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가", "お気に入りはまだありません · ペインヘッダーの ☆+ で追加"), font, pal().sub, DT_CENTER);
        }
        for (k, (f, (row, remove))) in favs.iter().zip(&g.rows).enumerate() {
            if hot.is_some_and(|(i, _)| i == k) {
                fill(dc, *row, pal().pill);
                let x_on = hot == Some((k, true));
                text(dc, *remove, "\u{E711}", icon_font, if x_on { pal().text } else { pal().sub }, DT_CENTER);
            }
            let icon = RECT { left: row.left + s(6), right: row.left + s(28), ..*row };
            text(dc, icon, "\u{E8B7}", icon_font, pal().sub, DT_CENTER);
            let line = |top: i32, hgt: i32| RECT { left: icon.right + s(6), right: remove.left - s(4), top, bottom: top + hgt };
            text(dc, line(row.top + s(3), s(19)), &fav_label(f), font, pal().text, DT_LEFT);
            text(dc, line(row.top + s(21), s(16)), fav_path(f), small, pal().sub, DT_LEFT | DT_PATH_ELLIPSIS);
        }
    });
}

fn fav_hit(g: &FavGeo, x: i32, y: i32) -> Option<(usize, bool)> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    g.rows.iter().position(|(row, _)| inside(row)).map(|k| (k, inside(&g.rows[k].1)))
}

extern "system" fn fav_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = app(|a| fav_geo(a.s.favs.len(), a.dpi));
        match msg {
            WM_PAINT => paint_favs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = fav_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.fav_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP | WM_KEYDOWN => {
                // ✕ / Del: drop it and shrink the list in place
                let remove = |k: usize| {
                    app(|a| {
                        if k < a.s.favs.len() {
                            a.s.favs.remove(k);
                        }
                        a.fav_hot = None;
                    });
                    let g = app(|a| fav_geo(a.s.favs.len(), a.dpi));
                    _ = SetWindowPos(h, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                    _ = InvalidateRect(Some(h), None, false);
                    repaint_fav_buttons();
                    settings::refresh();
                };
                let open = |k: usize| {
                    let (i, path) = app(|a| (a.s.active, fav_path(&a.s.favs[k]).to_string()));
                    _ = DestroyWindow(h);
                    if !navigate(i, &path) {
                        _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
                    }
                    focus_view(i);
                };
                if msg == WM_LBUTTONUP {
                    let (x, y) = xy(lp);
                    match fav_hit(&g, x, y) {
                        Some((k, true)) => remove(k),
                        Some((k, false)) => open(k),
                        None => {}
                    }
                    return LRESULT(0);
                }
                let (n, cur) = app(|a| (a.s.favs.len(), a.fav_hot.map(|(k, _)| k)));
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => {
                        let (main, prev, active) = app(|a| (a.main, a.fav_prev, a.s.active));
                        _ = DestroyWindow(h);
                        let fav_btn = GetDlgItem(Some(main), ID_TB_FAV).unwrap_or_default();
                        if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && prev != fav_btn {
                            _ = SetFocus(Some(prev));
                        } else {
                            focus_view(active);
                        }
                    }
                    _ if n == 0 => {}
                    k @ (VK_DOWN | VK_UP) => {
                        let next = if k == VK_DOWN { cur.map_or(0, |k| (k + 1) % n) } else { cur.map_or(n - 1, |k| (k + n - 1) % n) };
                        app(|a| a.fav_hot = Some((next, false)));
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    VK_RETURN => if let Some(k) = cur { open(k) },
                    VK_DELETE => if let Some(k) = cur {
                        remove(k);
                        app(|a| a.fav_hot = if n > 1 { Some((k.min(n - 2), false)) } else { None });
                    },
                    _ => {}
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.fav_popup, a.fav_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// Ctrl+Shift+N: a new folder in the active pane, then the shell's own rename (F2) once it shows up.
unsafe fn new_folder(i: usize) {
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = tr("New folder", "새 폴더", "新しいフォルダー");
    let name = fresh_folder(base, |n| Path::new(&dir).join(n).exists());
    if std::fs::create_dir(Path::new(&dir).join(&name)).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let full = Path::new(&dir).join(&name);
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_MKDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_NEW.lock() {
        *p = Some(PendingNew { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWF, 40, None);
    }
    focus_view(i);
}

// The view lists the new folder a moment after Refresh. Select it and start the shell rename.
unsafe fn select_new_folder(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // SVSI_EDIT is the shell's rename. If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

unsafe fn popup_menu(menu: HMENU, id: i32) {
    let main = app(|a| a.main);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), id) {
        _ = GetWindowRect(b, &mut rc);
    }
    _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom + 4, None, main, None);
}

// ---------- background paste ----------

// Copies running on worker threads; the status line says so and skips its disk calls meanwhile.
static COPYING: AtomicUsize = AtomicUsize::new(0);

struct Busy;

impl Busy {
    fn new() -> Busy {
        COPYING.fetch_add(1, Ordering::Relaxed);
        Busy
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        COPYING.fetch_sub(1, Ordering::Relaxed);
    }
}

thread_local! {
    // joined after the message loop so closing Grids doesn't cut a copy short
    static JOBS: RefCell<Vec<std::thread::JoinHandle<()>>> = const { RefCell::new(Vec::new()) };
}

// Ctrl+V in a file list: a held key's repeats are swallowed; the first press starts a copy, or goes
// to the shell when `start` can't take it. True = handled, don't pass it on.
fn paste_key(repeat: bool, start: impl FnOnce() -> bool) -> bool {
    repeat || start()
}

// Some(true): every source sits in `dst` (copy with "- Copy" names), Some(false): none does,
// None: empty, not a file system path or mixed (left to the shell).
fn paste_mode(srcs: &[String], dst: &str) -> Option<bool> {
    let dst = dst.trim_end_matches('\\').to_lowercase();
    let parent = |p: &String| Some(p.trim_end_matches('\\').rsplit_once('\\')?.0.trim_end_matches('\\').to_lowercase());
    let same: Vec<bool> = srcs.iter().map(|p| if is_fs(p) { parent(p).map(|d| d == dst) } else { None }).collect::<Option<_>>()?;
    match (same.iter().all(|&s| s), same.iter().any(|&s| s)) {
        _ if same.is_empty() => None,
        (true, _) => Some(true),
        (false, false) => Some(false),
        _ => None,
    }
}

// The ⇄ menu's copy / move: like a paste, but a move into the folder the files already sit in is refused.
fn send_mode(srcs: &[String], dst: &str, mv: bool) -> Option<bool> {
    paste_mode(srcs, dst).filter(|&same| !(mv && same))
}

// The ⇄ menu sends a whole selection or nothing: never a partial list from a capped or unreadable read.
const SEND_MAX: u32 = 10_000;

#[derive(Debug, PartialEq)]
enum Pick {
    Empty,
    TooMany,
    Unreadable,
}

fn collect_all(n: u32, mut get: impl FnMut(u32) -> Option<String>) -> std::result::Result<Vec<String>, Pick> {
    match n {
        0 => Err(Pick::Empty),
        n if n > SEND_MAX => Err(Pick::TooMany),
        n => (0..n).map(|k| get(k).ok_or(Pick::Unreadable)).collect(),
    }
}

// Ctrl+Shift+C in a file list: the selected items' paths (or the folder's), quoted, one per line.
// Virtual items without a file path are skipped. The clipboard is only touched once the text is ready.
unsafe fn copy_paths(a: usize) {
    let Some(fv) = view_of::<IFolderView2>(a) else { return };
    let path = |it: IShellItem| it.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p));
    let mut paths: Vec<String> = Vec::new();
    if let Ok(arr) = fv.GetSelection(false) {
        paths = (0..arr.GetCount().unwrap_or(0)).filter_map(|k| path(arr.GetItemAt(k).ok()?)).collect();
    }
    if fv.ItemCount(SVGIO_SELECTION).unwrap_or(0) == 0 {
        paths = fv.GetFolder::<IShellItem>().ok().and_then(path).into_iter().collect();
    }
    if paths.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let text: Vec<u16> = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\r\n").encode_utf16().chain([0]).collect();
    let main = app(|x| x.main);
    let mut ok = false;
    if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2) {
        let p = GlobalLock(h) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            _ = GlobalUnlock(h);
            if OpenClipboard(Some(main)).is_ok() {
                ok = EmptyClipboard().is_ok() && SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))).is_ok();
                _ = CloseClipboard();
            }
        }
        if !ok {
            _ = GlobalFree(Some(h)); // after a successful SetClipboardData the system owns it
        }
    }
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let n = paths.len();
    let note = match (ui(), n) {
        (1, 1) => "경로 복사함".into(),
        (1, n) => format!("경로 {n}개 복사함"),
        (2, 1) => "パスをコピーしました".into(),
        (2, n) => format!("パスを {n} 個コピーしました"),
        (_, 1) => "Copied path".into(),
        (_, n) => format!("Copied {n} paths"),
    };
    app(|x| x.note = Some((note, Instant::now())));
    update_status();
}

// Plain file copies (Ctrl+C, then Ctrl+V into a folder) run on their own thread, so Grids stays usable.
// Cut, links, virtual items and anything else stay with the shell.
unsafe fn try_paste(a: usize) -> bool {
    let (dst, main) = app(|x| (x.s.paths[a].clone(), x.main));
    if !is_fs(&dst) || OpenClipboard(Some(main)).is_err() {
        return false;
    }
    let mut srcs = Vec::new();
    if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
        let drop = HDROP(h.0);
        for k in 0..DragQueryFileW(drop, u32::MAX, None) {
            let mut buf = vec![0u16; DragQueryFileW(drop, k, None) as usize + 1];
            let n = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
            srcs.push(String::from_utf16_lossy(&buf[..n]));
        }
    }
    let mut effect = None;
    if let Ok(h) = GetClipboardData(RegisterClipboardFormatW(w!("Preferred DropEffect"))) {
        let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
        if !p.is_null() {
            effect = Some(*p);
            _ = GlobalUnlock(HGLOBAL(h.0));
        }
    }
    _ = CloseClipboard();
    let copy = effect.is_none_or(|e| e & DROPEFFECT_MOVE.0 == 0 && e & DROPEFFECT_COPY.0 != 0);
    let Some(rename) = paste_mode(&srcs, &dst).filter(|_| copy) else { return false };
    start_copy(srcs, dst, rename, false)
}

// Copies (or, with `mv`, moves) `srcs` into `dst` on a worker thread. False = the thread didn't start.
unsafe fn start_copy(srcs: Vec<String>, dst: String, rename: bool, mv: bool) -> bool {
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Err(e) = run_copy(&srcs, &dst, rename, mv) {
            if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                let what = if mv { tr("Moving failed: ", "이동하지 못했습니다: ", "移動できませんでした: ") } else { tr("Copying failed: ", "복사하지 못했습니다: ", "コピーできませんでした: ") };
                let text = HSTRING::from(format!("{what}{}", e.message()));
                MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    JOBS.with_borrow_mut(|j| {
        j.retain(|j| !j.is_finished());
        j.push(job);
    });
    true
}

// No owner window: the progress and conflict dialogs belong to the worker thread.
unsafe fn run_copy(srcs: &[String], dst: &str, rename: bool, mv: bool) -> Result<()> {
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD;
    op.SetOperationFlags(if rename { flags | FOF_RENAMEONCOLLISION } else { flags })?;
    let to: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dst), None)?;
    for s in srcs {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(s), None)?;
        if mv {
            op.MoveItem(&item, &to, PCWSTR::null(), None)?;
        } else {
            op.CopyItem(&item, &to, PCWSTR::null(), None)?;
        }
    }
    op.PerformOperations()
}

unsafe fn delete_selection(i: usize, shift: bool) {
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let Ok(paths) = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) }) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if paths.is_empty() {
        return;
    }
    let (ask, permanent) = app(|a| (a.s.confirm, shift || a.s.del == 1));
    if ask {
        let n = paths.len();
        let verb = if permanent {
            tr("Permanently delete", "완전히 삭제", "完全に削除")
        } else {
            tr("Move to the Recycle Bin", "휴지통으로 이동", "ごみ箱へ移動")
        };
        let text = HSTRING::from(format!("{verb} ({n})"));
        let owner = app(|a| a.main);
        if MessageBoxW(Some(owner), &text, w!("Grids"), MB_YESNO | MB_ICONQUESTION) != IDYES {
            return;
        }
    }
    start_delete(paths, permanent);
}

unsafe fn start_delete(srcs: Vec<String>, permanent: bool) -> bool {
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Err(e) = run_delete(&srcs, permanent) {
            if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                let text = HSTRING::from(format!("{}{}", tr("Delete failed: ", "삭제하지 못했습니다: ", "削除できませんでした: "), e.message()));
                _ = MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    JOBS.with_borrow_mut(|j| {
        j.retain(|j| !j.is_finished());
        j.push(job);
    });
    true
}

unsafe fn run_delete(srcs: &[String], permanent: bool) -> Result<()> {
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    let mut flags = FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR;
    if !permanent {
        flags |= FOF_ALLOWUNDO | FOFX_ADDUNDORECORD;
    }
    op.SetOperationFlags(flags)?;
    for s in srcs {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(s), None)?;
        op.DeleteItem(&item, None)?;
    }
    op.PerformOperations()
}

fn default_new_name() -> String {
    tr("New Text Document.txt", "새 텍스트 문서.txt", "新しいテキスト ドキュメント.txt").into()
}

unsafe fn create_text_file() {
    let i = active_pane();
    let dir = app(|a| a.s.paths[i].clone());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = app(|a| if a.s.new_name.trim().is_empty() { default_new_name() } else { a.s.new_name.clone() });
    let dirp = PathBuf::from(&dir);
    let mut path = dirp.join(&base);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut n = 2u32;
    while path.exists() && n < 1000 {
        let name = if ext.is_empty() { format!("{stem} ({n})") } else { format!("{stem} ({n}).{ext}") };
        path = dirp.join(name);
        n += 1;
    }
    if std::fs::write(&path, b"").is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
}

unsafe fn pick_folder(owner: HWND) -> Option<String> {
    let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
    let opts = dlg.GetOptions().unwrap_or_default();
    dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST).ok()?;
    dlg.Show(Some(owner)).ok()?;
    let name = dlg.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    Some(pwstr(name))
}

unsafe fn pick_open(owner: HWND) -> Option<String> {
    file_dialog(owner, false)
}

unsafe fn file_dialog(owner: HWND, save: bool) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let mut buf = [0u16; 520];
    if save {
        for (d, c) in buf.iter_mut().zip("grids.ini".encode_utf16()) {
            *d = c;
        }
    }
    let filter: Vec<u16> = "Settings\0*.ini\0All\0*.*\0\0".encode_utf16().chain(std::iter::once(0)).collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        Flags: if save { OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST } else { OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST },
        ..Default::default()
    };
    let ok = if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) };
    ok.as_bool().then(|| {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    })
}

fn settings_filter() -> Vec<u16> {
    let mut s = String::new();
    for (label, spec) in [(tr("JSON", "JSON", "JSON"), "*.json"), (tr("Settings", "설정", "設定"), "*.json;*.ini"), (tr("All", "모두", "すべて"), "*.*")] {
        s.push_str(label);
        s.push('\0');
        s.push_str(spec);
        s.push('\0');
    }
    s.push('\0');
    s.encode_utf16().collect()
}

/// Export / import picker. Save defaults to grids.json. Open accepts JSON and a legacy ini.
unsafe fn pick_settings(owner: HWND, save: bool) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let mut buf = [0u16; 520];
    if save {
        for (d, c) in buf.iter_mut().zip("grids.json".encode_utf16()) {
            *d = c;
        }
    }
    let filter = settings_filter();
    let def = w!("json");
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        Flags: if save { OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST } else { OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST },
        lpstrDefExt: if save { def } else { PCWSTR::null() },
        ..Default::default()
    };
    let ok = if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) };
    ok.as_bool().then(|| {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    })
}

unsafe fn reset_prefs() {
    let fresh = Session::default();
    app(|a| {
        a.s.theme = fresh.theme;
        a.s.font = fresh.font;
        a.s.bar = fresh.bar;
        a.s.stat = fresh.stat;
        a.s.view_new = fresh.view_new;
        a.s.start_new = fresh.start_new;
        a.s.start_path.clear();
        a.s.ext = fresh.ext;
        a.s.tree_on = fresh.tree_on;
        a.s.edge = fresh.edge;
        a.s.confirm = fresh.confirm;
        a.s.del = fresh.del;
        a.s.new_name.clear();
        a.s.dropm = fresh.dropm;
        a.s.boot = fresh.boot;
        a.s.boot_file.clear();
        a.s.autosave = fresh.autosave;
        a.s.dot = fresh.dot;
        a.s.single = fresh.single;
        a.s.dblup = fresh.dblup;
        a.s.accent = fresh.accent;
        a.s.accent_dark = fresh.accent_dark;
    });
    set_language(Lang::Auto);
    apply_ext(true);
    relayout();
    save_session();
    settings::refresh();
}

/// JSON replaces the settings. A file that is not JSON is read as a legacy ini.
unsafe fn import_settings(text: &str) -> bool {
    let Ok(mut s) = store::import_text(text) else { return false };
    let (win, lang_now) = app(|a| (a.s.win, a.s.lang));
    s.win = win;
    s.dark = resolve_dark(s.theme);
    let lang = s.lang;
    install_grids(s);
    if lang != lang_now {
        set_language(lang);
    } else {
        apply_theme();
        apply_font();
    }
    apply_ext(app(|a| a.s.ext));
    relayout();
    save_session();
    settings::refresh();
    true
}

fn apply_boot(s: &mut Session) {
    match s.boot {
        2 => {
            s.set_heights(vec![2, 2]);
            s.paths = vec![String::new(); MAXP];
            s.views = vec![(0, 0); MAXP];
            s.cols = vec![String::new(); MAXP];
            s.active = 0;
            s.file.clear();
            s.tabs.clear();
            s.tab = 0;
            s.anchor();
        }
        1 if !s.boot_file.is_empty() => {
            if let Ok(t) = std::fs::read_to_string(&s.boot_file) {
                let file = s.boot_file.clone();
                s.take_layout(&Session::parse(&t));
                s.file = file;
            }
        }
        // 0, the default: every tab loaded from grids.json stays. Only the active one is shown.
        _ => {
            if s.tabs.is_empty() {
                s.anchor();
            }
            s.publish();
        }
    }
}

// ---------- keyboard ----------

// "New Text Document.txt", then "New Text Document (2).txt", … The number sits before the extension.
fn fresh_file(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (base, String::new()),
    };
    let mut n = 2u32;
    loop {
        let name = format!("{stem} ({n}){ext}");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

// File created by Ctrl+Alt+N, waiting for the view to list it so it can be selected and renamed.
struct PendingFile {
    pane: usize,
    name: String,
    left: u8,
}
static PENDING_FILE: Mutex<Option<PendingFile>> = Mutex::new(None);

// Ctrl+Alt+N: an empty text file in the active pane, then the shell's own rename (F2) once it shows up.
unsafe fn new_file(i: usize) {
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = app(|a| if a.s.new_name.trim().is_empty() { default_new_name() } else { a.s.new_name.clone() });
    let name = fresh_file(&base, |n| Path::new(&dir).join(n).exists());
    let full = Path::new(&dir).join(&name);
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&full).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_FILE.lock() {
        *p = Some(PendingFile { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWFILE, 40, None);
    }
    focus_view(i);
}

// The view lists the new file a moment after Refresh. Select it and start the shell rename.
unsafe fn select_new_file(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // SVSI_EDIT is the shell's rename (the same one F2 uses). If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

unsafe fn key(m: &MSG) -> bool {
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    // Typing in Settings must not fire pane shortcuts. Esc closes the window.
    if settings::captures(m.hwnd) {
        if !ctrl && !alt && VIRTUAL_KEY(m.wParam.0 as u16) == VK_ESCAPE {
            settings::close();
            return true;
        }
        return false;
    }
    if m.hwnd == app(|a| a.tab_edit) && app(|a| a.tab_renaming.is_some()) {
        match VIRTUAL_KEY(m.wParam.0 as u16) {
            VK_RETURN => end_rename(true),
            VK_ESCAPE => end_rename(false),
            _ => return false,
        }
        return true;
    }
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = active_pane();
    let addr = pane_ctl(a, ID_EDIT) == Some(m.hwnd);
    let filt = pane_ctl(a, ID_FILTER) == Some(m.hwnd);
    // F2, Del, Ctrl+C/V ... are the shell view's accelerators: hand them over.
    let shell = || {
        let Some(sv) = view_of::<IShellView>(a) else { return false };
        let svh = sv.GetWindow().unwrap_or_default();
        (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool()) && sv.TranslateAccelerator(m) == S_OK
    };
    let list = || view_of::<IFolderView2>(a).is_some_and(|fv| view_list(&fv) == m.hwnd);
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        // Ctrl+Tab used to cycle panes. It now moves between tabs; panes use Ctrl+PageDown / Ctrl+PageUp.
        VK_T if ctrl && shift && !alt => reopen_closed_at(0),
        VK_T if ctrl && !shift && !alt => new_tab(),
        VK_W if ctrl && !shift && !alt => close_tab(app(|x| x.s.tab)),
        VK_TAB if ctrl && !alt => switch_rel(if shift { -1 } else { 1 }),
        VK_NEXT if ctrl && !alt => cycle(1),
        VK_PRIOR if ctrl && !alt => cycle(-1),
        k if ctrl && !shift && !alt && (VK_1.0..=VK_9.0).contains(&k.0) && ((k.0 - VK_1.0) as usize) < app(|x| x.s.panes()) => {
            let i = (k.0 - VK_1.0) as usize;
            set_active(i);
            focus_view(i);
        }
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && shift && !ctrl => match k {
            VK_LEFT => move_pane(-1, 0),
            VK_RIGHT => move_pane(1, 0),
            VK_UP => move_pane(0, -1),
            _ => move_pane(0, 1),
        },
        VK_LEFT if alt => browse(a, SBSP_NAVIGATEBACK),
        VK_RIGHT if alt => browse(a, SBSP_NAVIGATEFORWARD),
        VK_UP if alt => browse(a, SBSP_PARENT),
        VK_L if ctrl => return focus_ctl(a, ID_EDIT),
        VK_D if alt => return focus_ctl(a, ID_EDIT),
        VK_F4 if !alt => return focus_ctl(a, ID_EDIT),
        VK_F if ctrl => return focus_ctl(a, ID_FILTER),
        VK_H if ctrl => toggle_hidden(),
        VK_OEM_COMMA if ctrl && !shift && !alt => settings::open(app(|a| a.main)),
        VK_S if ctrl => save_current(),
        VK_B if ctrl && !shift && !alt => open_favs(),
        VK_M if ctrl && shift && !alt => toggle_zoom(),
        VK_N if ctrl && alt && !shift && !addr && !filt => new_file(a),
        VK_N if ctrl && shift && !alt && !addr && !filt => new_folder(a),
        VK_F1 => open_help(),
        VK_RETURN if addr => go_edit(a),
        VK_RETURN if filt => {
            apply_filter(a);
            focus_view(a);
        }
        VK_ESCAPE if addr => addr_reset(a, m.hwnd),
        VK_ESCAPE if filt => {
            _ = SetWindowTextW(m.hwnd, w!(""));
            apply_filter(a);
            focus_view(a);
        }
        _ if addr || filt => return false,
        VK_C if ctrl && shift && !alt && list() => copy_paths(a),
        VK_DOWN if alt && !ctrl && !shift => recent_menu(a),
        VK_F5 if list() => {
            resum_sizes();
            return shell();
        }
        VK_DELETE if !ctrl && !alt && list() => delete_selection(a, shift),
        VK_V if ctrl && !shift && !alt && list() => {
            if !paste_key(m.lParam.0 & (1 << 30) != 0, || try_paste(a)) {
                return shell();
            }
        }
        _ => return shell(),
    }
    true
}

unsafe fn pre_translate(m: &MSG) -> bool {
    if m.message == WM_LBUTTONDOWN || m.message == WM_KEYDOWN {
        app(|a| a.tree_input |= a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool());
    }
    match m.message {
        WM_MOUSEMOVE => {
            set_hover(if is_icon_button(m.hwnd) { m.hwnd } else { HWND::default() });
            false
        }
        WM_MOUSELEAVE => {
            if app(|a| a.hover == m.hwnd) {
                set_hover(HWND::default());
            }
            false
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => key(m),
        _ => false,
    }
}

// ---------- window procs ----------

fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe fn cursor_for(h: HWND) -> Option<Hit> {
    let mut pt = POINT::default();
    _ = GetCursorPos(&mut pt);
    _ = ScreenToClient(h, &mut pt);
    app(|a| hit(&geo(a), pt.x, pt.y))
}

extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // While the user drags the window edge, lay the panes out once at the end (same cost reason as splitters).
            WM_SIZE => {
                if !app(|a| a.sizing) {
                    relayout();
                    redraw_all(); // maximize / restore
                }
            }
            WM_ENTERSIZEMOVE => app(|a| a.sizing = true),
            WM_EXITSIZEMOVE => {
                app(|a| a.sizing = false);
                relayout();
                redraw_all();
                note_save();
            }
            WM_PAINT => paint_main(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_main covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_TIMER if wp.0 == TIMER_REVEAL => {
                _ = KillTimer(Some(h), TIMER_REVEAL);
                reveal_tree_selection();
                let left = app(|a| {
                    a.reveal_tries = a.reveal_tries.saturating_sub(1);
                    a.reveal_tries
                });
                if left > 0 {
                    SetTimer(Some(h), TIMER_REVEAL, if left == 2 { 330 } else { 550 }, None);
                }
            }
            WM_TIMER if wp.0 == TIMER_STATUS => {
                update_status();
                check_dirty();
            }
            WM_TIMER if wp.0 == TIMER_SAVE => {
                _ = KillTimer(Some(h), TIMER_SAVE);
                save_session();
            }
            WM_SETCURSOR if wp.0 == h.0 as usize && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let Some(d) = app(|a| a.drag).or_else(|| cursor_for(h)) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                let c = match d {
                    Hit::Side | Hit::Col(_) => IDC_SIZEWE,
                    Hit::Row(..) => IDC_SIZENS,
                };
                SetCursor(LoadCursorW(None, c).ok());
                return LRESULT(1);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| hit(&geo(a), x, y)) {
                    app(|a| a.drag = Some(d));
                    SetCapture(h);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let moved = app(|a| {
                    let d = a.drag?;
                    let g = geo(a);
                    let min = 60 * a.dpi / 96;
                    match d {
                        Hit::Side => a.s.side = x - g.pad - g.gap / 2,
                        Hit::Col(k) => drag_ratio(&mut a.s.colr, k, x - g.gap / 2, g.gx, g.w - g.pad - g.gx, g.gap, min),
                        Hit::Row(c, k) => drag_ratio(&mut a.s.rowr[c], k, y - g.gap / 2, g.top, g.bot - g.top, g.gap, min),
                    }
                    a.s.side = geo(a).side;
                    Some(())
                });
                if moved.is_some() {
                    show_bars(); // panes stay put until release: relaying out 16 Explorer views per mouse move is ~120ms
                }
            }
            WM_LBUTTONUP => {
                _ = ReleaseCapture();
                end_drag();
            }
            WM_CAPTURECHANGED => end_drag(),
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                let d = app(|a| hit(&geo(a), x, y));
                app(|a| match d {
                    Some(Hit::Side) => a.s.side = 240 * a.dpi / 96,
                    Some(Hit::Col(_)) => a.s.colr = even(a.s.heights.len()),
                    Some(Hit::Row(c, _)) => a.s.rowr[c] = even(a.s.heights[c]),
                    None => {}
                });
                relayout();
                redraw_all();
            }
            WM_COMMAND => match wp.0 & 0xffff {
                id if id == ID_TB_LAYOUT as usize => open_grid(),
                id if id == ID_TB_SAVE as usize => save_current(),
                id if id == ID_TB_LOAD as usize => open_load(),
                id if id == ID_TB_FAV as usize => open_favs(),
                id if id == ID_TB_THEME as usize => {
                    let dark = app(|a| a.s.dark);
                    app(|a| a.s.theme = if dark { 1 } else { 2 });
                    apply_theme();
                    save_session();
                }
                ID_NEWFILE => create_text_file(),
                id if id == ID_TB_SET as usize => settings::open(h),
                id if id == ID_TB_VIEW as usize => popup_menu(app(|a| a.view_menu), ID_TB_VIEW),
                ID_HIDDEN => toggle_hidden(),
                ID_DBLUP => {
                    app(|a| a.s.dblup ^= true);
                    settings::redraw();
                }
                ID_SETTINGS => settings::open(h),
                ID_LANG_AUTO => set_language(Lang::Auto),
                ID_LANG_EN => set_language(Lang::En),
                ID_LANG_KO => set_language(Lang::Ko),
                ID_LANG_JA => set_language(Lang::Ja),
                ID_HELP => open_help(),
                ID_ABOUT => about(h),
                id @ ID_ACCENT..=1405 => set_accent(pal().accents[id - ID_ACCENT].1),
                ID_ACCENT_PICK => pick_accent(),
                ID_KEYS => _ = MessageBoxW(Some(h), &HSTRING::from(tr(KEYS_EN, KEYS_KO, KEYS_JA)), &HSTRING::from(tr("Keyboard shortcuts", "단축키", "ショートカット キー")), MB_OK),
                _ => {}
            },
            WM_INITMENUPOPUP => {
                let (menu, dblup, accent, lang) = app(|a| (a.view_menu, a.s.dblup, a.s.accent_now(), a.s.lang));
                let check = |id: usize, on: bool| CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
                check(ID_HIDDEN, hidden());
                check(ID_DBLUP, dblup);
                check(ID_LANG_AUTO, lang == Lang::Auto);
                check(ID_LANG_EN, lang == Lang::En);
                check(ID_LANG_KO, lang == Lang::Ko);
                check(ID_LANG_JA, lang == Lang::Ja);
                for (k, (_, c)) in pal().accents.iter().enumerate() {
                    check(ID_ACCENT + k, *c == accent);
                }
            }
            WM_COPYDATA => {
                let cd = &*(lp.0 as *const CopyData);
                if cd.dw != COPY_ID || cd.lp.is_null() {
                    return LRESULT(0);
                }
                // copy first: the sender's buffer is only valid during this call
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(cd.lp as *const u16, cd.cb as usize / 2));
                open_args(text.split('\n').map(String::from).collect());
                return LRESULT(1);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as i32;
                app(|a| {
                    a.s.side = a.s.side * dpi / a.dpi;
                    a.dpi = dpi;
                });
                apply_font();
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            WM_CLOSE => {
                _ = KillTimer(Some(h), TIMER_SAVE);
                if app(|a| a.s.autosave && !a.s.file.is_empty()) {
                    let path = app(|a| a.s.file.clone());
                    write_session(Path::new(&path));
                }
                save_session();
                _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                _ = KillTimer(Some(h), TIMER_STATUS);
                _ = KillTimer(Some(h), TIMER_SAVE);
                destroy_every_pane();
                app(|a| a.tree = None);
                PostQuitMessage(0);
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

extern "system" fn pane_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let nav = |flags: u32| {
            if let Some(i) = pane_of(h) {
                set_active(i);
                browse(i, flags);
                focus_view(i);
            }
        };
        match msg {
            WM_TIMER if wp.0 == TIMER_SETTLE => settle_cols(h),
            WM_TIMER if wp.0 == TIMER_NEWFILE => {
                let job = PENDING_FILE.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_file(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_FILE.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWFILE);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWFILE);
                }
            }
            WM_TIMER if wp.0 == TIMER_NEWF => {
                let job = PENDING_NEW.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_folder(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_NEW.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWF);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWF);
                }
            }
            WM_SIZE => layout_pane(h),
            WM_LBUTTONDBLCLK => {
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    toggle_zoom();
                }
            }
            WM_PAINT => paint_pane(h),
            // folder dropped on the top bar: still this pane's? then open it here, as ⇄ does
            WM_DROP_OPEN => {
                let queued = app(|a| {
                    let mine = a.drop_open.as_ref().is_some_and(|(w, i, _)| *w == h && a.panes[*i].as_ref().is_some_and(|p| p.hwnd == h));
                    if mine {
                        a.drop_open.take()
                    } else {
                        None
                    }
                });
                if let Some((_, i, item)) = queued {
                    browse_item(i, &item);
                    set_active(i);
                    focus_view(i);
                }
            }
            WM_ERASEBKGND => return LRESULT(1), // paint_pane covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            // borderless edits sit on the field colour
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().pill));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().pill));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                // pill gets an accent outline while its edit has focus
                (ID_EDIT | ID_FILTER, EN_SETFOCUS | EN_KILLFOCUS) => _ = InvalidateRect(Some(h), None, false),
                (ID_BACK, _) => nav(SBSP_NAVIGATEBACK),
                (ID_FWD, _) => nav(SBSP_NAVIGATEFORWARD),
                (ID_UP, _) => nav(SBSP_PARENT),
                (ID_OPEN_IN, _) => {
                    if let Some(i) = pane_of(h) {
                        open_in_menu(h, i);
                    }
                }
                (ID_FAV, _) => {
                    if let Some(i) = pane_of(h) {
                        toggle_fav(i);
                    }
                }
                // typing in the filter box applies after a short pause
                (ID_FILTER, EN_CHANGE) => _ = SetTimer(Some(h), TIMER_FILTER, 250, None),
                _ => {}
            },
            WM_TIMER if wp.0 == TIMER_FILTER => {
                _ = KillTimer(Some(h), TIMER_FILTER);
                if let Some(i) = pane_of(h) {
                    apply_filter(i);
                }
            }
            // right-click on ← / →: recent folders; anywhere else as before
            WM_CONTEXTMENU if [ID_BACK, ID_FWD].iter().any(|&id| GetDlgItem(Some(h), id).is_ok_and(|b| b.0 as usize == wp.0)) => {
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    recent_menu(i);
                }
            }
            // mouse side buttons
            WM_APPCOMMAND => match (lp.0 >> 16) & 0x0fff {
                1 => nav(SBSP_NAVIGATEBACK),
                2 => nav(SBSP_NAVIGATEFORWARD),
                _ => return DefWindowProcW(h, msg, wp, lp),
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// The README on GitHub, in the UI language.
unsafe fn open_help() {
    let url = format!("{HOME}/blob/main/{}", tr("README.md", "README.ko.md", "README.md"));
    ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
}

unsafe fn about(owner: HWND) {
    let text = format!(
        "Grids {}\n{}\n\n{}\n{HOME}\n\n{}",
        env!("CARGO_PKG_VERSION"),
        tr("Fast multi-pane file manager for Windows", "Windows용 빠른 다중 패널 파일 관리자", "Windows 向けの高速マルチペイン ファイル マネージャー"),
        tr("MIT License · © 2026 efww", "MIT 라이선스 · © 2026 efww", "MIT ライセンス · © 2026 efww"),
        tr("Open the project page?", "프로젝트 페이지를 열까요?", "プロジェクト ページを開きますか?"),
    );
    if MessageBoxW(Some(owner), &HSTRING::from(text), &HSTRING::from(tr("About Grids", "Grids 정보", "Grids のバージョン情報")), MB_YESNO | MB_ICONINFORMATION) == IDYES {
        ShellExecuteW(None, w!("open"), &HSTRING::from(HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

// Toolbar buttons: id, accessible name, tooltip.
fn toolbar_tips() -> [(i32, &'static str, &'static str); 7] {
    [
        (ID_TB_LAYOUT, "", tr("Layout: click = even grid, drag = rows per column", "레이아웃: 클릭 = 균등, 끌기 = 열마다 칸 수", "レイアウト: クリック = 均等、ドラッグ = 列ごとの行数")),
        (ID_TB_SAVE, tr("Save session", "세션 저장", "セッションを保存"), ""), // tooltip set by refresh_save_button
        (ID_TB_LOAD, tr("Load", "불러오기", "読み込み"), tr("Load a session", "세션 불러오기", "セッションを読み込む")),
        (ID_TB_VIEW, tr("View", "보기", "表示"), tr("View: hidden files · colours · language · settings · help", "보기: 숨김 파일 · 선택 색 · 언어 · 설정 · 도움말", "表示: 隠しファイル · 色 · 言語 · 設定 · ヘルプ")),
        (ID_TB_FAV, tr("Favourites", "즐겨찾기", "お気に入り"), tr("Favourite folders (Ctrl+B; open in the active pane)", "즐겨찾기 폴더 (Ctrl+B, 선택한 패널에서 열기)", "お気に入りフォルダー (Ctrl+B、アクティブなペインで開く)")),
        (ID_TB_THEME, tr("Theme", "테마", "テーマ"), tr("Light / dark theme", "라이트 / 다크 테마", "ライト / ダーク テーマ")),
        (ID_TB_SET, tr("Settings", "설정", "設定"), tr("Settings", "설정", "設定")),
    ]
}

// Switches the UI language now: menus, fonts, toolbar tooltips, panes, and an open settings window.
unsafe fn set_language(lang: Lang) {
    UI.store(lang.resolve(), Ordering::Relaxed);
    let (main, old) = app(|a| {
        a.s.lang = lang;
        (a.main, a.view_menu)
    });
    if let Ok(m) = make_menus() {
        app(|a| a.view_menu = m);
        _ = DestroyMenu(old);
    }
    for (id, _, hint) in toolbar_tips() {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            tip_text(b, hint, TTM_UPDATETIPTEXTW);
        }
    }
    refresh_save_button();
    app(|a| a.status_key.clear());
    apply_font();
    apply_theme();
    save_session();
}

// The view popup opened from the top toolbar (no classic menu bar).
unsafe fn make_menus() -> Result<HMENU> {
    let view = CreatePopupMenu()?;
    let item = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| AppendMenuW(menu, flags, id, &HSTRING::from(text));
    item(view, MF_STRING, ID_HIDDEN, tr("Show hidden files\tCtrl+H", "숨김 파일 표시\tCtrl+H", "隠しファイルを表示\tCtrl+H"))?;
    item(view, MF_STRING, ID_NEWFILE, tr("New text file", "새 텍스트 파일", "新しいテキスト ファイル"))?;
    item(view, MF_STRING, ID_DBLUP, tr("Double-click empty space: parent folder", "빈 곳 더블클릭: 위 폴더로", "空白をダブルクリック: 上のフォルダー"))?;
    let colors = CreatePopupMenu()?;
    for (k, (name, _)) in LIGHT.accents.iter().enumerate() {
        item(colors, MF_STRING, ID_ACCENT + k, tr(ACCENT_EN[k], name, ACCENT_JA[k]))?;
    }
    AppendMenuW(colors, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(colors, MF_STRING, ID_ACCENT_PICK, tr("Custom...", "직접 선택...", "カスタム..."))?;
    item(view, MF_POPUP, colors.0 as usize, tr("Selection colour", "선택 색", "選択色"))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_SETTINGS, tr("Settings...", "설정...", "設定..."))?;
    let langs = CreatePopupMenu()?;
    item(langs, MF_STRING, ID_LANG_AUTO, tr("Follow Windows", "Windows 설정 따르기", "Windows の設定に従う"))?;
    item(langs, MF_STRING, ID_LANG_EN, "English")?;
    item(langs, MF_STRING, ID_LANG_KO, "한국어")?;
    item(langs, MF_STRING, ID_LANG_JA, "日本語")?;
    item(view, MF_POPUP, langs.0 as usize, tr("Language", "언어", "言語"))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_KEYS, tr("Keyboard shortcuts...", "단축키...", "ショートカット キー..."))?;
    item(view, MF_STRING, ID_HELP, tr("Help (online)\tF1", "도움말 (온라인)\tF1", "ヘルプ (オンライン)\tF1"))?;
    item(view, MF_STRING, ID_ABOUT, tr("About Grids", "Grids 정보", "Grids のバージョン情報"))?;
    Ok(view)
}

fn main() -> Result<()> {
    unsafe {
        let args: Vec<String> = std::env::args().skip(1).map(resolve).collect();
        // Read first so single=0 can opt out of handing arguments to the running window.
        // grids.json is the store. A leftover grids.ini (or qdir.ini) is migrated once.
        let mut s = store::load();
        if session_arg(&args).is_none() && args.is_empty() {
            apply_boot(&mut s);
        }
        s.dark = resolve_dark(s.theme);
        if s.single {
            if let Some(prev) = running_instance() {
                if !args.is_empty() {
                    let data: Vec<u16> = args.join("\n").encode_utf16().collect();
                    let cd = CopyData { dw: COPY_ID, cb: (data.len() * 2) as u32, lp: data.as_ptr() as _ };
                    let lp = LPARAM(&cd as *const CopyData as isize);
                    SendMessageTimeoutW(prev, WM_COPYDATA, WPARAM(0), lp, SMTO_ABORTIFHUNG, 5000, None);
                }
                if IsIconic(prev).as_bool() {
                    _ = ShowWindow(prev, SW_RESTORE);
                }
                _ = SetForegroundWindow(prev);
                return Ok(());
            }
        }

        OleInitialize(None)?; // OLE (not just COM) for drag & drop
        match session_arg(&args).and_then(|f| std::fs::read_to_string(f).ok()) {
            Some(t) => s.take_layout(&Session::parse(&t)),
            None => apply_args(&mut s, args),
        }

        _ = BufferedPaintInit();

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(), // app.ico, group id 1
            lpszClassName: w!("GridsMain"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, // the plate is drawn to the window size
            lpfnWndProc: Some(pane_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPane"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst,
            lpszClassName: w!("GridsBar"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(fav_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsFav"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(load_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsLoad"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(grid_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPicker"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(tabs_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsTabs"),
            ..Default::default()
        });
        UI.store(s.lang.resolve(), Ordering::Relaxed);
        let view_menu = make_menus()?;
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("GridsMain"),
            w!("Grids"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1400,
            900,
            None,
            None,
            Some(hinst),
            None,
        )?;
        // caption in the ground colour, so title bar and toolbar read as one surface
        let caption = cref(pal().bg);
        _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
        let square = DWMWCP_DONOTROUND;
        _ = DwmSetWindowAttribute(main, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
        let dpi = GetDpiForWindow(main) as i32;
        if s.side <= 0 {
            s.side = 240 * dpi / 96;
        }
        let win = s.win;
        let tip_style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02); // TTS_ALWAYSTIP | TTS_NOPREFIX
        let tip = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, tip_style, 0, 0, 0, 0, Some(main), None, None, None)?;
        s.anchor();
        let ntabs = s.tabs.len().max(1);
        app(|a| {
            (a.s, a.main, a.dpi, a.view_menu, a.tip) = (s, main, dpi, view_menu, tip);
            a.park = (0..ntabs).map(|_| no_panes()).collect();
            a.park_lru.clear();
        });
        // Match the file we just loaded so opening the window does not rewrite grids.json.
        let seeded = app(|a| store::sync_text(&a.s));
        LAST_SAVED.with(|l| *l.borrow_mut() = seeded);
        let tabs_hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsTabs"), w!(""), WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN, 0, 0, 0, 0, Some(main), None, Some(hinst), None)?;
        let tab_edit = child(tabs_hwnd, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), ID_TAB_EDIT, HFONT::default());
        _ = ShowWindow(tab_edit, SW_HIDE);
        app(|a| (a.tabs_hwnd, a.tab_edit) = (tabs_hwnd, tab_edit));
        let bar_ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let mut bars = [HWND::default(); 2];
        for b in &mut bars {
            *b = CreateWindowExW(bar_ex, w!("GridsBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None)?;
            _ = SetLayeredWindowAttributes(*b, COLORREF(0), 170, LWA_ALPHA);
        }
        app(|a| a.bars = bars);
        for (id, label, hint) in toolbar_tips() {
            add_tip(child(main, w!("BUTTON"), PCWSTR(HSTRING::from(label).as_ptr()), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), id, HFONT::default()), hint);
        }
        if let Ok((t, th)) = create_tree(main) {
            app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
        }
        apply_theme();
        apply_font();
        match win {
            Some([l, t, r, b, max]) => {
                let wp = WINDOWPLACEMENT {
                    length: size_of::<WINDOWPLACEMENT>() as u32,
                    showCmd: if max != 0 { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                    rcNormalPosition: RECT { left: l, top: t, right: r, bottom: b },
                    ..Default::default()
                };
                _ = SetWindowPlacement(main, &wp);
            }
            None => _ = ShowWindow(main, SW_SHOWNORMAL),
        }
        relayout();
        apply_ext(app(|a| a.s.ext));
        focus_view(app(|a| a.s.active));
        let file = app(|a| a.s.file.clone());
        let base = std::fs::read_to_string(&file).ok().map(|t| Session::parse(&t));
        app(|a| a.base = Some(base.unwrap_or_else(|| a.s.clone())));
        refresh_save_button();
        SetTimer(Some(main), TIMER_STATUS, 300, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !pre_translate(&msg) {
                let up = up_candidate(&msg);
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if let Some((i, g, svh)) = up {
                    let query = || {
                        let view = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()).map(|h| h.0 as isize);
                        let sel = view_of::<IFolderView2>(i).and_then(|fv| fv.ItemCount(SVGIO_SELECTION).ok()).map(|n| n as u32);
                        (view, sel)
                    };
                    if confirm((g, svh), query, gen) {
                        browse(i, SBSP_PARENT);
                    }
                }
            }
        }
        // The window is gone; copies still running finish first. Waiting keeps COM calls and messages
        // moving, in case a shell extension calls back into this apartment.
        for job in JOBS.take() {
            if !job.is_finished() {
                let h = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&job));
                _ = CoWaitForMultipleHandles((COWAIT_DISPATCH_CALLS | COWAIT_DISPATCH_WINDOW_MESSAGES).0 as u32, INFINITE, &[h]);
            }
            _ = job.join();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_ini() {
        assert_eq!(Session::parse("").lang, Lang::Auto);
        assert_eq!(Session::parse("lang=auto").lang, Lang::Auto);
        assert_eq!(Session::parse("lang=en").lang, Lang::En);
        assert_eq!(Session::parse("lang=ko").lang, Lang::Ko);
        assert_eq!(Session::parse("lang=ja").lang, Lang::Ja);
        assert!(Session::parse("dblup=1").single);
        assert!(!Session::parse("single=0").single);
        let fresh = Session::default();
        let round = Session::parse(&fresh.to_ini());
        assert_eq!(round.theme, 0);
        assert_eq!((round.font, round.bar, round.stat, round.view_new, round.start_new), (0, true, true, 4, 0));
        assert_eq!((round.ext, round.tree_on, round.edge, round.confirm, round.del), (true, true, true, true, 0));
        assert_eq!((round.dropm, round.boot, round.autosave, round.dot), (0, 0, false, true));
        assert!(round.new_name.is_empty() && round.start_path.is_empty() && round.boot_file.is_empty());
        // An older file has dark= and no theme=. That stays an explicit choice, and new keys keep defaults.
        let old = Session::parse("dark=1\ndblup=1\n");
        assert_eq!(old.theme, 2);
        assert!(old.dark && old.bar && old.dot);
        let t = Session::parse("theme=auto\ndark=1\nfont=2\nbar=0\nstat=0\nviewnew=1\nstart=2\nstartp=C:\\Work\next=0\ntree=0\nedge=0\nconfirm=0\ndel=1\nnewname=note.txt\ndrop=1\nboot=1\nbootf=C:\\s.grids\nautosave=1\ndot=0\nfav=C:\\WORK\tWork\n");
        assert_eq!(t.theme, 0);
        assert!(t.dark);
        assert_eq!(t.font, 2);
        assert!(!t.bar && !t.stat && !t.ext && !t.tree_on && !t.edge && !t.confirm && !t.dot);
        assert_eq!(t.view_new, 1);
        assert_eq!(t.start_new, 2);
        assert_eq!(t.start_path, "C:\\Work");
        assert_eq!(t.del, 1);
        assert_eq!(t.new_name, "note.txt");
        assert_eq!(t.dropm, 1);
        assert_eq!(t.boot, 1);
        assert_eq!(t.boot_file, "C:\\s.grids");
        assert!(t.autosave);
        assert_eq!(fav_path(&t.favs[0]), "C:\\WORK");
        assert_eq!(fav_label(&t.favs[0]), "Work");
        let back = Session::parse(&t.to_ini());
        assert_eq!((back.font, back.new_name, back.start_path, back.boot_file, back.favs), (t.font, t.new_name.clone(), t.start_path.clone(), t.boot_file.clone(), t.favs.clone()));
        let mut s = Session::default();
        s.lang = Lang::Ja;
        s.single = false;
        let back = Session::parse(&s.to_ini());
        assert_eq!((back.lang, back.single), (Lang::Ja, false));
        assert!(back.to_ini().contains("lang=ja\n"));
        assert!(Session::parse("lang=ko").to_ini().contains("lang=ko\n"));
    }

    #[test]
    fn neighbours() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        // 2×2 with an 8px gap: slots run down each column
        let grid = [(0, r(0, 0, 100, 100)), (1, r(0, 108, 100, 208)), (2, r(108, 0, 208, 100)), (3, r(108, 108, 208, 208))];
        let n = |cells: &[(usize, RECT)], cur, dx, dy| neighbour(cells, cur, dx, dy);
        assert_eq!([n(&grid, 3, 0, -1), n(&grid, 3, -1, 0), n(&grid, 0, 1, 0), n(&grid, 0, 0, 1)], [Some(2), Some(1), Some(2), Some(1)]);
        // edges
        assert_eq!([n(&grid, 0, -1, 0), n(&grid, 0, 0, -1), n(&grid, 3, 1, 0), n(&grid, 3, 0, 1)], [None; 4]);
        // [1,3]: one tall cell on the left, three on the right
        let tall = [(0, r(0, 0, 100, 316)), (1, r(108, 0, 208, 100)), (2, r(108, 108, 208, 208)), (3, r(108, 216, 208, 316))];
        assert_eq!([n(&tall, 0, 1, 0), n(&tall, 3, -1, 0), n(&tall, 2, 0, -1), n(&tall, 2, 0, 1)], [Some(2), Some(0), Some(1), Some(3)]);
        assert_eq!(n(&tall, 1, 0, -1), None); // stays in its column
        // a pane that failed to open: neither it nor the diagonal one is picked
        let gone = [grid[0], grid[2], grid[3]];
        assert_eq!([n(&gone, 0, 0, 1), n(&gone, 3, -1, 0), n(&gone, 2, 0, 1)], [None, None, Some(3)]);
        assert_eq!(n(&grid, 7, 1, 0), None);
    }

    #[test]
    fn columns() {
        let k = |pid| PROPERTYKEY { fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC), pid };
        let text = cols_text(&[(k(10), 220), (k(12), 80)], &[(k(12), -1)]);
        assert_eq!(parse_cols(&text), (vec![(k(10), 220), (k(12), 80)], vec![(k(12), -1)]));
        assert_eq!(parse_cols("junk;"), (vec![], vec![]));
        // kept in the session; a change counts wherever the pane is (columns are a pane setting)
        let mut base = Session { paths: vec![r"C:\a".into(); MAXP], ..Default::default() };
        base.cols[0] = text.clone();
        let base = Session::parse(&base.to_ini());
        assert_eq!(base.cols[0], text);
        let mut cur = base.clone();
        cur.cols[0] = cols_text(&[(k(12), 80), (k(10), 220)], &[(k(12), -1)]);
        assert!(layout_differs(&cur, &base));
        cur.paths[0] = r"C:\b".into();
        assert!(layout_differs(&cur, &base));
        assert!(is_fs(r"C:\") && is_fs(r"\\srv\share") && !is_fs("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        let titles = [r"C:\Users\x\Downloads", "", r"C:\"].map(window_title);
        assert_eq!(titles, ["Downloads - Grids", "Grids", "C: - Grids"]);
    }

    #[test]
    fn dblclick_up() {
        let c = |time, x, y| Click { win: 7, gen: 1, time, x, y };
        let next = |prev, now, bg| next_click(prev, now, bg, 500, 4, 4);
        // background, then background in time and box: up, and the pair is consumed
        let (rec, up) = next(None, c(100, 10, 10), true);
        assert!(rec == Some(c(100, 10, 10)) && !up);
        let (rec, up) = next(rec, c(400, 12, 8), true);
        assert!(rec.is_none() && up);
        assert!(!next(rec, c(450, 12, 8), true).1); // third click
        // first press on an item: the next one only records
        let (rec, up) = next(None, c(100, 10, 10), false);
        assert!(rec.is_none() && !up);
        assert!(!next(rec, c(200, 10, 10), true).1);
        // out of time, x, y
        let p = Some(c(100, 10, 10));
        assert!(!next(p, c(601, 10, 10), true).1);
        assert!(!next(p, c(200, 13, 10), true).1);
        assert!(!next(p, c(200, 10, 7), true).1);
        // MSG.time wraps
        assert!(next(Some(c(u32::MAX - 10, 10, 10)), c(20, 10, 10), true).1);
        // a navigation between the clicks
        assert!(!next(p, Click { gen: 2, ..c(200, 10, 10) }, true).1);

        assert!(bg_role(33));
        assert!([34, 20, 43, 25, 3].iter().all(|&r| !bg_role(r)));

        // a navigation during the probe voids it; g0 is the value before
        let g = Cell::new(5);
        assert_eq!(bg_probe(|| g.get(), || { g.set(6); true }), (5, false));
        assert_eq!(bg_probe(|| g.get(), || true), (6, true));

        // the generation is read after the queries (sol regression)
        let g = Cell::new(5);
        assert!(!confirm((5, 9), || { g.set(6); (Some(9), Some(0)) }, || g.get()));
        let g = || 5;
        assert!(!confirm((5, 9), || (Some(8), Some(0)), g));
        assert!(!confirm((5, 9), || (Some(9), Some(1)), g));
        assert!(!confirm((5, 9), || (Some(9), None), g));
        assert!(!confirm((5, 9), || (None, Some(0)), g));
        assert!(confirm((5, 9), || (Some(9), Some(0)), g));
    }

    #[test]
    fn addr() {
        let x = r"C:\Program Files\x.exe";
        assert_eq!(addr_parts("cmd"), Some((Some("cmd"), Some(("cmd", "")))));
        assert_eq!(addr_parts("powershell -NoExit").unwrap().1, Some(("powershell", "-NoExit")));
        assert_eq!(addr_parts(r#""C:\Program Files\x.exe" -a"#), Some((None, Some((x, "-a")))));
        assert_eq!(addr_parts(r#""C:\Program Files\x.exe""#), Some((Some(x), Some((x, "")))));
        assert_eq!(addr_parts(r#""C:\Windows\notepad.exe"#), None);
        assert_eq!(addr_parts(r"C:\Program Files\x.exe -a").unwrap().1, None);
        assert_eq!(addr_parts(r#"cmd /k "echo hi""#), Some((None, Some(("cmd", r#"/k "echo hi""#)))));
        assert_eq!(addr_parts(r#""a"b"#), Some((None, None)));
        assert!(is_rel("..") && is_rel(r"sub\x"));
        assert!(!is_rel(r"C:\") && !is_rel(r"\\srv\s") && !is_rel("shell:Downloads") && !is_rel("::{GUID}"));
    }

    #[test]
    fn paste() {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(paste_mode(&v(&[r"C:\a.txt"]), r"C:\"), Some(true));
        assert_eq!(paste_mode(&v(&[r"c:\X\a.txt", r"C:\x\b\"]), r"C:\X\"), Some(true));
        assert_eq!(paste_mode(&v(&[r"C:\a.txt"]), r"D:\x"), Some(false));
        assert_eq!(paste_mode(&v(&[r"\\srv\share\a.txt"]), r"\\SRV\share"), Some(true));
        assert_eq!(paste_mode(&v(&[r"\\srv\share\a.txt"]), r"C:\x"), Some(false));
        assert_eq!(paste_mode(&v(&[r"C:\a.txt", r"D:\b.txt"]), r"C:\"), None); // mixed
        assert_eq!(paste_mode(&[], r"C:\"), None);
        assert_eq!(paste_mode(&v(&["::{x}", r"C:\a"]), r"D:\"), None);
        assert_eq!(paste_mode(&v(&[r"C:\"]), r"D:\"), None); // a drive itself has no parent

        // (tries, shell): first press starts one copy, no shell; a held key's repeat: neither; unsupported: shell
        let run = |repeat: bool, ok: bool| {
            let mut tries = 0;
            let handled = paste_key(repeat, || {
                tries += 1;
                ok
            });
            (tries, !handled)
        };
        assert_eq!(run(false, true), (1, false));
        assert_eq!(run(true, true), (0, false));
        assert_eq!(run(false, false), (1, true));

        // ⇄ copy / move: a move into the same folder and mixed paths are refused
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt"]), r"C:\x", false), Some(true));
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt"]), r"C:\x", true), None);
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt"]), r"D:\y", true), Some(false));
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt", r"D:\y\b.txt"]), r"D:\y", false), None);

        // the whole selection or nothing
        let mut calls = 0;
        let all = collect_all(10_000, |k| {
            calls += 1;
            Some(k.to_string())
        });
        assert_eq!((all.map(|v| v.len()), calls), (Ok(10_000), 10_000));
        let mut calls = 0;
        let many = collect_all(10_001, |k| {
            calls += 1;
            Some(k.to_string())
        });
        assert_eq!((many, calls), (Err(Pick::TooMany), 0));
        assert_eq!(collect_all(0, |_| Some(String::new())), Err(Pick::Empty));
        assert_eq!(collect_all(5, |k| (k != 2).then(|| k.to_string())), Err(Pick::Unreadable));
    }

    #[test]
    fn tabs_session() {
        let mut s = Session::default();
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\one".into();
        s.views[0] = (4, 16);
        s.anchor();
        let mut other = Session::default();
        other.set_heights(vec![2, 2]);
        other.paths[1] = r"D:\two".into();
        other.views[1] = (3, 16);
        other.cols[1] = "{g}/1=80;".into();
        other.active = 1;
        s.tabs.push(Tab::from_live(&other, "Work".into(), true, vec!["*.pdf".into()]));
        assert_eq!(Session::parse(&s.to_ini()), s);
        assert!(s.snapshot().contains("\"locked\":true") || s.snapshot().contains("\"locked\": true"));
        assert!(!s.snapshot().contains("win="));

        let old = Session::parse("cols=1,3\np0=C:\\a\nactive=1\n");
        assert_eq!((old.tabs.len(), old.tabs[0].name.as_str(), old.tabs[0].locked), (1, "1", false));
        assert_eq!((old.heights.clone(), old.paths[0].as_str(), old.active), (vec![1, 3], r"C:\a", 1));
        assert_eq!(next_tab_name(&old.tabs), "2");
        assert!(!can_close_tab(&old.tabs, 0));

        let mut tabs = old.tabs.clone();
        tabs.push(Tab::from_live(&Session::default(), "2".into(), false, vec![]));
        tabs[0].locked = true;
        assert!(!can_close_tab(&tabs, 0) && can_close_tab(&tabs, 1));
        assert_eq!(close_other_indexes(&tabs, 0), vec![1]);
        assert!(close_other_indexes(&tabs, 1).is_empty());
        assert_eq!(duplicate_name(&tabs, "2"), "3");
        assert_eq!(duplicate_name(&tabs, "Work"), "Work (2)");
        let mut again = tabs.clone();
        again.push(Tab::from_live(&Session::default(), "Work (2)".into(), false, vec![]));
        assert_eq!(duplicate_name(&again, "Work"), "Work (3)");

        let mut names = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        reorder_vec(&mut names, 0, 2);
        assert_eq!(names, ["2", "3", "1"]);
        assert_eq!(moved_active(0, 0, 2), 2);
        assert_eq!(moved_active(0, 2, 0), 1);
        assert_eq!(moved_active(1, 0, 2), 0);

        let lay = tab_lay(800, 3, &[false, true, false], 96);
        assert_eq!(lay.body.len(), 3);
        assert_eq!(lay.body[0].right - lay.body[0].left, 90);
        let hidpi = tab_lay(1600, 3, &[false, true, false], 192);
        assert_eq!(hidpi.body[0].right - hidpi.body[0].left, 180);
        let crowd = tab_lay(120, 8, &[false; 8], 96);
        let cw = crowd.body[0].right - crowd.body[0].left;
        assert!(cw < 90 && cw >= 28);
        assert!(lay.body.windows(2).all(|w| w[0].right <= w[1].left));
        assert!(lay.plus.left >= lay.body[2].right && lay.plus.right <= 800);
        assert!(lay.close[0].is_some() && lay.close[1].is_none() && lay.close[2].is_some());
        let c = lay.close[0].unwrap();
        assert!(c.left >= lay.body[0].left && c.right <= lay.body[0].right);
        assert!(matches!(tab_hit(&lay, lay.plus.left + 2, lay.plus.top + 2), TabHit::Plus));
        assert!(matches!(tab_hit(&lay, c.left + 1, c.top + 1), TabHit::Close(0)));
        assert!(matches!(tab_hit(&lay, lay.body[1].left + 4, lay.body[1].top + 4), TabHit::Tab(1)));

        let junk = Session::parse("cols=1\ntabs=[{\"id\":\"later\"}]\n");
        assert_eq!((junk.tabs.len(), junk.heights.clone(), junk.tabs[0].name.as_str()), (1, vec![1], "1"));

        let mut wide = other.clone();
        wide.tabs.push(Tab::from_live(&Session::default(), "B".into(), false, vec![]));
        wide.tab = 1;
        wide.anchor();
        let back = Session::parse(&wide.to_ini());
        assert_eq!(back, wide);
        assert_eq!(back.tab, 1);
        assert_eq!(back.tabs[0].name, "1");
        assert_eq!((back.heights.clone(), back.paths[1].as_str(), back.active), (vec![2, 2], r"D:\two", 1));
    }

    #[test]
    fn three_tabs_restore_from_json() {
        assert_eq!(Session::default().boot, 0);
        let mut s = Session::default();
        s.win = Some([10, 20, 1100, 700, 0]);
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\alpha".into();
        s.views[0] = (4, 16);
        s.cols[0] = "{g}/1=40;".into();
        s.anchor();
        s.tabs[0].name = "Docs".into();
        s.tabs[0].filters[0] = "*.txt".into();

        let mut pics = Session::default();
        pics.set_heights(vec![2]);
        pics.paths[0] = r"D:\beta".into();
        pics.paths[1] = r"D:\gamma".into();
        pics.views[1] = (3, 32);
        pics.cols[1] = "{g}/2=80;".into();
        pics.active = 1;
        s.tabs.push(Tab::from_live(&pics, "Pics".into(), true, vec!["*.png".into(), "*.jpg".into()]));

        let mut work = Session::default();
        work.set_heights(vec![1, 1]);
        work.paths[0] = r"E:\delta".into();
        work.paths[1] = r"E:\epsilon".into();
        s.tabs.push(Tab::from_live(&work, "Work".into(), false, vec![]));
        s.tab = 2;
        s.publish();

        let text = store::sync_text(&s);
        let mut back = store::import_text(&text).unwrap();
        apply_boot(&mut back);
        assert_eq!(back.boot, 0);
        assert_eq!(back.tabs.len(), 3);
        assert_eq!(back.tab, 2);
        assert_eq!(back.win, Some([10, 20, 1100, 700, 0]));
        assert_eq!(back.tabs[0].name, "Docs");
        assert!(!back.tabs[0].locked);
        assert_eq!(back.tabs[0].paths[0], r"C:\alpha");
        assert_eq!(back.tabs[0].filters[0], "*.txt");
        assert_eq!(back.tabs[0].cols[0], "{g}/1=40;");
        assert_eq!(back.tabs[0].views[0], (4, 16));
        assert!(back.tabs[1].locked);
        assert_eq!(back.tabs[1].name, "Pics");
        assert_eq!((back.tabs[1].paths[0].as_str(), back.tabs[1].paths[1].as_str()), (r"D:\beta", r"D:\gamma"));
        assert_eq!(back.tabs[1].filters[1], "*.jpg");
        assert_eq!(back.tabs[1].cols[1], "{g}/2=80;");
        assert_eq!(back.tabs[1].views[1], (3, 32));
        assert_eq!((back.tabs[1].heights.clone(), back.tabs[1].active), (vec![2], 1));
        assert_eq!(back.tabs[2].name, "Work");
        assert_eq!((back.paths[0].as_str(), back.paths[1].as_str()), (r"E:\delta", r"E:\epsilon"));
        assert_eq!(back.heights, vec![1, 1]);

        // A different startup mode still stores the whole tab list. Only the next launch's boot step uses it.
        s.boot = 2;
        let forced = store::sync_text(&s);
        assert!(forced.contains("\"Docs\"") && forced.contains("\"Pics\"") && forced.contains("\"Work\""));
        let mut wiped = store::import_text(&forced).unwrap();
        assert_eq!(wiped.tabs.len(), 3);
        apply_boot(&mut wiped);
        assert_eq!((wiped.boot, wiped.tabs.len(), wiped.tabs[0].name.as_str()), (2, 1, "1"));
    }

    #[test]
    fn tab_color_persists_and_one_tab_session_drops_the_others() {
        let mut s = Session::default();
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\docs".into();
        s.anchor();
        s.tabs[0].name = "Docs".into();
        s.tabs[0].color = Some(0x3D7EE0);
        let mut pics = Session::default();
        pics.set_heights(vec![1]);
        pics.paths[0] = r"D:\pics-only".into();
        s.tabs.push(Tab::from_live(&pics, "Pics".into(), true, vec![]));
        s.tabs[1].color = Some(0xE15A4A);

        let back = store::import_text(&store::sync_text(&s)).unwrap();
        assert_eq!(back.tabs[0].color, Some(0x3D7EE0));
        assert_eq!(back.tabs[1].color, Some(0xE15A4A));
        assert_eq!(back.tabs[0].paths[0], r"C:\docs");
        let again = Session::parse(&s.to_ini());
        assert_eq!((again.tabs[0].color, again.tabs[1].color), (Some(0x3D7EE0), Some(0xE15A4A)));

        let old = store::import_text(r#"{"version":1,"tabs":[{"name":"1","locked":false,"active":true,"layout":{"columns":[1]},"panes":[{"index":0,"path":"C:\\plain"}]}]}"#).unwrap();
        assert_eq!(old.tabs[0].color, None);

        let file = tab_session_text(&s, 0).unwrap();
        let one = Session::parse(&file);
        assert_eq!(one.tabs.len(), 1);
        assert_eq!(one.tabs[0].name, "Docs");
        assert_eq!(one.tabs[0].color, Some(0x3D7EE0));
        assert_eq!(one.paths[0], r"C:\docs");
        assert!(!file.contains("pics-only"));
        assert!(file.contains("3D7EE0"));
    }

    #[test]
    fn session_layout_args() {
        let mut s = Session { side: 200, active: 3, ..Default::default() };
        s.set_heights(vec![1, 3]);
        (s.colr, s.rowr[1]) = (vec![0.25, 0.75], vec![0.5, 0.25, 0.25]);
        s.win = Some([1, 2, 300, 400, 1]);
        s.paths[1] = r"C:\a=b\폴더".into();
        s.views[1] = (4, 16);
        assert_eq!(Session::parse(&s.to_ini()), s);
        // garbage in the ini falls back to sane values
        let bad = Session::parse("cols=9,0,2\ncolr=1,-1\nrowr=x|1|1,1\nactive=99\nv3=x\np99=C:\\");
        assert_eq!((bad.heights.clone(), bad.colr.clone(), bad.active), (vec![4, 1, 2], even(3), 0));
        assert_eq!(bad.rowr, vec![even(4), vec![1.0], even(2)]);
        // an older even-grid ini still loads: rows x cols, one row-ratio group for every column
        let old = Session::parse("grid=3,2\nrowr=0.5,0.25,0.25");
        assert_eq!((old.heights.clone(), old.rowr[1].clone()), (vec![3, 3], vec![0.5, 0.25, 0.25]));
        // a named session brings grid + folders but not the window
        let mut cur = Session { side: 300, win: Some([0, 0, 9, 9, 0]), ..Default::default() };
        cur.take_layout(&s);
        assert_eq!((cur.heights.clone(), cur.paths[1].as_str(), cur.side, cur.win), (vec![1, 3], s.paths[1].as_str(), 300, Some([0, 0, 9, 9, 0])));

        // panes are numbered down each column; the picker drag sets one column at a time
        assert_eq!([slot(&[1, 3], 0), slot(&[1, 3], 1), slot(&[1, 3], 3)], [(0, 0), (1, 0), (1, 2)]);
        assert_eq!(drag_heights(drag_heights(vec![1], 0, 1), 2, 1), vec![1, 3]); // left 1, then right column down to row 3
        assert_eq!(drag_heights(vec![2, 2, 2], 1, 0), vec![2]); // moving back left drops columns
        assert_eq!((layout_label(&[2, 2]), layout_label(&[1, 3])), ("2 × 2".into(), "1 | 3".into()));

        // green dot: layout, ratios and view modes count; folders and unknown (0) baseline views don't
        let base = s.clone();
        let mut cur = s.clone();
        (cur.paths[0], cur.views[2]) = ("D:\\".into(), (1, 48));
        assert!(!layout_differs(&cur, &base));
        cur.colr = vec![0.3, 0.7];
        assert!(layout_differs(&cur, &base));
        assert!(!s.snapshot().contains("win=") && !s.snapshot().contains("session="));
        // the drawn star: solid middle and empty corners when filled; hollow middle as an outline
        let (full, ring) = (star_coverage(15, Star::Filled), star_coverage(15, Star::Outline));
        assert_eq!((full[7 * 15 + 7], full[0], full[14 * 15 + 14]), (255, 0, 0));
        assert!(ring[8 * 15 + 7] == 0 && ring.iter().any(|&a| a == 255));
        for n in [15, 22] {
            // the + (what Plus adds to the outline) sits exactly on the box centre in both directions
            let (plus, ring) = (star_coverage(n, Star::Plus), star_coverage(n, Star::Outline));
            let added: Vec<(i32, i32)> = (0..n * n).filter(|&k| plus[k as usize] != ring[k as usize]).map(|k| (k % n, k / n)).collect();
            let span = |f: fn(&(i32, i32)) -> i32| added.iter().map(f).min().unwrap() + added.iter().map(f).max().unwrap();
            assert_eq!((span(|p| p.0), span(|p| p.1)), (n - 1, n - 1), "{n}");
        }
        // favourites: kept in grids.ini, case-insensitive, never in a named session
        let mut f = Session { favs: vec![r"C:\Work".into(), r"D:\".into()], ..Default::default() };
        f = Session::parse(&f.to_ini());
        assert!(f.is_fav(r"c:\work") && !f.is_fav(r"C:\Work\x") && !f.is_fav(""));
        assert!(fav_removes(&[r"C:\Work"], |p| p.eq_ignore_ascii_case(r"C:\Work")));
        assert!(!fav_removes(&[], |_| true) && !fav_removes(&[r"C:\Work", r"D:\"], |p| p.eq_ignore_ascii_case(r"C:\Work")));
        assert_eq!((fav_anchor(Some(5), 8), fav_anchor(None, 8), fav_anchor(Some(9), 3), fav_anchor(None, 0)), (5, 7, 2, 0));
        assert!(!f.snapshot().contains("fav="));
        assert_eq!((short_name(r"C:\Users\x\"), short_name("C:\\"), short_name(THIS_PC)), ("x".into(), "C:".into(), "This PC".into()));

        // cells tile the length exactly, with `gap` between neighbours
        let c = cells(10, 305, &[0.25, 0.25, 0.5], 5);
        assert_eq!((c[0].0, c[2].1), (10, 315));
        assert!(c.windows(2).all(|w| w[0].1 + 5 == w[1].0));

        // dragging respects the minimum size and keeps the pair's total
        let mut r = vec![0.5, 0.5];
        drag_ratio(&mut r, 0, -1000, 0, 205, 5, 20);
        assert_eq!(r[0], 0.1);
        assert!((r[0] + r[1] - 1.0).abs() < 1e-12);

        let mut s = Session::default();
        apply_args(&mut s, (0..5).map(|i| format!("::{{x{i}}}")).collect());
        assert_eq!((s.heights.clone(), s.paths[4].as_str()), (vec![2, 2, 2], "::{x4}"));
        assert_eq!((session_arg(&["a.GRIDS".into()]), session_arg(&["old.qdr".into()])), (Some("a.GRIDS"), Some("old.qdr")));
        assert_eq!(session_arg(&["a.grids".into(), "b".into()]), None);
        assert_eq!(fresh_file("New Text Document.txt", |_| false), "New Text Document.txt");
        assert_eq!(fresh_file("New Text Document.txt", |n| n == "New Text Document.txt"), "New Text Document (2).txt");
        assert_eq!(fresh_file("새 텍스트 문서.txt", |n| n == "새 텍스트 문서.txt" || n == "새 텍스트 문서 (2).txt"), "새 텍스트 문서 (3).txt");

        assert_eq!(fresh_folder("New folder", |_| false), "New folder");
        assert_eq!(fresh_folder("New folder", |n| n == "New folder"), "New folder (2)");
        assert_eq!(fresh_folder("New folder", |n| n == "New folder" || n == "New folder (2)"), "New folder (3)");
        assert_eq!(filter_spec(" pdf; *.TXT ;"), "*pdf*;*.TXT");
        assert_eq!((human(10), human(1536), human(5 << 30)), ("10 B".into(), "1.5 KB".into(), "5.0 GB".into()));

        // pane top bar: number, back, fwd, up, address, ⇄, filter left to right, nothing overlapping, inside the plate
        for (w, dpi) in [(800, 96), (260, 96), (1200, 144)] {
            let p = pane_geo(w, 500, dpi);
            let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
            assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
            assert!(p.num.right > p.num.left && p.fav.right <= w && p.view.top >= p.btn[0].bottom);
        }
        // the number only shows while the address edit keeps s(32); otherwise the bar is laid out as before
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            for w in 60..=600 {
                let p = pane_geo(w, 500, dpi);
                if p.num.right > p.num.left {
                    let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
                    assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
                    assert!((p.addr.right - s(6)) - (p.addr.left + s(8)) >= s(32), "{w}@{dpi}");
                } else {
                    assert!(p.btn[0].left == s(4) && p.addr.left == s(4) + 3 * s(24) + s(4), "{w}@{dpi}");
                }
            }
        }
        let shown = |w, dpi| {
            let p = pane_geo(w, 500, dpi);
            p.num.right > p.num.left
        };
        assert!(!shown(240, 96) && shown(241, 96) && !shown(361, 144) && shown(362, 144));
    }

    #[test]
    fn recent_visits() {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut r = Vec::new();
        visit(&mut r, r"C:\a", r"C:\b");
        assert_eq!(r, v(&[r"C:\b", r"C:\a"]));
        visit(&mut r, r"C:\b", r"c:\A"); // case-insensitive: moves up, no duplicate
        assert_eq!(r, v(&[r"c:\A", r"C:\b"]));
        let before = r.clone();
        for (old, new) in [(r"C:\b", r"c:\B"), ("", r"C:\x"), (r"C:\b", THIS_PC)] {
            visit(&mut r, old, new); // refresh / restore, fresh pane, shell place: no visit
        }
        assert_eq!(r, before);
        visit(&mut r, THIS_PC, r"D:\"); // leaving a shell place records only the folder
        assert_eq!(r[0], r"D:\");
        assert!(!r.iter().any(|p| p == THIS_PC));
        for k in 0..30 {
            visit(&mut r, r"C:\b", &format!(r"C:\n{k}"));
        }
        assert_eq!((r.len(), r[0].as_str(), r[1].as_str()), (RECENT, r"C:\n29", r"C:\b"));
    }

    #[test]
    fn recent_survives_restore() {
        let full: Vec<String> = (0..RECENT).map(|k| format!(r"C:\f{k}")).collect();
        let mut r = full.clone();
        // 16 panes come back to their folders, twice (start-up, then a theme / language rebuild)
        for _ in 0..2 {
            for p in &full[..16] {
                visit(&mut r, p, p);
            }
        }
        assert_eq!(r, full);
        let items = recent_items(&r, r"c:\F0");
        assert_eq!(items.len(), RECENT - 1);
        assert_eq!(items.last(), full.last()); // the oldest is still offered
    }

    #[test]
    fn recent_ini() {
        let mut s = Session { recent: vec![r"C:\b".into(), r"\\srv\share".into(), r"C:\a".into()], ..Default::default() };
        s = Session::parse(&s.to_ini());
        assert_eq!(s.recent, [r"C:\b", r"\\srv\share", r"C:\a"]);
        assert!(!s.snapshot().contains("recent="));
        let many: String = (0..30).map(|k| format!("recent=C:\\{k}\n")).collect();
        assert_eq!(Session::parse(&many).recent.len(), RECENT);
    }

    // A fresh folder under %TEMP% holding `files` (relative path, size).
    fn tree(name: &str, files: &[(String, usize)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("grids-test-{}-{name}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for (f, n) in files {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![0u8; *n]).unwrap();
        }
        root
    }

    fn many(prefix: &str, count: usize, size: usize) -> Vec<(String, usize)> {
        (0..count).map(|k| (format!("{prefix}{k}.bin"), size)).collect()
    }

    #[test]
    fn folder_size_walk() {
        use std::cell::Cell;
        let never = || false;
        let mut quiet = |_, _| {};
        let root = tree("walk", &[("a.bin".into(), 100), (r"sub\b.bin".into(), 20), (r"sub\deep\c.bin".into(), 3)]);
        assert_eq!(dir_bytes(&root, &never, &never, &mut quiet), Some((123, false)));
        assert_eq!(dir_bytes(&root.join("missing"), &never, &never, &mut quiet), Some((0, true)));
        // a junction pointing back up is not followed
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(root.join(r"sub\loop")).arg(&root).output();
        assert!(made.is_ok_and(|o| o.status.success()) && root.join(r"sub\loop\a.bin").exists());
        assert_eq!(dir_bytes(&root, &never, &never, &mut quiet), Some((123, false)));
        // a stop mid-walk gives no total
        let calls = Cell::new(0);
        let later = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert_eq!(dir_bytes(&root, &later, &never, &mut quiet), None);

        // many files, no subfolders: a copy that starts and ends between status ticks only pauses the walk
        let flat = tree("flat", &many("f", 300, 2));
        let seen = Cell::new(0);
        let copying = || {
            seen.set(seen.get() + 1);
            (100..103).contains(&seen.get())
        };
        assert_eq!(dir_bytes(&flat, &never, &copying, &mut quiet), Some((600, false)));
        assert!(seen.get() > 300);
        // a cancel still gets through while paused
        let paused = Cell::new(0);
        let copy_on = || {
            paused.set(paused.get() + 1);
            paused.get() > 50
        };
        let cancel = || paused.get() > 52;
        assert_eq!(dir_bytes(&flat, &cancel, &copy_on, &mut quiet), None);
        assert!(paused.get() < 60);
        // progress comes with the running total
        let big = tree("big", &many("g", 2100, 1));
        let mut last = 0;
        let mut progress = |b, _| last = b;
        assert_eq!(dir_bytes(&big, &never, &never, &mut progress), Some((2100, false)));
        assert!(last >= 2000);

        // F5 after a file was added: the same selection is summed again
        let mut job = SizeJob::default();
        let target = vec![flat.to_string_lossy().into_owned()];
        assert!(job.want(&target, false));
        assert!(job.publish(job.gen, 600, true, false));
        std::fs::write(flat.join("new.bin"), [0u8; 50]).unwrap();
        assert!(!job.want(&target, false));
        job.want(&[], true); // what resum_sizes does
        assert!(job.want(&target, false) && !job.done);
        assert_eq!(dir_bytes(&flat, &never, &never, &mut quiet), Some((650, false)));
        for r in [root, flat, big] {
            _ = std::fs::remove_dir_all(r);
        }
    }

    #[test]
    fn folder_size_job() {
        let (a, b) = (vec![r"C:\a".to_string()], vec![r"C:\b".to_string()]);
        let mut j = SizeJob::default();
        assert!(j.want(&a, false));
        let g = j.gen;
        assert!(!j.want(&a, false)); // same target: keeps running
        assert!(j.publish(g, 10, true, false) && j.done);
        assert!(!j.want(&a, false) && j.done && j.bytes == 10); // and keeps its result
        assert!(!j.want(&a, true)); // a copy starts: cancelled
        assert!(!j.publish(g, 99, true, false) && j.bytes == 10); // the old walk's result is dropped
        assert!(j.want(&a, false) && j.bytes == 0 && !j.done); // the copy ended: summed again
        assert!(!j.want(&[], false) && j.target.is_empty()); // no folders: cancelled
        assert!(j.want(&a, false));
        let g = j.gen;
        assert!(j.want(&b, false) && j.gen != g);
        assert!(!j.publish(g, 5, false, false));
        assert!(j.publish(j.gen, 5, false, true) && j.partial);
    }

    #[test]
    fn folder_size_text() {
        let kb = human(1024);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(size_text(0, true, None, false), " (calculating…)");
        assert_eq!(size_text(0, true, Some((0, false, false)), false), " (calculating…)");
        assert_eq!(size_text(1024, true, Some((1024, false, false)), false), format!(" ({}…)", human(2048)));
        assert_eq!(size_text(0, true, Some((1024, true, false)), false), format!(" ({kb})"));
        assert_eq!(size_text(0, true, Some((1024, true, true)), false), format!(" ({kb}, partial)"));
        // something missed while collecting the selection isn't shown as a full total
        assert_eq!(size_text(0, true, Some((1024, true, false)), true), format!(" ({kb}, partial)"));
        assert_eq!(size_text(1024, false, None, false), format!(" ({kb})"));
        assert_eq!(size_text(1024, false, None, true), format!(" ({kb}, partial)"));
        assert_eq!(size_text(0, false, None, false), "");
        assert_eq!(size_text(0, false, None, true), " (size unknown)");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(size_text(0, true, Some((1024, true, false)), true), format!(" ({kb}, 일부만)"));
        assert_eq!(size_text(0, false, None, true), " (크기 모름)");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(size_text(0, false, None, true), " (サイズ不明)");
        UI.store(0, Ordering::Relaxed);
    }

    #[test]
    fn folder_size_collect() {
        let root = tree("collect", &[("f.bin".into(), 7), (r"d\x.bin".into(), 1)]);
        let path = |p: &str| root.join(p).to_string_lossy().into_owned();
        let (f, d) = (path("f.bin"), path("d"));
        let none = Sel { file_bytes: 0, dirs: vec![], partial: true };
        assert_eq!(collect_sel(None, |_| None), none); // GetSelection / GetCount failed
        assert_eq!(collect_sel(Some(1), |_| None), none); // GetItemAt / no file system path
        assert_eq!(collect_sel(Some(1), |_| Some(path("gone"))), none);
        assert_eq!(collect_sel(Some(1), |_| Some(r"C:\".into())), none); // a drive root isn't walked
        let both = [f.clone(), d.clone()];
        assert_eq!(collect_sel(Some(2), |k| Some(both[k as usize].clone())), Sel { file_bytes: 7, dirs: vec![d], partial: false });
        let calls = std::cell::Cell::new(0);
        let s = collect_sel(Some(10_001), |_| {
            calls.set(calls.get() + 1);
            Some(f.clone())
        });
        assert_eq!((s.file_bytes, s.partial, calls.get()), (70_000, true, 10_000));
        _ = std::fs::remove_dir_all(root);
    }
}
