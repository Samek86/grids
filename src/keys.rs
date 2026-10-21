
pub(crate) const KEYS_EN: &str = "Ctrl+T\tNew tab\n\
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

pub(crate) const KEYS_KO: &str = "Ctrl+T\t새 탭\n\
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

pub(crate) const KEYS_JA: &str = "Ctrl+T\t新しいタブ\n\
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

