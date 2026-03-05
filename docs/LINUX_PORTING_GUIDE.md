# Saba-chan Linux 포팅 가이드

> **원칙: 기존 Windows 로직은 절대 수정하지 않는다.**  
> 모든 변경은 `#[cfg(unix)]`, `process.platform !== 'win32'`, `if-else` 분기 추가의 형태로만 이루어져야 한다.

---

## 목차

1. [Electron GUI 빌드 설정](#1-electron-gui-빌드-설정)
2. [릴리즈 매니페스트](#2-릴리즈-매니페스트)
3. [CI/CD 워크플로우](#3-cicd-워크플로우)
4. [빌드 스크립트](#4-빌드-스크립트)
5. [Updater — lib.rs 바이너리 교체 로직](#5-updater--librs-바이너리-교체-로직)
6. [GUI main.js 크로스플랫폼 보완](#6-gui-mainjs-크로스플랫폼-보완)
7. [Systemd 서비스 파일 신규 작성](#7-systemd-서비스-파일-신규-작성)
8. [인스톨러 조건부 컴파일](#8-인스톨러-조건부-컴파일)
9. [게임 모듈 TOML — Linux 경로 추가](#9-게임-모듈-toml--linux-경로-추가)
10. [Docker 확장 — ARM64 분기](#10-docker-확장--arm64-분기)

---

## 1. Electron GUI 빌드 설정

### 대상 파일
- `saba-chan-gui/package.json` (build 섹션)

### 현재 상태
```json
"win": {
  "target": [{ "target": "portable", "arch": ["x64"] }],
  "icon": "build/icon.ico"
},
"portable": {
  "artifactName": "${productName}.exe"
}
```
Linux 타겟이 전혀 정의되어 있지 않다.

### 해야 할 일

#### 1-1. `linux` 빌드 타겟 블록 추가

기존 `"win"` 블록을 건드리지 않고, 같은 레벨에 `"linux"` 블록을 추가한다.

```json
"linux": {
  "target": [
    { "target": "AppImage", "arch": ["x64"] },
    { "target": "tar.gz",   "arch": ["x64"] }
  ],
  "icon": "build/icon.png",
  "category": "Game",
  "artifactName": "${productName}-${version}-linux-${arch}.${ext}"
}
```

- `AppImage`: 범용 Linux 배포 포맷 (단일 실행파일)
- `tar.gz`: 포터블 아카이브
- `icon.png`: Linux용 아이콘 (ICO 대신 PNG 필요 — 256×256 이상)
- 필요 시 `deb`/`rpm` 타겟도 추가 가능

#### 1-2. `extraFiles` — 플랫폼별 바이너리 경로 분기

현재 `extraFiles`에 `.exe`가 하드코딩되어 있다:
```json
"extraFiles": [
  { "from": "../target/release/saba-core.exe", "to": "." },
  { "from": "../target/release/saba-chan-updater.exe", "to": "." }
]
```

**방법 A — 플랫폼별 `extraFiles` 오버라이드 (권장):**

electron-builder는 `win.extraFiles` / `linux.extraFiles`로 플랫폼별 오버라이드를 지원한다. 공통 `extraFiles`에서 바이너리를 제거하고 각 플랫폼 섹션으로 이동:

```json
"extraFiles": [
  { "from": "../shared", "to": "shared" },
  { "from": "../config", "to": "config" },
  { "from": "../locales", "to": "locales" },
  { "from": "../discord_bot", "to": "discord_bot", "filter": ["..."] }
],
"win": {
  "extraFiles": [
    { "from": "../target/release/saba-core.exe", "to": "." },
    { "from": "../target/release/saba-chan-updater.exe", "to": "." }
  ]
},
"linux": {
  "extraFiles": [
    { "from": "../target/release/saba-core", "to": "." },
    { "from": "../target/release/saba-chan-updater", "to": "." }
  ]
}
```

#### 1-3. Linux 아이콘 파일 추가

`saba-chan-gui/build/` 디렉토리에 `icon.png` (256×256 이상) 추가. 기존 `icon.ico`는 그대로 둔다.

---

## 2. 릴리즈 매니페스트

### 대상 파일
- `release-manifest.json`

### 현재 상태
모든 에셋 이름이 `*-windows-x64` 패턴:
```json
"saba-core": { "asset": "saba-core-windows-x64.zip" }
```

### 해야 할 일

매니페스트를 플랫폼별 구조로 확장한다. 기존 Windows 에셋 정의는 그대로 유지:

```json
{
  "release_version": "0.1.0",
  "platforms": {
    "windows-x64": {
      "components": {
        "saba-core":   { "version": "0.1.0", "asset": "saba-core-windows-x64.zip" },
        "cli":         { "version": "0.1.0", "asset": "saba-chan-cli-windows-x64.zip" },
        "gui":         { "version": "0.1.0", "asset": "saba-chan-gui-windows-x64.zip" },
        "updater":     { "version": "0.1.0", "asset": "saba-chan-updater-windows-x64.zip" },
        "installer":   { "version": "0.1.0", "asset": "saba-chan-installer-windows-x64.zip" }
      }
    },
    "linux-x64": {
      "components": {
        "saba-core":   { "version": "0.1.0", "asset": "saba-core-linux-x64.tar.gz" },
        "cli":         { "version": "0.1.0", "asset": "saba-chan-cli-linux-x64.tar.gz" },
        "gui":         { "version": "0.1.0", "asset": "saba-chan-gui-linux-x64.tar.gz" },
        "updater":     { "version": "0.1.0", "asset": "saba-chan-updater-linux-x64.tar.gz" }
      }
    },
    "shared": {
      "components": {
        "discord_bot": { "version": "0.1.0", "asset": "discord-bot.zip" },
        "locales":     { "version": "0.1.0", "asset": "locales.zip" }
      }
    }
  }
}
```

> ⚠️ 이 구조 변경은 매니페스트를 파싱하는 모든 코드(`updater/src/lib.rs` 등)에 대응 수정이 필요하다. 기존 파싱 로직에 `platforms` 키 존재 시 분기를 추가하거나, 매니페스트 버전 필드로 하위호환을 유지한다.
>
> **대안 (최소 변경):** 기존 구조를 유지하면서 각 component에 `asset_linux` 필드만 추가:
> ```json
> "saba-core": {
>   "version": "0.1.0",
>   "asset": "saba-core-windows-x64.zip",
>   "asset_linux": "saba-core-linux-x64.tar.gz"
> }
> ```

---

## 3. CI/CD 워크플로우

### 대상 파일
- `.github/workflows/build.yml`
- `.github/workflows/ci.yml`
- `.github/workflows/test.yml`

### 현재 상태
Rust 빌드/테스트가 `windows-latest`에서만 실행된다.

### 해야 할 일

#### 3-1. `build.yml` — 빌드 매트릭스 추가

기존 Windows 빌드 job은 그대로 두고, Linux 빌드 job을 **별도로** 추가한다:

```yaml
jobs:
  build-windows:
    runs-on: windows-latest
    # ... 기존 Windows 빌드 로직 전체 유지 (변경 없음)

  build-linux:                        # ← 신규 job
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Build Rust binaries
        run: cargo build --release

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Build Electron GUI (Linux)
        working-directory: saba-chan-gui
        run: |
          npm ci
          npm run build
          npx electron-builder --linux

      - name: Upload Linux artifacts
        uses: actions/upload-artifact@v4
        with:
          name: linux-x64-binaries
          path: |
            target/release/saba-core
            target/release/saba-chan-cli
            target/release/saba-chan-updater
            saba-chan-gui/electron-dist/*.AppImage
            saba-chan-gui/electron-dist/*.tar.gz
```

#### 3-2. `ci.yml` — Rust 테스트 매트릭스

기존 `rust` job에 매트릭스 strategy를 추가한다. 기존 Windows 설정은 첫 번째 항목으로 보존:

```yaml
  rust:
    strategy:
      matrix:
        os: [windows-latest, ubuntu-latest]    # ← ubuntu 추가
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all
```

#### 3-3. `test.yml` — 동일 패턴

```yaml
  test:
    strategy:
      matrix:
        os: [windows-latest, ubuntu-latest]
    runs-on: ${{ matrix.os }}
```

---

## 4. 빌드 스크립트

### 대상
- `scripts/` 디렉토리에 `build-linux.sh` 신규 작성

### 현재 상태
빌드 스크립트가 모두 `.ps1`(PowerShell)이다. `run-test.sh`만 bash로 존재한다.

### 해야 할 일

`scripts/build-windows.ps1`의 로직을 bash로 포팅한 `scripts/build-linux.sh`를 신규 작성한다. **`build-windows.ps1`은 건드리지 않는다.**

```bash
#!/usr/bin/env bash
set -euo pipefail

# --- Saba-chan Linux Release Build Script ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="${1:-release-build}"
RELEASE_DIR="$PROJECT_ROOT/$OUTPUT_DIR"
DIST_DIR="$RELEASE_DIR/saba-chan-v0.1.0"

echo "=== Saba-chan Linux Build ==="

# 1. Rust 빌드
echo "[1/5] Building Rust binaries..."
cd "$PROJECT_ROOT"
cargo build --release

# 2. Electron GUI 빌드
echo "[2/5] Building Electron GUI..."
cd "$PROJECT_ROOT/saba-chan-gui"
npm ci
npm run build
npx electron-builder --linux

# 3. 배포 디렉토리 구성
echo "[3/5] Assembling distribution..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

cp "$PROJECT_ROOT/target/release/saba-core"          "$DIST_DIR/"
cp "$PROJECT_ROOT/target/release/saba-chan-cli"       "$DIST_DIR/"
cp "$PROJECT_ROOT/target/release/saba-chan-updater"   "$DIST_DIR/"
cp -r "$PROJECT_ROOT/shared"                         "$DIST_DIR/shared"
cp -r "$PROJECT_ROOT/config"                         "$DIST_DIR/config"
cp -r "$PROJECT_ROOT/locales"                        "$DIST_DIR/locales"
cp -r "$PROJECT_ROOT/discord_bot"                    "$DIST_DIR/discord_bot"

chmod +x "$DIST_DIR/saba-core"
chmod +x "$DIST_DIR/saba-chan-cli"
chmod +x "$DIST_DIR/saba-chan-updater"

# 4. 아카이브 생성
echo "[4/5] Creating archives..."
cd "$RELEASE_DIR"
tar -czf saba-core-linux-x64.tar.gz     -C "$DIST_DIR" saba-core
tar -czf saba-chan-cli-linux-x64.tar.gz  -C "$DIST_DIR" saba-chan-cli

# GUI: electron-builder 출력물 복사
cp "$PROJECT_ROOT/saba-chan-gui/electron-dist/"*.AppImage "$RELEASE_DIR/" 2>/dev/null || true
cp "$PROJECT_ROOT/saba-chan-gui/electron-dist/"*.tar.gz   "$RELEASE_DIR/" 2>/dev/null || true

# 5. 요약
echo "[5/5] Done!"
ls -lh "$RELEASE_DIR"/*.tar.gz "$RELEASE_DIR"/*.AppImage 2>/dev/null
```

---

## 5. Updater — lib.rs 바이너리 교체 로직

### 대상 파일
- `updater/src/lib.rs`

### 5-1. `apply_gui_update` — `.exe` 하드코딩 해소

**위치:** L1948~L1982 부근

현재:
```rust
let portable_exe = self.install_root.join("saba-chan-gui.exe");
```

수정 — 기존 Windows 줄을 `cfg!` 분기로 감싼다:
```rust
let gui_exe_name = if cfg!(target_os = "windows") {
    "saba-chan-gui.exe"
} else {
    "saba-chan-gui"
};
let portable_exe = self.install_root.join(gui_exe_name);
```

### 5-2. 아카이브 추출 시 `.exe` 확장자 체크

**위치:** L1913~L1935 부근

현재 로직은 `.exe` 확장자가 있을 때만 `.old`로 rename한다. Linux 바이너리는 확장자가 없으므로 이 분기를 통과하지 못한다.

수정 — 기존 if 조건 **아래에** Linux용 else-if를 추가:
```rust
if out_path.exists() && out_path.extension().map(|e| e == "exe").unwrap_or(false) {
    // 기존 Windows 로직 그대로 유지
    let backup = out_path.with_extension("exe.old");
    if let Err(e) = Self::rename_with_retry(&out_path, &backup, 5) {
        anyhow::bail!("Cannot replace {}: {}", out_path.display(), e);
    }
} else if cfg!(unix) && out_path.exists() && !out_path.extension().is_some() {
    // Linux: 실행 중 바이너리도 unlink 후 재생성 가능
    // 안전하게 .old 백업 후 교체
    let backup = out_path.with_extension("old");
    let _ = std::fs::rename(&out_path, &backup);  // 실패해도 덮어쓰기 진행
}
```

### 5-3. `process_names` 매칭 — 바이너리 이름 분기

**위치:** L1886~L1891 부근 (Windows `#[cfg]` 블록 내부)

이미 `#[cfg(target_os = "windows")]` 블록 안에 있다면, 대응하는 `#[cfg(not(target_os = "windows"))]` 블록의 바이너리 이름에서 `.exe`를 제거했는지 확인한다:

```rust
#[cfg(target_os = "windows")]
let process_names: Vec<&str> = match binary_name {
    n if n.contains("core") => vec!["saba-core.exe"],
    n if n.contains("cli")  => vec!["saba-chan-cli.exe"],
    n if n.contains("gui")  => vec!["saba-chan-gui.exe"],
    _ => vec![],
};

#[cfg(not(target_os = "windows"))]  // ← 추가
let process_names: Vec<&str> = match binary_name {
    n if n.contains("core") => vec!["saba-core"],
    n if n.contains("cli")  => vec!["saba-chan-cli"],
    n if n.contains("gui")  => vec!["saba-chan-gui"],
    _ => vec![],
};
```

### 5-4. 릴리즈 매니페스트 파싱 — Linux 에셋 선택

매니페스트 구조 변경에 맞춰, 에셋 이름을 가져오는 로직에 플랫폼 분기를 추가한다:

```rust
fn get_asset_name(component: &serde_json::Value) -> &str {
    if cfg!(target_os = "windows") {
        component["asset"].as_str().unwrap_or_default()      // 기존 로직
    } else {
        component["asset_linux"].as_str()
            .unwrap_or_else(|| component["asset"].as_str().unwrap_or_default())
    }
}
```

---

## 6. GUI main.js 크로스플랫폼 보완

### 대상 파일
- `saba-chan-gui/main.js`

### 현재 상태

이미 대부분의 플랫폼 분기가 `process.platform === 'win32'` / `else` 패턴으로 구현되어 있다:
- ✅ 데몬 스폰 (L530): `saba-core.exe` vs `saba-core`
- ✅ `spawnDetached` (L775): `cmd.exe /c start` vs `detached: true`
- ✅ 데몬 종료 (L855): `taskkill` vs `pkill`
- ✅ 봇 프로세스 종료 (L3050): `Win32_Process` vs `pkill`
- ✅ 레지스트리 조회 (L2819): `process.platform === 'win32'` 가드

### 해야 할 일 (최소 변경)

#### 6-1. `getInstallRoot()` — Linux AppImage 대응

**위치:** L197~L206

현재:
```javascript
function getInstallRoot() {
    if (!app.isPackaged) return path.join(__dirname, '..');
    if (process.env.PORTABLE_EXECUTABLE_DIR) {
        return process.env.PORTABLE_EXECUTABLE_DIR;
    }
    return path.dirname(app.getPath('exe'));
}
```

수정 — AppImage 환경변수 분기 추가:
```javascript
function getInstallRoot() {
    if (!app.isPackaged) return path.join(__dirname, '..');
    if (process.env.PORTABLE_EXECUTABLE_DIR) {           // Windows portable
        return process.env.PORTABLE_EXECUTABLE_DIR;
    }
    if (process.env.APPIMAGE) {                           // ← Linux AppImage 추가
        return path.dirname(process.env.APPIMAGE);
    }
    return path.dirname(app.getPath('exe'));
}
```

`APPIMAGE` 환경변수는 AppImage 실행 시 자동 설정된다. 기존 Windows 로직에 전혀 영향 없음.

#### 6-2. 트레이 아이콘 — Linux용 PNG 분기 (해당 시)

Windows에서 `.ico` 형식을 사용한다면, Linux에서는 `.png`가 필요하다. 트레이 아이콘 설정 코드에 분기 추가:

```javascript
const trayIcon = process.platform === 'win32'
    ? path.join(__dirname, 'build', 'icon.ico')
    : path.join(__dirname, 'build', 'icon.png');     // ← Linux 추가
```

---

## 7. Systemd 서비스 파일 신규 작성

### 대상
- 신규 파일: `config/saba-chan.service`
- 신규 파일: `scripts/install-service.sh`

### 현재 상태
본체에 systemd 서비스 파일이 전혀 없다. 헤드리스 Linux 서버에서 데몬을 자동 시작/관리할 방법이 없다.

### 해야 할 일

#### 7-1. systemd 유닛 파일 작성

```ini
# config/saba-chan.service
[Unit]
Description=Saba-chan Game Server Manager Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=saba-chan
Group=saba-chan
WorkingDirectory=/opt/saba-chan
ExecStart=/opt/saba-chan/saba-core
Restart=on-failure
RestartSec=10
Environment="SABA_DATA_DIR=/opt/saba-chan/data"

# 보안 강화
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/opt/saba-chan

[Install]
WantedBy=multi-user.target
```

#### 7-2. 서비스 설치 헬퍼 스크립트

```bash
#!/usr/bin/env bash
# scripts/install-service.sh
set -euo pipefail

SERVICE_FILE="config/saba-chan.service"
INSTALL_DIR="${1:-/opt/saba-chan}"

echo "Installing saba-chan systemd service..."

# 사용자 생성 (없을 경우)
if ! id "saba-chan" &>/dev/null; then
    sudo useradd -r -s /usr/sbin/nologin -d "$INSTALL_DIR" saba-chan
fi

# 서비스 파일 복사
sudo cp "$SERVICE_FILE" /etc/systemd/system/saba-chan.service
sudo sed -i "s|/opt/saba-chan|$INSTALL_DIR|g" /etc/systemd/system/saba-chan.service
sudo systemctl daemon-reload
sudo systemctl enable saba-chan

echo "Done. Start with: sudo systemctl start saba-chan"
```

> 이 작업은 Windows에 전혀 영향을 주지 않는다 — 단순 파일 추가이며 기존 코드를 수정하지 않는다.

---

## 8. 인스톨러 조건부 컴파일

### 대상 파일
- `installer/gui/src-tauri/src/lib.rs`
- `installer/gui/src-tauri/src/registry.rs`
- `installer/gui/src-tauri/src/shortcuts.rs`
- `installer/gui/src-tauri/src/uninstall.rs`

### 현재 상태
`registry.rs`, `shortcuts.rs`가 Windows API(`winreg`, `windows-sys`)에 직접 의존한다. Linux에서는 컴파일 자체가 불가능하다.

### 해야 할 일

#### 8-1. 모듈 선언부에 `cfg` 가드 추가

`lib.rs`에서 Windows 전용 모듈에 조건부 컴파일 추가:

```rust
pub mod github;
pub mod runtime_bootstrap;

#[cfg(target_os = "windows")]     // ← 추가
pub mod registry;

#[cfg(target_os = "windows")]     // ← 추가
pub mod shortcuts;

pub mod uninstall;
```

#### 8-2. `uninstall.rs` 내 레지스트리/바로가기 호출부 분기

`uninstall.rs`에서 `registry::*`와 `shortcuts::*`를 호출하는 곳에 `#[cfg]` 가드 추가:

```rust
#[cfg(target_os = "windows")]
{
    registry::remove_uninstall_entry()?;
    shortcuts::remove_desktop_shortcut()?;
    shortcuts::remove_start_menu_shortcut()?;
}

#[cfg(not(target_os = "windows"))]
{
    // Linux: .desktop 파일 제거
    let desktop_file = dirs::data_dir()
        .unwrap_or_default()
        .join("applications/saba-chan.desktop");
    let _ = std::fs::remove_file(desktop_file);
}
```

#### 8-3. Linux 인스톨러 대안 — 장기 과제

Linux 인스톨러는 즉시 구현할 필요는 없다. 초기에는 다음으로 대체:
- tar.gz 아카이브 + `install-service.sh` 스크립트 (섹션 7 참조)
- AppImage는 인스톨러 불필요 (자체 실행)
- 향후 `.deb`/`.rpm` 패키지 빌드를 고려

---

## 9. 게임 모듈 TOML — Linux 경로 추가

### 대상 파일
- `saba-chan-modules/minecraft/module.toml`
- `saba-chan-modules/palworld/module.toml`
- `saba-chan-modules/zomboid/module.toml`

### 현재 상태
`common_paths`, `process_name`, `server_executable` 등이 Windows 값만 포함한다.

### 해야 할 일

기존 Windows 값은 그대로 두고, `[detection.linux]` 또는 `linux_*` 필드를 추가한다.

> ⚠️ 이 변경은 모듈 TOML 파서(`src/plugin/mod.rs` 또는 Python lifecycle)가 플랫폼별 필드를 읽을 수 있도록 대응 코드 수정도 함께 필요하다.

#### 9-1. Minecraft

```toml
# 기존 (유지)
[detection]
process_name = "java.exe"
common_paths = [
    "C:\\Users\\*\\Desktop\\MinecraftServer",
    "C:\\MinecraftServer",
]

# 추가
[detection.linux]
process_name = "java"
common_paths = [
    "/home/*/minecraft-server",
    "/opt/minecraft-server",
    "/srv/minecraft",
]
```

#### 9-2. Palworld

```toml
# 기존 (유지)
[detection]
server_executable = "PalServer.exe"
process_name = "PalServer-Win64-Shipping-Cmd"
platform = "windows"
common_paths = [
    "C:\\Program Files (x86)\\Steam\\steamapps\\common\\PalServer",
]

# 추가
[detection.linux]
server_executable = "PalServer.sh"
process_name = "PalServer-Linux-Shipping"
common_paths = [
    "/home/*/.steam/steam/steamapps/common/PalServer",
    "/opt/palworld",
]
```

#### 9-3. Zomboid

```toml
# 기존 (유지)
[detection]
server_executable = "StartServer64.bat"
process_name = "java.exe"
common_paths = [
    "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Project Zomboid Dedicated Server",
]

# 추가
[detection.linux]
server_executable = "start-server.sh"
process_name = "java"
common_paths = [
    "/home/*/.steam/steam/steamapps/common/Project Zomboid Dedicated Server",
    "/opt/zomboid-server",
]
```

#### 9-4. 모듈 TOML 파서 수정

TOML 파서에서 현재 OS에 맞는 섹션을 선택하도록 분기 추가:

```rust
// src/plugin/mod.rs (개념적 예시)
let detection = if cfg!(target_os = "windows") {
    &module_config["detection"]                    // 기존 로직 그대로
} else {
    module_config.get("detection.linux")
        .unwrap_or(&module_config["detection"])    // Linux 섹션 없으면 기본값 폴백
};
```

---

## 10. Docker 확장 — ARM64 분기

### 대상 파일
- `saba-chan-extensions/docker/docker_engine.py`

### 현재 상태
Docker Engine과 Compose 다운로드 URL이 `x86_64`로 하드코딩:
```python
_DOCKER_ENGINE_URL = (
    "https://download.docker.com/linux/static/stable/x86_64/docker-27.5.1.tgz"
)
_COMPOSE_URL = (
    "https://github.com/docker/compose/releases/download/"
    "v2.33.1/docker-compose-linux-x86_64"
)
```

### 해야 할 일

기존 URL 상수를 유지하되, 아키텍처 감지 로직을 추가:

```python
import platform

def _get_docker_engine_url():
    arch = platform.machine()
    if arch == "aarch64":
        return "https://download.docker.com/linux/static/stable/aarch64/docker-27.5.1.tgz"
    return _DOCKER_ENGINE_URL  # 기존 x86_64 URL 그대로 사용

def _get_compose_url():
    arch = platform.machine()
    if arch == "aarch64":
        return (
            "https://github.com/docker/compose/releases/download/"
            "v2.33.1/docker-compose-linux-aarch64"
        )
    return _COMPOSE_URL  # 기존 x86_64 URL 그대로 사용
```

기존 `_DOCKER_ENGINE_URL`, `_COMPOSE_URL` 상수는 삭제하지 않고, 함수 내에서 폴백으로 참조한다.

---

## 우선순위 로드맵

| 순서 | 항목 | 난이도 | 영향도 | 비고 |
|------|------|--------|--------|------|
| **P0** | 빌드 스크립트 (`build-linux.sh`) | 낮음 | 높음 | 파일 추가만 |
| **P0** | CI/CD 매트릭스 확장 | 낮음 | 높음 | Linux 컴파일 검증 |
| **P1** | Electron 빌드 설정 (`linux` 타겟) | 낮음 | 높음 | `package.json` 수정 |
| **P1** | Updater `lib.rs` — `.exe` 분기 | 중간 | 높음 | 3~4곳 `cfg!()` 추가 |
| **P1** | 릴리즈 매니페스트 확장 | 중간 | 높음 | 파서 대응 필요 |
| **P2** | Systemd 서비스 파일 | 낮음 | 중간 | 파일 추가만 |
| **P2** | GUI `getInstallRoot()` AppImage 대응 | 낮음 | 중간 | 2줄 추가 |
| **P2** | 인스톨러 `#[cfg]` 가드 | 중간 | 중간 | 컴파일은 통과시킬 수 있음 |
| **P3** | 게임 모듈 TOML Linux 경로 | 낮음 | 낮음 | 파서 수정도 필요 |
| **P3** | Docker ARM64 URL 분기 | 낮음 | 낮음 | x86_64 환경에선 불필요 |

---

## 검증 체크리스트

- [ ] `cargo build --release` — Ubuntu 22.04/24.04에서 컴파일 성공
- [ ] `cargo test --all` — Linux에서 전체 테스트 통과
- [ ] `npx electron-builder --linux` — AppImage/tar.gz 생성 확인
- [ ] AppImage 실행 → Electron GUI 정상 로드
- [ ] GUI에서 데몬 자동 스폰 → REST API 통신 정상
- [ ] CLI (`saba-chan-cli`) 실행 → TUI 렌더링 정상
- [ ] 업데이터가 Linux 에셋을 올바르게 다운로드/적용
- [ ] systemd 서비스로 데몬 start/stop/restart 동작
- [ ] Python/Node 포터블 환경 자동 부트스트랩 정상
- [ ] 게임 모듈(Minecraft 등) 서버 감지 및 시작/종료 정상
