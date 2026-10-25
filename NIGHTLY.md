# Panegrid Nightly AI 업데이트 인덱스

Claude가 제안하고 gpt-6.1-sol이 동의한 개선을 사람 확인 없이 구현해, 몇 개씩 묶어 공개한 기록입니다. 번호는 계속 이어집니다.
정식 릴리즈와 별개이며, 최신 exe는 [nightly-ai 릴리즈](https://github.com/efww/grids/releases/tag/nightly-ai)에 있습니다.

## 릴리즈 R1 · 2026-10-21 10:56 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #1 | 2026-10-21 10:55 | UX | Folder path suggestions in the address bar | When you type a path in a pane's address bar (Ctrl+L), a list of matching folders drops down, the same as in Explorer. Pick one with the arrow keys and press Enter to go there, so a mistyped path no longer just beeps. | 1 file changed, 3 insertions(+), 1 deletion(-) |

## 릴리즈 R2 · 2026-10-21 11:48 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #12 | 2026-10-21 11:44 | 기능 | Ctrl+V 파일 복사를 백그라운드 스레드에서 실행 | 파일을 Ctrl+C 후 Ctrl+V로 대량으로 붙여넣으면 Windows 복사 진행 창만 따로 돌고, 그동안 Panegrid의 다른 창 탐색, 트리, 레이아웃 조작이 계속 된다. 하단 상태줄에는 '복사 중…'이 표시되고, 복사 중에 Panegrid를 닫아도 복사는 끝까지 진행된다. | 2 files changed, 168 insertions(+), 10 deletions(-) |
| #20 | 2026-10-21 11:47 | UX | Show the active pane's folder in the window title | The title bar, taskbar button and Alt+Tab now show the active pane's folder, such as "Downloads - Panegrid", and update when you switch panes or folders. | 1 file changed, 11 insertions(+) |
| #18 | 2026-10-21 11:48 | 디자인 | 필터가 적용된 패널의 필터 칸에 선택 색 배지 표시 | 필터가 걸린 패널은 필터 칸 왼쪽 돋보기가 선택 색 배지와 필터 아이콘으로 바뀐다. 그래서 활성 패널이 아니어도 어느 패널에서 파일이 가려져 있는지 한눈에 보인다. 필터를 지우면 바로 돋보기로 돌아가고, 선택 색을 바꾸면 모든 패널의 배지가 함께 바뀐다. | 1 file changed, 23 insertions(+), 8 deletions(-) |

## 릴리즈 R3 · 2026-10-21 12:00 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #23 | 2026-10-21 11:58 | UX | Ctrl+1~9로 해당 번호 패널로 바로 이동 | Ctrl+1부터 Ctrl+9까지 누르면 해당 번호 패널이 바로 활성화되고 포커스가 그 목록으로 옮겨 갑니다. 번호는 ⇄ 메뉴의 '패널 N'과 같습니다. 단축키 창과 README에도 새 키가 나옵니다. | 3 files changed, 9 insertions(+) |
| #17 | 2026-10-21 11:59 | UX | Keep the current folder's name visible in narrow address bars | In small panes the address bar now shows the end of the path, the folder you're in, instead of the drive and the first few folders. You can tell 9 or 16 panes apart at a glance. Typing in the address bar and Ctrl+L work as before. | 1 file changed, 23 insertions(+), 3 deletions(-) |
| #21 | 2026-10-21 12:00 | UX | Open and use the favourites list from the keyboard (Ctrl+B, ↑/↓, Enter, Del), and put focus back where it was on Esc | Ctrl+B opens the favourites list. ↑/↓ moves through it, Enter opens the folder in the active pane, Del removes the highlighted entry, and Esc closes the list and puts the cursor back where it was. The shortcut also appears in the tooltip, the shortcut list and the README. | 3 files changed, 59 insertions(+), 22 deletions(-) |

## 릴리즈 R4 · 2026-10-21 12:06 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #26 | 2026-10-21 12:06 | 디자인 | 다크 테마에서 툴팁·파일 정보 말풍선도 어둡게 | 다크 테마에서 버튼에 마우스를 올렸을 때 뜨는 이름 말풍선, 파일 위에 뜨는 정보 말풍선, 트리의 잘린 폴더 이름 말풍선이 흰 상자 대신 어두운 색으로 나옵니다. 라이트 테마로 바꾸면 원래대로 돌아옵니다. | 1 file changed, 16 insertions(+), 1 deletion(-) |

## 릴리즈 R5 · 2026-10-21 12:29 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #28 | 2026-10-21 12:22 | 기능 | 활성 패널 잠깐 크게 보기 토글 (Ctrl+Shift+M / 패널 머리줄 더블클릭) | Ctrl+Shift+M을 누르거나 패널 머리줄의 빈 곳을 더블클릭하면, 지금 쓰는 패널이 그리드 전체를 차지하도록 커집니다. 한 번 더 누르거나 다른 패널로 옮기면(Ctrl+Tab, Ctrl+1~9, ⇄, 다른 곳에서 폴더 열기) 맞춰 둔 칸 비율 그대로 원래 배치로 돌아옵니다. 크게 보는 동안에는 상태줄에 복원 방법이 표시됩니다. | 3 files changed, 66 insertions(+), 7 deletions(-) |
| #30 | 2026-10-21 12:26 | 기능 | ⇄ 메뉴에서 선택한 파일을 다른 패널로 복사·이동 (Total Commander F5/F6식, 선택 전체가 아니면 보내지 않음) | 파일을 고른 뒤 ⇄ 버튼을 누르면 '선택한 3개 복사 ▸ / 이동 ▸' 아래에 다른 패널들이 나오고, 하나를 고르면 그 패널 폴더로 바로 복사되거나 옮겨진다. 복사하는 동안에도 Panegrid를 계속 쓸 수 있다. 보낼 수 없는 선택이면 메뉴에 회색 글씨로 이유가 나온다(너무 많음, 일반 파일 아님). | 3 files changed, 115 insertions(+), 14 deletions(-) |
| #27 | 2026-10-21 12:29 | 기능 | 최근 폴더 목록: Alt+↓ 또는 뒤로/앞으로 버튼 오른쪽 클릭 (모든 패널 공통, 재시작·테마·언어 변경에도 순서 유지) | 패널의 ← 또는 → 버튼을 오른쪽 클릭하거나 Alt+↓를 누르면, 모든 패널에서 최근에 드나든 폴더가 최신 순으로 메뉴에 나온다. 하나를 고르면 지금 패널이 그 폴더로 간다. Panegrid를 다시 켜거나 테마·언어를 바꿔도 목록과 순서가 그대로다. | 3 files changed, 121 insertions(+), 5 deletions(-) |

## 릴리즈 R6 · 2026-10-21 12:44 KST · 업데이트 2개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #29 | 2026-10-21 12:31 | 디자인 | 패널 머리줄 왼쪽에 패널 번호 표시 (주소 칸이 좁아지면 생략) | 각 패널 머리줄 맨 왼쪽에 1, 2, 3… 번호가 작게 보이고, 활성 패널에서는 선택 색 위 글자색으로 나옵니다. 패널을 아주 좁게 줄이면 번호가 사라지고 버튼과 주소 칸은 지금과 똑같이 놓입니다. | 4 files changed, 117 insertions(+), 12 deletions(-) |
| #31 | 2026-10-21 12:39 | 기능 | 선택한 폴더의 크기를 상태줄에 백그라운드로 계산해 표시 (작업자 1개·세대 취소·복사 중 일시정지·누락 시 '일부만') | 패널에서 폴더를 고르면 아래 상태줄에 '2개 선택 (3.4 GB…)'처럼 크기가 나오고, 계산이 진행되면서 숫자가 늘다가 끝나면 '…'이 사라집니다. 읽지 못한 항목이 있으면 '(3.4 GB, 일부만)'으로 알려 주고, 계산은 뒤에서 돌기 때문에 그동안 탐색과 붙여넣기가 그대로 됩니다. | 4 files changed, 409 insertions(+), 24 deletions(-) |

## 릴리즈 R7 · 2026-10-21 13:03 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #39 | 2026-10-21 12:53 | UX | Ctrl+Shift+C: 목록에서 선택한 항목(없으면 현재 폴더)의 전체 경로를 클립보드로 복사 + 상태줄 맨 앞에 확인 표시 | 패널 목록에서 파일을 고르고 Ctrl+Shift+C를 누르면 "C:\…\파일.pdf" 형식의 전체 경로가 복사되고, 상태줄 맨 앞에 '경로 2개 복사함'이 3초 동안 뜹니다. 아무것도 고르지 않았으면 지금 보고 있는 폴더의 경로가 복사됩니다. | 3 files changed, 61 insertions(+), 3 deletions(-) |
| #32 | 2026-10-21 12:54 | 기능 | 주소창에서 상대 경로·파일 열기·명령(cmd, wt, powershell) 실행 | 주소창에 cmd, wt, powershell을 치고 Enter를 누르면 그 패널이 보고 있는 폴더에서 터미널이 열립니다. `..`이나 하위 폴더 이름만 쳐도 이동하고, 파일 경로를 치면 그 파일이 열립니다. 맞는 게 없으면 예전처럼 경고음만 납니다. | 4 files changed, 97 insertions(+), 13 deletions(-) |
| #36 | 2026-10-21 13:02 | 기능 | 폴더를 패널 머리줄에 끌어다 놓으면 그 패널에서 열기 (Q-Dir·Directory Opus식, 효과는 LINK→COPY→NONE 하나만 돌려줌) | 다른 패널, 왼쪽 트리, 탐색기, 바탕화면에서 폴더를 어떤 패널의 윗줄(주소·버튼 줄)로 끌고 가면 그 패널의 윗줄과 테두리가 선택 색으로 바뀝니다. 놓으면 그 패널이 그 폴더로 이동합니다. 파일을 놓으면 그 파일이 든 폴더가 열리고, 원본은 복사도 이동도 되지 않습니다. | 3 files changed, 143 insertions(+), 7 deletions(-) |

## 릴리즈 R8 · 2026-10-21 13:34 KST · 업데이트 2개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #46 | 2026-10-21 13:18 | UX | Alt+Shift+방향키로 상하좌우 이웃 패널로 이동 (Windows Terminal·tmux·VS Code 창 이동식, 빈 슬롯은 실제 번호로 매핑) | Alt+Shift+→를 누르면 바로 오른쪽 패널로, Alt+Shift+↓를 누르면 바로 아래 패널로 커서가 옮겨 간다. 패널을 16개 띄워도 번호를 세거나 Ctrl+Tab을 여러 번 누를 필요 없이, 화면에 보이는 방향대로 이동할 수 있다. | 3 files changed, 63 insertions(+) |
| #50 | 2026-10-21 13:20 | 기능 | 빈 곳 더블클릭을 두 번의 '목록 배경' 클릭으로 직접 판별해 상위 폴더로 이동 (MSAA 목록 역할 판정 + 조회 전후 탐색 세대 재확인, 세대는 마지막에 읽음) | Windows 11 자세히 보기를 포함해 모든 패널에서 파일 목록 아래쪽 같은 빈 곳을 더블클릭하면 다시 상위 폴더로 올라갑니다. 파일·그룹 머리·접기 버튼을 누른 경우, Ctrl·Shift를 누른 채 클릭한 경우, 클릭하는 사이 폴더가 바뀐 경우에는 올라가지 않습니다. | 2 files changed, 162 insertions(+), 12 deletions(-) |
