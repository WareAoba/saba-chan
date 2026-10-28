//! # Boot Selector — 부트 선택 화면
//!
//! 데몬을 직접 실행할 때 부트 선택기를 표시합니다.
//! - GUI / CLI / Daemon Only 모드 중 선택
//! - 마지막 선택을 기억하고, 5초 카운트다운 후 자동 실행
//! - 카운트다운 중 DEL 키로 설정 변경
//!
//! GUI/CLI가 데몬을 스폰할 때는 `--spawned` 플래그를 전달하여 이 화면을 건너뜁니다.

use saba_chan_updater_lib::config_file::ConfigFile;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

// ══════════════════════════════════════════════════════
//  Types
// ══════════════════════════════════════════════════════

/// 부트 모드 선택지
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootMode {
    Gui,
    Cli,
    #[serde(alias = "daemon_only")]
    DaemonOnly,
}

impl std::fmt::Display for BootMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootMode::Gui => write!(f, "GUI"),
            BootMode::Cli => write!(f, "CLI"),
            BootMode::DaemonOnly => write!(f, "Daemon Only"),
        }
    }
}

/// 부트 설정 (마지막 선택 기억용)
#[derive(Debug, Serialize, Deserialize)]
pub struct BootConfig {
    pub last_mode: BootMode,
}

impl Default for BootConfig {
    fn default() -> Self {
        Self {
            last_mode: BootMode::Gui,
        }
    }
}

// ══════════════════════════════════════════════════════
//  Config Persistence
// ══════════════════════════════════════════════════════

/// 디스크에서 부트 설정 로드 (`config.json`의 `boot`, 실패 시 기본값)
pub fn load_config() -> BootConfig {
    serde_json::from_value(ConfigFile::default_location().section("boot")).unwrap_or_default()
}

/// 부트 설정을 디스크에 저장
pub fn save_config(mode: BootMode) {
    let config = BootConfig { last_mode: mode };
    if let Ok(value) = serde_json::to_value(&config) {
        if let Err(e) = ConfigFile::default_location().set_section("boot", value) {
            tracing::warn!("Failed to save boot config: {}", e);
        }
    }
}

// ══════════════════════════════════════════════════════
//  Key Input Types
// ══════════════════════════════════════════════════════

// 비 Windows에서는 키 입력을 읽지 않으므로 Delete 외 변형이 만들어지지 않는다.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, PartialEq)]
enum Key {
    Delete,
    Enter,
    Up,
    Down,
    Char(char),
}

// ══════════════════════════════════════════════════════
//  Windows Console FFI
// ══════════════════════════════════════════════════════

#[cfg(windows)]
mod win32 {
    #![allow(non_snake_case, clippy::upper_case_acronyms)]

    type HANDLE = *mut std::ffi::c_void;
    type HWND = *mut std::ffi::c_void;
    type DWORD = u32;
    type WORD = u16;
    type BOOL = i32;

    pub const STD_INPUT_HANDLE: DWORD = (-10i32) as DWORD;
    pub const STD_OUTPUT_HANDLE: DWORD = (-11i32) as DWORD;
    pub const SW_HIDE: i32 = 0;
    pub const KEY_EVENT: WORD = 0x0001;
    pub const ENABLE_VIRTUAL_TERMINAL_PROCESSING: DWORD = 0x0004;

    // VK_* 키코드
    pub const VK_RETURN: WORD = 0x0D;
    pub const VK_UP: WORD = 0x26;
    pub const VK_DOWN: WORD = 0x28;
    pub const VK_DELETE: WORD = 0x2E;

    #[repr(C)]
    pub struct KeyEventRecord {
        pub key_down: BOOL,
        pub repeat_count: WORD,
        pub virtual_key_code: WORD,
        pub virtual_scan_code: WORD,
        pub unicode_char: WORD,
        pub control_key_state: DWORD,
    }

    /// Win32 INPUT_RECORD — EventType + union(16 bytes)
    #[repr(C)]
    pub struct InputRecord {
        pub event_type: WORD,
        _alignment: WORD,
        pub event: [u8; 16],
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetStdHandle(nStdHandle: DWORD) -> HANDLE;
        pub fn PeekConsoleInputW(
            h: HANDLE,
            buf: *mut InputRecord,
            len: DWORD,
            read: *mut DWORD,
        ) -> BOOL;
        pub fn ReadConsoleInputW(
            h: HANDLE,
            buf: *mut InputRecord,
            len: DWORD,
            read: *mut DWORD,
        ) -> BOOL;
        pub fn FlushConsoleInputBuffer(h: HANDLE) -> BOOL;
        pub fn GetConsoleMode(h: HANDLE, mode: *mut DWORD) -> BOOL;
        pub fn SetConsoleMode(h: HANDLE, mode: DWORD) -> BOOL;
        pub fn GetConsoleWindow() -> HWND;
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    }

    /// ANSI 이스케이프 시퀀스를 위한 Virtual Terminal Processing 활성화
    pub fn enable_vtp() {
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode: DWORD = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }

    /// 콘솔 입력 버퍼 플러시 (부트 셀렉터 시작 전 잔여 키 입력 제거)
    pub fn flush_input() {
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let _ = FlushConsoleInputBuffer(handle);
        }
    }
}

// ══════════════════════════════════════════════════════
//  Key Polling (Platform-specific)
// ══════════════════════════════════════════════════════

/// 주어진 타임아웃 내에 키 입력을 폴링. None = 타임아웃
fn poll_key(timeout: Duration) -> Option<Key> {
    #[cfg(windows)]
    return poll_key_windows(timeout);

    #[cfg(not(windows))]
    return poll_key_fallback(timeout);
}

#[cfg(windows)]
fn poll_key_windows(timeout: Duration) -> Option<Key> {
    use std::mem;

    let handle = unsafe { win32::GetStdHandle(win32::STD_INPUT_HANDLE) };
    let deadline = std::time::Instant::now() + timeout;

    while std::time::Instant::now() < deadline {
        let mut record: win32::InputRecord = unsafe { mem::zeroed() };
        let mut count: u32 = 0;

        // 이벤트가 있는지 확인 (블로킹하지 않음)
        let peek_ok =
            unsafe { win32::PeekConsoleInputW(handle, &mut record, 1, &mut count) } != 0;

        if peek_ok && count > 0 {
            // 이벤트 소비
            unsafe {
                win32::ReadConsoleInputW(handle, &mut record, 1, &mut count);
            }

            if record.event_type == win32::KEY_EVENT {
                let key_event =
                    unsafe { &*(record.event.as_ptr() as *const win32::KeyEventRecord) };

                // key-down 이벤트만 처리
                if key_event.key_down != 0 {
                    match key_event.virtual_key_code {
                        win32::VK_DELETE => return Some(Key::Delete),
                        win32::VK_RETURN => return Some(Key::Enter),
                        win32::VK_UP => return Some(Key::Up),
                        win32::VK_DOWN => return Some(Key::Down),
                        _ => {
                            if key_event.unicode_char > 0 {
                                if let Some(c) = char::from_u32(key_event.unicode_char as u32) {
                                    if !c.is_control() {
                                        return Some(Key::Char(c));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    None
}

/// Unix/비-Windows 폴백: 타임아웃 후 None 반환 (인터랙티브 키 입력 미지원)
#[cfg(not(windows))]
fn poll_key_fallback(timeout: Duration) -> Option<Key> {
    std::thread::sleep(timeout);
    None
}

// ══════════════════════════════════════════════════════
//  Terminal Rendering
// ══════════════════════════════════════════════════════

/// 스피너 프레임 — npm install 스타일
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ANSI dim(회색) 색상
const DIM: &str = "\x1b[2m";
/// ANSI 밝은 흰색(볼드)
const BOLD: &str = "\x1b[1m";
/// ANSI 초록색
const GREEN: &str = "\x1b[32m";
/// ANSI 리셋
const RESET: &str = "\x1b[0m";
/// 커서 숨기기
const HIDE_CURSOR: &str = "\x1b[?25l";
/// 커서 보이기
const SHOW_CURSOR: &str = "\x1b[?25h";

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = std::io::stdout().flush();
}

fn setup_terminal() {
    #[cfg(windows)]
    win32::enable_vtp();

    #[cfg(windows)]
    win32::flush_input();

    // 터미널 제목 설정
    let ver = env!("CARGO_PKG_VERSION");
    print!("\x1b]0;saba-chan Core Daemon v{}\x07", ver);
    print!("{}", HIDE_CURSOR);
    let _ = std::io::stdout().flush();

    clear_screen();
}

fn restore_terminal() {
    print!("{}", SHOW_CURSOR);
    let _ = std::io::stdout().flush();
}

fn render_boot_screen(default_mode: BootMode) {
    clear_screen();
    let ver = env!("CARGO_PKG_VERSION");
    println!();
    println!("  {}saba-chan{} v{}", BOLD, RESET, ver);
    println!("  {}Core Daemon{}", DIM, RESET);
    println!();
    println!("  Boot target: {}{}{}", GREEN, default_mode, RESET);
    println!();
    // Line 7: countdown + spinner placeholder (row 7 from top, 1-indexed)
    println!();
    println!();
    println!("  {}Press [DEL] to change boot target{}", DIM, RESET);
    let _ = std::io::stdout().flush();
}

fn render_countdown(default_mode: BootMode, seconds: u64, frame: usize) {
    let spinner = SPINNER[frame % SPINNER.len()];
    // 커서를 7번째 줄(1-indexed)로 이동
    let msg = format!(
        "  {} Starting {} in {}s...",
        spinner, default_mode, seconds
    );
    print!("\x1b[7;1H\x1b[2K{}", msg);
    let _ = std::io::stdout().flush();
}

fn render_menu(selected: usize) {
    clear_screen();
    let modes = ["GUI", "CLI", "Daemon Only"];
    let descs = [
        "Launch graphical interface",
        "Launch terminal interface",
        "Run headless (log to console)",
    ];

    println!();
    println!("  {}Boot Configuration{}", BOLD, RESET);
    println!();

    for (i, (mode, desc)) in modes.iter().zip(descs.iter()).enumerate() {
        if i == selected {
            println!("  {} > {}{:<14}{}{}", GREEN, BOLD, mode, RESET, desc);
        } else {
            println!("    {:<14}{}{}{}", mode, DIM, desc, RESET);
        }
    }

    println!();
    println!(
        "  {}↑/↓ Navigate  1-3 Quick select  Enter Confirm{}",
        DIM, RESET
    );
    let _ = std::io::stdout().flush();
}

// ══════════════════════════════════════════════════════
//  Selection Menu (interactive)
// ══════════════════════════════════════════════════════

fn show_selection_menu(current: BootMode) -> BootMode {
    let modes = [BootMode::Gui, BootMode::Cli, BootMode::DaemonOnly];

    let mut selected = modes
        .iter()
        .position(|m| *m == current)
        .unwrap_or(0);

    // 초기 1회 렌더링
    render_menu(selected);

    loop {
        match poll_key(Duration::from_millis(100)) {
            Some(Key::Up) => {
                if selected > 0 {
                    selected -= 1;
                    render_menu(selected);
                }
            }
            Some(Key::Down) => {
                if selected < modes.len() - 1 {
                    selected += 1;
                    render_menu(selected);
                }
            }
            Some(Key::Char('1')) => return modes[0],
            Some(Key::Char('2')) => return modes[1],
            Some(Key::Char('3')) => return modes[2],
            Some(Key::Enter) => return modes[selected],
            _ => {}
        }
    }
}

// ══════════════════════════════════════════════════════
//  Main Entry Point
// ══════════════════════════════════════════════════════

/// 인터랙티브 부트 선택기 실행. 선택된 BootMode를 반환합니다.
pub fn run() -> BootMode {
    setup_terminal();

    let config = load_config();
    let default_mode = config.last_mode;

    render_boot_screen(default_mode);

    // 5초 카운트다운 — 80ms 간격으로 스피너 프레임 갱신, DEL 키로 인터럽트
    let mut frame: usize = 0;
    for remaining in (1..=5).rev() {
        // 1초를 ~80ms 간격의 작은 단위로 분할 → 스피너 + 키 폴링
        let ticks = 12; // 12 × ~83ms ≈ 1초
        for _ in 0..ticks {
            render_countdown(default_mode, remaining, frame);
            frame += 1;
            if poll_key(Duration::from_millis(83)) == Some(Key::Delete) {
                restore_terminal();
                let selected = show_selection_menu(default_mode);
                save_config(selected);
                clear_screen();
                return selected;
            }
        }
    }

    // 카운트다운 만료 → 기본값으로 자동 실행
    restore_terminal();
    save_config(default_mode);
    clear_screen();
    default_mode
}

// ══════════════════════════════════════════════════════
//  Post-Boot Actions
// ══════════════════════════════════════════════════════

/// 데몬 온리(또는 폴백) 진입 시 화면 정리 + 안내 출력
pub fn clear_for_daemon_only(port: u16) {
    clear_screen();
    let ver = env!("CARGO_PKG_VERSION");
    eprintln!(
        "  {}saba-chan{} v{} — daemon-only on port {}",
        BOLD, RESET, ver, port
    );
    eprintln!("  {}Press Ctrl+C to shut down.{}", DIM, RESET);
    eprintln!();
}

/// 인터페이스 바이너리 탐색 (데몬 exe와 같은 디렉토리)
pub fn find_interface_binary(name: &str) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;

    let binary = if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };

    let path = exe_dir.join(&binary);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// GUI 프로세스를 분리(detach)하여 스폰
pub fn spawn_gui() -> std::io::Result<()> {
    let gui_path = find_interface_binary("saba-chan-gui").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "saba-chan-gui binary not found next to daemon executable",
        )
    })?;
    spawn_gui_with(&gui_path, &["--spawned-by-daemon".to_string()], &[])?;
    Ok(())
}

/// GUI를 표준 입출력 없이 띄운다. pid를 돌려준다 (CLI에서 GUI로 전환할 때도 쓴다)
pub fn spawn_gui_with(gui: &std::path::Path, args: &[String], env: &[(&str, String)]) -> std::io::Result<u32> {
    let child = std::process::Command::new(gui)
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let pid = child.id();
    // 오래 사는 데몬이 띄우므로 끝난 GUI를 거둔다 (유닉스에서 좀비로 남지 않게)
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(pid)
}

/// CLI 프로세스를 현재 콘솔에 마운트 (stdio 상속)
pub fn spawn_cli() -> std::io::Result<std::process::Child> {
    let cli_path = find_interface_binary("saba-chan-cli").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "saba-chan-cli binary not found next to daemon executable",
        )
    })?;

    std::process::Command::new(&cli_path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
}

/// Windows: CLI를 새 콘솔 창에 띄운다 ("터미널로 열기", I4). CLI가 새 창의 콘솔을 자기 입출력으로 쓰고 대화형이 된다. pid를 돌려준다
#[cfg(windows)]
pub fn spawn_cli_in_new_console(cli: &std::path::Path, args: &[String]) -> std::io::Result<u32> {
    crate::utils::spawn_in_new_console(cli, args, cli.parent(), &[]).map(|(pid, _)| pid)
}

/// 리눅스 터미널 프로그램과, 그 뒤에 실행할 프로그램을 붙이는 인자. 앞의 것부터 찾는다
#[cfg(any(target_os = "linux", test))]
const TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("ptyxis", &["--"]),
    ("kgx", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("xterm", &["-e"]),
];

/// 리눅스: CLI를 띄울 터미널. `$TERMINAL`(사용자가 고른 것)이 있으면 그것을, 없으면 `TERMINALS`에서 찾는다.
/// 모르는 `$TERMINAL`은 대부분의 터미널이 받는 `-e`로 붙인다. `find`는 PATH에서 프로그램을 찾는다
#[cfg(any(target_os = "linux", test))]
fn terminal_command(preferred: Option<&str>, find: impl Fn(&str) -> Option<PathBuf>) -> Option<(PathBuf, Vec<String>)> {
    let args_of = |name: &str| -> Vec<String> {
        let base = std::path::Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or(name);
        TERMINALS
            .iter()
            .find(|(t, _)| *t == base)
            .map_or_else(|| vec!["-e".to_string()], |(_, a)| a.iter().map(|s| s.to_string()).collect())
    };
    if let Some(term) = preferred.map(str::trim).filter(|t| !t.is_empty()) {
        if let Some(path) = find(term) {
            return Some((path, args_of(term)));
        }
    }
    TERMINALS.iter().find_map(|(name, _)| find(name).map(|p| (p, args_of(name))))
}

/// PATH에서 실행 파일을 찾는다 (경로가 주어지면 그 파일)
#[cfg(target_os = "linux")]
fn find_program(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |p: &std::path::Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if name.contains('/') {
        let p = PathBuf::from(name);
        return runnable(&p).then_some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| runnable(p))
}

/// 리눅스: CLI를 띄울 터미널이 있는가 (GUI의 "CLI로 전환")
#[cfg(target_os = "linux")]
pub fn has_terminal() -> bool {
    terminal_command(std::env::var("TERMINAL").ok().as_deref(), find_program).is_some()
}

/// 터미널로 띄운 CLI를 찾는 환경 변수. 터미널 프로그램(gnome-terminal 등)은 띄우자마자 끝나거나 서버 프로세스에
/// 창을 맡기므로, 띄운 pid가 CLI가 아니다: 이 표시를 단 CLI를 찾아 그 pid를 기록한다
#[cfg(any(target_os = "linux", test))]
pub const LAUNCH_TAG_ENV: &str = "SABA_LAUNCH_TAG";

/// 리눅스: CLI를 새 터미널 창에 띄운다 ("CLI로 전환"). 터미널 pid를 돌려주고, CLI에는 `tag`를 환경으로 단다
#[cfg(target_os = "linux")]
pub fn spawn_cli_in_new_console(cli: &std::path::Path, args: &[String], tag: &str) -> std::io::Result<u32> {
    use std::os::unix::process::CommandExt;
    let (term, prefix) = terminal_command(std::env::var("TERMINAL").ok().as_deref(), find_program)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal program found"))?;
    let mut cmd = std::process::Command::new(term);
    cmd.args(prefix)
        .arg(cli)
        .args(args)
        .env(LAUNCH_TAG_ENV, tag)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    if let Some(dir) = cli.parent() {
        cmd.current_dir(dir);
    }
    let child = cmd.spawn()?;
    let pid = child.id();
    // 오래 사는 데몬이 띄우므로 끝난 터미널을 거둔다
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(pid)
}

/// 리눅스: 환경에 `LAUNCH_TAG_ENV=tag`를 달고 실행 파일 이름이 `name`인 이 사용자의 프로세스
#[cfg(target_os = "linux")]
pub fn find_tagged_process(tag: &str, name: &str) -> Option<u32> {
    let wanted = format!("{LAUNCH_TAG_ENV}={tag}");
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let exe = std::fs::read_link(entry.path().join("exe")).ok()?;
        if exe.file_name()?.to_str()? != name {
            return None;
        }
        let environ = std::fs::read(entry.path().join("environ")).ok()?;
        environ.split(|&b| b == 0).any(|kv| kv == wanted.as_bytes()).then_some(pid)
    })
}

/// Windows: 데몬 콘솔 창 숨기기 (GUI 모드에서 사용)
#[cfg(windows)]
pub fn hide_console_window() {
    unsafe {
        let hwnd = win32::GetConsoleWindow();
        if !hwnd.is_null() {
            win32::ShowWindow(hwnd, win32::SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_console_window() {
    // Unix/macOS에서는 콘솔 창 숨기기 불필요
}

/// IPC 포트가 열릴 때까지 대기 (비동기)
pub async fn wait_for_ipc_port(port: u16, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

// ══════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// 리눅스 "CLI로 전환": 고른 터미널(`$TERMINAL`)이 먼저고, 없으면 알려진 터미널을 차례로 찾는다.
    /// 터미널마다 실행할 프로그램을 붙이는 인자가 다르다
    #[test]
    fn the_cli_opens_in_the_users_terminal() {
        let have = |names: &'static [&'static str]| move |n: &str| names.contains(&n).then(|| PathBuf::from(format!("/usr/bin/{n}")));
        assert_eq!(
            terminal_command(None, have(&["xterm", "gnome-terminal"])),
            Some((PathBuf::from("/usr/bin/gnome-terminal"), vec!["--".to_string()]))
        );
        assert_eq!(
            terminal_command(Some("konsole"), have(&["konsole", "gnome-terminal"])),
            Some((PathBuf::from("/usr/bin/konsole"), vec!["-e".to_string()]))
        );
        // 모르는 터미널은 -e로, 찾지 못하면 다음 후보로
        assert_eq!(terminal_command(Some("myterm"), have(&["myterm"])).unwrap().1, vec!["-e".to_string()]);
        assert_eq!(terminal_command(Some("missing"), have(&["kitty"])), Some((PathBuf::from("/usr/bin/kitty"), vec![])));
        assert_eq!(terminal_command(Some("  "), have(&[])), None);
    }

    /// 터미널이 띄운 CLI는 환경의 표시로 찾는다 (완전히 종료할 때 그 CLI를 닫으려고)
    #[cfg(target_os = "linux")]
    #[test]
    fn a_tagged_process_is_found_by_its_environment() {
        let mut child = std::process::Command::new("sleep").arg("30").env(LAUNCH_TAG_ENV, "t-123").spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let found = loop {
            if let Some(pid) = find_tagged_process("t-123", "sleep") {
                break Some(pid);
            }
            if std::time::Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(found, Some(child.id()));
        assert_eq!(find_tagged_process("t-other", "sleep"), None);
        assert_eq!(find_tagged_process("t-123", "saba-chan-cli"), None);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn boot_mode_display() {
        assert_eq!(format!("{}", BootMode::Gui), "GUI");
        assert_eq!(format!("{}", BootMode::Cli), "CLI");
        assert_eq!(format!("{}", BootMode::DaemonOnly), "Daemon Only");
    }

    #[test]
    fn boot_config_default_is_gui() {
        let config = BootConfig::default();
        assert_eq!(config.last_mode, BootMode::Gui);
    }

    #[test]
    fn boot_config_serde_roundtrip() {
        for mode in [BootMode::Gui, BootMode::Cli, BootMode::DaemonOnly] {
            let config = BootConfig { last_mode: mode };
            let json = serde_json::to_string(&config).unwrap();
            let parsed: BootConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.last_mode, mode);
        }
    }

    #[test]
    fn boot_config_json_format() {
        // JSON 형식이 boot-config.json 파일과 호환되는지 확인
        let json = r#"{"last_mode":"gui"}"#;
        let config: BootConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.last_mode, BootMode::Gui);

        let json = r#"{"last_mode":"cli"}"#;
        let config: BootConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.last_mode, BootMode::Cli);

        let json = r#"{"last_mode":"daemononly"}"#;
        let config: BootConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.last_mode, BootMode::DaemonOnly);

        // alias 지원 확인
        let json = r#"{"last_mode":"daemon_only"}"#;
        let config: BootConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.last_mode, BootMode::DaemonOnly);
    }

    #[test]
    fn boot_config_file_roundtrip() {
        let tmp = std::env::temp_dir().join("saba-test-boot-config.json");

        // Write
        let config = BootConfig {
            last_mode: BootMode::Cli,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        std::fs::write(&tmp, &json).unwrap();

        // Read
        let content = std::fs::read_to_string(&tmp).unwrap();
        let loaded: BootConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.last_mode, BootMode::Cli);

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn boot_config_missing_file_returns_default() {
        // 존재하지 않는 경로에서 로드하면 기본값을 반환해야 함
        let config: BootConfig = std::fs::read_to_string("/nonexistent/boot-config.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        assert_eq!(config.last_mode, BootMode::Gui);
    }

    #[test]
    fn boot_config_corrupted_json_returns_default() {
        // 잘못된 JSON에서 로드하면 기본값을 반환해야 함
        let config: BootConfig = serde_json::from_str("not valid json")
            .ok()
            .unwrap_or_default();
        assert_eq!(config.last_mode, BootMode::Gui);
    }
}
