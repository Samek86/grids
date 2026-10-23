// Settings window: Win32, no webview. Nav, cards, toggles and the footer are drawn here;
// edits, combos and buttons are real child controls sitting on that paint.

use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

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
const S_EV: i32 = 27;
const S_FACE: i32 = 28;
const S_LAYOUT: i32 = 29;
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
const TG_ONE: u8 = 13;
const TG_PANE_TABS: u8 = 14;
const TG_EV: u8 = 15;
const TG_DRIVES: u8 = 16;

const CB_SETMINVISIBLE: u32 = 0x1701;
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
    Face,
    Start,
    Theme,
    Font,
    ViewMode,
    StartDir,
    DelMode,
    NewName,
    Drop,
    Layout,
    Accent,
    CopyEng,
    Sess,
    About,
    Adv,
    AdvReset,
    EvStatus,
    EvAct,
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
    Spec { title: s("Font", "글꼴", "フォント"), desc: s("Default follows the display language. A chosen face applies to the whole window right away.", "기본값은 표시 언어의 글꼴이에요. 고르면 창 전체에 바로 적용돼요.", "既定は表示言語のフォントです。選ぶとウィンドウ全体にすぐに適用されます。"), badge: true, live: true, ctl: Ctl::Face },
    Spec { title: s("On startup", "시작할 때", "起動時"), desc: s("Restore every tab, its folders and the window, or open a chosen session.", "모든 탭과 폴더, 창을 복원하거나 정해진 세션을 열어요.", "すべてのタブとフォルダー、ウィンドウを復元するか、指定したセッションを開きます。"), badge: false, live: true, ctl: Ctl::Start },
    Spec { title: s("Default layout", "기본 레이아웃", "既定のレイアウト"), desc: s("Panes in a new tab or a new window.", "새 탭이나 새 창을 열 때의 패널 배치예요.", "新しいタブや新しいウィンドウを開くときのペイン配置です。"), badge: true, live: true, ctl: Ctl::Layout },
    Spec { title: s("Single instance", "중복 실행 막기", "二重起動を防ぐ"), desc: s("A second launch opens a new window in this process, unless single-window mode is on.", "다시 실행하면 이 프로세스에 새 창이 열려요. 창 하나만 쓰기가 켜져 있으면 탭으로 열어요.", "もう一度起動すると、このプロセスに新しいウィンドウを開きます。単一ウィンドウがオンのときはタブで開きます。"), badge: false, live: true, ctl: Ctl::Toggle(TG_SINGLE) },
    Spec { title: s("Single-window mode", "창 하나만 쓰기", "単一ウィンドウ"), desc: s("One window. Another launch brings it forward and restores it if minimized; paths open as tabs. Ctrl+N, dragging a tab out, Move to new window, and Win+Alt+E when Grids is already in front open a tab here.", "창은 하나예요. 다시 실행하면 그 창을 앞으로 가져오고, 최소화되어 있으면 복원해요. 경로는 탭으로 열립니다. Ctrl+N, 탭을 밖으로 끌기, 새 창으로 이동, 이미 앞에 있을 때의 Win+Alt+E는 이 창의 탭을 열어요.", "ウィンドウは一つです。もう一度起動すると前面に出し、最小化中なら元に戻します。パスはタブで開きます。Ctrl+N、タブを外へドラッグ、新しいウィンドウへ移動、すでに前面のときの Win+Alt+E は、このウィンドウのタブを開きます。"), badge: false, live: true, ctl: Ctl::Toggle(TG_ONE) },
    Spec { title: s("Double-click empty space for the parent folder", "빈 곳 더블클릭으로 상위 폴더", "空白のダブルクリックで上のフォルダー"), desc: s("This is the dblup setting.", "지금 있는 dblup 설정이에요.", "現在の dblup 設定です。"), badge: false, live: true, ctl: Ctl::Toggle(TG_DBLUP) },
    Spec { title: s("Tabs in panes", "패널 안의 탭", "ペイン内のタブ"), desc: s("A strip under each pane header for that pane's folders. Separate from the window tabs. Hiding the strips keeps the folders. Saved as pane_tabs.", "패널 머리줄 아래에 그 패널의 폴더 탭이 생깁니다. 창 위쪽 탭과는 다릅니다. 줄을 숨겨도 폴더는 남아요. pane_tabs로 저장됩니다.", "各ペインの見出しの下に、そのペインのフォルダーのタブ列を出します。ウィンドウ上部のタブとは別です。列を隠してもフォルダーは残ります。pane_tabs に保存します。"), badge: false, live: true, ctl: Ctl::Toggle(TG_PANE_TABS) },
    Spec { title: s("Status", "상태", "状態"), desc: s("Installed, running, version", "설치, 실행, 버전", "インストール、実行、バージョン"), badge: false, live: true, ctl: Ctl::EvStatus },
    Spec { title: s("Use Everything for search", "검색에 Everything 사용", "検索に Everything を使う"), desc: s("On by default. Subfolder search asks Everything when it is running, and otherwise walks the folders.", "기본값은 켜짐. Everything이 실행 중이면 하위 폴더 검색이 Everything에 묻고, 아니면 폴더를 훑어요.", "既定はオンです。Everything が実行中ならサブフォルダー検索は Everything に問い合わせ、そうでなければフォルダーをたどります。"), badge: false, live: true, ctl: Ctl::Toggle(TG_EV) },
    Spec { title: s("Get Everything", "Everything 준비", "Everything の準備"), desc: s("Start it when it is installed. Install uses winget when that is available, otherwise the voidtools download page opens.", "설치되어 있으면 시작해요. 설치는 winget이 있으면 winget을 쓰고, 없으면 voidtools 다운로드 페이지를 열어요.", "インストール済みなら起動します。インストールは winget があれば winget、なければ voidtools のダウンロードページを開きます。"), badge: false, live: true, ctl: Ctl::EvAct },
];

const EV_HEAD: [&str; 3] = s("Everything", "Everything", "Everything");
const LOOK: &[Spec] = &[
    Spec { title: s("Theme", "테마", "テーマ"), desc: s("", "", ""), badge: false, live: true, ctl: Ctl::Theme },
    Spec { title: s("Accent colour", "강조 색", "アクセント カラー"), desc: s("Light and dark each keep their own (accent / accent_dark).", "라이트와 다크에 따로 정할 수 있어요(지금 있는 accent / accent_dark).", "ライトとダークで別々に選べます（accent / accent_dark）。"), badge: false, live: true, ctl: Ctl::Accent },
    Spec { title: s("Font size", "글꼴 크기", "フォント サイズ"), desc: s("The toolbar, panes and this window.", "툴바, 패널, 이 창에 적용돼요.", "ツールバー、ペイン、このウィンドウに適用します。"), badge: true, live: true, ctl: Ctl::Font },
    Spec { title: s("Show the toolbar", "툴바 표시", "ツールバーを表示"), desc: s("", "", ""), badge: true, live: true, ctl: Ctl::Toggle(TG_BAR) },
    Spec { title: s("Show drive buttons", "드라이브 버튼 표시", "ドライブボタンを表示"), desc: s("C: D: E: on the tab row. Off, the tabs use that space.", "탭 줄의 C: D: E: 버튼이에요. 끄면 그 자리를 탭이 써요.", "タブ列の C: D: E: です。オフにするとその分をタブが使います。"), badge: true, live: true, ctl: Ctl::Toggle(TG_DRIVES) },
    Spec { title: s("Show the status bar", "상태 표시줄 표시", "ステータス バーを表示"), desc: s("Item count and free space on the drive.", "항목 수와 드라이브 남은 공간이에요.", "項目数とドライブの空き容量です。"), badge: true, live: true, ctl: Ctl::Toggle(TG_STATUS) },
];
const VIEW: &[Spec] = &[
    Spec { title: s("Default view for a new pane", "새 패널의 기본 보기", "新しいペインの既定の表示"), desc: s("Used when a pane has no view of its own yet.", "아직 보기가 없는 패널에 써요.", "まだ表示を持たないペインに使います。"), badge: true, live: true, ctl: Ctl::ViewMode },
    Spec { title: s("Starting folder for a new pane", "새 패널의 시작 폴더", "新しいペインの開始フォルダー"), desc: s("Used when a new pane has no folder yet.", "새 패널에 폴더가 없을 때 열어요.", "新しいペインにフォルダーがないときに開きます。"), badge: true, live: true, ctl: Ctl::StartDir },
    Spec { title: s("Show hidden files", "숨김 파일 표시", "隠しファイルを表示"), desc: s("Same as Ctrl+H.", "Ctrl+H와 같아요.", "Ctrl+H と同じです。"), badge: false, live: true, ctl: Ctl::Toggle(TG_HIDDEN) },
    Spec { title: s("Show file extensions", "파일 확장자 표시", "ファイル拡張子を表示"), desc: s("This is Windows' own setting, so Explorer follows it too.", "Windows 설정을 바꿔요. 탐색기도 같이 바뀌어요.", "Windows の設定を変えるので、エクスプローラーも従います。"), badge: true, live: true, ctl: Ctl::Toggle(TG_EXT) },
    Spec { title: s("Show the left panel", "왼쪽 패널 표시", "左のパネルを表示"), desc: s("The folder tree is on top and favourites are below. Drag the divider between them. This hides both.", "위는 폴더 트리, 아래는 즐겨찾기예요. 사이 구분선을 끌어 나눠요. 이 항목은 둘 다 숨겨요.", "上はフォルダー ツリー、下はお気に入りです。間の区切りをドラッグして分けます。この項目で両方を隠します。"), badge: false, live: true, ctl: Ctl::Toggle(TG_TREE) },
    Spec { title: s("Emphasise the active pane border", "활성 패널 테두리 강조", "アクティブなペインの枠を強調"), desc: s("", "", ""), badge: true, live: true, ctl: Ctl::Toggle(TG_EDGE) },
];
const FILE: &[Spec] = &[
    Spec { title: s("Copy / move progress", "복사·이동 진행 창", "コピー / 移動の進行状況"), desc: s("The Windows progress window runs on its own thread (always on).", "Windows 표준 진행 창을 별도 스레드에서 띄워요(항상 켜짐).", "Windows の進行状況ウィンドウを別スレッドで表示します（常にオン）。"), badge: false, live: true, ctl: Ctl::CopyEng },
    Spec { title: s("Confirm delete", "삭제할 때 확인", "削除の確認"), desc: s("Del in a pane. Shift+Del is always permanent.", "패널에서 Del. Shift+Del은 항상 완전 삭제예요.", "ペインで Del。Shift+Del は常に完全削除です。"), badge: true, live: true, ctl: Ctl::Toggle(TG_CONFIRM) },
    Spec { title: s("Default delete", "기본 삭제 방식", "既定の削除方法"), desc: s("", "", ""), badge: true, live: true, ctl: Ctl::DelMode },
    Spec { title: s("New file name", "새 파일 이름", "新しいファイル名"), desc: s("Used by New text file on the view menu and by Ctrl+Alt+N. Empty follows the display language.", "보기 메뉴의 새 텍스트 파일과 Ctrl+Alt+N이 이 이름을 써요. 비우면 표시 언어를 따라요.", "表示メニューの新しいテキスト ファイルと Ctrl+Alt+N が使います。空なら表示言語に従います。"), badge: true, live: true, ctl: Ctl::NewName },
    Spec { title: s("Default drag and drop", "끌어 놓기 기본 동작", "ドラッグ アンド ドロップの既定"), desc: s("The ⇄ menu. A drag inside a pane still follows Explorer.", "⇄ 메뉴에 적용돼요. 패널 안 끌어 놓기는 탐색기 그대로예요.", "⇄ メニューに適用します。ペイン内のドラッグはエクスプローラーのままです。"), badge: true, live: true, ctl: Ctl::Drop },
];
const FAV: &[Spec] = &[
    Spec { title: s("Show add or remove favourites on the right-click menu", "우클릭 메뉴에 즐겨찾기 추가/제거 표시", "右クリック メニューにお気に入りの追加と削除を表示"), desc: s("Always on. The item sits directly above Properties.", "항상 켜져 있어요. 속성 바로 위에 있어요.", "常にオンです。項目はプロパティのすぐ上です。"), badge: false, live: true, ctl: Ctl::Toggle(TG_FAVMENU) },
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

const KEY_NOTE: [&str; 3] = s(
    "Double-click a row or press Change, then the new shortcut. Esc cancels. A conflict can be swapped.",
    "행을 더블클릭하거나 변경을 누른 뒤 새 단축키를 누르세요. Esc는 취소, 겹치면 맞바꿀 수 있어요.",
    "行をダブルクリックするか変更を押し、新しいキーを押します。Esc で取消。重複は入れ替えできます。",
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

fn key_hit_i(i: usize, q: &str) -> bool {
    let (cat, act, chord) = crate::keys::row(i);
    has(&cat, q) || has(&act, q) || has(&chord, q)
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
        return (0..crate::keys::len()).any(|i| key_hit_i(i, q)) || has(tx(KEY_NOTE), q);
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
    /// Installed families. `None` until this window asks; dropped when the window closes.
    faces: Option<Vec<String>>,
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
            faces: None,
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
    KeyRow { i: usize, rect: RECT, change: RECT, reset: RECT },
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
    if spec.ctl == Ctl::Adv || spec.ctl == Ctl::EvStatus || spec.ctl == Ctl::EvAct {
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
    let mut ev_head = false;
    for spec in page_cards(page) {
        if !rq.is_empty() && !card_hit(spec, rq) {
            continue;
        }
        if is_ev_card(spec) {
            if !ev_head {
                g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, EV_HEAD));
                y += sc(dpi, 28);
                ev_head = true;
            }
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
        Ctl::EvStatus => RECT { left: rect.right - pad, top: rect.top, right: rect.right - pad, bottom: rect.bottom },
        Ctl::EvAct => right_box(rect, sc(dpi, 200), sc(dpi, 32), pad),
        Ctl::CopyEng => right_box(rect, sc(dpi, 140), sc(dpi, 22), pad),
        Ctl::Lang | Ctl::Face | Ctl::Start | Ctl::Theme | Ctl::Font | Ctl::ViewMode | Ctl::StartDir | Ctl::DelMode | Ctl::NewName | Ctl::Drop | Ctl::Layout => {
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
    let rows: Vec<usize> = (0..crate::keys::len()).filter(|&i| {
        (rq.is_empty() || key_hit_i(i, rq)) && (sn.keyq.is_empty() || key_hit_i(i, &sn.keyq))
    }).collect();
    let reset = right_box(RECT { left, top: y, right, bottom: y + sc(dpi, 40) }, sc(dpi, 210), sc(dpi, 32), 0);
    let edit = RECT { left, top: reset.top, right: reset.left - sc(dpi, 8), bottom: reset.bottom };
    g.items.push(Item::KeyBar { edit, reset });
    y += sc(dpi, 48);
    let head_h = sc(dpi, 32);
    let table_top = y;
    let probe = RECT { left, top: 0, right, bottom: 0 };
    let labels: Vec<String> = rows.iter().map(|&i| crate::keys::chord_text(i)).collect();
    let heights = unsafe { win::key_row_heights(&labels, probe, dpi, sn.mono) };
    let body: i32 = heights.iter().sum();
    let table = RECT { left, top: table_top, right, bottom: table_top + head_h + body };
    g.items.push(Item::Table(table));
    g.items.push(Item::KeyHead(RECT { left, top: table_top, right, bottom: table_top + head_h }));
    y = table_top + head_h;
    if rows.is_empty() {
        let h = heights.first().copied().unwrap_or(sc(dpi, 40));
        g.items.push(Item::KeyRow { i: usize::MAX, rect: RECT { left, top: y, right, bottom: y + h }, change: RECT::default(), reset: RECT::default() });
        y += h;
    }
    for (n, i) in rows.into_iter().enumerate() {
        let h = heights.get(n).copied().unwrap_or(sc(dpi, 40));
        let rect = RECT { left, top: y, right, bottom: y + h };
        let (change, reset) = win::key_btn_rects(rect, dpi);
        g.items.push(Item::KeyRow { i, rect, change, reset });
        y += h;
    }
    y += sc(dpi, 8);
    g.items.push(Item::Note(RECT { left, top: y, right, bottom: y + sc(dpi, 40) }, KEY_NOTE));
    y + sc(dpi, 40)
}

fn is_ev_card(spec: &Spec) -> bool {
    matches!(spec.ctl, Ctl::EvStatus | Ctl::EvAct | Ctl::Toggle(TG_EV))
}

fn toggle_on(id: u8) -> bool {
    match id {
        TG_SINGLE => app(|a| a.s.single),
        TG_ONE => app(|a| a.s.one_window),
        TG_DBLUP => app(|a| a.s.dblup),
        TG_PANE_TABS => app(|a| a.s.pane_tabs_on),
        TG_HIDDEN => unsafe { super::hidden() },
        TG_BAR => app(|a| a.s.bar),
        TG_DRIVES => app(|a| a.s.drive_buttons),
        TG_STATUS => app(|a| a.s.stat),
        TG_EXT => app(|a| a.s.ext),
        TG_TREE => app(|a| a.s.tree_on),
        TG_EDGE => app(|a| a.s.edge),
        TG_CONFIRM => app(|a| a.s.confirm),
        TG_AUTOSAVE => app(|a| a.s.autosave),
        TG_DOT => app(|a| a.s.dot),
        TG_EV => app(|a| a.s.use_everything),
        // The favourites item is always on the shell menu, directly above Properties.
        TG_FAVMENU => true,
        _ => false,
    }
}

fn ctl_id(c: Ctl) -> Option<i32> {
    Some(match c {
        Ctl::Lang => S_LANG,
        Ctl::Face => S_FACE,
        Ctl::Start => S_START,
        Ctl::Theme => S_THEME,
        Ctl::Font => S_FONT,
        Ctl::ViewMode => S_VIEW,
        Ctl::StartDir => S_DIR,
        Ctl::DelMode => S_DEL,
        Ctl::NewName => S_NEW,
        Ctl::Drop => S_DROP,
        Ctl::Layout => S_LAYOUT,
        Ctl::Sess => S_SESS,
        Ctl::About => S_GITHUB,
        Ctl::Accent => S_ACCENT,
        _ => return None,
    })
}


pub(crate) mod store;
mod win;
pub(crate) use win::*;
