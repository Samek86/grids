use crate::imports::*;

pub(crate) const MAXP: usize = 16;
pub(crate) const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
// child ids inside a pane
pub(crate) const ID_BACK: i32 = 101;
pub(crate) const ID_FWD: i32 = 102;
pub(crate) const ID_UP: i32 = 103;
pub(crate) const ID_EDIT: i32 = 104;
pub(crate) const ID_OPEN_IN: i32 = 105;
pub(crate) const ID_FILTER: i32 = 106;
pub(crate) const ID_FAV: i32 = 107; // ☆+ / ★: add or remove the pane's folder from favourites
// top toolbar buttons (children of the main window)
pub(crate) const ID_TB_LAYOUT: i32 = 201;
pub(crate) const ID_TB_SAVE: i32 = 202;
pub(crate) const ID_TB_LOAD: i32 = 204;
pub(crate) const ID_TB_THEME: i32 = 205;
pub(crate) const ID_TB_FAV: i32 = 206;
pub(crate) const ID_TB_VIEW: i32 = 203;
pub(crate) const ID_TB_SET: i32 = 207; // gear: settings window
// menu ids
pub(crate) const ID_HIDDEN: usize = 1100;
pub(crate) const ID_KEYS: usize = 1101;
pub(crate) const ID_DBLUP: usize = 1103;
pub(crate) const ID_LANG_AUTO: usize = 1110;
pub(crate) const ID_LANG_EN: usize = 1111;
pub(crate) const ID_LANG_KO: usize = 1112;
pub(crate) const ID_LANG_JA: usize = 1113;
pub(crate) const ID_SETTINGS: usize = 1114;
pub(crate) const ID_NEWFILE: usize = 1115;
pub(crate) const ID_HELP: usize = 1105;
pub(crate) const ID_ABOUT: usize = 1106;
pub(crate) const HOME: &str = "https://github.com/efww/grids";
pub(crate) const ID_ACCENT: usize = 1400; // + preset index
pub(crate) const ID_ACCENT_PICK: usize = 1410;
pub(crate) const EM_SETSEL: u32 = 0xB1;
pub(crate) const EM_GETSEL: u32 = 0xB0;
pub(crate) const EM_SCROLLCARET: u32 = 0xB7;
pub(crate) const TIMER_STATUS: usize = 1;
pub(crate) const TIMER_FILTER: usize = 2;
pub(crate) const TIMER_REVEAL: usize = 3;
pub(crate) const TIMER_SETTLE: usize = 4; // on a pane: narrow column widths going back on after a folder change
pub(crate) const TIMER_SAVE: usize = 5; // debounced grids.json write
pub(crate) const SAVE_SOON_MS: u32 = 1500;
pub(crate) const SAVE_LATEST_MS: u128 = 30_000;
/// Hidden tabs that still hold Explorer windows. Older hidden tabs keep only their saved grid.
pub(crate) const PARK_CACHE: usize = 1;
pub(crate) const TIMER_NEWF: usize = 6; // on a pane: select and rename a folder just created
pub(crate) const TIMER_NEWFILE: usize = 7; // on a pane: select and rename a file just created
pub(crate) const TIMER_WINS: usize = 8; // after the first window is shown, open the other saved windows
// tree-view messages (the sidebar's SysTreeView32)
pub(crate) const TVM_GETNEXTITEM: u32 = 0x110A;
pub(crate) const TVM_ENSUREVISIBLE: u32 = 0x1114;
pub(crate) const TVM_GETITEMRECT: u32 = 0x1104;
pub(crate) const TVM_GETITEMHEIGHT: u32 = 0x111C;
pub(crate) const TVGN_CARET: usize = 9;
pub(crate) const TVM_SETBKCOLOR: u32 = 0x111D;
pub(crate) const TVM_GETTOOLTIPS: u32 = 0x1119;
// list-view messages (the Explorer list inside each pane)
pub(crate) const LVM_SETBKCOLOR: u32 = 0x1001;
pub(crate) const LVM_GETHEADER: u32 = 0x101F;
pub(crate) const LVM_GETTOOLTIPS: u32 = 0x104E;
pub(crate) const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;
pub(crate) const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
pub(crate) const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
pub(crate) const LVS_EX_AUTOSIZECOLUMNS: usize = 0x1000_0000;
pub(crate) const LVS_EX_COLUMNSNAPPOINTS: usize = 0x4000_0000;
pub(crate) const HDM_GETITEMCOUNT: u32 = 0x1200;
pub(crate) const LVM_SETTEXTCOLOR: u32 = 0x1024;
pub(crate) const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
pub(crate) const LVM_SETSELECTEDCOLUMN: u32 = 0x108C;
pub(crate) const LVM_GETSELECTEDCOLUMN: u32 = 0x10AE;
pub(crate) const TVM_SETTEXTCOLOR: u32 = 0x111E;
pub(crate) const COPY_ID: usize = 0x4752_4944; // "GRID": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
pub(crate) const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

