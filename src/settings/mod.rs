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
const S_FV_DEF: i32 = 33;
const S_FV_RESET: i32 = 34;
const S_FV_CLEAR: i32 = 35;
pub(crate) const S_CTX_RESET: i32 = 36;
const S_LAYOUT: i32 = 29;
const S_WM: i32 = 30;
const S_WM_BROWSE: i32 = 31;
const S_FAV0: i32 = 20000;
/// Delete buttons are S_FAV0 + i, rename buttons S_REN0 + i. Control ids stay under 0x8000.
const FAV_MAX: i32 = 4096;
const S_REN0: i32 = S_FAV0 + FAV_MAX;
/// One past the highest favourite row whose buttons were created, so loops skip the empty range.
static FAV_MADE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

const TG_SINGLE: u8 = 1;
const TG_DBLUP: u8 = 2;
const TG_BAR: u8 = 3;
const TG_STATUS: u8 = 4;
const TG_HIDDEN: u8 = 5;
const TG_EXT: u8 = 6;
const TG_TREE: u8 = 7;
const TG_EDGE: u8 = 8;
const TG_CONFIRM: u8 = 9;
const TG_AUTOSAVE: u8 = 11;
const TG_DOT: u8 = 12;
const TG_ONE: u8 = 13;
const TG_EV: u8 = 15;
const TG_DRIVES: u8 = 16;
const TG_SPACE: u8 = 17;
const TG_WHEEL: u8 = 18;
const TG_UPDATE: u8 = 19;
const TG_REMEMBER: u8 = 20;
/// Default folder app. The knob follows HKCU, not a saved preference.
const TG_FOLDER: u8 = 21;

const CB_SETMINVISIBLE: u32 = 0x1701;
const EM_SETCUEBANNER: u32 = 0x1501;

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

/// Settings search frame, DIP at 96 DPI. The rect runs from `SEARCH_TOP_DIP` to 44.
const SEARCH_TOP_DIP: i32 = 12;
const SEARCH_FRAME_DIP: i32 = 32;
/// Gap between that frame and the borderless edit, and the corner radius. Shared with the name prompt
/// and the favourites window search box.
pub(crate) const FIELD_INSET_DIP: i32 = 2;
pub(crate) const FIELD_RADIUS_DIP: i32 = 4;

/// Painted height of the settings search frame. `sc(44) - sc(12)`, so it matches `Geo.search` at every DPI.
pub(crate) fn search_frame_h(dpi: i32) -> i32 {
    sc(dpi, SEARCH_TOP_DIP + SEARCH_FRAME_DIP) - sc(dpi, SEARCH_TOP_DIP)
}

fn tx(t: [&'static str; 3]) -> &'static str {
    tr(t[0], t[1], t[2])
}

const CHROME_LIGHT: u32 = 0x0067C0;
const CHROME_DARK: u32 = 0x4CC2FF;
const WHITE: u32 = 0xFFFFFF;

/// Accent used by this window. A chosen colour is used as-is. The old built-in gray
/// selection colour reads as no accent, so that default is Windows blue (#0067c0 / #4cc2ff).
pub(crate) fn chrome_color() -> u32 {
    chrome()
}

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

const TITLE: [&str; 3] = s("Panegrid settings", "Panegrid 설정", "Panegrid 設定");
const FOOT: [&str; 3] = s("Changes apply right away", "변경은 바로 적용돼요", "変更はすぐに適用されます");
const CLOSE_L: [&str; 3] = s("Close", "닫기", "閉じる");
const FIND: [&str; 3] = s("Find a setting", "설정 검색", "設定を検索");
const NONE: [&str; 3] = s("No matches", "결과 없음", "一致なし");
/// Content shown when the nav query matches no page. Distinct from `NONE`, which is the nav label and an empty shortcut row.
const NO_RESULTS: [&str; 3] = s("No results", "검색 결과 없음", "検索結果なし");
pub(crate) const PAGE_N: usize = 10;
/// Right-click menu page. About stays last so earlier page numbers do not move.
pub(crate) const CTX_PAGE: usize = 8;
const PAGES: [[&str; 3]; PAGE_N] = [
    s("General", "일반", "一般"),
    s("Appearance", "모양", "外観"),
    s("Panes & view", "패널·보기", "ペインと表示"),
    s("File operations", "파일 작업", "ファイル操作"),
    s("Favourites", "즐겨찾기", "お気に入り"),
    s("Shortcuts", "단축키", "ショートカット"),
    s("Sessions", "세션", "セッション"),
    s("Advanced", "고급", "詳細"),
    s("Right-click menu items", "우클릭 메뉴 항목", "右クリックメニュー項目"),
    s("About", "정보", "情報"),
];

const CTX_NOTE: [&str; 3] = s(
    "These items appear in Panegrid's right-click menu. Windows items such as Open, Copy and Properties stay. A new item starts on. Explorer is unchanged.",
    "Panegrid의 우클릭 메뉴에 나오는 항목이에요. 열기, 복사, 속성 같은 Windows 항목은 그대로예요. 새 항목은 켜진 채로 시작해요. 탐색기는 바뀌지 않아요.",
    "Panegrid の右クリックメニューに出る項目です。開く、コピー、プロパティなどの Windows の項目はそのままです。新しい項目はオンで始まります。エクスプローラーは変わりません。",
);
const CTX_RESET_L: [&str; 3] = s("Reset to all enabled", "모두 사용", "すべて有効に戻す");
const CTX_EMPTY: [&str; 3] = s("No third-party menu items", "타사 메뉴 항목이 없어요", "サードパーティの項目はありません");

#[derive(Clone, Copy, PartialEq)]
enum Ctl {
    Lang,
    Face,
    Start,
    Theme,
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
    WmStatus,
    WmPath,
    Toggle(u8),
    FolderDefault,
    FolderReset,
    FolderClear,
}

#[derive(Clone, Copy)]
struct Spec {
    title: [&'static str; 3],
    desc: [&'static str; 3],
    live: bool,
    ctl: Ctl,
}

const GEN: &[Spec] = &[
    Spec { title: s("Display language", "표시 언어", "表示言語"), desc: s("The default follows the Windows display language (Korean, Japanese, or English); otherwise English. A change applies right away.", "기본값은 Windows 표시 언어(한국어, 일본어, 영어)를 따라요. 그 외에는 영어예요. 바꾸면 바로 적용돼요.", "既定は Windows の表示言語（韓国語、日本語、英語）に従います。それ以外は英語です。変えるとすぐに適用されます。"), live: true, ctl: Ctl::Lang },
    Spec { title: s("Font", "글꼴", "フォント"), desc: s("Default follows the display language. A chosen face applies to the whole window right away.", "기본값은 표시 언어의 글꼴이에요. 고르면 창 전체에 바로 적용돼요.", "既定は表示言語のフォントです。選ぶとウィンドウ全体にすぐに適用されます。"), live: true, ctl: Ctl::Face },
    Spec { title: s("On startup", "시작할 때", "起動時"), desc: s("Restore every tab, its folders and the window, or open a chosen session.", "모든 탭과 폴더, 창을 복원하거나 정해진 세션을 열어요.", "すべてのタブとフォルダー、ウィンドウを復元するか、指定したセッションを開きます。"), live: true, ctl: Ctl::Start },
    Spec { title: s("Use as default folder app", "기본 폴더 앱으로 사용", "既定のフォルダーアプリとして使用"), desc: s("Win+E and special folders stay with Explorer.", "Win+E와 특수 폴더는 탐색기 그대로예요.", "Win+E と特殊フォルダーはエクスプローラーのままです。"), live: true, ctl: Ctl::Toggle(TG_FOLDER) },
    Spec { title: s("Default layout", "기본 레이아웃", "既定のレイアウト"), desc: s("Panes in a new window, and on a first start. A new tab copies the current tab.", "새 창과 첫 실행의 패널 배치예요. 새 탭은 지금 탭의 배치를 그대로 가져와요.", "新しいウィンドウと初回起動のペイン配置です。新しいタブは今のタブの配置を引き継ぎます。"), live: true, ctl: Ctl::Layout },
    Spec { title: s("Single instance", "중복 실행 막기", "二重起動を防ぐ"), desc: s("A second launch opens a new window in this process, unless single-window mode is on.", "다시 실행하면 이 프로세스에 새 창이 열려요. 창 하나만 쓰기가 켜져 있으면 탭으로 열어요.", "もう一度起動すると、このプロセスに新しいウィンドウを開きます。単一ウィンドウがオンのときはタブで開きます。"), live: true, ctl: Ctl::Toggle(TG_SINGLE) },
    Spec { title: s("Single-window mode", "창 하나만 쓰기", "単一ウィンドウ"), desc: s("One window. Another launch brings it forward and restores it if minimized; paths open as tabs. Ctrl+N, dragging a tab out, Move to new window, and Win+Shift+E when Panegrid is already in front open a tab here.", "창은 하나예요. 다시 실행하면 그 창을 앞으로 가져오고, 최소화되어 있으면 복원해요. 경로는 탭으로 열립니다. Ctrl+N, 탭을 밖으로 끌기, 새 창으로 이동, 이미 앞에 있을 때의 Win+Shift+E는 이 창의 탭을 열어요.", "ウィンドウは一つです。もう一度起動すると前面に出し、最小化中なら元に戻します。パスはタブで開きます。Ctrl+N、タブを外へドラッグ、新しいウィンドウへ移動、すでに前面のときの Win+Shift+E は、このウィンドウのタブを開きます。"), live: true, ctl: Ctl::Toggle(TG_ONE) },
    Spec { title: s("Double-click empty space for the parent folder", "빈 곳 더블클릭으로 상위 폴더", "空白のダブルクリックで上のフォルダー"), desc: s("In a file list, a double-click on empty space opens the parent folder.", "파일 목록의 빈 곳을 더블클릭하면 상위 폴더를 열어요.", "ファイル一覧の空白をダブルクリックすると、上のフォルダーを開きます。"), live: true, ctl: Ctl::Toggle(TG_DBLUP) },
    Spec { title: s("Status", "상태", "状態"), desc: s("Installed, running, version", "설치, 실행, 버전", "インストール、実行、バージョン"), live: true, ctl: Ctl::EvStatus },
    Spec { title: s("Use Everything for search", "검색에 Everything 사용", "検索に Everything を使う"), desc: s("On by default. Subfolder search asks Everything when it is running, and otherwise walks the folders.", "기본값은 켜짐. Everything이 실행 중이면 하위 폴더 검색이 Everything에 묻고, 아니면 폴더를 훑어요.", "既定はオンです。Everything が実行中ならサブフォルダー検索は Everything に問い合わせ、そうでなければフォルダーをたどります。"), live: true, ctl: Ctl::Toggle(TG_EV) },
    Spec { title: s("Get Everything", "Everything 준비", "Everything の準備"), desc: s("Start it when it is installed. Install uses winget when that is available, otherwise the voidtools download page opens.", "설치되어 있으면 시작해요. 설치는 winget이 있으면 winget을 쓰고, 없으면 voidtools 다운로드 페이지를 열어요.", "インストール済みなら起動します。インストールは winget があれば winget、なければ voidtools のダウンロードページを開きます。"), live: true, ctl: Ctl::EvAct },
    Spec { title: s("Auto-detect", "자동 감지", "自動検出"), desc: s("WinMerge", "WinMerge", "WinMerge"), live: true, ctl: Ctl::WmStatus },
    Spec { title: s("WinMerge path", "WinMerge 경로", "WinMerge のパス"), desc: s("Empty uses auto-detect. A folder or WinMergeU.exe both work.", "비우면 자동으로 찾아요. 폴더나 WinMergeU.exe 모두 돼요.", "空なら自動検出です。フォルダーでも WinMergeU.exe でも構いません。"), live: true, ctl: Ctl::WmPath },
];

const EV_HEAD: [&str; 3] = s("Everything", "Everything", "Everything");
const WM_HEAD: [&str; 3] = s("WinMerge", "WinMerge", "WinMerge");
const PREVIEW_HEAD: [&str; 3] = s("Preview", "미리보기", "プレビュー");
const LOOK: &[Spec] = &[
    Spec { title: s("Theme", "테마", "テーマ"), desc: s("", "", ""), live: true, ctl: Ctl::Theme },
    Spec { title: s("Accent colour", "강조 색", "アクセント カラー"), desc: s("Light and dark each keep their own colour.", "라이트와 다크에 색을 따로 정할 수 있어요.", "ライトとダークで色を別々に選べます。"), live: true, ctl: Ctl::Accent },
    Spec { title: s("Show the toolbar", "툴바 표시", "ツールバーを表示"), desc: s("", "", ""), live: true, ctl: Ctl::Toggle(TG_BAR) },
    Spec { title: s("Show drive buttons", "드라이브 버튼 표시", "ドライブボタンを表示"), desc: s("C: D: E: on the tab row. Off, the tabs use that space.", "탭 줄의 C: D: E: 버튼이에요. 끄면 그 자리를 탭이 써요.", "タブ列の C: D: E: です。オフにするとその分をタブが使います。"), live: true, ctl: Ctl::Toggle(TG_DRIVES) },
    Spec { title: s("Show the status bar", "상태 표시줄 표시", "ステータス バーを表示"), desc: s("Item count and free space on the drive.", "항목 수와 드라이브 남은 공간이에요.", "項目数とドライブの空き容量です。"), live: true, ctl: Ctl::Toggle(TG_STATUS) },
];
const VIEW: &[Spec] = &[
    Spec { title: s("Default view for a new pane", "새 패널의 기본 보기", "新しいペインの既定の表示"), desc: s("Used when a pane has no view of its own yet.", "아직 보기가 없는 패널에 써요.", "まだ表示を持たないペインに使います。"), live: true, ctl: Ctl::ViewMode },
    Spec { title: s("Remember view per folder", "폴더마다 보기 기억", "フォルダーごとに表示を記憶"), desc: s("Sort, columns, view mode, icon size and grouping are restored in any pane. Saved in folder-views.json next to the settings. On by default.", "정렬, 열, 보기 방식, 아이콘 크기, 그룹을 어느 패널에서든 되돌립니다. 설정 옆의 folder-views.json에 저장해요. 기본값은 켜짐.", "並べ替え、列、表示、アイコンの大きさ、グループをどのペインでも戻します。設定の隣の folder-views.json に保存します。既定はオンです。"), live: true, ctl: Ctl::Toggle(TG_REMEMBER) },
    Spec { title: s("Default view for every folder", "모든 폴더의 기본 보기", "すべてのフォルダーの既定の表示"), desc: s("Folders with no saved view use this. Takes the active pane's sort, columns, view mode, icon size and grouping.", "기억된 보기가 없는 폴더가 이것을 씁니다. 활성 패널의 정렬, 열, 보기 방식, 아이콘 크기, 그룹을 가져와요.", "記憶がないフォルダーはこれを使います。アクティブなペインの並べ替え、列、表示、アイコンの大きさ、グループを取り込みます。"), live: true, ctl: Ctl::FolderDefault },
    Spec { title: s("Reset this folder's view", "이 폴더의 보기 지우기", "このフォルダーの表示を消す"), desc: s("Forgets the active pane's folder. It uses the default view from here on.", "활성 패널의 폴더만 잊습니다. 이제부터 기본 보기를 씁니다.", "アクティブなペインのフォルダーだけ忘れます。ここからは既定の表示を使います。"), live: true, ctl: Ctl::FolderReset },
    Spec { title: s("Clear all remembered views", "기억한 보기를 모두 지우기", "記憶した表示をすべて消す"), desc: s("Forgets every folder. The default view stays.", "모든 폴더의 기억을 지웁니다. 기본 보기는 남아요.", "すべてのフォルダーの記憶を消します。既定の表示は残ります。"), live: true, ctl: Ctl::FolderClear },
    Spec { title: s("Starting folder for a new pane", "새 패널의 시작 폴더", "新しいペインの開始フォルダー"), desc: s("Used when a new pane has no folder yet.", "새 패널에 폴더가 없을 때 열어요.", "新しいペインにフォルダーがないときに開きます。"), live: true, ctl: Ctl::StartDir },
    Spec { title: s("Show hidden files", "숨김 파일 표시", "隠しファイルを表示"), desc: s("Same as Ctrl+H.", "Ctrl+H와 같아요.", "Ctrl+H と同じです。"), live: true, ctl: Ctl::Toggle(TG_HIDDEN) },
    Spec { title: s("Show file extensions", "파일 확장자 표시", "ファイル拡張子を表示"), desc: s("This is Windows' own setting, so Explorer follows it too.", "Windows 설정을 바꿔요. 탐색기도 같이 바뀌어요.", "Windows の設定を変えるので、エクスプローラーも従います。"), live: true, ctl: Ctl::Toggle(TG_EXT) },
    Spec { title: s("Show the left panel", "왼쪽 패널 표시", "左のパネルを表示"), desc: s("The folder tree is on top and favourites are below. Drag the divider between them. This hides both.", "위는 폴더 트리, 아래는 즐겨찾기예요. 사이 구분선을 끌어 나눠요. 이 항목은 둘 다 숨겨요.", "上はフォルダー ツリー、下はお気に入りです。間の区切りをドラッグして分けます。この項目で両方を隠します。"), live: true, ctl: Ctl::Toggle(TG_TREE) },
    Spec { title: s("Emphasise the active pane border", "활성 패널 테두리 강조", "アクティブなペインの枠を強調"), desc: s("", "", ""), live: true, ctl: Ctl::Toggle(TG_EDGE) },
    Spec { title: s("Preview with Space", "스페이스로 미리보기", "スペースでプレビュー"), desc: s("Same as F3 in a file list. A space within a second of typing, or while Korean or Japanese IME text is being composed, stays part of the name. On by default.", "파일 목록에서는 F3과 같아요. 글자를 입력한 뒤 1초 안의 스페이스나, 한글·일본어를 조합하는 중의 스페이스는 이름의 일부로 들어가요. 기본값은 켜짐.", "ファイル一覧では F3 と同じです。文字を入れてから 1 秒以内のスペース、または韓国語・日本語の変換中のスペースは名前の一部になります。既定はオンです。"), live: true, ctl: Ctl::Toggle(TG_SPACE) },
    Spec { title: s("Wheel moves to next/previous file", "휠로 다음/이전 파일로 넘기기", "ホイールで次/前のファイルへ"), desc: s("Scrolling past the end of the content moves to the next or previous file.", "내용 끝에서 마우스 휠을 더 돌리면 다음·이전 파일로 넘어가요.", "内容の端でさらにホイールを回すと次・前のファイルに移ります。"), live: true, ctl: Ctl::Toggle(TG_WHEEL) },
];
const FILE: &[Spec] = &[
    Spec { title: s("Copy / move progress", "복사·이동 진행 창", "コピー / 移動の進行状況"), desc: s("The Windows progress window runs on its own thread (always on).", "Windows 표준 진행 창을 별도 스레드에서 띄워요(항상 켜짐).", "Windows の進行状況ウィンドウを別スレッドで表示します（常にオン）。"), live: true, ctl: Ctl::CopyEng },
    Spec { title: s("Confirm delete", "삭제할 때 확인", "削除の確認"), desc: s("Del in a pane. Shift+Del is always permanent.", "패널에서 Del. Shift+Del은 항상 완전 삭제예요.", "ペインで Del。Shift+Del は常に完全削除です。"), live: true, ctl: Ctl::Toggle(TG_CONFIRM) },
    Spec { title: s("Default delete", "기본 삭제 방식", "既定の削除方法"), desc: s("", "", ""), live: true, ctl: Ctl::DelMode },
    Spec { title: s("New file name", "새 파일 이름", "新しいファイル名"), desc: s("Used by Ctrl+Alt+N. Empty follows the display language.", "Ctrl+Alt+N이 이 이름을 써요. 비우면 표시 언어를 따라요.", "Ctrl+Alt+N が使います。空なら表示言語に従います。"), live: true, ctl: Ctl::NewName },
    Spec { title: s("Default drag and drop", "끌어 놓기 기본 동작", "ドラッグ アンド ドロップの既定"), desc: s("The ⇄ menu. A drag inside a pane still follows Explorer.", "⇄ 메뉴에 적용돼요. 패널 안 끌어 놓기는 탐색기 그대로예요.", "⇄ メニューに適用します。ペイン内のドラッグはエクスプローラーのままです。"), live: true, ctl: Ctl::Drop },
];
const SESS: &[Spec] = &[
    Spec { title: s("Session folder", "세션 폴더", "セッション フォルダー"), desc: s("%APPDATA%\\Panegrid\\panegrid-sessions.", "%APPDATA%\\Panegrid\\panegrid-sessions 폴더예요.", "%APPDATA%\\Panegrid\\panegrid-sessions フォルダーです。"), live: true, ctl: Ctl::Sess },
    Spec { title: s("Save on exit", "종료할 때 자동 저장", "終了時に自動保存"), desc: s("Writes the named session file when one is open.", "열어 둔 세션 파일이 있을 때 그 파일에 저장해요.", "開いているセッション ファイルがあるとき、そのファイルに保存します。"), live: true, ctl: Ctl::Toggle(TG_AUTOSAVE) },
    Spec { title: s("Unsaved-changes mark (green dot)", "저장 안 된 변경 표시 (초록 점)", "未保存の変更を表示（緑の点）"), desc: s("On the save button.", "저장 버튼에 표시해요.", "保存ボタンに表示します。"), live: true, ctl: Ctl::Toggle(TG_DOT) },
];
const ADV: &[Spec] = &[
    Spec { title: s("Check for updates on startup", "시작할 때 업데이트 확인", "起動時に更新を確認"), desc: s("Asks GitHub for a newer Panegrid release and offers to install it.", "GitHub에서 새 Panegrid 릴리스를 확인하고 설치를 제안해요.", "GitHub で新しい Panegrid リリースを確認し、インストールを提案します。"), live: true, ctl: Ctl::Toggle(TG_UPDATE) },
    Spec { title: s("Settings file", "설정 파일", "設定ファイル"), desc: s("portable.txt next to the exe keeps this file there.", "exe 옆 portable.txt가 있으면 이 파일을 그 옆에 둬요.", "exe の隣の portable.txt があると、このファイルをそこに置きます。"), live: true, ctl: Ctl::Adv },
    Spec { title: s("Reset all settings", "모든 설정 초기화", "すべての設定を初期化"), desc: s("Preferences return to their defaults. The layout, folders and favourites stay.", "환경설정만 기본값으로 돌아가요. 배치, 폴더, 즐겨찾기는 남아요.", "環境設定だけ既定に戻します。レイアウト、フォルダー、お気に入りはそのままです。"), live: true, ctl: Ctl::AdvReset },
];

const KEY_NOTE: [&str; 3] = s(
    "Double-click a row or press Change, then the new shortcut. Esc cancels. A conflict can be swapped.",
    "행을 더블클릭하거나 변경을 누른 뒤 새 단축키를 누르세요. Esc는 취소, 겹치면 맞바꿀 수 있어요.",
    "行をダブルクリックするか変更を押し、新しいキーを押します。Esc で取消。重複は入れ替えできます。",
);
const LIST_H: [&str; 3] = s("List", "목록", "一覧");
const ABOUT: &[Spec] = &[Spec {
    title: s("Panegrid", "Panegrid", "Panegrid"),
    desc: s(
        "Panegrid is a fork of efww/grids by efww · fork Samek86/panegrid",
        "Panegrid는 efww의 efww/grids 포크입니다 · 포크 Samek86/panegrid",
        "Panegrid は efww による efww/grids のフォークです · フォーク Samek86/panegrid",
    ),
    live: true,
    ctl: Ctl::About,
}];

fn page_cards(p: usize) -> &'static [Spec] {
    match p {
        0 => GEN,
        1 => LOOK,
        2 => VIEW,
        3 => FILE,
        6 => SESS,
        7 => ADV,
        9 => ABOUT,
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
        let rows = fav_tree_rows();
        return !crate::filter_fav_rows(&rows, q).is_empty() || has(tx(LIST_H), q);
    }
    if page == 5 {
        return (0..crate::keys::len()).any(|i| key_hit_i(i, q)) || has(tx(KEY_NOTE), q);
    }
    if page == CTX_PAGE {
        return has(tx(CTX_NOTE), q) || has(tx(CTX_RESET_L), q) || has(tx(CTX_EMPTY), q) || crate::shell::ctxmenu::name_hit(q);
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

/// True when `q` matches no settings page. An empty query matches every page.
fn search_miss(q: &str) -> bool {
    !q.is_empty() && !(0..PAGES.len()).any(|p| page_match(q, p))
}

/// Page to show for a nav-search change.
///
/// `anchor` is the page the user had before this search auto-navigated (a nav click,
/// or the page when the box was last empty). A query that matches nothing restores
/// that page and does not stay on a page a shorter prefix opened. A prefix such as
/// "Ag" matches only Shortcuts, because chord text includes PageUp / PageDown, and
/// the next letter must not leave Shortcuts selected.
fn pick_search(anchor: usize, page: usize, q: &str) -> usize {
    if q.is_empty() || page_match(q, page) {
        page
    } else {
        (0..PAGES.len()).find(|&p| page_match(q, p)).unwrap_or(anchor)
    }
}

/// `(anchor, page)` after one search-box change. Clearing the box keeps the page
/// that was showing and makes it the next anchor.
fn commit_search(anchor: usize, page: usize, q: &str) -> (usize, usize) {
    let page = pick_search(anchor, page, q);
    let anchor = if q.is_empty() { page } else { anchor };
    (anchor, page)
}

struct St {
    hwnd: HWND,
    page: usize,
    /// Page to restore when a search matches nothing. Not updated when search auto-switches.
    anchor: usize,
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
    /// Tree id of the favourites row being dragged. Indices shift as the node moves.
    fav_drag: Option<Vec<usize>>,
    /// Handler ids expanded on the right-click menu page.
    ctx_open: Vec<String>,
}

impl Default for St {
    fn default() -> Self {
        St {
            hwnd: HWND::default(),
            page: 0,
            anchor: 0,
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
            ctx_open: Vec::new(),
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
    ctx_open: Vec<String>,
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
            ctx_open: st.ctx_open.clone(),
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
    CtxNote(RECT),
    CtxBar { reset: RECT },
    CtxRow {
        id: String,
        rect: RECT,
        mark: RECT,
        chev: RECT,
        name: String,
        targets: u8,
        off: u8,
        chips: Vec<(u8, RECT)>,
    },
    CtxSub { id: String, bit: u8, attached: u8, rect: RECT, mark: RECT, on: bool },
    CtxEmpty(RECT),
    KeyBar { edit: RECT, reset: RECT },
    KeyHead(RECT),
    KeyRow { i: usize, rect: RECT, change: RECT, reset: RECT },
}

struct Geo {
    nav_w: i32,
    nav: [RECT; PAGE_N],
    show: [bool; PAGE_N],
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
    if spec.ctl == Ctl::WmPath {
        sc(dpi, 120)
    } else if spec.ctl == Ctl::Adv || spec.ctl == Ctl::EvStatus || spec.ctl == Ctl::EvAct || spec.ctl == Ctl::WmStatus {
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
    let search = RECT { left: sc(dpi, 8), top: sc(dpi, SEARCH_TOP_DIP), right: nav_w - sc(dpi, 8), bottom: sc(dpi, SEARCH_TOP_DIP + SEARCH_FRAME_DIP) };
    let close = right_box(footer, sc(dpi, 88), sc(dpi, 32), sc(dpi, 16));
    let hint = RECT { left: sc(dpi, 16), top: footer.top, right: close.left - sc(dpi, 8), bottom: footer.bottom };
    let q = sn.query.as_str();
    let mut show = [false; PAGE_N];
    let mut nav = [RECT::default(); PAGE_N];
    let mut y = search.bottom + sc(dpi, 8);
    for i in 0..PAGE_N {
        show[i] = page_match(q, i);
        if !show[i] {
            continue;
        }
        nav[i] = RECT { left: sc(dpi, 8), top: y, right: nav_w - sc(dpi, 8), bottom: y + sc(dpi, 36) };
        y += sc(dpi, 36);
    }
    // No page matches: leave the body empty so paint can show the miss message
    // instead of the retained page (Shortcuts would otherwise lay out an empty table).
    let miss = search_miss(q);
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
    if miss {
        return g;
    }
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
    let mut wm_head = false;
    let mut preview_head = false;
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
        if is_wm_card(spec) {
            if !wm_head {
                g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, WM_HEAD));
                y += sc(dpi, 28);
                wm_head = true;
            }
        }
        if is_preview_card(spec) {
            if !preview_head {
                g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, PREVIEW_HEAD));
                y += sc(dpi, 28);
                preview_head = true;
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
    if page == CTX_PAGE {
        y = ctx_items(g, sn, left, right, y, rq);
    }
    (y + sn.scroll) - start + sc(dpi, 16)
}

fn ctx_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    let dpi = sn.dpi.max(96);
    if rq.is_empty() || has(tx(CTX_NOTE), rq) {
        let h = sc(dpi, 72);
        g.items.push(Item::CtxNote(RECT { left, top: y, right, bottom: y + h }));
        y += h + sc(dpi, 8);
    }
    let off = app(|a| a.s.ctx_off.clone());
    let rows = crate::shell::ctxmenu::ui_rows(&off, rq);
    let bw = sc(dpi, 220);
    let bh = sc(dpi, 32);
    let reset = RECT { left: (right - bw).max(left), top: y, right, bottom: y + bh };
    g.items.push(Item::CtxBar { reset });
    y += bh + sc(dpi, 12);
    if rows.is_empty() {
        g.items.push(Item::CtxEmpty(RECT { left, top: y, right, bottom: y + sc(dpi, 36) }));
        return y + sc(dpi, 44);
    }
    for row in rows {
        let h = sc(dpi, 48);
        let rect = RECT { left, top: y, right, bottom: y + h };
        let mark = RECT { left: left + sc(dpi, 14), top: y + (h - sc(dpi, 16)) / 2, right: left + sc(dpi, 30), bottom: y + (h + sc(dpi, 16)) / 2 };
        let chev = RECT { left: mark.right + sc(dpi, 2), top: y, right: mark.right + sc(dpi, 26), bottom: y + h };
        let chips = ctx_chips(row.targets, rect, dpi);
        let open = sn.ctx_open.iter().any(|id| id == &row.id);
        g.items.push(Item::CtxRow {
            id: row.id.clone(),
            rect,
            mark,
            chev,
            name: row.name,
            targets: row.targets,
            off: row.off,
            chips,
        });
        y += h;
        if open {
            for bit in crate::shell::ctxmenu::target_bits() {
                if row.targets & bit == 0 {
                    continue;
                }
                let sh = sc(dpi, 36);
                let sub = RECT { left: left + sc(dpi, 36), top: y, right, bottom: y + sh };
                let sbox = RECT {
                    left: sub.left + sc(dpi, 14),
                    top: y + (sh - sc(dpi, 16)) / 2,
                    right: sub.left + sc(dpi, 30),
                    bottom: y + (sh + sc(dpi, 16)) / 2,
                };
                g.items.push(Item::CtxSub {
                    id: row.id.clone(),
                    bit,
                    attached: row.targets,
                    rect: sub,
                    mark: sbox,
                    on: crate::shell::ctxmenu::target_enabled(row.off, bit),
                });
                y += sh;
            }
        }
        y += sc(dpi, 6);
    }
    y
}

fn ctx_chips(targets: u8, rect: RECT, dpi: i32) -> Vec<(u8, RECT)> {
    let mut x = rect.right - sc(dpi, 12);
    let mut out = Vec::new();
    for bit in crate::shell::ctxmenu::target_bits().into_iter().rev() {
        if targets & bit == 0 {
            continue;
        }
        let label = crate::shell::ctxmenu::target_label(bit);
        let w = sc(dpi, 16) + label.chars().count() as i32 * sc(dpi, 7);
        let r = RECT { left: x - w, top: rect.top + sc(dpi, 14), right: x, bottom: rect.top + sc(dpi, 34) };
        x = r.left - sc(dpi, 6);
        out.push((bit, r));
    }
    out.reverse();
    out
}

fn card_item(rect: RECT, spec: Spec, dpi: i32) -> CardItem {
    let pad = sc(dpi, 16);
    let mut sw = Vec::new();
    let mut adv = [RECT::default(); 4];
    let ctl = match spec.ctl {
        Ctl::Toggle(_) => right_box(rect, sc(dpi, 40), sc(dpi, 20), pad),
        Ctl::EvStatus | Ctl::WmStatus => RECT { left: rect.right - pad, top: rect.top, right: rect.right - pad, bottom: rect.bottom },
        Ctl::EvAct => right_box(rect, sc(dpi, 200), sc(dpi, 32), pad),
        Ctl::WmPath => {
            let bw = sc(dpi, 108);
            let top = rect.top + sc(dpi, 12);
            adv[0] = RECT { left: rect.right - pad - bw, top, right: rect.right - pad, bottom: top + sc(dpi, 32) };
            let eh = sc(dpi, 32);
            RECT { left: rect.left + pad, top: rect.bottom - sc(dpi, 12) - eh, right: rect.right - pad, bottom: rect.bottom - sc(dpi, 12) }
        }
        Ctl::CopyEng => right_box(rect, sc(dpi, 140), sc(dpi, 22), pad),
        Ctl::Lang | Ctl::Face | Ctl::Start | Ctl::Theme | Ctl::ViewMode | Ctl::StartDir | Ctl::DelMode | Ctl::NewName | Ctl::Drop | Ctl::Layout => {
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
        Ctl::Adv | Ctl::AdvReset | Ctl::FolderDefault | Ctl::FolderReset | Ctl::FolderClear => {
            let (n, bw) = match spec.ctl {
                Ctl::Adv => (3, sc(dpi, 108)),
                Ctl::FolderDefault => (1, sc(dpi, 188)),
                Ctl::FolderReset | Ctl::FolderClear => (1, sc(dpi, 132)),
                _ => (1, sc(dpi, 96)),
            };
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

/// Stored favourites tree, depth-first, including children of a collapsed group.
fn fav_tree_rows() -> Vec<crate::FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        crate::tree_rows(&a.s.tree)
    })
}

fn fav_node(i: usize) -> Option<crate::FavRow> {
    fav_tree_rows().get(i).cloned()
}

fn fav_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    let dpi = sn.dpi.max(96);
    let all = fav_tree_rows();
    let rows = crate::filter_fav_rows(&all, rq);
    if !rq.is_empty() && rows.is_empty() && !has(tx(LIST_H), rq) {
        return y;
    }
    g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, LIST_H));
    y += sc(dpi, 28);
    let board = crate::layout_fav_board(&rows, left, right, y, dpi);
    g.items.push(Item::Table(board.table));
    g.items.push(Item::FavHead(board.head));
    if let Some(rect) = board.empty {
        g.items.push(Item::FavRow { i: usize::MAX, rect, rename: RECT::default(), btn: RECT::default() });
    }
    for slot in board.slots {
        g.items.push(Item::FavRow { i: slot.index, rect: slot.rect, rename: slot.rename, btn: slot.delete });
    }
    board.bottom + sc(dpi, 8)
}

fn key_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    // A nav query that matches no page must not fall through to this empty table.
    // `rq` filters rows, but the bar, header and a blank row were still emitted.
    if search_miss(&sn.query) {
        return y;
    }
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

fn is_wm_card(spec: &Spec) -> bool {
    matches!(spec.ctl, Ctl::WmStatus | Ctl::WmPath)
}

fn is_preview_card(spec: &Spec) -> bool {
    matches!(spec.ctl, Ctl::Toggle(TG_SPACE) | Ctl::Toggle(TG_WHEEL))
}

fn toggle_on(id: u8) -> bool {
    match id {
        TG_SINGLE => app(|a| a.s.single),
        TG_ONE => app(|a| a.s.one_window),
        TG_DBLUP => app(|a| a.s.dblup),
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
        TG_UPDATE => app(|a| a.s.update_check),
        TG_EV => app(|a| a.s.use_everything),
        TG_SPACE => app(|a| a.s.space_preview),
        TG_WHEEL => app(|a| a.s.peek_wheel_nav),
        TG_REMEMBER => app(|a| a.s.remember_views),
        TG_FOLDER => crate::folderapp::shown_on(),
        _ => false,
    }
}

fn ctl_id(c: Ctl) -> Option<i32> {
    Some(match c {
        Ctl::Lang => S_LANG,
        Ctl::Face => S_FACE,
        Ctl::Start => S_START,
        Ctl::Theme => S_THEME,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn ui_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct Lang(u8);
    impl Drop for Lang {
        fn drop(&mut self) {
            crate::UI.store(self.0, Ordering::Relaxed);
        }
    }

    fn lang(v: u8) -> Lang {
        let prev = crate::UI.load(Ordering::Relaxed);
        crate::UI.store(v, Ordering::Relaxed);
        Lang(prev)
    }

    fn shot(page: usize, query: &str) -> Snap {
        Snap {
            page,
            scroll: 0,
            dpi: 96,
            query: query.to_string(),
            keyq: String::new(),
            hot: None,
            font: HFONT::default(),
            small: HFONT::default(),
            title_font: HFONT::default(),
            mono: HFONT::default(),
            icon: HFONT::default(),
            ctx_open: Vec::new(),
        }
    }

    fn client() -> RECT {
        RECT { left: 0, top: 0, right: 1000, bottom: 700 }
    }

    #[test]
    fn no_results_copy_follows_the_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert_eq!(tx(NO_RESULTS), "No results");
        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(tx(NO_RESULTS), "검색 결과 없음");
        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!(tx(NO_RESULTS), "検索結果なし");
    }

    #[test]
    fn typing_agy_does_not_open_shortcuts_or_lay_out_its_table() {
        let _lock = ui_lock();
        let _keep = lang(1);
        // "Ag" matches only Shortcuts: chord text is "Ctrl+PageUp" / "Ctrl+PageDown".
        // "Agy" matches nothing. Keeping the page a prefix opened is what showed the empty table.
        assert!(page_match("Ag", 5));
        assert_eq!((0..PAGES.len()).find(|&p| page_match("Ag", p)), Some(5));
        assert!(search_miss("Agy"));
        assert!((0..PAGES.len()).all(|p| !page_match("Agy", p)));

        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!((0..PAGES.len()).find(|&p| page_match("Ag", p)), Some(5));
        assert!(search_miss("Agy"));
        crate::UI.store(1, Ordering::Relaxed);

        let mut anchor = 0usize;
        let mut page = 0usize;
        for q in ["A", "Ag", "Agy"] {
            (anchor, page) = commit_search(anchor, page, q);
        }
        assert_eq!(anchor, 0);
        assert_eq!(page, 0, "Agy must not leave Shortcuts selected");

        let g = geo(client(), &shot(page, "Agy"));
        assert!(g.show.iter().all(|on| !*on), "nav has no items");
        assert!(g.items.is_empty(), "content must not lay out the shortcuts table");
        assert!(!g.has_bar);

        (anchor, page) = commit_search(anchor, page, "");
        assert_eq!((anchor, page), (0, 0));
        let restored = geo(client(), &shot(page, ""));
        assert!(restored.show.iter().all(|on| *on));
        assert!(!restored.items.is_empty(), "clearing the search restores the page");
    }

    #[test]
    fn a_miss_keeps_shortcuts_when_that_was_the_page_but_hides_the_table() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert_eq!(pick_search(5, 5, "Agy"), 5);
        assert_eq!(commit_search(5, 5, "Agy"), (5, 5));
        assert!(search_miss("Agy"));
        let g = geo(client(), &shot(5, "Agy"));
        assert!(g.items.is_empty());
        assert!(g.show.iter().all(|on| !*on));
        // The page the user picked still wins when the query actually matches it.
        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(pick_search(0, 0, "단축키"), 5);
        assert_eq!(pick_search(2, 5, "단축키"), 5);
        assert!(!search_miss("단축키"));
        assert!(!search_miss(""));
    }

    #[test]
    fn default_folder_app_toggle_is_on_general_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == Ctl::Toggle(TG_FOLDER)));
        let (anchor, page) = commit_search(2, 2, "Use as default folder app");
        assert_eq!((anchor, page), (2, 0));
        let g = geo(client(), &shot(page, "Use as default folder app"));
        assert!(g.show[0] && hit(&g));
        assert!(hit(&geo(client(), &shot(0, "Win+E and special folders"))));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "기본 폴더 앱으로 사용");
        assert_eq!(page, 0);
        assert!(hit(&geo(client(), &shot(page, "기본 폴더 앱으로 사용"))));
        assert!(hit(&geo(client(), &shot(0, "특수 폴더"))));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "既定のフォルダーアプリとして使用");
        assert_eq!(page, 0);
        assert!(hit(&geo(client(), &shot(page, "既定のフォルダーアプリとして使用"))));
        assert!(hit(&geo(client(), &shot(0, "特殊フォルダー"))));
    }

    #[test]
    fn folder_view_memory_is_on_the_panes_page_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo, ctl: Ctl| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == ctl));
        let (anchor, page) = commit_search(0, 0, "Remember view per folder");
        assert_eq!((anchor, page), (0, 2));
        let g = geo(client(), &shot(page, "Remember view per folder"));
        assert!(hit(&g, Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "folder-views.json")), Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "Reset this folder")), Ctl::FolderReset));
        assert!(hit(&geo(client(), &shot(2, "Clear all remembered")), Ctl::FolderClear));
        assert!(hit(&geo(client(), &shot(2, "Default view for every folder")), Ctl::FolderDefault));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "폴더마다 보기");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "폴더마다 보기")), Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "이 폴더의 보기")), Ctl::FolderReset));
        assert!(hit(&geo(client(), &shot(2, "기억한 보기를 모두")), Ctl::FolderClear));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "フォルダーごとに表示");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "フォルダーごとに表示")), Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "このフォルダーの表示")), Ctl::FolderReset));
        assert!(hit(&geo(client(), &shot(2, "記憶した表示をすべて")), Ctl::FolderClear));
    }

    #[test]
    fn wheel_file_step_is_found_from_the_search_box() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == Ctl::Toggle(TG_WHEEL)));
        let (anchor, page) = commit_search(0, 0, "Wheel moves to next");
        assert_eq!((anchor, page), (0, 2));
        let g = geo(client(), &shot(page, "Wheel moves to next"));
        assert!(g.show[2] && hit(&g));
        let g = geo(client(), &shot(2, "Scrolling past the end"));
        assert!(hit(&g));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "휠로 다음/이전");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "휠로 다음/이전"))));
        assert!(hit(&geo(client(), &shot(2, "내용 끝에서 마우스 휠"))));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "ホイールで次");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "ホイールで次"))));
        assert!(hit(&geo(client(), &shot(2, "内容の端でさらにホイール"))));
    }

    #[test]
    fn clearing_a_real_shortcut_hit_stays_on_that_page() {
        let _lock = ui_lock();
        let _keep = lang(1);
        let (anchor, page) = commit_search(0, 0, "Ag");
        assert_eq!((anchor, page), (0, 5));
        assert_eq!(commit_search(anchor, page, ""), (5, 5));
    }

    #[test]
    fn a_miss_restores_the_page_from_before_search_moved() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert!(!page_match("Ag", 6));
        let (anchor, page) = commit_search(6, 6, "Ag");
        assert_eq!(anchor, 6);
        assert_ne!(page, 6);
        assert_eq!(commit_search(anchor, page, "Agy"), (6, 6));
        // Current page is kept when it still matches, even if an earlier page matches too.
        assert!(page_match("a", 3));
        assert_eq!(pick_search(0, 3, "a"), 3);
        let body = geo(client(), &shot(0, "language"));
        assert!(body.show[0]);
        assert!(!body.items.is_empty());
    }

    #[test]
    fn right_click_menu_page_is_found_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert_eq!(CTX_PAGE, 8);
        assert_eq!(PAGES.len(), PAGE_N);
        assert_eq!(tx(PAGES[CTX_PAGE]), "Right-click menu items");
        assert_eq!(tx(CTX_RESET_L), "Reset to all enabled");
        let (anchor, page) = commit_search(0, 0, "Right-click menu");
        assert_eq!((anchor, page), (0, CTX_PAGE));
        let g = geo(client(), &shot(page, "Right-click menu"));
        assert!(g.show[CTX_PAGE]);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxNote(_))));
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxBar { .. })));

        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(tx(PAGES[CTX_PAGE]), "우클릭 메뉴 항목");
        assert_eq!(tx(CTX_RESET_L), "모두 사용");
        assert_eq!(tx(CTX_EMPTY), "타사 메뉴 항목이 없어요");
        let (_, page) = commit_search(0, 0, "우클릭 메뉴 항목");
        assert_eq!(page, CTX_PAGE);
        let g = geo(client(), &shot(page, "우클릭"));
        assert!(g.show[CTX_PAGE]);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxBar { .. })));

        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!(tx(PAGES[CTX_PAGE]), "右クリックメニュー項目");
        assert_eq!(tx(CTX_RESET_L), "すべて有効に戻す");
        assert_eq!(tx(CTX_NOTE), CTX_NOTE[2]);
        let (_, page) = commit_search(0, 0, "右クリックメニュー");
        assert_eq!(page, CTX_PAGE);
        let g = geo(client(), &shot(page, "右クリック"));
        assert!(g.show[CTX_PAGE]);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxNote(_))));
    }
}
