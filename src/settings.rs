// Settings window: Win32, no webview. Nav, cards, toggles and the footer are drawn here;
// edits, combos and buttons are real child controls sitting on that paint.

use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::Once;
use windows::core::{w, BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{app, tr, Lang};

const S_SEARCH: i32 = 1;
const S_CLOSE: i32 = 2;
const S_KEYQ: i32 = 3;
const S_RESET: i32 = 4;
const S_LANG: i32 = 10;
const S_START: i32 = 11;
const S_THEME: i32 = 12;
const S_FONT: i32 = 13;
const S_VIEW: i32 = 14;
const S_DIR: i32 = 15;
const S_DEL: i32 = 16;
const S_NEW: i32 = 17;
const S_DROP: i32 = 18;
const S_SESS: i32 = 20;
const S_ACCENT: i32 = 21;
const S_GITHUB: i32 = 22;
const S_ADV_OPEN: i32 = 23;
const S_ADV_EX: i32 = 24;
const S_ADV_IM: i32 = 25;
const S_ADV_RESET: i32 = 26;
const S_FAV0: i32 = 400;

const TG_SINGLE: u8 = 1;
const TG_DBLUP: u8 = 2;
const TG_BAR: u8 = 3;
const TG_STATUS: u8 = 4;
const TG_HIDDEN: u8 = 5;
const TG_EXT: u8 = 6;
const TG_TREE: u8 = 7;
const TG_EDGE: u8 = 8;
const TG_CONFIRM: u8 = 9;
const TG_FAVMENU: u8 = 10;
const TG_AUTOSAVE: u8 = 11;
const TG_DOT: u8 = 12;

const CB_ADDSTRING: u32 = 0x0143;
const CB_RESETCONTENT: u32 = 0x014B;
const CB_SETCURSEL: u32 = 0x014E;
const CB_SETMINVISIBLE: u32 = 0x1701;
const CBN_SELCHANGE: u32 = 1;
const EN_CHANGE: u32 = 0x0300;
const EM_SETCUEBANNER: u32 = 0x1501;

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

fn tx(t: [&'static str; 3]) -> &'static str {
    tr(t[0], t[1], t[2])
}

const CHROME_LIGHT: u32 = 0x0067C0;
const CHROME_DARK: u32 = 0x4CC2FF;
const WHITE: u32 = 0xFFFFFF;

/// Accent used by this window. A chosen colour is used as-is. The old built-in gray
/// selection colour reads as no accent, so that default is Windows blue (#0067c0 / #4cc2ff).
fn chrome() -> u32 {
    let (c, dark) = app(|a| (a.s.accent_now(), a.s.dark));
    let plain = if dark { super::DARK.accents[1].1 } else { super::LIGHT.accents[1].1 };
    if c == plain {
        if dark { CHROME_DARK } else { CHROME_LIGHT }
    } else {
        c
    }
}

fn on_chrome(c: u32) -> u32 {
    if c == CHROME_LIGHT {
        WHITE
    } else {
        super::on_color(c)
    }
}

const fn s(en: &'static str, ko: &'static str, ja: &'static str) -> [&'static str; 3] {
    [en, ko, ja]
}

const TITLE: [&str; 3] = s("grids settings", "grids 설정", "grids 設定");
const FOOT: [&str; 3] = s("Changes apply right away", "변경은 바로 적용돼요", "変更はすぐに適用されます");
const CLOSE_L: [&str; 3] = s("Close", "닫기", "閉じる");
const FIND: [&str; 3] = s("Find a setting", "설정 검색", "設定を検索");
const NONE: [&str; 3] = s("No matches", "결과 없음", "一致なし");
const NEW: [&str; 3] = s("New", "신규", "新規");
const PAGES: [[&str; 3]; 9] = [
    s("General", "일반", "一般"),
    s("Appearance", "모양", "外観"),
    s("Panes & view", "패널·보기", "ペインと表示"),
    s("File operations", "파일 작업", "ファイル操作"),
    s("Favourites", "즐겨찾기", "お気に入り"),
    s("Shortcuts", "단축키", "ショートカット"),
    s("Sessions", "세션", "セッション"),
    s("Advanced", "고급", "詳細"),
    s("About", "정보", "情報"),
];

#[derive(Clone, Copy, PartialEq)]
enum Ctl {
    Lang,
    Start,
    Theme,
    Font,
    ViewMode,
    StartDir,
    DelMode,
    NewName,
    Drop,
    Accent,
    CopyEng,
    Sess,
    About,
    Adv,
    AdvReset,
    Toggle(u8),
}

#[derive(Clone, Copy)]
struct Spec {
    title: [&'static str; 3],
    desc: [&'static str; 3],
    badge: bool,
    live: bool,
    ctl: Ctl,
}

const GEN: &[Spec] = &[
    Spec { title: s("Display language", "표시 언어", "表示言語"), desc: s("The default follows the Windows display language. A change applies right away.", "기본값은 Windows 표시 언어를 따라요. 바꾸면 바로 적용돼요.", "既定では Windows の表示言語に従います。変えるとすぐに適用されます。"), badge: true, live: true, ctl: Ctl::Lang },
    Spec { title: s("On startup", "시작할 때", "起動時"), desc: s("Restore every tab, its folders and the window, or open a chosen session.", "모든 탭과 폴더, 창을 복원하거나 정해진 세션을 열어요.", "すべてのタブとフォルダー、ウィンドウを復元するか、指定したセッションを開きます。"), badge: false, live: true, ctl: Ctl::Start },
    Spec { title: s("Single instance", "중복 실행 막기", "二重起動を防ぐ"), desc: s("If it is already open, bring that window forward.", "이미 켜져 있으면 그 창을 앞으로 가져와요.", "すでに起動していれば、そのウィンドウを前面に出します。"), badge: false, live: true, ctl: Ctl::Toggle(TG_SINGLE) },
    Spec { title: s("Double-click empty space for the parent folder", "빈 곳 더블클릭으로 상위 폴더", "空白のダブルクリックで上のフォルダー"), desc: s("This is the dblup setting.", "지금 있는 dblup 설정이에요.", "現在の dblup 設定です。"), badge: false, live: true, ctl: Ctl::Toggle(TG_DBLUP) },
];
const LOOK: &[Spec] = &[
    Spec { title: s("Theme", "테마", "テーマ"), desc: s("", "", ""), badge: false, live: true, ctl: Ctl::Theme },
    Spec { title: s("Accent colour", "강조 색", "アクセント カラー"), desc: s("Light and dark each keep their own (accent / accent_dark).", "라이트와 다크에 따로 정할 수 있어요(지금 있는 accent / accent_dark).", "ライトとダークで別々に選べます（accent / accent_dark）。"), badge: false, live: true, ctl: Ctl::Accent },
    Spec { title: s("Font size", "글꼴 크기", "フォント サイズ"), desc: s("The toolbar, panes and this window.", "툴바, 패널, 이 창에 적용돼요.", "ツールバー、ペイン、このウィンドウに適用します。"), badge: true, live: true, ctl: Ctl::Font },
    Spec { title: s("Show the toolbar", "툴바 표시", "ツールバーを表示"), desc: s("", "", ""), badge: true, live: true, ctl: Ctl::Toggle(TG_BAR) },
    Spec { title: s("Show the status bar", "상태 표시줄 표시", "ステータス バーを表示"), desc: s("Item count and free space on the drive.", "항목 수와 드라이브 남은 공간이에요.", "項目数とドライブの空き容量です。"), badge: true, live: true, ctl: Ctl::Toggle(TG_STATUS) },
];
const VIEW: &[Spec] = &[
    Spec { title: s("Default view for a new pane", "새 패널의 기본 보기", "新しいペインの既定の表示"), desc: s("Used when a pane has no view of its own yet.", "아직 보기가 없는 패널에 써요.", "まだ表示を持たないペインに使います。"), badge: true, live: true, ctl: Ctl::ViewMode },
    Spec { title: s("Starting folder for a new pane", "새 패널의 시작 폴더", "新しいペインの開始フォルダー"), desc: s("Used when a new pane has no folder yet.", "새 패널에 폴더가 없을 때 열어요.", "新しいペインにフォルダーがないときに開きます。"), badge: true, live: true, ctl: Ctl::StartDir },
    Spec { title: s("Show hidden files", "숨김 파일 표시", "隠しファイルを表示"), desc: s("Same as Ctrl+H.", "Ctrl+H와 같아요.", "Ctrl+H と同じです。"), badge: false, live: true, ctl: Ctl::Toggle(TG_HIDDEN) },
    Spec { title: s("Show file extensions", "파일 확장자 표시", "ファイル拡張子を表示"), desc: s("This is Windows' own setting, so Explorer follows it too.", "Windows 설정을 바꿔요. 탐색기도 같이 바뀌어요.", "Windows の設定を変えるので、エクスプローラーも従います。"), badge: true, live: true, ctl: Ctl::Toggle(TG_EXT) },
    Spec { title: s("Show the left tree", "왼쪽 트리 표시", "左のツリーを表示"), desc: s("", "", ""), badge: false, live: true, ctl: Ctl::Toggle(TG_TREE) },
    Spec { title: s("Emphasise the active pane border", "활성 패널 테두리 강조", "アクティブなペインの枠を強調"), desc: s("", "", ""), badge: true, live: true, ctl: Ctl::Toggle(TG_EDGE) },
];
const FILE: &[Spec] = &[
    Spec { title: s("Copy / move progress", "복사·이동 진행 창", "コピー / 移動の進行状況"), desc: s("The Windows progress window runs on its own thread (always on).", "Windows 표준 진행 창을 별도 스레드에서 띄워요(항상 켜짐).", "Windows の進行状況ウィンドウを別スレッドで表示します（常にオン）。"), badge: false, live: true, ctl: Ctl::CopyEng },
    Spec { title: s("Confirm delete", "삭제할 때 확인", "削除の確認"), desc: s("Del in a pane. Shift+Del is always permanent.", "패널에서 Del. Shift+Del은 항상 완전 삭제예요.", "ペインで Del。Shift+Del は常に完全削除です。"), badge: true, live: true, ctl: Ctl::Toggle(TG_CONFIRM) },
    Spec { title: s("Default delete", "기본 삭제 방식", "既定の削除方法"), desc: s("", "", ""), badge: true, live: true, ctl: Ctl::DelMode },
    Spec { title: s("New file name", "새 파일 이름", "新しいファイル名"), desc: s("Used by New text file on the view menu. Empty follows the display language.", "보기 메뉴의 새 텍스트 파일이 이 이름을 써요. 비우면 표시 언어를 따라요.", "表示メニューの新しいテキスト ファイルが使います。空なら表示言語に従います。"), badge: true, live: true, ctl: Ctl::NewName },
    Spec { title: s("Default drag and drop", "끌어 놓기 기본 동작", "ドラッグ アンド ドロップの既定"), desc: s("The ⇄ menu. A drag inside a pane still follows Explorer.", "⇄ 메뉴에 적용돼요. 패널 안 끌어 놓기는 탐색기 그대로예요.", "⇄ メニューに適用します。ペイン内のドラッグはエクスプローラーのままです。"), badge: true, live: true, ctl: Ctl::Drop },
];
const FAV: &[Spec] = &[
    Spec { title: s("Show add or remove favourites on the right-click menu", "우클릭 메뉴에 즐겨찾기 추가/제거 표시", "右クリック メニューにお気に入りの追加と削除を表示"), desc: s("Not in this version. The shell menu is still Explorer's.", "이 버전에는 아직 없어요. 우클릭 메뉴는 탐색기 그대로예요.", "このバージョンではまだありません。右クリックはエクスプローラーのままです。"), badge: false, live: false, ctl: Ctl::Toggle(TG_FAVMENU) },
];
const SESS: &[Spec] = &[
    Spec { title: s("Session folder", "세션 폴더", "セッション フォルダー"), desc: s("The sessions folder next to the exe.", "exe 옆 sessions 폴더예요.", "exe の隣にある sessions フォルダーです。"), badge: false, live: true, ctl: Ctl::Sess },
    Spec { title: s("Save on exit", "종료할 때 자동 저장", "終了時に自動保存"), desc: s("Writes the named session file when one is open.", "열어 둔 세션 파일이 있을 때 그 파일에 저장해요.", "開いているセッション ファイルがあるとき、そのファイルに保存します。"), badge: false, live: true, ctl: Ctl::Toggle(TG_AUTOSAVE) },
    Spec { title: s("Unsaved-changes mark (green dot)", "저장 안 된 변경 표시 (초록 점)", "未保存の変更を表示（緑の点）"), desc: s("On the save button.", "저장 버튼에 표시해요.", "保存ボタンに表示します。"), badge: false, live: true, ctl: Ctl::Toggle(TG_DOT) },
];
const ADV: &[Spec] = &[
    Spec { title: s("Settings file", "설정 파일", "設定ファイル"), desc: s("portable.txt next to the exe keeps this file there.", "exe 옆 portable.txt가 있으면 이 파일을 그 옆에 둬요.", "exe の隣の portable.txt があると、このファイルをそこに置きます。"), badge: false, live: true, ctl: Ctl::Adv },
    Spec { title: s("Reset all settings", "모든 설정 초기화", "すべての設定を初期化"), desc: s("Preferences return to their defaults. The layout, folders and favourites stay.", "환경설정만 기본값으로 돌아가요. 배치, 폴더, 즐겨찾기는 남아요.", "環境設定だけ既定に戻します。レイアウト、フォルダー、お気に入りはそのままです。"), badge: false, live: true, ctl: Ctl::AdvReset },
];

struct Kr {
    cat: [&'static str; 3],
    act: [&'static str; 3],
    keys: &'static [&'static str],
}

const KEYS: &[Kr] = &[
    Kr { cat: s("Tab", "탭", "タブ"), act: s("New tab", "새 탭", "新しいタブ"), keys: &["Ctrl+T"] },
    Kr { cat: s("Tab", "탭", "タブ"), act: s("Close tab", "탭 닫기", "タブを閉じる"), keys: &["Ctrl+W"] },
    Kr { cat: s("Tab", "탭", "タブ"), act: s("Next / previous tab", "다음 / 이전 탭", "次 / 前のタブ"), keys: &["Ctrl+Tab", "Ctrl+Shift+Tab"] },
    Kr { cat: s("Tab", "탭", "タブ"), act: s("Reopen closed tab", "닫은 탭 다시 열기", "閉じたタブを開き直す"), keys: &["Ctrl+Shift+T"] },
    Kr { cat: s("Pane", "패널", "ペイン"), act: s("Next / previous pane", "다음 / 이전 패널", "次 / 前のペイン"), keys: &["Ctrl+PageDown", "Ctrl+PageUp"] },
    Kr { cat: s("Pane", "패널", "ペイン"), act: s("Go to pane 1–9", "1–9번 패널로 이동", "ペイン 1〜9 へ移動"), keys: &["Ctrl+1 … Ctrl+9"] },
    Kr { cat: s("Move", "이동", "移動"), act: s("Left / right / above / below", "왼쪽 / 오른쪽 / 위 / 아래", "左 / 右 / 上 / 下"), keys: &["Alt+Shift+←", "Alt+Shift+→", "Alt+Shift+↑", "Alt+Shift+↓"] },
    Kr { cat: s("Move", "이동", "移動"), act: s("Back / forward", "뒤로 / 앞으로", "戻る / 進む"), keys: &["Alt+←", "Alt+→"] },
    Kr { cat: s("Move", "이동", "移動"), act: s("Parent folder", "상위 폴더", "上のフォルダー"), keys: &["Alt+↑"] },
    Kr { cat: s("Move", "이동", "移動"), act: s("Recent folders", "최근 폴더", "最近のフォルダー"), keys: &["Alt+↓"] },
    Kr { cat: s("Move", "이동", "移動"), act: s("Address bar", "주소창", "アドレス バー"), keys: &["Ctrl+L", "Alt+D", "F4"] },
    Kr { cat: s("View", "보기", "表示"), act: s("Filter", "필터", "フィルター"), keys: &["Ctrl+F"] },
    Kr { cat: s("View", "보기", "表示"), act: s("Show hidden files", "숨김 파일 표시", "隠しファイルを表示"), keys: &["Ctrl+H"] },
    Kr { cat: s("View", "보기", "表示"), act: s("Maximise / restore the pane", "패널 크게 보기 / 복원", "ペインを最大化 / 元に戻す"), keys: &["Ctrl+Shift+M"] },
    Kr { cat: s("File", "파일", "ファイル"), act: s("Copy full path", "전체 경로 복사", "フルパスをコピー"), keys: &["Ctrl+Shift+C"] },
    Kr { cat: s("Session", "세션", "セッション"), act: s("Save session", "세션 저장", "セッションを保存"), keys: &["Ctrl+S"] },
    Kr { cat: s("Favourites", "즐겨찾기", "お気に入り"), act: s("Favourites list", "즐겨찾기 목록", "お気に入り一覧"), keys: &["Ctrl+B"] },
    Kr { cat: s("Help", "도움말", "ヘルプ"), act: s("Settings", "설정", "設定"), keys: &["Ctrl+,"] },
    Kr { cat: s("Help", "도움말", "ヘルプ"), act: s("Shortcut help", "단축키 도움말", "ショートカットのヘルプ"), keys: &["F1"] },
];

const KEY_NOTE: [&str; 3] = s(
    "Read only. F2, Del, Ctrl+C/X/V and F5 belong to Explorer. Changing a shortcut comes later.",
    "읽기 전용이에요. F2, Del, Ctrl+C/X/V, F5는 탐색기 동작이에요. 단축키 변경은 나중에 들어와요.",
    "読み取り専用です。F2、Del、Ctrl+C/X/V、F5 はエクスプローラーの操作です。キーの変更は後で入ります。",
);
const FAV_NOTE: [&str; 3] = s(
    "Folder groups and file favourites will be managed here once the favourites tree arrives.",
    "나중에 9b(즐겨찾기 트리)가 들어오면 여기서 폴더 그룹과 파일 즐겨찾기도 관리해요.",
    "お気に入りツリーが入ったら、ここでフォルダー グループとファイルのお気に入りも管理します。",
);
const LIST_H: [&str; 3] = s("List", "목록", "一覧");
const ABOUT: &[Spec] = &[Spec {
    title: s("grids", "grids", "grids"),
    desc: s("Upstream efww/grids · fork Samek86/grids", "원본 efww/grids · 포크 Samek86/grids", "上流 efww/grids · フォーク Samek86/grids"),
    badge: false,
    live: true,
    ctl: Ctl::About,
}];

fn page_cards(p: usize) -> &'static [Spec] {
    match p {
        0 => GEN,
        1 => LOOK,
        2 => VIEW,
        3 => FILE,
        4 => FAV,
        6 => SESS,
        7 => ADV,
        8 => ABOUT,
        _ => &[],
    }
}

fn has(hay: &str, q: &str) -> bool {
    q.is_empty() || hay.to_lowercase().contains(&q.to_lowercase())
}

fn card_hit(c: &Spec, q: &str) -> bool {
    has(tx(c.title), q) || has(tx(c.desc), q)
}

fn key_hit(k: &Kr, q: &str) -> bool {
    has(tx(k.cat), q) || has(tx(k.act), q) || k.keys.iter().any(|s| has(s, q))
}

fn page_match(q: &str, page: usize) -> bool {
    if q.is_empty() || has(tx(PAGES[page]), q) {
        return true;
    }
    if page_cards(page).iter().any(|c| card_hit(c, q)) {
        return true;
    }
    if page == 4 {
        return app(|a| a.s.favs.iter().any(|f| has(f, q) || has(&super::short_name(f), q))) || has(tx(LIST_H), q);
    }
    if page == 5 {
        return KEYS.iter().any(|k| key_hit(k, q)) || has(tx(KEY_NOTE), q);
    }
    false
}

/// Empty when the page title already matched, so the whole page stays visible.
fn row_q(q: &str, page: usize) -> &str {
    if q.is_empty() || has(tx(PAGES[page]), q) {
        ""
    } else {
        q
    }
}

struct St {
    hwnd: HWND,
    page: usize,
    scroll: i32,
    scroll_max: i32,
    dpi: i32,
    query: String,
    keyq: String,
    hot: Option<usize>,
    font: HFONT,
    small: HFONT,
    title_font: HFONT,
    mono: HFONT,
    icon: HFONT,
    track: RECT,
    thumb_h: i32,
    drag: Option<i32>,
    fav_drag: Option<usize>,
}

impl Default for St {
    fn default() -> Self {
        St {
            hwnd: HWND::default(),
            page: 0,
            scroll: 0,
            scroll_max: 0,
            dpi: 96,
            query: String::new(),
            keyq: String::new(),
            hot: None,
            font: HFONT::default(),
            small: HFONT::default(),
            title_font: HFONT::default(),
            mono: HFONT::default(),
            icon: HFONT::default(),
            track: RECT::default(),
            thumb_h: 0,
            drag: None,
            fav_drag: None,
        }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

struct Snap {
    page: usize,
    scroll: i32,
    dpi: i32,
    query: String,
    keyq: String,
    hot: Option<usize>,
    font: HFONT,
    small: HFONT,
    title_font: HFONT,
    mono: HFONT,
    icon: HFONT,
}

fn snap() -> Snap {
    ST.with(|s| {
        let st = s.borrow();
        Snap {
            page: st.page,
            scroll: st.scroll,
            dpi: st.dpi,
            query: st.query.clone(),
            keyq: st.keyq.clone(),
            hot: st.hot,
            font: st.font,
            small: st.small,
            title_font: st.title_font,
            mono: st.mono,
            icon: st.icon,
        }
    })
}

fn hwnd() -> HWND {
    ST.with(|s| s.borrow().hwnd)
}

struct CardItem {
    spec: Spec,
    rect: RECT,
    ctl: RECT,
    sw: Vec<RECT>,
    adv: [RECT; 4],
}

enum Item {
    Card(CardItem),
    Head(RECT, [&'static str; 3]),
    Table(RECT),
    FavHead(RECT),
    FavRow { i: usize, rect: RECT, rename: RECT, btn: RECT },
    Note(RECT, [&'static str; 3]),
    KeyBar { edit: RECT, reset: RECT },
    KeyHead(RECT),
    KeyRow { i: usize, rect: RECT },
}

struct Geo {
    nav_w: i32,
    nav: [RECT; 9],
    show: [bool; 9],
    search: RECT,
    content: RECT,
    title: RECT,
    footer: RECT,
    close: RECT,
    hint: RECT,
    items: Vec<Item>,
    scroll_max: i32,
    track: RECT,
    thumb: RECT,
    has_bar: bool,
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn right_box(card: RECT, w: i32, h: i32, pad: i32) -> RECT {
    let cy = (card.top + card.bottom) / 2;
    RECT { left: card.right - pad - w, top: cy - h / 2, right: card.right - pad, bottom: cy + h / 2 }
}

fn card_h(spec: &Spec, dpi: i32) -> i32 {
    // 12px vertical padding, 14px label, optional 12px description.
    // The settings-file card adds a line for the live path.
    if spec.ctl == Ctl::Adv {
        sc(dpi, 84)
    } else if spec.desc[0].is_empty() {
        sc(dpi, 48)
    } else {
        sc(dpi, 64)
    }
}

fn geo(rc: RECT, sn: &Snap) -> Geo {
    let dpi = sn.dpi.max(96);
    let nav_w = sc(dpi, 220);
    let foot_h = sc(dpi, 56);
    let footer = RECT { left: 0, top: rc.bottom - foot_h, right: rc.right, bottom: rc.bottom };
    let content = RECT { left: nav_w, top: 0, right: rc.right, bottom: footer.top };
    let search = RECT { left: sc(dpi, 8), top: sc(dpi, 12), right: nav_w - sc(dpi, 8), bottom: sc(dpi, 44) };
    let close = right_box(footer, sc(dpi, 88), sc(dpi, 32), sc(dpi, 16));
    let hint = RECT { left: sc(dpi, 16), top: footer.top, right: close.left - sc(dpi, 8), bottom: footer.bottom };
    let q = sn.query.as_str();
    let mut show = [false; 9];
    let mut nav = [RECT::default(); 9];
    let mut y = search.bottom + sc(dpi, 8);
    for i in 0..9 {
        show[i] = page_match(q, i);
        if !show[i] {
            continue;
        }
        nav[i] = RECT { left: sc(dpi, 8), top: y, right: nav_w - sc(dpi, 8), bottom: y + sc(dpi, 36) };
        y += sc(dpi, 36);
    }
    let mut g = Geo {
        nav_w,
        nav,
        show,
        search,
        content,
        title: RECT::default(),
        footer,
        close,
        hint,
        items: Vec::new(),
        scroll_max: 0,
        track: RECT::default(),
        thumb: RECT::default(),
        has_bar: false,
    };
    // Lay out once to learn the height, then again inset for the scrollbar if needed.
    // A scrollbar narrows the shortcut column, and that can wrap another chord line,
    // so the height that is painted (not the wider first pass) decides the scroll range.
    let natural = fill_items(&mut g, sn, false);
    let view_h = (content.bottom - content.top).max(1);
    let mut has_bar = natural > view_h;
    g.items.clear();
    let mut content_h = fill_items(&mut g, sn, has_bar);
    if !has_bar && content_h > view_h {
        has_bar = true;
        g.items.clear();
        content_h = fill_items(&mut g, sn, true);
    }
    g.has_bar = has_bar;
    g.scroll_max = (content_h - view_h).max(0);
    if g.has_bar {
        let track = RECT {
            left: rc.right - sc(dpi, 10),
            top: content.top + sc(dpi, 8),
            right: rc.right - sc(dpi, 4),
            bottom: content.bottom - sc(dpi, 8),
        };
        let span = (track.bottom - track.top).max(1);
        let thumb_h = (span * view_h / content_h.max(view_h)).clamp(sc(dpi, 24), span);
        let travel = (span - thumb_h).max(1);
        let top = track.top + if g.scroll_max == 0 { 0 } else { sn.scroll.clamp(0, g.scroll_max) * travel / g.scroll_max };
        g.thumb = RECT { left: track.left, top, right: track.right, bottom: top + thumb_h };
        g.track = track;
    }
    g
}

fn fill_items(g: &mut Geo, sn: &Snap, bar: bool) -> i32 {
    let dpi = sn.dpi.max(96);
    let pad = sc(dpi, 28);
    let right_pad = if bar { sc(dpi, 18) } else { 0 };
    let left = g.content.left + pad;
    let right = g.content.right - pad - right_pad;
    let start = g.content.top + sc(dpi, 16);
    let mut y = start - sn.scroll;
    g.title = RECT { left, top: y, right, bottom: y + sc(dpi, 32) };
    y += sc(dpi, 32) + sc(dpi, 16);
    let page = sn.page;
    let rq = row_q(&sn.query, page);
    let gap = sc(dpi, 4);
    for spec in page_cards(page) {
        if !rq.is_empty() && !card_hit(spec, rq) {
            continue;
        }
        let h = card_h(spec, dpi);
        let rect = RECT { left, top: y, right, bottom: y + h };
        g.items.push(Item::Card(card_item(rect, *spec, dpi)));
        y += h + gap;
    }
    if page == 4 {
        y = fav_items(g, sn, left, right, y, rq);
    }
    if page == 5 {
        y = key_items(g, sn, left, right, y, rq);
    }
    (y + sn.scroll) - start + sc(dpi, 16)
}

fn card_item(rect: RECT, spec: Spec, dpi: i32) -> CardItem {
    let pad = sc(dpi, 16);
    let mut sw = Vec::new();
    let mut adv = [RECT::default(); 4];
    let ctl = match spec.ctl {
        Ctl::Toggle(_) => right_box(rect, sc(dpi, 40), sc(dpi, 20), pad),
        Ctl::CopyEng => right_box(rect, sc(dpi, 140), sc(dpi, 22), pad),
        Ctl::Lang | Ctl::Start | Ctl::Theme | Ctl::Font | Ctl::ViewMode | Ctl::StartDir | Ctl::DelMode | Ctl::NewName | Ctl::Drop => {
            right_box(rect, sc(dpi, 230), sc(dpi, 32), pad)
        }
        Ctl::Sess => right_box(rect, sc(dpi, 108), sc(dpi, 32), pad),
        Ctl::About => right_box(rect, sc(dpi, 120), sc(dpi, 32), pad),
        Ctl::Accent => {
            let btn = right_box(rect, sc(dpi, 108), sc(dpi, 32), pad);
            let mut x = btn.left - sc(dpi, 10) - sc(dpi, 22);
            for _ in 0..6 {
                sw.push(RECT { left: x, top: (rect.top + rect.bottom) / 2 - sc(dpi, 11), right: x + sc(dpi, 22), bottom: (rect.top + rect.bottom) / 2 + sc(dpi, 11) });
                x -= sc(dpi, 28);
            }
            sw.reverse();
            RECT { left: sw.first().map(|r| r.left).unwrap_or(btn.left), ..btn }
        }
        Ctl::Adv | Ctl::AdvReset => {
            let n = if spec.ctl == Ctl::Adv { 3 } else { 1 };
            let bw = sc(dpi, if spec.ctl == Ctl::Adv { 108 } else { 96 });
            let gap = sc(dpi, 6);
            let total = bw * n + gap * (n - 1);
            let mut x = rect.right - pad - total;
            // The settings-file card is taller so the path can use the full width under the buttons.
            let top = if spec.ctl == Ctl::Adv { rect.top + sc(dpi, 14) } else { (rect.top + rect.bottom) / 2 - sc(dpi, 16) };
            for i in 0..n as usize {
                adv[i] = RECT { left: x, top, right: x + bw, bottom: top + sc(dpi, 32) };
                x += bw + gap;
            }
            RECT { left: adv[0].left, top: adv[0].top, right: rect.right - pad, bottom: adv[0].bottom }
        }
    };
    // The accent button is the right-hand part of `ctl`; keep that as the button rect via adv[0] unused.
    // Swatches live in `sw`. The button itself is ctl's right sc(108).
    let ctl = if spec.ctl == Ctl::Accent {
        RECT { left: ctl.right - sc(dpi, 108), ..ctl }
    } else {
        ctl
    };
    CardItem { spec, rect, ctl, sw, adv }
}

fn fav_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    let dpi = sn.dpi.max(96);
    let favs: Vec<String> = app(|a| a.s.favs.clone());
    let rows: Vec<usize> = favs.iter().enumerate().filter(|(_, f)| rq.is_empty() || has(f, rq) || has(&super::short_name(f), rq)).map(|(i, _)| i).collect();
    if !rq.is_empty() && rows.is_empty() && !has(tx(LIST_H), rq) && !has(tx(FAV_NOTE), rq) {
        return y;
    }
    g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, LIST_H));
    y += sc(dpi, 28);
    let head_h = sc(dpi, 32);
    let row_h = sc(dpi, 40);
    let table = RECT { left, top: y, right, bottom: y + head_h + row_h * if rows.is_empty() { 1 } else { rows.len() as i32 } };
    g.items.push(Item::Table(table));
    g.items.push(Item::FavHead(RECT { left, top: y, right, bottom: y + head_h }));
    y += head_h;
    if rows.is_empty() {
        g.items.push(Item::FavRow { i: usize::MAX, rect: RECT { left, top: y, right, bottom: y + row_h }, rename: RECT::default(), btn: RECT::default() });
        y += row_h;
    }
    for i in rows {
        let del = RECT { left: right - sc(dpi, 16) - sc(dpi, 72), top: y + sc(dpi, 6), right: right - sc(dpi, 16), bottom: y + row_h - sc(dpi, 6) };
        let rename = RECT { left: del.left - sc(dpi, 8) - sc(dpi, 88), top: del.top, right: del.left - sc(dpi, 8), bottom: del.bottom };
        g.items.push(Item::FavRow { i, rect: RECT { left, top: y, right, bottom: y + row_h }, rename, btn: del });
        y += row_h;
    }
    y += sc(dpi, 8);
    g.items.push(Item::Note(RECT { left, top: y, right, bottom: y + sc(dpi, 36) }, FAV_NOTE));
    y + sc(dpi, 36)
}

fn key_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    let dpi = sn.dpi.max(96);
    // Both queries apply: a key typed in the page box, and a nav query that did not match the title.
    let rows: Vec<usize> = KEYS.iter().enumerate().filter(|(_, k)| {
        (rq.is_empty() || key_hit(k, rq)) && (sn.keyq.is_empty() || key_hit(k, &sn.keyq))
    }).map(|(i, _)| i).collect();
    let reset = right_box(RECT { left, top: y, right, bottom: y + sc(dpi, 40) }, sc(dpi, 210), sc(dpi, 32), 0);
    let edit = RECT { left, top: reset.top, right: reset.left - sc(dpi, 8), bottom: reset.bottom };
    g.items.push(Item::KeyBar { edit, reset });
    y += sc(dpi, 48);
    let head_h = sc(dpi, 32);
    let table_top = y;
    let probe = RECT { left, top: 0, right, bottom: 0 };
    let heights = unsafe { key_row_heights(&rows, probe, dpi, sn.mono) };
    let body: i32 = heights.iter().sum();
    let table = RECT { left, top: table_top, right, bottom: table_top + head_h + body };
    g.items.push(Item::Table(table));
    g.items.push(Item::KeyHead(RECT { left, top: table_top, right, bottom: table_top + head_h }));
    y = table_top + head_h;
    if rows.is_empty() {
        let h = heights.first().copied().unwrap_or(sc(dpi, 40));
        g.items.push(Item::KeyRow { i: usize::MAX, rect: RECT { left, top: y, right, bottom: y + h } });
        y += h;
    }
    for (n, i) in rows.into_iter().enumerate() {
        let h = heights.get(n).copied().unwrap_or(sc(dpi, 40));
        g.items.push(Item::KeyRow { i, rect: RECT { left, top: y, right, bottom: y + h } });
        y += h;
    }
    y += sc(dpi, 8);
    g.items.push(Item::Note(RECT { left, top: y, right, bottom: y + sc(dpi, 40) }, KEY_NOTE));
    y + sc(dpi, 40)
}

fn toggle_on(id: u8) -> bool {
    match id {
        TG_SINGLE => app(|a| a.s.single),
        TG_DBLUP => app(|a| a.s.dblup),
        TG_HIDDEN => unsafe { super::hidden() },
        TG_BAR => app(|a| a.s.bar),
        TG_STATUS => app(|a| a.s.stat),
        TG_EXT => app(|a| a.s.ext),
        TG_TREE => app(|a| a.s.tree_on),
        TG_EDGE => app(|a| a.s.edge),
        TG_CONFIRM => app(|a| a.s.confirm),
        TG_AUTOSAVE => app(|a| a.s.autosave),
        TG_DOT => app(|a| a.s.dot),
        // The shell context menu is still Explorer's.
        TG_FAVMENU => false,
        _ => false,
    }
}

fn ctl_id(c: Ctl) -> Option<i32> {
    Some(match c {
        Ctl::Lang => S_LANG,
        Ctl::Start => S_START,
        Ctl::Theme => S_THEME,
        Ctl::Font => S_FONT,
        Ctl::ViewMode => S_VIEW,
        Ctl::StartDir => S_DIR,
        Ctl::DelMode => S_DEL,
        Ctl::NewName => S_NEW,
        Ctl::Drop => S_DROP,
        Ctl::Sess => S_SESS,
        Ctl::About => S_GITHUB,
        Ctl::Accent => S_ACCENT,
        _ => return None,
    })
}

pub unsafe fn open(owner: HWND) {
    ensure_class();
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(h);
        return;
    }
    let dpi = GetDpiForWindow(owner) as i32;
    let (ww, hh) = (sc(dpi.max(96), 1000), sc(dpi.max(96), 640));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE;
    let _ = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsSettings"), w!("grids"), style, x, y, ww, hh, Some(owner), None, None, None);
}

pub unsafe fn close() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub unsafe fn captures(h: HWND) -> bool {
    let w = hwnd();
    !w.0.is_null() && IsWindow(Some(w)).as_bool() && (h == w || IsChild(w, h).as_bool())
}

pub unsafe fn redraw() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = InvalidateRect(Some(h), None, false);
    }
}

pub unsafe fn refresh() {
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    style_window(h);
    rebuild_fonts();
    let font = ST.with(|s| s.borrow().font);
    each_child(h, |c| {
        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        theme_ctl(c);
    });
    fill_strings(h);
    let _ = SetWindowTextW(h, &HSTRING::from(tx(TITLE)));
    cue(h, S_SEARCH, tx(FIND));
    cue(h, S_KEYQ, tx(s("Search actions or keys", "동작 또는 키 검색", "操作またはキーを検索")));
    layout(h);
    let _ = InvalidateRect(Some(h), None, true);
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(settings_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: w!("GridsSettings"),
            ..Default::default()
        });
    });
}

unsafe fn style_window(h: HWND) {
    let dark = super::DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = super::cref(super::pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
}

fn make_font(face: &str, px: i32, weight: i32, dpi: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -(px * dpi / 96).max(1),
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn rebuild_fonts() {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let face = super::ui_face();
    let scale = match app(|a| a.s.font) {
        1 => 125,
        2 => 150,
        _ => 100,
    };
    let px = |n: i32| (n * scale / 100).max(1);
    let (title_face, title_w) = match super::ui() {
        1 => ("Malgun Gothic", 700), // no semibold; bold is the Korean title face
        2 => ("Yu Gothic UI", 700),
        _ => ("Segoe UI Semibold", 600),
    };
    let new = (
        make_font(face, px(14), 400, dpi),
        make_font(face, px(12), 400, dpi),
        make_font(title_face, px(24), title_w, dpi),
        make_font("Consolas", 12, 400, dpi),
        make_font("Segoe MDL2 Assets", 14, 400, dpi),
    );
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.small, st.title_font, st.mono, st.icon);
        (st.font, st.small, st.title_font, st.mono, st.icon) = new;
        old
    });
    unsafe {
        for f in [old.0, old.1, old.2, old.3, old.4] {
            if !f.is_invalid() {
                let _ = DeleteObject(f.into());
            }
        }
    }
}

unsafe fn theme_ctl(h: HWND) {
    let dark = super::DARK_ON.load(Ordering::Relaxed);
    let name = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

fn child_at(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    unsafe { super::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font) }
}

unsafe fn build_children(h: HWND) {
    let font = ST.with(|s| s.borrow().font);
    let tab = 0x00010000u32;
    let dis = 0x08000000u32;
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_SEARCH, font);
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_KEYQ, font);
    let combo = |id, on| child_at(h, w!("COMBOBOX"), "", 3 | 0x200 | 0x00200000 | tab | if on { 0 } else { dis }, id, font);
    combo(S_LANG, true);
    combo(S_THEME, true);
    for id in [S_START, S_FONT, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP] {
        combo(id, true);
    }
    let btn = |id, label: &str, on| child_at(h, w!("BUTTON"), label, 0xB | tab | if on { 0 } else { dis }, id, font);
    btn(S_CLOSE, tx(CLOSE_L), true);
    btn(S_RESET, tx(s("Reset all to defaults", "기본값으로 모두 되돌리기", "すべて既定に戻す")), false);
    btn(S_SESS, tx(s("Open folder", "폴더 열기", "フォルダーを開く")), true);
    btn(S_ACCENT, tx(s("Custom…", "직접 지정…", "直接指定…")), true);
    btn(S_GITHUB, tx(s("Open GitHub", "GitHub 열기", "GitHub を開く")), true);
    btn(S_ADV_OPEN, tx(s("Open folder", "폴더 열기", "フォルダーを開く")), true);
    btn(S_ADV_EX, tx(s("Export", "내보내기", "エクスポート")), true);
    btn(S_ADV_IM, tx(s("Import", "가져오기", "インポート")), true);
    btn(S_ADV_RESET, tx(s("Reset…", "초기화…", "初期化…")), true);
}

fn each_child(h: HWND, mut f: impl FnMut(HWND)) {
    let ids = [S_SEARCH, S_CLOSE, S_KEYQ, S_RESET, S_LANG, S_START, S_THEME, S_FONT, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_SESS, S_ACCENT, S_GITHUB, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET];
    for id in ids {
        let c = unsafe { GetDlgItem(Some(h), id).unwrap_or_default() };
        if !c.0.is_null() {
            f(c);
        }
    }
    for i in 0..256 {
        let c = unsafe { GetDlgItem(Some(h), S_FAV0 + i).unwrap_or_default() };
        if c.0.is_null() {
            continue;
        }
        f(c);
    }
}

unsafe fn fill_strings(h: HWND) {
    let auto = {
        let name = match Lang::Auto.resolve() {
            1 => "한국어",
            2 => "日本語",
            _ => "English",
        };
        format!("{} ({name})", tx(s("Follow Windows", "Windows 설정 따르기", "Windows の設定に従う")))
    };
    let lang = app(|a| a.s.lang);
    let lang_i = match lang {
        Lang::Auto => 0,
        Lang::En => 1,
        Lang::Ko => 2,
        Lang::Ja => 3,
    };
    fill_combo(h, S_LANG, &[auto, "English".into(), "한국어".into(), "日本語".into()], lang_i);
    let theme = app(|a| a.s.theme) as usize;
    fill_combo(h, S_THEME, &[tx(s("Follow Windows", "Windows 설정 따르기", "Windows の設定に従う")).into(), tx(s("Light", "라이트", "ライト")).into(), tx(s("Dark", "다크", "ダーク")).into()], theme.min(2));
    let boot = app(|a| a.s.boot) as usize;
    fill_combo(h, S_START, &[tx(s("Restore last state", "마지막 상태 복원", "前回の状態を復元")).into(), tx(s("Open a session…", "세션 열기…", "セッションを開く…")).into(), tx(s("Default layout (2×2)", "기본 배치(2×2)", "既定のレイアウト (2×2)")).into()], boot.min(2));
    let font = app(|a| a.s.font) as usize;
    fill_combo(h, S_FONT, &[tx(s("Default", "기본", "既定")).into(), tx(s("Large", "크게", "大きく")).into(), tx(s("Larger", "더 크게", "さらに大きく")).into()], font.min(2));
    let view_i = match app(|a| a.s.view_new) { 3 => 1, 1 => 2, _ => 0 };
    fill_combo(h, S_VIEW, &[tx(s("Details", "자세히", "詳細")).into(), tx(s("List", "목록", "一覧")).into(), tx(s("Large icons", "큰 아이콘", "大きなアイコン")).into()], view_i);
    let start = app(|a| a.s.start_new) as usize;
    fill_combo(h, S_DIR, &[tx(s("This PC", "내 PC", "PC")).into(), tx(s("Desktop", "바탕 화면", "デスクトップ")).into(), tx(s("Choose a folder…", "지정 폴더…", "フォルダーを指定…")).into()], start.min(2));
    let del = app(|a| a.s.del) as usize;
    fill_combo(h, S_DEL, &[tx(s("Recycle Bin", "휴지통으로", "ごみ箱へ")).into(), tx(s("Delete permanently", "완전 삭제", "完全に削除")).into()], del.min(1));
    let custom = app(|a| a.s.new_name.clone());
    let new_label = if custom.is_empty() { tx(s("Custom…", "직접 지정…", "直接指定…")).to_string() } else { custom };
    let new_i = usize::from(!app(|a| a.s.new_name.is_empty()));
    fill_combo(h, S_NEW, &[tx(s("New Text Document.txt", "새 텍스트 문서.txt", "新しいテキスト ドキュメント.txt")).into(), new_label], new_i);
    let dropm = app(|a| a.s.dropm) as usize;
    fill_combo(h, S_DROP, &[tx(s("Windows rules (same drive = move)", "Windows 규칙(같은 드라이브=이동)", "Windows の規則（同じドライブ=移動）")).into(), tx(s("Always copy", "항상 복사", "常にコピー")).into(), tx(s("Always ask", "항상 묻기", "常に確認")).into()], dropm.min(2));
    let labels = [
        (S_CLOSE, tx(CLOSE_L)),
        (S_RESET, tx(s("Reset all to defaults", "기본값으로 모두 되돌리기", "すべて既定に戻す"))),
        (S_SESS, tx(s("Open folder", "폴더 열기", "フォルダーを開く"))),
        (S_ACCENT, tx(s("Custom…", "직접 지정…", "直接指定…"))),
        (S_GITHUB, tx(s("Open GitHub", "GitHub 열기", "GitHub を開く"))),
        (S_ADV_OPEN, tx(s("Open folder", "폴더 열기", "フォルダーを開く"))),
        (S_ADV_EX, tx(s("Export", "내보내기", "エクスポート"))),
        (S_ADV_IM, tx(s("Import", "가져오기", "インポート"))),
        (S_ADV_RESET, tx(s("Reset…", "초기화…", "初期化…"))),
    ];
    for (id, label) in labels {
        let _ = SetWindowTextW(ctl(h, id), &HSTRING::from(label));
    }
}

unsafe fn fill_combo(h: HWND, id: i32, items: &[String], sel: usize) {
    let c = ctl(h, id);
    if c.0.is_null() {
        return;
    }
    BUSY.with(|b| b.set(true));
    SendMessageW(c, CB_RESETCONTENT, None, None);
    for item in items {
        let hs = HSTRING::from(item.as_str());
        SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    SendMessageW(c, CB_SETCURSEL, Some(WPARAM(sel)), None);
    SendMessageW(c, CB_SETMINVISIBLE, Some(WPARAM(8)), None);
    BUSY.with(|b| b.set(false));
}

unsafe fn cue(h: HWND, id: i32, text: &str) {
    let c = ctl(h, id);
    let hs = HSTRING::from(text);
    SendMessageW(c, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let m = sc(dpi, 8) as isize;
    SendMessageW(c, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16)))); // EM_SETMARGINS
}

fn ctl(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn layout(h: HWND) {
    let mut sn = snap();
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    let mut g = geo(rc, &sn);
    if sn.scroll > g.scroll_max {
        sn.scroll = g.scroll_max;
        ST.with(|s| s.borrow_mut().scroll = sn.scroll);
        g = geo(rc, &sn);
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.scroll_max = g.scroll_max;
        st.track = g.track;
        st.thumb_h = g.thumb.bottom - g.thumb.top;
    });
    unsafe { place(h, &g, &sn); }
}

unsafe fn place(h: HWND, g: &Geo, sn: &Snap) {
    let hide = |id: i32| {
        let c = ctl(h, id);
        if !c.0.is_null() {
            let _ = ShowWindow(c, SW_HIDE);
        }
    };
    for id in [S_KEYQ, S_RESET, S_LANG, S_START, S_THEME, S_FONT, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_SESS, S_ACCENT, S_GITHUB, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET] {
        hide(id);
    }
    let n = app(|a| a.s.favs.len()) as i32;
    for base in [S_FAV0, S_FAV0 + 128] {
        for i in 0..128 {
            let c = ctl(h, base + i);
            if c.0.is_null() {
                continue;
            }
            if i >= n {
                let _ = DestroyWindow(c);
            } else {
                let _ = ShowWindow(c, SW_HIDE);
            }
        }
    }
    move_to(ctl(h, S_SEARCH), inset(g.search, sc(sn.dpi, 2)), true);
    move_to(ctl(h, S_CLOSE), g.close, true);
    let content = g.content;
    for item in &g.items {
        match item {
            Item::Card(c) => {
                if let Some(id) = ctl_id(c.spec.ctl) {
                    move_to(ctl(h, id), c.ctl, intersects(c.ctl, content));
                }
                if c.spec.ctl == Ctl::Adv {
                    let ids = [S_ADV_OPEN, S_ADV_EX, S_ADV_IM];
                    for (i, id) in ids.iter().enumerate() {
                        move_to(ctl(h, *id), c.adv[i], intersects(c.adv[i], content));
                    }
                }
                if c.spec.ctl == Ctl::AdvReset {
                    move_to(ctl(h, S_ADV_RESET), c.adv[0], intersects(c.adv[0], content));
                }
            }
            Item::KeyBar { edit, reset } => {
                move_to(ctl(h, S_KEYQ), inset(*edit, sc(sn.dpi, 2)), intersects(*edit, content));
                move_to(ctl(h, S_RESET), *reset, intersects(*reset, content));
            }
            Item::FavRow { i, rename, btn, .. } if *i != usize::MAX && btn.right > btn.left => {
                let id = S_FAV0 + *i as i32;
                let mut b = ctl(h, id);
                if b.0.is_null() {
                    b = child_at(h, w!("BUTTON"), tx(s("Delete", "삭제", "削除")), 0xB | 0x00010000, id, sn.font);
                }
                let _ = SetWindowTextW(b, &HSTRING::from(tx(s("Delete", "삭제", "削除"))));
                move_to(b, *btn, intersects(*btn, content));
                let rid = S_FAV0 + 128 + *i as i32;
                let mut r = ctl(h, rid);
                if r.0.is_null() {
                    r = child_at(h, w!("BUTTON"), tx(s("Rename", "이름 변경", "名前の変更")), 0xB | 0x00010000, rid, sn.font);
                }
                let _ = SetWindowTextW(r, &HSTRING::from(tx(s("Rename", "이름 변경", "名前の変更"))));
                move_to(r, *rename, intersects(*rename, content));
            }
            _ => {}
        }
    }
}

fn intersects(a: RECT, b: RECT) -> bool {
    a.right > a.left && a.bottom > a.top && a.bottom > b.top && a.top < b.bottom && a.right > b.left && a.left < b.right
}

fn inset(r: RECT, d: i32) -> RECT {
    RECT { left: r.left + d, top: r.top + d, right: (r.right - d).max(r.left + d), bottom: (r.bottom - d).max(r.top + d) }
}

unsafe fn move_to(h: HWND, r: RECT, show: bool) {
    if h.0.is_null() {
        return;
    }
    if !show {
        let _ = ShowWindow(h, SW_HIDE);
        return;
    }
    let _ = SetWindowPos(h, None, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

unsafe fn paint(h: HWND) {
    let sn = snap();
    super::buffered(h, |dc, rc| {
        let g = geo(rc, &sn);
        draw(dc, rc, &g, &sn);
    });
}

unsafe fn draw(dc: HDC, rc: RECT, g: &Geo, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    let accent = chrome();
    super::fill(dc, rc, super::pal().bg);
    super::fill(dc, RECT { bottom: g.footer.top + 1, top: g.footer.top, ..g.footer }, super::pal().line);
    rfill(dc, g.search, sc(dpi, 4), super::pal().card, Some(super::pal().line));
    if !g.show.iter().any(|s| *s) {
        super::text(dc, RECT { left: sc(dpi, 16), top: g.search.bottom + sc(dpi, 12), right: g.nav_w - sc(dpi, 8), bottom: g.search.bottom + sc(dpi, 40) }, tx(NONE), sn.font, super::pal().sub, DT_LEFT);
    }
    for i in 0..9 {
        if !g.show[i] {
            continue;
        }
        let r = g.nav[i];
        let on = i == sn.page;
        if on || sn.hot == Some(i) {
            let t = if on { 0.08 } else { 0.04 };
            rfill(dc, r, sc(dpi, 4), super::blend(super::pal().bg, super::pal().text, t), None);
        }
        if on {
            let bar = RECT { left: r.left, top: r.top + sc(dpi, 10), right: r.left + sc(dpi, 3), bottom: r.bottom - sc(dpi, 10) };
            rfill(dc, bar, sc(dpi, 2), accent, None);
        }
        super::text(dc, RECT { left: r.left + sc(dpi, 14), ..r }, tx(PAGES[i]), sn.font, super::pal().text, DT_LEFT);
    }
    super::text(dc, g.hint, tx(FOOT), sn.small, super::pal().sub, DT_LEFT);
    let _ = SaveDC(dc);
    let _ = IntersectClipRect(dc, g.content.left, g.content.top, g.content.right, g.content.bottom);
    let title = tx(PAGES[sn.page]);
    super::text(dc, g.title, title, sn.title_font, super::pal().text, DT_LEFT);
    for item in &g.items {
        match item {
            Item::Card(c) => draw_card(dc, c, sn, accent),
            Item::Head(r, t) => super::text(dc, *r, tx(*t), sn.font, super::pal().text, DT_LEFT),
            Item::Table(r) => rfill(dc, *r, sc(dpi, 6), super::pal().card, Some(super::pal().line)),
            Item::FavHead(r) => {
                super::text(dc, RECT { left: r.left + sc(dpi, 40), right: r.left + (r.right - r.left) * 32 / 100, ..*r }, tx(s("Name", "이름", "名前")), sn.small, super::pal().sub, DT_LEFT);
                super::text(dc, RECT { left: r.left + (r.right - r.left) * 32 / 100, ..*r }, tx(s("Path", "경로", "パス")), sn.small, super::pal().sub, DT_LEFT);
                super::fill(dc, RECT { top: r.bottom - 1, ..*r }, super::pal().line);
            }
            Item::FavRow { i, rect, .. } => draw_fav_row(dc, *i, *rect, sn),
            Item::Note(r, t) => super::text(dc, *r, tx(*t), sn.small, super::pal().sub, DT_LEFT),
            Item::KeyBar { edit, .. } => rfill(dc, *edit, sc(dpi, 4), super::pal().card, Some(super::pal().line)),
            Item::KeyHead(r) => {
                let (a, b) = key_cols(*r);
                super::text(dc, a, tx(s("Category", "분류", "分類")), sn.small, super::pal().sub, DT_LEFT);
                super::text(dc, RECT { left: a.right, right: b, ..*r }, tx(s("Action", "동작", "操作")), sn.small, super::pal().sub, DT_LEFT);
                super::text(dc, RECT { left: b, ..*r }, tx(s("Shortcut", "단축키", "ショートカット")), sn.small, super::pal().sub, DT_LEFT);
                super::fill(dc, RECT { top: r.bottom - 1, ..*r }, super::pal().line);
            }
            Item::KeyRow { i, rect } => draw_key_row(dc, *i, *rect, sn),
        }
    }
    if g.has_bar {
        rfill(dc, g.track, sc(dpi, 4), super::blend(super::pal().bg, super::pal().text, 0.08), None);
        rfill(dc, g.thumb, sc(dpi, 4), super::blend(super::pal().bg, super::pal().text, 0.35), None);
    }
    let _ = RestoreDC(dc, -1);
}

fn key_cols(r: RECT) -> (RECT, i32) {
    let w = r.right - r.left;
    let a = RECT { left: r.left + (w * 3 / 100).max(8), right: r.left + w * 22 / 100, ..r };
    (a, r.left + w * 50 / 100)
}

fn chip_h(dpi: i32) -> i32 {
    sc(dpi, 24)
}

fn chord_gap(dpi: i32) -> i32 {
    sc(dpi, 6)
}

fn chord_inset(dpi: i32) -> i32 {
    sc(dpi, 12)
}

/// Shortcut column: x where chips start, and the width they must stay inside.
fn chord_box(r: RECT, dpi: i32) -> (i32, i32) {
    let (_, keys_x) = key_cols(r);
    (keys_x, (r.right - chord_inset(dpi) - keys_x).max(sc(dpi, 48)))
}

/// Pack chip widths onto lines that stay within `max_w`. A chip wider than the
/// cell is clamped so the row cannot paint past the table's right edge.
fn pack_chords(widths: &[i32], max_w: i32, gap: i32) -> Vec<Vec<i32>> {
    let max_w = max_w.max(1);
    let mut lines: Vec<Vec<i32>> = Vec::new();
    let mut line: Vec<i32> = Vec::new();
    let mut used = 0i32;
    for &raw in widths {
        let w = raw.clamp(1, max_w);
        if !line.is_empty() && used.saturating_add(gap).saturating_add(w) > max_w {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(w);
        used = if line.len() == 1 { w } else { used.saturating_add(gap).saturating_add(w) };
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn key_row_h_from_lines(n: i32, dpi: i32) -> i32 {
    let n = n.max(1);
    let min_h = sc(dpi, 40);
    if n == 1 {
        return min_h;
    }
    let pad = sc(dpi, 8);
    let gap = sc(dpi, 4);
    (pad * 2 + n * chip_h(dpi) + (n - 1) * gap).max(min_h)
}

unsafe fn chord_widths(dc: HDC, font: HFONT, keys: &[&str], dpi: i32) -> Vec<i32> {
    let pad = sc(dpi, 14);
    keys.iter()
        .map(|k| {
            let tw = if font.is_invalid() || dc.is_invalid() {
                sc(dpi, 8) * k.chars().count() as i32
            } else {
                text_w(dc, font, k)
            };
            (tw + pad).max(pad)
        })
        .collect()
}

unsafe fn key_row_height(dc: HDC, keys: &[&str], row: RECT, dpi: i32, font: HFONT) -> i32 {
    let (_, max_w) = chord_box(row, dpi);
    let lines = pack_chords(&chord_widths(dc, font, keys, dpi), max_w, chord_gap(dpi));
    key_row_h_from_lines(lines.len() as i32, dpi)
}

unsafe fn key_row_heights(rows: &[usize], row: RECT, dpi: i32, font: HFONT) -> Vec<i32> {
    if rows.is_empty() {
        return vec![sc(dpi, 40)];
    }
    let dc = GetDC(None);
    let heights = rows
        .iter()
        .map(|&i| key_row_height(dc, KEYS[i].keys, row, dpi, if dc.is_invalid() { HFONT::default() } else { font }))
        .collect();
    if !dc.is_invalid() {
        let _ = ReleaseDC(None, dc);
    }
    heights
}

unsafe fn draw_card(dc: HDC, c: &CardItem, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    // Disabled rows stay on a white (or dark-theme card) ground; only the label grays.
    rfill(dc, c.rect, sc(dpi, 6), super::pal().card, Some(super::pal().line));
    let color = if c.spec.live { super::pal().text } else { super::pal().sub };
    let title = if c.spec.ctl == Ctl::About {
        format!("grids v{}", env!("CARGO_PKG_VERSION"))
    } else {
        tx(c.spec.title).to_string()
    };
    let text_r = RECT { left: c.rect.left + sc(dpi, 16), top: c.rect.top + sc(dpi, 12), right: c.ctl.left.min(c.sw.first().map(|s| s.left).unwrap_or(i32::MAX)) - sc(dpi, 12), bottom: c.rect.bottom - sc(dpi, 12) };
    let title_line = if c.spec.desc[0].is_empty() { text_r } else { RECT { bottom: text_r.top + sc(dpi, 20), ..text_r } };
    super::text(dc, title_line, &title, sn.font, color, DT_LEFT);
    if c.spec.badge {
        let bw = text_w(dc, sn.small, tx(NEW)) + sc(dpi, 10);
        let tw = text_w(dc, sn.font, &title);
        let br = RECT { left: title_line.left + tw + sc(dpi, 8), top: (title_line.top + title_line.bottom) / 2 - sc(dpi, 8), right: title_line.left + tw + sc(dpi, 8) + bw, bottom: (title_line.top + title_line.bottom) / 2 + sc(dpi, 8) };
        if br.right < text_r.right {
            rfill(dc, br, sc(dpi, 3), accent, None);
            super::text(dc, br, tx(NEW), sn.small, on_chrome(accent), DT_CENTER);
        }
    }
    if c.spec.ctl == Ctl::Adv {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: title_line.bottom + line, ..text_r };
        super::text(dc, desc_r, tx(c.spec.desc), sn.small, super::pal().sub, DT_LEFT);
        let path = super::store::settings_path().to_string_lossy().into_owned();
        let path_r = RECT { left: c.rect.left + sc(dpi, 16), top: desc_r.bottom, right: c.rect.right - sc(dpi, 16), bottom: c.rect.bottom - sc(dpi, 6) };
        super::text(dc, path_r, &path, sn.small, super::pal().text, DT_LEFT);
    } else if !c.spec.desc[0].is_empty() {
        super::text(dc, RECT { top: title_line.bottom, ..text_r }, tx(c.spec.desc), sn.small, super::pal().sub, DT_LEFT);
    }
    match c.spec.ctl {
        Ctl::Toggle(id) => draw_toggle(dc, c.ctl, toggle_on(id), c.spec.live, accent, dpi),
        Ctl::CopyEng => super::text(dc, c.ctl, "IFileOperation", sn.small, super::pal().sub, DT_RIGHT),
        Ctl::Accent => {
            let colors = super::pal().accents;
            let cur = app(|a| a.s.accent_now());
            for (i, r) in c.sw.iter().enumerate() {
                let col = colors.get(i).map(|(_, c)| *c).unwrap_or(0);
                if col == cur {
                    ellipse(dc, RECT { left: r.left - sc(dpi, 2), top: r.top - sc(dpi, 2), right: r.right + sc(dpi, 2), bottom: r.bottom + sc(dpi, 2) }, super::pal().text, None);
                }
                ellipse(dc, *r, col, None);
            }
        }
        _ => {}
    }
}

unsafe fn draw_toggle(dc: HDC, r: RECT, on: bool, enabled: bool, accent: u32, dpi: i32) {
    // 40×20 pill. Off and disabled stay an outlined gray track on the white card.
    let (fill, border, knob) = if !enabled || !on {
        (super::pal().card, super::pal().sub, super::pal().sub)
    } else {
        (accent, accent, on_chrome(accent))
    };
    let rad = (r.bottom - r.top).max(1);
    rfill(dc, r, rad, fill, Some(border));
    let k = sc(dpi, 12);
    let pad = sc(dpi, 3);
    let x = if on { r.right - pad - k } else { r.left + pad };
    let y = r.top + (r.bottom - r.top - k) / 2;
    ellipse(dc, RECT { left: x, top: y, right: x + k, bottom: y + k }, knob, None);
}

unsafe fn draw_fav_row(dc: HDC, i: usize, r: RECT, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    if i == usize::MAX {
        super::text(dc, r, tr("No favourites yet · add one with ☆+ in a pane header", "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가", "お気に入りはまだありません · ペインヘッダーの ☆+ で追加"), sn.small, super::pal().sub, DT_CENTER);
        return;
    }
    let path = app(|a| a.s.favs.get(i).cloned().unwrap_or_default());
    if path.is_empty() {
        return;
    }
    let icon = RECT { left: r.left + sc(dpi, 12), right: r.left + sc(dpi, 34), ..r };
    super::text(dc, icon, "\u{E8B7}", sn.icon, super::pal().sub, DT_CENTER);
    let name_r = RECT { left: icon.right + sc(dpi, 4), right: r.left + (r.right - r.left) * 32 / 100, ..r };
    super::text(dc, name_r, &super::fav_label(&path), sn.font, super::pal().text, DT_LEFT);
    let path_r = RECT { left: name_r.right + sc(dpi, 8), right: r.right - sc(dpi, 180), ..r };
    super::text(dc, path_r, super::fav_path(&path), sn.small, super::pal().sub, DT_LEFT);
    super::fill(dc, RECT { top: r.bottom - 1, ..r }, super::pal().line);
}

unsafe fn draw_key_row(dc: HDC, i: usize, r: RECT, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    if i == usize::MAX || i >= KEYS.len() {
        super::text(dc, r, tx(NONE), sn.small, super::pal().sub, DT_CENTER);
        return;
    }
    let k = &KEYS[i];
    let (cat, keys_x) = key_cols(r);
    super::text(dc, cat, tx(k.cat), sn.font, super::pal().sub, DT_LEFT);
    super::text(dc, RECT { left: cat.right, right: keys_x - sc(dpi, 8), ..r }, tx(k.act), sn.font, super::pal().text, DT_LEFT);
    let (_, max_w) = chord_box(r, dpi);
    let widths = chord_widths(dc, sn.mono, k.keys, dpi);
    let lines = pack_chords(&widths, max_w, chord_gap(dpi));
    let n = lines.len().max(1) as i32;
    let ch = chip_h(dpi);
    let lg = sc(dpi, 4);
    let stack = n * ch + (n - 1) * lg;
    let mut y = r.top + (r.bottom - r.top - stack).max(0) / 2;
    let limit = keys_x + max_w;
    let mut ki = 0usize;
    for line in &lines {
        let mut x = keys_x;
        for &w in line {
            let key = k.keys.get(ki).copied().unwrap_or("");
            ki += 1;
            let right = (x + w).min(limit);
            let chip = RECT { left: x, top: y, right, bottom: (y + ch).min(r.bottom - 1) };
            if chip.right > chip.left && chip.bottom > chip.top {
                rfill(dc, chip, sc(dpi, 6), super::pal().bg, Some(super::pal().line));
                super::fill(dc, RECT { top: chip.bottom - sc(dpi, 2).max(1), left: chip.left + 1, right: chip.right - 1, bottom: chip.bottom - 1 }, super::blend(super::pal().line, super::pal().text, 0.35));
                super::text(dc, chip, key, sn.mono, super::pal().text, DT_CENTER);
            }
            x = right + chord_gap(dpi);
        }
        y += ch + lg;
    }
    super::fill(dc, RECT { top: r.bottom - 1, ..r }, super::pal().line);
}

unsafe fn text_w(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    let _ = DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right
}

unsafe fn rfill(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(super::cref(fill));
    let pen = match border {
        Some(c) => CreatePen(PS_SOLID, 1, super::cref(c)),
        None => HPEN(GetStockObject(NULL_PEN).0),
    };
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, rad.max(1), rad.max(1));
    SelectObject(dc, op);
    SelectObject(dc, ob);
    let _ = DeleteObject(brush.into());
    if border.is_some() {
        let _ = DeleteObject(pen.into());
    }
}

unsafe fn ellipse(dc: HDC, r: RECT, fill: u32, border: Option<u32>) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(super::cref(fill));
    let pen = match border {
        Some(c) => CreatePen(PS_SOLID, 1, super::cref(c)),
        None => HPEN(GetStockObject(NULL_PEN).0),
    };
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    let _ = Ellipse(dc, r.left, r.top, r.right, r.bottom);
    SelectObject(dc, op);
    SelectObject(dc, ob);
    let _ = DeleteObject(brush.into());
    if border.is_some() {
        let _ = DeleteObject(pen.into());
    }
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    let dis = d.itemState.0 & 4 != 0;
    let pressed = d.itemState.0 & 1 != 0;
    let hot = app(|a| a.hover) == d.hwndItem;
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = ST.with(|s| s.borrow().font);
    let accent = chrome();
    let on_card = id != S_CLOSE && id != S_RESET;
    let ground = if on_card { super::pal().card } else { super::pal().bg };
    super::fill(dc_of(d), d.rcItem, ground);
    let (fill, border, ink) = if id == S_CLOSE && !dis {
        (accent, accent, on_chrome(accent))
    } else if dis {
        (super::blend(ground, super::pal().sub, 0.12), super::pal().line, super::pal().sub)
    } else if pressed {
        (super::blend(ground, super::pal().text, 0.08), super::pal().line, super::pal().text)
    } else if hot {
        (super::blend(ground, accent, 0.12), accent, super::pal().text)
    } else if id == S_ACCENT && app(|a| !super::pal().accents.iter().any(|(_, c)| *c == a.s.accent_now())) {
        (ground, accent, super::pal().text)
    } else if id == S_ADV_RESET {
        (ground, super::pal().line, 0xC42B1C)
    } else {
        (ground, super::pal().line, super::pal().text)
    };
    rfill(dc_of(d), d.rcItem, sc(dpi, 4), fill, Some(border));
    let label = super::text_of(d.hwndItem);
    super::text(dc_of(d), d.rcItem, &label, font, ink, DT_CENTER);
}

fn dc_of(d: &DRAWITEMSTRUCT) -> HDC {
    d.hDC
}

fn flip(id: u8) {
    match id {
        TG_SINGLE => app(|a| a.s.single ^= true),
        TG_DBLUP => app(|a| a.s.dblup ^= true),
        TG_HIDDEN => {
            unsafe { super::toggle_hidden() };
            return;
        }
        TG_BAR => app(|a| a.s.bar ^= true),
        TG_STATUS => app(|a| a.s.stat ^= true),
        TG_EXT => {
            let on = app(|a| {
                a.s.ext ^= true;
                a.s.ext
            });
            unsafe {
                super::apply_ext(on);
                super::save_session();
                redraw();
            }
            return;
        }
        TG_TREE => app(|a| a.s.tree_on ^= true),
        TG_EDGE => app(|a| a.s.edge ^= true),
        TG_CONFIRM => app(|a| a.s.confirm ^= true),
        TG_AUTOSAVE => app(|a| a.s.autosave ^= true),
        TG_DOT => {
            app(|a| a.s.dot ^= true);
            unsafe {
                super::save_session();
                super::check_dirty();
                redraw();
            }
            return;
        }
        _ => return,
    }
    unsafe {
        super::save_session();
        super::relayout();
        super::redraw_all();
        redraw();
    }
}

unsafe fn delete_fav(k: usize) {
    let (pop, dpi, n) = app(|a| {
        if k < a.s.favs.len() {
            a.s.favs.remove(k);
        }
        a.fav_hot = None;
        (a.fav_popup, a.dpi, a.s.favs.len())
    });
    if !pop.0.is_null() && IsWindow(Some(pop)).as_bool() {
        let g = super::fav_geo(n, dpi);
        let _ = SetWindowPos(pop, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = InvalidateRect(Some(pop), None, false);
    }
    super::repaint_fav_buttons();
    super::save_session();
    refresh();
}

unsafe fn on_command(h: HWND, id: i32, code: u32) {
    if BUSY.with(|b| b.get()) {
        return;
    }
    if code == EN_CHANGE && id == S_SEARCH {
        let q = super::text_of(ctl(h, S_SEARCH));
        let page = ST.with(|s| s.borrow().page);
        let page = if page_match(&q, page) { page } else { (0..9).find(|p| page_match(&q, *p)).unwrap_or(page) };
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.query = q;
            st.page = page;
            st.scroll = 0;
        });
        layout(h);
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == EN_CHANGE && id == S_KEYQ {
        let q = super::text_of(ctl(h, S_KEYQ));
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.keyq = q;
            st.scroll = 0;
        });
        layout(h);
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == CBN_SELCHANGE && id == S_LANG {
        let sel = SendMessageW(ctl(h, S_LANG), 0x0147, None, None).0 as usize;
        let lang = [Lang::Auto, Lang::En, Lang::Ko, Lang::Ja].get(sel).copied().unwrap_or(Lang::Auto);
        if app(|a| a.s.lang) != lang {
            super::set_language(lang);
        }
        return;
    }
    if code == CBN_SELCHANGE {
        let sel = SendMessageW(ctl(h, id), 0x0147, None, None).0 as usize;
        if combo_pick(h, id, sel) {
            return;
        }
    }
    if code != 0 {
        return;
    }
    match id {
        S_CLOSE => close(),
        S_SESS => {
            let d = super::sessions_dir();
            let _ = std::fs::create_dir_all(&d);
            let p = HSTRING::from(d.to_string_lossy().as_ref());
            ShellExecuteW(Some(h), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ACCENT => {
            super::pick_accent();
            super::save_session();
            redraw();
        }
        S_GITHUB => {
            ShellExecuteW(Some(h), w!("open"), &HSTRING::from(super::HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ADV_OPEN => {
            let d = super::store::config_dir();
            let _ = std::fs::create_dir_all(&d);
            let p = HSTRING::from(d.to_string_lossy().as_ref());
            ShellExecuteW(Some(h), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ADV_EX => export_settings(h),
        S_ADV_IM => import_settings(h),
        S_ADV_RESET => reset_ini(h),
        id if (S_FAV0..S_FAV0 + 128).contains(&id) => delete_fav((id - S_FAV0) as usize),
        id if (S_FAV0 + 128..S_FAV0 + 256).contains(&id) => rename_fav(h, (id - S_FAV0 - 128) as usize),
        _ => {}
    }
}

fn combo_pick(h: HWND, id: i32, sel: usize) -> bool {
    match id {
        S_THEME => {
            app(|a| a.s.theme = sel.min(2) as u8);
            unsafe {
                super::apply_theme();
                super::save_session();
            }
        }
        S_FONT => {
            app(|a| a.s.font = sel.min(2) as u8);
            unsafe {
                super::apply_font();
                super::save_session();
                refresh();
            }
        }
        S_VIEW => {
            let mode = [4, 3, 1][sel.min(2)];
            app(|a| a.s.view_new = mode);
            unsafe { super::save_session(); }
        }
        S_DIR => {
            if sel == 2 {
                let path = unsafe { super::pick_folder(h) };
                match path {
                    Some(p) => app(|a| {
                        a.s.start_new = 2;
                        a.s.start_path = p;
                    }),
                    None => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            } else {
                app(|a| a.s.start_new = sel.min(1) as u8);
            }
            unsafe { super::save_session() };
        }
        S_DEL => {
            app(|a| a.s.del = if sel == 1 { 1 } else { 0 });
            unsafe { super::save_session() };
        }
        S_NEW => {
            if sel == 0 {
                app(|a| a.s.new_name.clear());
            } else {
                let cur = app(|a| if a.s.new_name.is_empty() { super::default_new_name() } else { a.s.new_name.clone() });
                match unsafe { ask(h, tx(s("New file name", "새 파일 이름", "新しいファイル名")), &cur) } {
                    Some(name) if !name.trim().is_empty() => app(|a| a.s.new_name = name),
                    _ => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            }
            unsafe {
                super::save_session();
                refresh();
            }
        }
        S_DROP => {
            app(|a| a.s.dropm = sel.min(2) as u8);
            unsafe { super::save_session() };
        }
        S_START => {
            if sel == 1 {
                match unsafe { super::pick_open(h) } {
                    Some(p) => app(|a| {
                        a.s.boot = 1;
                        a.s.boot_file = p;
                    }),
                    None => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            } else {
                app(|a| a.s.boot = if sel >= 2 { 2 } else { 0 });
            }
            unsafe { super::save_session() };
        }
        _ => return false,
    }
    true
}

unsafe fn export_settings(h: HWND) {
    let Some(path) = super::pick_settings(h, true) else { return };
    super::save_session();
    let text = super::store::current_text();
    if super::store::write_atomic(std::path::Path::new(&path), text.as_bytes()).is_err() {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(s("Could not export the settings file.", "설정 파일을 내보내지 못했어요.", "設定ファイルを書き出せませんでした。"))), w!("grids"), MB_ICONWARNING);
    }
}

unsafe fn import_settings(h: HWND) {
    let Some(path) = super::pick_settings(h, false) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(s("Could not read that file.", "그 파일을 읽지 못했어요.", "そのファイルを読めませんでした。"))), w!("grids"), MB_ICONWARNING);
        return;
    };
    let q = tx(s("Replace the current settings with this file?", "이 파일로 지금 설정을 바꿀까요?", "このファイルで現在の設定を置き換えますか?"));
    if MessageBoxW(Some(h), &HSTRING::from(q), w!("grids"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    if !super::import_settings(&text) {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(s("That file is not valid settings JSON.", "올바른 설정 JSON이 아니에요.", "正しい設定 JSON ではありません。"))), w!("grids"), MB_ICONWARNING);
    }
}

unsafe fn reset_ini(h: HWND) {
    let q = tx(s("Reset every preference to its default? The layout, folders and favourites stay.", "환경설정을 모두 기본값으로 되돌릴까요? 배치, 폴더, 즐겨찾기는 남아요.", "環境設定をすべて既定に戻しますか? レイアウト、フォルダー、お気に入りはそのままです。"));
    if MessageBoxW(Some(h), &HSTRING::from(q), w!("grids"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    super::reset_prefs();
}

unsafe fn rename_fav(h: HWND, k: usize) {
    let cur = app(|a| a.s.favs.get(k).map(|f| super::fav_label(f)).unwrap_or_default());
    let Some(name) = ask(h, tx(s("Rename", "이름 변경", "名前の変更")), &cur) else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    app(|a| {
        if let Some(f) = a.s.favs.get_mut(k) {
            let path = super::fav_path(f).to_string();
            *f = format!("{path}\t{name}");
        }
    });
    super::save_session();
    super::repaint_fav_buttons();
    refresh();
}

unsafe fn on_click(h: HWND, x: i32, y: i32) {
    let sn = snap();
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let g = geo(rc, &sn);
    for i in 0..9 {
        if g.show[i] && inside(g.nav[i], x, y) {
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.page = i;
                st.scroll = 0;
            });
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
            return;
        }
    }
    for item in &g.items {
        if let Item::FavRow { i, rect, rename, btn } = item {
            if *i != usize::MAX && inside(*rect, x, y) && !inside(*rename, x, y) && !inside(*btn, x, y) {
                ST.with(|s| s.borrow_mut().fav_drag = Some(*i));
                let _ = SetCapture(h);
                return;
            }
        }
        if let Item::Card(c) = item {
            if let Ctl::Toggle(id) = c.spec.ctl {
                if c.spec.live && inside(c.ctl, x, y) {
                    flip(id);
                    return;
                }
            }
            if c.spec.ctl == Ctl::Accent {
                let colors = super::pal().accents;
                for (i, r) in c.sw.iter().enumerate() {
                    if inside(*r, x, y) {
                        if let Some((_, col)) = colors.get(i) {
                            super::set_accent(*col);
                            super::save_session();
                        }
                        return;
                    }
                }
            }
        }
    }
    if g.has_bar && inside(g.track, x, y) {
        if inside(g.thumb, x, y) {
            ST.with(|s| s.borrow_mut().drag = Some(y - g.thumb.top));
            let _ = SetCapture(h);
        } else {
            let page = (g.content.bottom - g.content.top).max(1);
            let dir = if y < g.thumb.top { -1 } else { 1 };
            let max = g.scroll_max;
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.scroll = (st.scroll + dir * page).clamp(0, max);
            });
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

extern "system" fn settings_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let dpi = GetDpiForWindow(h) as i32;
                ST.with(|s| {
                    let mut st = s.borrow_mut();
                    *st = St::default();
                    st.hwnd = h;
                    st.dpi = dpi.max(96);
                });
                rebuild_fonts();
                build_children(h);
                refresh();
                return LRESULT(0);
            }
            WM_SIZE => {
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as i32;
                ST.with(|s| s.borrow_mut().dpi = dpi.max(96));
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                refresh();
            }
            WM_GETMINMAXINFO => {
                let dpi = ST.with(|s| s.borrow().dpi.max(96));
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize.x = sc(dpi, 780);
                m.ptMinTrackSize.y = sc(dpi, 520);
                return LRESULT(0);
            }
            WM_PAINT => paint(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_DRAWITEM => {
                draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as _);
                let hw = HWND(lp.0 as _);
                let off = !IsWindowEnabled(hw).as_bool();
                SetBkMode(dc, TRANSPARENT);
                SetBkColor(dc, super::cref(super::pal().card));
                SetTextColor(dc, super::cref(if off { super::pal().sub } else { super::pal().text }));
                SetDCBrushColor(dc, super::cref(super::pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
            WM_LBUTTONDOWN => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                on_click(h, x, y);
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                if let Some(from) = ST.with(|s| s.borrow().fav_drag) {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let to = g.items.iter().find_map(|it| match it {
                        Item::FavRow { i, rect, .. } if *i != usize::MAX && y >= rect.top && y < rect.bottom => Some(*i),
                        _ => None,
                    });
                    if let Some(to) = to.filter(|to| *to != from) {
                        app(|a| {
                            if from < a.s.favs.len() && to < a.s.favs.len() {
                                let item = a.s.favs.remove(from);
                                a.s.favs.insert(to, item);
                            }
                        });
                        ST.with(|s| s.borrow_mut().fav_drag = Some(to));
                        layout(h);
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                    let _ = x;
                    return LRESULT(0);
                }
                if let Some(off) = ST.with(|s| s.borrow().drag) {
                    let (track, thumb_h, max) = ST.with(|s| {
                        let st = s.borrow();
                        (st.track, st.thumb_h, st.scroll_max)
                    });
                    let span = (track.bottom - track.top - thumb_h).max(1);
                    let top = (y - off).clamp(track.top, track.bottom - thumb_h);
                    let scroll = if max == 0 { 0 } else { (top - track.top) * max / span };
                    ST.with(|s| s.borrow_mut().scroll = scroll.clamp(0, max));
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                } else {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let hot = (0..9).find(|&i| g.show[i] && inside(g.nav[i], x, y));
                    let changed = ST.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.hot != hot {
                            st.hot = hot;
                            true
                        } else {
                            false
                        }
                    });
                    if changed {
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                }
            }
            WM_LBUTTONUP => {
                let fav = ST.with(|s| s.borrow_mut().fav_drag.take());
                if ST.with(|s| s.borrow().drag.is_some()) || fav.is_some() {
                    ST.with(|s| s.borrow_mut().drag = None);
                    let _ = ReleaseCapture();
                }
                if fav.is_some() {
                    super::save_session();
                    super::repaint_fav_buttons();
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) as i16 as i32) / 120;
                if delta != 0 {
                    let dpi = ST.with(|s| s.borrow().dpi.max(96));
                    let (scroll, max) = ST.with(|s| {
                        let st = s.borrow();
                        (st.scroll, st.scroll_max)
                    });
                    let next = (scroll - delta * sc(dpi, 48)).clamp(0, max);
                    if next != scroll {
                        ST.with(|s| s.borrow_mut().scroll = next);
                        layout(h);
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                }
            }
            WM_MOUSELEAVE => {
                ST.with(|s| s.borrow_mut().hot = None);
                let _ = InvalidateRect(Some(h), None, false);
            }
            WM_CLOSE => {
                let _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                let old = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let old = (st.font, st.small, st.title_font, st.mono, st.icon);
                    *st = St::default();
                    old
                });
                for f in [old.0, old.1, old.2, old.3, old.4] {
                    if !f.is_invalid() {
                        let _ = DeleteObject(f.into());
                    }
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

thread_local! {
    static ASK_EDIT: Cell<HWND> = Cell::new(HWND(std::ptr::null_mut()));
    static ASK_OUT: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn ask_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if id == 1 {
                let text = super::text_of(ASK_EDIT.with(|e| e.get()));
                ASK_OUT.with(|o| *o.borrow_mut() = Some(Some(text)));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
            if id == 2 {
                ASK_OUT.with(|o| *o.borrow_mut() = Some(None));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            ASK_OUT.with(|o| *o.borrow_mut() = Some(None));
            let _ = DestroyWindow(h);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// One-line prompt. None means cancel.
unsafe fn ask(owner: HWND, title: &str, initial: &str) -> Option<String> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ask_proc),
            hInstance: m.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            lpszClassName: w!("GridsAsk"),
            ..Default::default()
        });
    });
    ASK_OUT.with(|o| *o.borrow_mut() = None);
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = (sc(dpi, 420), sc(dpi, 140));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE;
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("GridsAsk"), &HSTRING::from(title), style, x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return None;
    }
    let font = ST.with(|s| s.borrow().font);
    let edit = child_at(h, w!("EDIT"), initial, 0x80 | 0x00010000 | WS_BORDER.0, 10, font);
    ASK_EDIT.with(|e| e.set(edit));
    let _ = child_at(h, w!("BUTTON"), tx(s("OK", "확인", "OK")), 0xB | 0x00010000, 1, font);
    let _ = child_at(h, w!("BUTTON"), tx(CLOSE_L), 0xB | 0x00010000, 2, font);
    let _ = SetWindowPos(edit, None, sc(dpi, 16), sc(dpi, 16), ww - sc(dpi, 48), sc(dpi, 28), SWP_NOZORDER);
    let _ = SetWindowPos(ctl(h, 1), None, ww - sc(dpi, 200), sc(dpi, 60), sc(dpi, 80), sc(dpi, 28), SWP_NOZORDER);
    let _ = SetWindowPos(ctl(h, 2), None, ww - sc(dpi, 108), sc(dpi, 60), sc(dpi, 80), sc(dpi, 28), SWP_NOZORDER);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
    let mut msg = MSG::default();
    while ASK_OUT.with(|o| o.borrow().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, true);
    let _ = SetForegroundWindow(owner);
    ASK_OUT.with(|o| o.borrow_mut().take().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(line: &[i32], gap: i32) -> i32 {
        if line.is_empty() {
            0
        } else {
            line.iter().sum::<i32>() + gap * (line.len() as i32 - 1)
        }
    }

    fn column_w(client: i32, bar: bool) -> i32 {
        let nav = sc(96, 220);
        let pad = sc(96, 28);
        let right_pad = if bar { sc(96, 18) } else { 0 };
        let row = RECT { left: nav + pad, top: 0, right: client - pad - right_pad, bottom: 0 };
        chord_box(row, 96).1
    }

    #[test]
    fn shortcut_chords_wrap_inside_the_cell() {
        let gap = chord_gap(96);
        // Four Alt+Shift arrow chips, about 110px each in Consolas 12.
        let arrows = [110, 110, 110, 110];
        for (client, bar) in [(1000, false), (1000, true), (780, true)] {
            let max_w = column_w(client, bar);
            let lines = pack_chords(&arrows, max_w, gap);
            assert!(lines.len() >= 2, "client {client} bar {bar} max {max_w} stayed one line");
            assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), 4);
            for line in &lines {
                assert!(span(line, gap) <= max_w, "line {line:?} exceeds {max_w}");
            }
            assert!(key_row_h_from_lines(lines.len() as i32, 96) > sc(96, 40));
        }
        // A chord wider than the cell is clamped to the cell.
        assert_eq!(pack_chords(&[900], 200, gap), vec![vec![200]]);
        // Short chords stay on one line and keep the 40px row.
        assert_eq!(pack_chords(&[70, 60, 40], column_w(1000, false), gap).len(), 1);
        assert_eq!(key_row_h_from_lines(1, 96), sc(96, 40));
        assert!(key_row_h_from_lines(2, 96) > key_row_h_from_lines(1, 96));
    }
}
