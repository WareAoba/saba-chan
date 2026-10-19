# E2E 하네스

빌드한 바이너리(`saba-core`, `saba-runner`, `saba-chan-cli`)와 Tauri GUI를 그대로 띄우고, 모듈·익스텐션 저장소 체크아웃을 그대로 써서 사용자가 하는 흐름을 밖에서 조작한다. 데몬 내부 함수는 부르지 않는다(그건 `cargo test`의 몫).

## 실행

| 어디서 | 명령 | 도는 것 |
|---|---|---|
| 리눅스 | `tests/e2e/run-cloud.sh` (`--slow`, `-k 식`) | 외부망 없이 도는 전부 |
| Windows PC | `powershell -ExecutionPolicy Bypass -File tests\e2e\run-pc.ps1` | 위 + Windows 전용(메모리 상한, 격리 실행, 터미널로 열기) |
| | `... -Network` | + 진짜 마인크래프트 바닐라·Paper·Fabric 설치·실행 |
| | `... -Network -Steam` | + Palworld·Zomboid (SteamCMD, 수 GB) |
| | `... -InstalledLayout` | 같은 시험을 설치본과 같은 폴더 구성으로 (아래 "설치본 구성") |

결과는 `<루트>/report-<시각>.md`, 시험마다 데몬 로그·콘솔·CLI 출력·GUI 스크린샷은 `<루트>/run-<시각>/<시험>/artifacts/`에 남는다. 루트는 `SABA_E2E_ROOT`(PC 기본 `C:\SabaE2E`, 리눅스 기본 `/tmp/saba-e2e`).

형제 저장소 위치는 `SABA_E2E_MODULES_REPO`, `SABA_E2E_EXTENSIONS_REPO`, `SABA_E2E_SERVER_CHAN_REPO`(기본: 이 저장소 옆 폴더), 바이너리 위치는 `SABA_E2E_BIN_DIR`(기본 `target/debug`)로 바꾼다.

## 격리

- 시험마다 폴더 하나를 만들고 `SABA_DATA_DIR`·`SABA_MODULES_PATH`·`SABA_EXTENSIONS_DIR`·`SABA_TOKEN_PATH`·`SABA_IPC_PORT`(빈 포트)와 `HOME`·`APPDATA`·`LOCALAPPDATA`·`TEMP`·`XDG_*`를 그 안으로 돌린다.
- 시작과 끝에 실제 데이터 폴더(`%APPDATA%\saba-chan`, `~/.config/saba-chan` 등)의 파일 목록·크기·수정 시각을 비교한다. 바뀌면 실행 전체가 실패한다(`run-*/guard.md`).
- 설치본 구성(`SABA_E2E_LAYOUT=installed`, `run-pc.ps1 -InstalledLayout`): 경로용 `SABA_*` 변수를 주지 않는다. 데몬·CLI·GUI는 설치본처럼 기본 위치를 스스로 찾고(`%APPDATA%\saba-chan`과 그 아래 `modules`·`extensions`·`instances`·`.ipc_token`, 리눅스는 `~/.config/saba-chan`), 그 `APPDATA`·`HOME`만 시험 폴더의 가짜 프로필이다. `-Root`를 주지 않으면 시험 루트가 `%USERPROFILE%\SabaE2E`라서 실제 `%APPDATA%`와 같은 권한(사용자 프로필 상속)을 받는다. 실제 프로필·설치본은 여전히 건드리지 않는다. 바이너리는 설치 폴더(`C:\Program Files\Saba-chan`)가 아니라 빌드 폴더에서 뜬다는 점만 설치본과 다르다.
- 통과한 시험의 폴더는 `artifacts/`만 남기고 지운다(`SABA_E2E_KEEP=1`이면 남긴다).
- 남은 프로세스는 명령줄에 이번 실행 폴더 경로가 들어간 것만 끝낸다. 이름으로 죽이지 않는다.

## 층과 파일

| 파일 | 층 | 내용 |
|---|---|---|
| `test_daemon_boot.py` | L1 | 인자·인증·Host 검사·단일 인스턴스·종료·재기동 |
| `test_server_run.py` | L1 | 가져오기, EULA, 시작·명령·정지, 크래시·자동 재시작·포기, 멈춤·강제 정지, 동시 실행·포트 충돌, 데몬 재시작 뒤 재연결, 콘솔 폭주 |
| `test_backup_schedule.py` | L1 | 백업·복원·보관, 예약(공지·백업·재시작 예고) |
| `test_cli.py` | L1c | 같은 흐름을 CLI로, 종료 코드 계약 |
| `test_relay_agent.py` | L2a | 가짜 relay: 서명·heartbeat·명령 왕복·알림 이벤트·버전 하한·페어링·비밀값/경로 미노출 |
| `test_relay_real.py` | L2b | 실제 relay-server(임시 PostgreSQL): 페어링·heartbeat·명령 왕복·/events·토큰 교체 |
| `test_install_mirror.py` | L3 | 마인크래프트 설치를 가로채기 미러로: 바닐라·Paper·Purpur·Fabric, 해시 불일치, 버전 목록, 버전 지정 |
| `test_updates.py` | L4 | 업데이트 확인·내려받기: 서명·변조·플랫폼·다운그레이드·해시 (`--features mock-relay` 빌드) |
| `test_platform.py` | L5 | 메모리 상한(Job), 격리 실행(AppContainer), 모니터, 터미널로 열기 |
| `test_real_games.py` | L5 | 진짜 게임 설치·실행 (`network`, `steam`) |
| `test_official_catalog.py` | L5 | 운영 카탈로그에서 받은 공식 모듈·익스텐션: 신뢰 표시 verified, 격리 없이 마인크래프트 실행(데몬이 찾은 Java)·Palworld 설치 (`network`, `steam`). `test_real_games.py`는 저장소 복사본(unverified)이라 Windows에서 격리 상태로 돈다 — 둘 다 돌려야 두 상태를 본다 |
| `test_gui.py` | L6 | Tauri GUI(tauri-driver WebDriver): 데몬에 붙기, 켜기·콘솔 명령·끄기, 데몬 끊김·재연결, 약관 동의, 데몬 기동 실패(배포 빌드) |

고정 장치:

- `fixtures/fake-minecraft/FakeMinecraft.java`: 바닐라 로그 형식·EULA·`server.properties`·포트 바인드·stdin 명령을 흉내 내는 가짜 서버. 시험 명령 `e2e-crash`, `e2e-hang`, `e2e-alloc <MB>`, `e2e-exit <코드>`, `e2e-spam <N>`. 하네스가 `javac`로 빌드해 캐시한다(JDK 17+).
- `harness/mirror.py`: 시험용 CA와 HTTPS 가로채기 프록시. `HTTPS_PROXY`·`SSL_CERT_FILE`로 모듈 Python의 다운로드를 fixture로 돌리고, 등록하지 않은 주소는 거절·기록한다.
- `harness/webdriver.py`: tauri-driver를 띄우고 W3C WebDriver로 GUI를 조작한다(접근성 이름·글자로 찾기). 리눅스는 WebKitWebDriver·Xvfb, Windows는 WebView2와 같은 버전의 msedgedriver가 필요하다(`run-pc.ps1`이 받는다). `SABA_E2E_GUI_EXE`로 다른 빌드를 시험한다.
- `harness/fake_relay.py`, `harness/real_relay.py`(server-chan `relay-server/test/e2e/harness.ts`를 띄움), `harness/fake_updates.py`.

## 제품 버그를 찾으면

시험은 그대로 두고 `@pytest.mark.xfail(strict=True, reason="E2E-Fn: …")`로 표시한다. 고쳐지면 XPASS로 실패하므로 표시를 걷는다. 발견 목록은 `/mnt/project-files/e2e/findings.md`.
