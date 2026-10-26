//! 로그인할 때 자동 실행 (끔 / 데몬만 / GUI까지 전체)
//!
//! 설정 파일에 따로 적지 않는다: OS에 등록된 항목 자체가 지금 값이다 (작업 관리자에서 끄거나 지워도 맞게 보인다).
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`의 `Saba-chan` 값.
//!   작업 관리자·설정 앱에서 끈 표시(`...\Explorer\StartupApproved\Run`)가 있으면 꺼진 것으로 보고, 사바쨩에서 다시 고르면 그 표시를 지운다
//! - Linux: `$XDG_CONFIG_HOME/autostart/saba-chan.desktop` (없으면 `~/.config/autostart`).
//!   GUI 없이 설치한 화면 없는 서버(D3)의 데몬만은 systemd 사용자 서비스
//!   `$XDG_CONFIG_HOME/systemd/user/saba-chan.service`(`default.target.wants`에 걸려 있어야 켜진 것)
//!
//! 등록 항목은 하나다. 설정 파일을 여러 설치본이 함께 쓰므로 마지막으로 켠 설치본의 실행 파일을 가리킨다.
//! 데몬만: 창 없이 트레이만 드는 GUI(`--tray`)가 데몬을 띄운다 (Windows에서 콘솔 창이 뜨지 않는다).
//! 전체: GUI를 그대로 띄운다 (창이 열리고 데몬을 띄운다). 어느 쪽이든 GUI가 있어야 한다:
//! 로그인 자동 실행은 데스크톱 세션만 돌린다. 화면 없는 리눅스 서버는 데몬을 서비스로 바로 돌리고,
//! 로그인하지 않아도 부팅할 때 켜려면 사용자가 `loginctl enable-linger`를 한 번 한다 (`lingering`)

use std::path::{Path, PathBuf};

/// 등록 항목 이름 (Windows 값 이름, Linux 파일 이름의 바탕)
pub const ENTRY_NAME: &str = "Saba-chan";
/// 창 없이 트레이만 드는 상주 GUI 인자 (데몬의 트레이 감독도 이것으로 띄운다)
pub const TRAY_ARG: &str = "--tray";
#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 작업 관리자·설정 앱의 켬/끔 표시 (값 이름은 Run과 같다)
#[cfg(target_os = "windows")]
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
#[cfg(not(target_os = "windows"))]
const DESKTOP_FILE: &str = "saba-chan.desktop";
#[cfg(not(target_os = "windows"))]
const SERVICE_FILE: &str = "saba-chan.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Daemon,
    Full,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Mode::Off),
            "daemon" => Some(Mode::Daemon),
            "full" => Some(Mode::Full),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Daemon => "daemon",
            Mode::Full => "full",
        }
    }
}

/// 로그인할 때 실행할 것
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Entry {
    /// 데몬을 바로 돌리는 서비스 항목인가 (화면 없는 리눅스)
    pub fn is_service(&self) -> bool {
        self.program.file_name().is_some_and(|n| n == crate::uninstall_entry::CORE_EXE)
    }

    /// 이 항목이 켜는 모드: 트레이 인자나 데몬 서비스면 데몬만, 아니면 GUI까지 전체
    pub fn mode(&self) -> Mode {
        if self.is_service() || self.args.iter().any(|a| a == TRAY_ARG) {
            Mode::Daemon
        } else {
            Mode::Full
        }
    }
}

/// 등록할 수 없는 이유 (`code`는 API `error_code`의 `autostart_<code>`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutostartError {
    pub code: &'static str,
    pub detail: String,
}

impl AutostartError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }
}

impl std::fmt::Display for AutostartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

/// 이 OS에서 자동 실행을 등록할 수 있는지 (macOS는 아직 아니다)
pub fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "linux"))
}

/// `install_dir`의 실행 파일로 `mode`를 켜는 항목. 끔은 None
pub fn entry_for(mode: Mode, install_dir: &Path) -> Result<Option<Entry>, AutostartError> {
    let gui = crate::find_gui_exe(install_dir);
    // 데스크톱에서는 데몬을 직접 등록하지 않는다: Windows의 데몬은 콘솔 프로그램이라 로그인할 때 콘솔 창이 뜬다
    let core = install_dir.join(crate::uninstall_entry::CORE_EXE);
    let entry = match (mode, gui) {
        (Mode::Off, _) => return Ok(None),
        (Mode::Daemon, None) if cfg!(target_os = "linux") && core.is_file() => Entry { program: core, args: Vec::new() },
        (_, None) => return Err(AutostartError::new("gui_missing", install_dir.to_string_lossy())),
        (Mode::Full, Some(gui)) => Entry { program: gui, args: Vec::new() },
        (Mode::Daemon, Some(gui)) => Entry { program: gui, args: vec![TRAY_ARG.to_string()] },
    };
    Ok(Some(entry))
}

// ── Windows Run 값 ──

/// Run 값 문자열: 프로그램은 늘 따옴표로 감싼다 (인자는 공백 없는 고정 값뿐이다)
pub fn command_line(entry: &Entry) -> String {
    let mut line = format!("\"{}\"", entry.program.to_string_lossy());
    for a in &entry.args {
        line.push(' ');
        line.push_str(a);
    }
    line
}

/// Run 값 문자열을 되읽는다 (따옴표 없이 적힌 경로는 첫 공백까지를 프로그램으로 본다)
pub fn parse_command_line(line: &str) -> Option<Entry> {
    let line = line.trim();
    let (program, rest) = if let Some(quoted) = line.strip_prefix('"') {
        let end = quoted.find('"')?;
        (&quoted[..end], &quoted[end + 1..])
    } else {
        line.split_once(char::is_whitespace).unwrap_or((line, ""))
    };
    if program.is_empty() {
        return None;
    }
    Some(Entry {
        program: PathBuf::from(program),
        args: rest.split_whitespace().map(str::to_string).collect(),
    })
}

// ── Linux .desktop ──

/// Desktop Entry `Exec`의 따옴표 안 글자 (`"` `` ` `` `$` `\`는 역슬래시, `%`는 `%%`)
fn desktop_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 문자열 값의 이스케이프 (`Exec` 따옴표 규칙보다 먼저 풀린다: 역슬래시는 두 번 적는다)
fn desktop_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
}

fn desktop_unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

pub fn desktop_file(entry: &Entry) -> String {
    let mut exec = desktop_quote(&entry.program.to_string_lossy());
    for a in &entry.args {
        exec.push(' ');
        exec.push_str(a);
    }
    let exec = desktop_escape(&exec);
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        ENTRY_NAME, exec
    )
}

/// `Exec` 줄을 되읽는다. 데스크톱이 끈 항목(`Hidden=true`, `X-GNOME-Autostart-enabled=false`)은 없는 것으로 본다
pub fn parse_desktop_file(text: &str) -> Option<Entry> {
    let key = |name: &str| text.lines().find_map(|l| l.trim().strip_prefix(name)?.trim_start().strip_prefix('=').map(str::trim));
    if key("Hidden") == Some("true") || key("X-GNOME-Autostart-enabled") == Some("false") {
        return None;
    }
    let exec = desktop_unescape(text.lines().find_map(|l| l.trim().strip_prefix("Exec="))?.trim());
    let exec = exec.as_str();
    let (program, rest) = if let Some(quoted) = exec.strip_prefix('"') {
        let mut program = String::new();
        let mut chars = quoted.char_indices();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => program.push(chars.next()?.1),
                '%' => {
                    program.push('%');
                    // `%%` → `%`
                    if quoted[i + 1..].starts_with('%') {
                        chars.next();
                    }
                }
                '"' => {
                    end = Some(i + 1);
                    break;
                }
                _ => program.push(c),
            }
        }
        (program, &quoted[end?..])
    } else {
        let (p, r) = exec.split_once(char::is_whitespace).unwrap_or((exec, ""));
        (p.to_string(), r)
    };
    if program.is_empty() {
        return None;
    }
    Some(Entry {
        program: PathBuf::from(program),
        args: rest.split_whitespace().map(str::to_string).collect(),
    })
}

// ── Linux systemd 사용자 서비스 ──

/// `ExecStart` 따옴표 안 글자 (`"` `\`는 역슬래시, `%`는 `%%`, `$`는 `$$`)
fn unit_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 데몬 서비스 단위. 멈춤 API로 끝나면(종료 코드 0) 다시 띄우지 않고, 죽었을 때만 다시 띄운다
pub fn systemd_unit(entry: &Entry) -> String {
    let mut exec = unit_quote(&entry.program.to_string_lossy());
    for a in &entry.args {
        exec.push(' ');
        exec.push_str(a);
    }
    format!(
        "[Unit]\nDescription={} daemon\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        ENTRY_NAME, exec
    )
}

/// `ExecStart` 줄을 되읽는다
pub fn parse_systemd_unit(text: &str) -> Option<Entry> {
    let exec = text.lines().find_map(|l| l.trim().strip_prefix("ExecStart="))?.trim();
    let (program, rest) = if let Some(quoted) = exec.strip_prefix('"') {
        let mut program = String::new();
        let mut chars = quoted.char_indices().peekable();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => program.push(chars.next()?.1),
                '%' | '$' if chars.peek().is_some_and(|(_, n)| *n == c) => {
                    chars.next();
                    program.push(c);
                }
                '"' => {
                    end = Some(i + 1);
                    break;
                }
                _ => program.push(c),
            }
        }
        (program, &quoted[end?..])
    } else {
        let (p, r) = exec.split_once(char::is_whitespace).unwrap_or((exec, ""));
        (p.to_string(), r)
    };
    if program.is_empty() {
        return None;
    }
    Some(Entry {
        program: PathBuf::from(program),
        args: rest.split_whitespace().map(str::to_string).collect(),
    })
}

/// 사용자 설정 폴더 (`$XDG_CONFIG_HOME`, 없으면 `~/.config`)
#[cfg(not(target_os = "windows"))]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".config")))
}

/// systemd 사용자 단위 폴더
#[cfg(not(target_os = "windows"))]
pub fn unit_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("systemd").join("user"))
}

/// 켜진(`default.target.wants`에 걸린) 서비스 항목
#[cfg(not(target_os = "windows"))]
fn read_service_in(units: &Path) -> Option<Entry> {
    std::fs::symlink_metadata(units.join("default.target.wants").join(SERVICE_FILE)).ok()?;
    parse_systemd_unit(&std::fs::read_to_string(units.join(SERVICE_FILE)).ok()?)
}

/// 단위 파일을 쓰고 켠다 (`systemctl --user enable`이 하는 것과 같은 링크. 다음 로그인·부팅부터 돈다)
#[cfg(not(target_os = "windows"))]
fn write_service_in(units: &Path, entry: &Entry) -> std::io::Result<()> {
    let wants = units.join("default.target.wants");
    std::fs::create_dir_all(&wants)?;
    crate::write_atomic(&units.join(SERVICE_FILE), systemd_unit(entry).as_bytes())?;
    let link = wants.join(SERVICE_FILE);
    match std::fs::remove_file(&link) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(units.join(SERVICE_FILE), &link)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn remove_service_in(units: &Path) -> std::io::Result<()> {
    for f in [units.join("default.target.wants").join(SERVICE_FILE), units.join(SERVICE_FILE)] {
        match std::fs::remove_file(f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// 등록 항목 하나: 데스크톱 항목, 없으면 켜진 서비스
#[cfg(not(target_os = "windows"))]
pub fn read_from(autostart: &Path, units: &Path) -> Option<Entry> {
    read_in(autostart).or_else(|| read_service_in(units))
}

/// 하나만 남긴다: 서비스를 쓰면 데스크톱 항목을, 데스크톱 항목을 쓰면 서비스를 지운다
#[cfg(not(target_os = "windows"))]
pub fn write_to(autostart: &Path, units: &Path, entry: &Entry) -> std::io::Result<()> {
    if entry.is_service() {
        write_service_in(units, entry)?;
        remove_in(autostart)
    } else {
        write_in(autostart, entry)?;
        remove_service_in(units)
    }
}

#[cfg(not(target_os = "windows"))]
pub fn remove_from(autostart: &Path, units: &Path) -> std::io::Result<()> {
    remove_in(autostart)?;
    remove_service_in(units)
}

/// 로그인하지 않아도 이 사용자의 서비스가 부팅할 때 도는가 (`loginctl enable-linger`). 리눅스가 아니면 None
pub fn lingering() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let user = std::env::var("USER")
            .ok()
            .or_else(|| std::env::var("LOGNAME").ok())
            .filter(|u| !u.is_empty())
            .or_else(this_user_name)?;
        Some(Path::new("/var/lib/systemd/linger").join(user).exists())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// `USER`가 없는 서비스·컨테이너 환경: 이 프로세스의 uid를 `/etc/passwd`에서 찾는다
#[cfg(target_os = "linux")]
fn this_user_name() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").ok()?.uid().to_string();
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        (f.nth(1)? == uid).then(|| name.to_string())
    })
}

/// 리눅스 자동 실행 폴더
#[cfg(not(target_os = "windows"))]
pub fn autostart_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("autostart"))
}

/// `dir`의 등록 항목
#[cfg(not(target_os = "windows"))]
pub fn read_in(dir: &Path) -> Option<Entry> {
    parse_desktop_file(&std::fs::read_to_string(dir.join(DESKTOP_FILE)).ok()?)
}

#[cfg(not(target_os = "windows"))]
pub fn write_in(dir: &Path, entry: &Entry) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    crate::write_atomic(&dir.join(DESKTOP_FILE), desktop_file(entry).as_bytes())
}

#[cfg(not(target_os = "windows"))]
pub fn remove_in(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(DESKTOP_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

// ── 이 사용자의 등록 (OS별) ──

/// 지금 등록된 항목
pub fn read() -> Option<Entry> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        let key = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).ok()?;
        let line: String = key.get_value(ENTRY_NAME).ok()?;
        if disabled_in_task_manager() {
            return None;
        }
        parse_command_line(&line)
    }
    #[cfg(not(target_os = "windows"))]
    {
        read_from(&autostart_dir()?, &unit_dir()?)
    }
}

/// 작업 관리자·설정 앱에서 껐는가 (표시 첫 바이트가 홀수면 꺼짐: 02·06 켬, 03·07 끔)
#[cfg(target_os = "windows")]
fn disabled_in_task_manager() -> bool {
    use winreg::enums::*;
    let Ok(key) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(APPROVED_KEY) else { return false };
    key.get_raw_value(ENTRY_NAME).is_ok_and(|v| v.bytes.first().is_some_and(|b| b & 1 == 1))
}

/// 지금 켜진 모드. 등록된 프로그램이 없으면(그 설치를 지웠다) 꺼진 것이다
pub fn current_mode() -> Mode {
    read().filter(|e| e.program.is_file()).map_or(Mode::Off, |e| e.mode())
}

/// `install_dir`의 실행 파일로 `mode`를 등록한다 (끔이면 지운다)
pub fn apply(mode: Mode, install_dir: &Path) -> Result<(), AutostartError> {
    if !supported() {
        return Err(AutostartError::new("unsupported", std::env::consts::OS));
    }
    let io = |e: std::io::Error| AutostartError::new("write_failed", e.to_string());
    match entry_for(mode, install_dir)? {
        None => remove().map_err(io),
        Some(entry) => write(&entry).map_err(io),
    }
}

fn write(entry: &Entry) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        let (key, _) = winreg::RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
        key.set_value(ENTRY_NAME, &command_line(entry))?;
        // 작업 관리자에서 껐던 표시가 남으면 새로 고른 값이 돌지 않는다
        clear_task_manager_flag()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let no_home = || std::io::Error::new(std::io::ErrorKind::NotFound, "no home folder");
        write_to(&autostart_dir().ok_or_else(no_home)?, &unit_dir().ok_or_else(no_home)?, entry)
    }
}

fn remove() -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        let key = match winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        match key.delete_value(ENTRY_NAME) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        clear_task_manager_flag()
    }
    #[cfg(not(target_os = "windows"))]
    {
        match (autostart_dir(), unit_dir()) {
            (Some(dir), Some(units)) => remove_from(&dir, &units),
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "windows")]
fn clear_task_manager_flag() -> std::io::Result<()> {
    use winreg::enums::*;
    let key = match winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(APPROVED_KEY, KEY_SET_VALUE) {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    match key.delete_value(ENTRY_NAME) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// 제거기: 등록 항목이 이 설치 폴더의 실행 파일을 가리키면 지운다 (다른 설치본의 것은 둔다). 지웠으면 true
pub fn remove_if_in(install_dir: &Path) -> std::io::Result<bool> {
    match read() {
        Some(entry) if points_into(&entry, install_dir) => remove().map(|_| true),
        _ => Ok(false),
    }
}

/// 항목의 프로그램이 이 폴더 바로 안에 있는가
pub fn points_into(entry: &Entry, install_dir: &Path) -> bool {
    entry.program.parent().is_some_and(|p| crate::same_path(p, install_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            std::fs::write(dir.path().join(f), b"").unwrap();
        }
        dir
    }

    #[test]
    fn modes_parse_from_what_the_api_and_cli_send() {
        for m in [Mode::Off, Mode::Daemon, Mode::Full] {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert_eq!(Mode::parse(" FULL "), Some(Mode::Full));
        assert_eq!(Mode::parse("gui"), None);
    }

    #[test]
    fn daemon_only_starts_the_tray_gui_and_full_starts_the_gui() {
        let dir = install_with(&[crate::GUI_EXE_NAMES[0], crate::uninstall_entry::CORE_EXE]);
        let gui = dir.path().join(crate::GUI_EXE_NAMES[0]);
        assert_eq!(entry_for(Mode::Off, dir.path()).unwrap(), None);
        let daemon = entry_for(Mode::Daemon, dir.path()).unwrap().unwrap();
        assert_eq!(daemon, Entry { program: gui.clone(), args: vec![TRAY_ARG.into()] });
        assert_eq!(daemon.mode(), Mode::Daemon);
        let full = entry_for(Mode::Full, dir.path()).unwrap().unwrap();
        assert_eq!(full, Entry { program: gui, args: vec![] });
        assert_eq!(full.mode(), Mode::Full);
    }

    #[test]
    fn without_a_gui_nothing_can_be_turned_on_but_off_still_works() {
        let dir = install_with(&[crate::uninstall_entry::CORE_EXE]);
        assert_eq!(entry_for(Mode::Full, dir.path()).unwrap_err().code, "gui_missing");
        if cfg!(target_os = "linux") {
            // 화면 없는 리눅스 서버 (D3): 데몬만은 데몬을 바로 systemd 사용자 서비스로 등록한다
            let daemon = entry_for(Mode::Daemon, dir.path()).unwrap().unwrap();
            assert_eq!(daemon, Entry { program: dir.path().join(crate::uninstall_entry::CORE_EXE), args: vec![] });
            assert_eq!(daemon.mode(), Mode::Daemon);
            assert!(daemon.is_service());
        } else {
            assert_eq!(entry_for(Mode::Daemon, dir.path()).unwrap_err().code, "gui_missing");
        }
        assert_eq!(entry_for(Mode::Off, dir.path()).unwrap(), None);
        let empty = install_with(&[]);
        assert_eq!(entry_for(Mode::Daemon, empty.path()).unwrap_err().code, "gui_missing");
    }

    #[test]
    fn the_systemd_unit_reads_back_and_restarts_only_after_a_crash() {
        let e = Entry { program: PathBuf::from("/home/a b/$sa\"ba%/saba-core"), args: vec![] };
        let text = systemd_unit(&e);
        assert!(text.contains(r#"ExecStart="/home/a b/$$sa\"ba%%/saba-core""#), "{}", text);
        assert!(text.contains("Restart=on-failure\n"), "{}", text);
        assert!(text.contains("WantedBy=default.target\n"), "{}", text);
        assert_eq!(parse_systemd_unit(&text), Some(e));
        assert_eq!(parse_systemd_unit("[Unit]\nDescription=x\n"), None);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_service_counts_only_while_it_is_enabled_and_replaces_the_desktop_entry() {
        let dir = tempfile::tempdir().unwrap();
        let (autostart, units) = (dir.path().join("autostart"), dir.path().join("systemd/user"));
        let tray = Entry { program: PathBuf::from("/opt/saba/saba-chan-gui"), args: vec![TRAY_ARG.into()] };
        let service = Entry { program: PathBuf::from("/opt/saba/saba-core"), args: vec![] };
        write_to(&autostart, &units, &tray).unwrap();
        assert_eq!(read_from(&autostart, &units), Some(tray.clone()));
        write_to(&autostart, &units, &service).unwrap();
        assert_eq!(read_from(&autostart, &units), Some(service.clone()));
        assert_eq!(read_in(&autostart), None, "only one entry");
        // `systemctl --user disable`로 끈 서비스는 등록되지 않은 것이다
        std::fs::remove_file(units.join("default.target.wants").join(SERVICE_FILE)).unwrap();
        assert_eq!(read_from(&autostart, &units), None);
        write_to(&autostart, &units, &service).unwrap();
        remove_from(&autostart, &units).unwrap();
        assert_eq!(read_from(&autostart, &units), None);
        assert!(!units.join(SERVICE_FILE).exists());
        write_to(&autostart, &units, &service).unwrap();
        write_to(&autostart, &units, &tray).unwrap();
        assert!(!units.join(SERVICE_FILE).exists(), "the desktop entry replaces the service");
    }

    #[test]
    fn the_run_value_reads_back_with_spaces_in_the_path() {
        let e = Entry { program: PathBuf::from(r"C:\Program Files\Saba chan\saba-chan-gui.exe"), args: vec![TRAY_ARG.into()] };
        let line = command_line(&e);
        assert_eq!(line, r#""C:\Program Files\Saba chan\saba-chan-gui.exe" --tray"#);
        assert_eq!(parse_command_line(&line), Some(e));
        // 사람이 따옴표 없이 적은 값
        let bare = parse_command_line(r"C:\Saba\saba-chan-gui.exe").unwrap();
        assert_eq!(bare.program, PathBuf::from(r"C:\Saba\saba-chan-gui.exe"));
        assert_eq!(bare.mode(), Mode::Full);
        assert_eq!(parse_command_line(r#""C:\unterminated"#), None);
        assert_eq!(parse_command_line("  "), None);
    }

    #[test]
    fn the_desktop_file_reads_back_with_characters_the_spec_escapes() {
        let e = Entry { program: PathBuf::from("/home/a b/$sa\"ba`%\\/saba-chan-gui"), args: vec![TRAY_ARG.into()] };
        let text = desktop_file(&e);
        // 따옴표 규칙의 역슬래시를 문자열 규칙이 한 번 더 겹친다 (`\$` → `\\$`, `\` → `\\\\`)
        assert!(text.contains(r#"Exec="/home/a b/\\$sa\\"ba\\`%%\\\\/saba-chan-gui" --tray"#), "{}", text);
        assert_eq!(parse_desktop_file(&text), Some(e));
        assert_eq!(parse_desktop_file("[Desktop Entry]\nName=x\n"), None);
    }

    #[test]
    fn a_desktop_file_turned_off_by_the_desktop_reads_as_not_registered() {
        let on = desktop_file(&Entry { program: PathBuf::from("/opt/saba/saba-chan-gui"), args: vec![TRAY_ARG.into()] });
        assert!(parse_desktop_file(&on).is_some());
        assert_eq!(parse_desktop_file(&format!("{}Hidden=true\n", on)), None);
        assert_eq!(parse_desktop_file(&on.replace("X-GNOME-Autostart-enabled=true", "X-GNOME-Autostart-enabled=false")), None);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_autostart_folder_holds_one_entry_that_can_be_replaced_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let autostart = dir.path().join("autostart");
        assert_eq!(read_in(&autostart), None);
        remove_in(&autostart).unwrap();
        let daemon = Entry { program: PathBuf::from("/opt/saba/saba-chan-gui"), args: vec![TRAY_ARG.into()] };
        write_in(&autostart, &daemon).unwrap();
        assert_eq!(read_in(&autostart), Some(daemon));
        let full = Entry { program: PathBuf::from("/opt/saba/saba-chan-gui"), args: vec![] };
        write_in(&autostart, &full).unwrap();
        assert_eq!(read_in(&autostart).map(|e| e.mode()), Some(Mode::Full));
        remove_in(&autostart).unwrap();
        assert_eq!(read_in(&autostart), None);
    }

    #[test]
    fn the_uninstaller_only_claims_entries_in_its_own_folder() {
        let e = Entry { program: PathBuf::from("/opt/saba/saba-chan-gui"), args: vec![] };
        assert!(points_into(&e, Path::new("/opt/saba")));
        assert!(points_into(&e, Path::new("/opt/saba/")));
        assert!(!points_into(&e, Path::new("/opt/other")));
        assert!(!points_into(&e, Path::new("/opt")));
    }
}
