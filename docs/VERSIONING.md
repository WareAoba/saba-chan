# Versioning Policy

> saba-chan 버전 관리 단일 진실의 출처(SoT) 및 정책 정의.

## 1. 버전의 종류

본 프로젝트에서 다음 **세 가지 버전 개념**을 명확히 분리한다:

### 1.1 제품 버전 (Product Version)
- **의미**: 사용자에게 보이는 마케팅 버전 (예: `0.1.1`, `1.0.0-rc.1`).
- **단일 SoT**: [`saba-chan/Cargo.toml`](../Cargo.toml) → `[workspace.package].version`.
- **상속**: 5개 Rust 컴포넌트(`saba-core`, `saba-chan-cli`, `saba-chan-updater-lib`, `saba-chan-updater`, `saba-chan-installer`)가 `version.workspace = true`로 자동 상속.
- **GUI(npm) 동기화**: [`scripts/sync-versions.ps1`](../scripts/sync-versions.ps1) 가 [`saba-chan-gui/package.json`](../saba-chan-gui/package.json) 의 `version`을 강제 일치시킨다.
- **Windows 리소스 메타데이터**: [`build.rs`](../build.rs) 가 `env!("CARGO_PKG_VERSION")` 로 자동 임베딩 → `.exe` 속성의 FileVersion/ProductVersion 일치.
- **증가 규칙**: SemVer (Breaking ↑MAJOR / Feature ↑MINOR / Fix ↑PATCH).

### 1.2 와이어 프로토콜 버전 (API Version)
- **의미**: 클라이언트 ↔ 데몬 / 클라이언트 ↔ 릴레이 사이의 IPC 계약 버전.
- **위치**: 릴레이 [`relay-server/src/index.ts`](../../server-chan/relay-server/src/index.ts) → `/info` 응답의 `apiVersion`.
- **증가 규칙**: 응답 스키마/엔드포인트 경로/헤더 시맨틱이 **Breaking** 으로 변경될 때만 증가. 단순 필드 추가는 증가시키지 않는다.
- **현재 값**: `2`.
- **관계**: 제품 버전과 **독립**. 제품 버전이 0.1.x → 1.0.x로 가도 wire 호환성이 유지되면 apiVersion은 그대로.

### 1.3 클라이언트 호환성 floor (`minAgentVersion`)
- **의미**: 릴레이가 받아들이는 최소 클라이언트(에이전트) 제품 버전.
- **위치**: [`relay-server/src/routes/heartbeat.ts`](../../server-chan/relay-server/src/routes/heartbeat.ts) `MIN_AGENT_VERSION`.
- **사용**: heartbeat 응답에서 `minVersion`으로 반환 → GUI/CLI에 강제 업그레이드 안내.
- **증가 규칙**: 구버전 클라이언트가 새 서버 기능을 필수로 요구할 때만 증가.

## 2. 컴포넌트별 버전 추적

### 2.1 코어 5개 (단일 버전)
| 컴포넌트 | 정의 위치 | 임베딩 방법 |
|---|---|---|
| `saba-core` | workspace.package | `env!("CARGO_PKG_VERSION")` |
| `saba-chan-cli` | workspace.package | clap `#[command(version)]` |
| `saba-chan-updater-lib` | workspace.package | `env!()` |
| `saba-chan-updater` (Tauri) | workspace.package | `env!()` |
| `saba-chan-installer` (Tauri) | workspace.package | `env!()` |
| `saba-chan-gui` (Electron) | package.json | `app.getVersion()` (sync-versions.ps1 강제) |

### 2.2 외부 확장 (독립 버전)
모듈/익스텐션은 코어와 독립 발전이 가능한 **외부 확장 모델**이다.
- 모듈: `saba-chan-modules/<name>/module.toml` → `[module].version`
- 익스텐션: `saba-chan-extensions/<name>/manifest.json` → `version`

호환성 제약은 다음 필드로 명시한다 (없으면 모든 코어 버전 허용):
- `module.toml`: `[compatibility].min_core_version`, `max_core_version`
- `manifest.json`: `min_app_version`, (선택) `max_app_version`

### 2.3 릴레이/뮤직/웹사이트 (별도 사이클)
서버 컴포넌트는 클라이언트와 독립 배포된다. apiVersion 호환성만 지키면 자유롭게 패치/마이너 가능.
- `server-chan/relay-server/package.json`
- `server-chan/music-server/package.json`
- `server-chan/website/package.json`

## 3. 런타임 버전 진실의 출처(SoT)

### 3.1 로컬 설치 상태
- **파일**: `%APPDATA%/Roaming/saba-chan/installed-manifest.json`
- **형식**: `HashMap<String, String>` (예: `"saba-core": "0.1.1"`)
- **갱신**: `UpdateManager::update_installed_versions_batch()` (apply 성공 시)
- **GUI 노출**: `GET /api/system/components` → 매니페스트 + 코어 fallback(데몬 자신 버전)

### 3.2 서버 매니페스트 (업데이트 가용 버전)
- 출처: 릴레이 서버 `GET /v1/updates/manifest/:version` → Ed25519 서명 검증 후 사용
- R2 바이너리 다운로드 URL과 SHA-256 해시 포함
- `release-manifest.json` (저장소 루트의 정적 템플릿)은 **폐기**되었다 — 사용 금지.

## 4. 릴리즈 절차 요약

1. CHANGELOG 항목 정리 — git-cliff 자동 생성:
   ```powershell
   # 다음 릴리즈 미정 상태로 갱신
   git cliff --unreleased --prepend CHANGELOG.md
   # 태그 확정 시 (예: v0.1.2)
   git cliff --tag v0.1.2 --output CHANGELOG.md
   ```
   설정 파일: [`cliff.toml`](../cliff.toml). Conventional Commits(`feat:`/`fix:`/`docs:` 등)를 따른 커밋만 포함된다.
2. `[workspace.package].version` 한 곳만 수정 (예: `0.1.1` → `0.1.2`).
3. `scripts/sync-versions.ps1` 실행 → npm 동기화.
4. 커밋 + 태그(`v0.1.2`) + GitHub Release.
5. CI(`release.yml`)가 `manifest.json` 생성 → R2 업로드 → 릴레이 등록.

## 5. 검증 방법

```powershell
# 모든 버전이 일치하는지 검증
.\scripts\sync-versions.ps1 -Check

# 빌드 시 자동 검증 (build-core.ps1 시작 시 점검)
.\scripts\build-core.ps1

# 메타데이터 확인
cargo metadata --no-deps --format-version=1 | python -c "import sys,json; [print(p['name'], p['version']) for p in json.loads(sys.stdin.read())['packages']]"
```
