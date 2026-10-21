# Grids Nightly AI 업데이트 인덱스

Claude가 제안하고 gpt-6.1-sol이 동의한 개선을 사람 확인 없이 구현해, 몇 개씩 묶어 공개한 기록입니다. 번호는 계속 이어집니다.
정식 릴리즈와 별개이며, 최신 exe는 [nightly-ai 릴리즈](https://github.com/efww/grids/releases/tag/nightly-ai)에 있습니다.

## 릴리즈 R1 · 2026-10-21 10:56 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #1 | 2026-10-21 10:55 | UX | Folder path suggestions in the address bar | When you type a path in a pane's address bar (Ctrl+L), a list of matching folders drops down, the same as in Explorer. Pick one with the arrow keys and press Enter to go there, so a mistyped path no longer just beeps. | 1 file changed, 3 insertions(+), 1 deletion(-) |

## 릴리즈 R2 · 2026-10-21 11:48 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #12 | 2026-10-21 11:44 | 기능 | Ctrl+V 파일 복사를 백그라운드 스레드에서 실행 | 파일을 Ctrl+C 후 Ctrl+V로 대량으로 붙여넣으면 Windows 복사 진행 창만 따로 돌고, 그동안 Grids의 다른 창 탐색, 트리, 레이아웃 조작이 계속 된다. 하단 상태줄에는 '복사 중…'이 표시되고, 복사 중에 Grids를 닫아도 복사는 끝까지 진행된다. | 2 files changed, 168 insertions(+), 10 deletions(-) |
| #20 | 2026-10-21 11:47 | UX | Show the active pane's folder in the window title | The title bar, taskbar button and Alt+Tab now show the active pane's folder, such as "Downloads - Grids", and update when you switch panes or folders. | 1 file changed, 11 insertions(+) |
| #18 | 2026-10-21 11:48 | 디자인 | 필터가 적용된 패널의 필터 칸에 선택 색 배지 표시 | 필터가 걸린 패널은 필터 칸 왼쪽 돋보기가 선택 색 배지와 필터 아이콘으로 바뀐다. 그래서 활성 패널이 아니어도 어느 패널에서 파일이 가려져 있는지 한눈에 보인다. 필터를 지우면 바로 돋보기로 돌아가고, 선택 색을 바꾸면 모든 패널의 배지가 함께 바뀐다. | 1 file changed, 23 insertions(+), 8 deletions(-) |

## 릴리즈 R3 · 2026-10-21 12:00 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #23 | 2026-10-21 11:58 | UX | Ctrl+1~9로 해당 번호 패널로 바로 이동 | Ctrl+1부터 Ctrl+9까지 누르면 해당 번호 패널이 바로 활성화되고 포커스가 그 목록으로 옮겨 갑니다. 번호는 ⇄ 메뉴의 '패널 N'과 같습니다. 단축키 창과 README에도 새 키가 나옵니다. | 3 files changed, 9 insertions(+) |
| #17 | 2026-10-21 11:59 | UX | Keep the current folder's name visible in narrow address bars | In small panes the address bar now shows the end of the path, the folder you're in, instead of the drive and the first few folders. You can tell 9 or 16 panes apart at a glance. Typing in the address bar and Ctrl+L work as before. | 1 file changed, 23 insertions(+), 3 deletions(-) |
| #21 | 2026-10-21 12:00 | UX | Open and use the favourites list from the keyboard (Ctrl+B, ↑/↓, Enter, Del), and put focus back where it was on Esc | Ctrl+B opens the favourites list. ↑/↓ moves through it, Enter opens the folder in the active pane, Del removes the highlighted entry, and Esc closes the list and puts the cursor back where it was. The shortcut also appears in the tooltip, the shortcut list and the README. | 3 files changed, 59 insertions(+), 22 deletions(-) |
