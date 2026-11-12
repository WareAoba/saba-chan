//! 사용자 PATH에 설치 폴더의 `bin`을 넣고 뺀다 (Windows `HKCU\Environment\Path`)
//!
//! `bin`에는 짧은 명령 실행기 `saba-chan.exe`만 있다(saba-chan-cli `launcher.rs`). 설치 폴더 자체를 넣지 않는 것은
//! 데몬·설치기 같은 다른 실행 파일까지 이름으로 불리지 않게 하려는 것이다.
//! 맥은 `~/.local/bin`에 CLI 링크(`saba-chan`, `saba-chan-cli`)를 두고, 그 폴더를 `~/.zprofile`의 표시한 한 줄로 PATH에 넣는다
//! (맥 기본 PATH에 없는 폴더다. 터미널 앱은 로그인 셸이라 이 파일을 읽는다). 리눅스는 install.sh가 같은 링크를 만든다.

use std::path::Path;

/// 명령 실행기가 드는 설치 폴더 안 폴더
pub const BIN_DIR: &str = "bin";

/// 같은 항목인지: 대소문자·끝 `\`·`/`를 가리지 않는다 (Windows 경로)
fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_matches('"')
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    !a.trim().is_empty() && norm(a) == norm(b)
}

/// `dir`을 넣은 PATH 값. 이미 있으면 None (바꿀 것이 없다)
pub fn with_entry(path: &str, dir: &str) -> Option<String> {
    if path.split(';').any(|p| same(p, dir)) {
        return None;
    }
    let base = path.trim_end_matches(';');
    Some(if base.is_empty() {
        dir.to_string()
    } else {
        format!("{base};{dir}")
    })
}

/// `dir`을 뺀 PATH 값. 없으면 None. 다른 항목과 그 순서는 그대로 둔다
pub fn without_entry(path: &str, dir: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(';').collect();
    if !parts.iter().any(|p| same(p, dir)) {
        return None;
    }
    Some(
        parts
            .into_iter()
            .filter(|p| !same(p, dir))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

/// 설치 폴더의 `bin`을 사용자 PATH에 넣는다. 실행기가 없으면(옛 CLI 묶음) 넣지 않는다
#[cfg(windows)]
pub fn add(install_dir: &Path) -> anyhow::Result<bool> {
    let bin = install_dir.join(BIN_DIR);
    if !bin.join("saba-chan.exe").is_file() {
        return Ok(false);
    }
    edit(|path| with_entry(path, &bin.to_string_lossy()))
}

/// 설치 폴더의 `bin`을 사용자 PATH에서 뺀다
#[cfg(windows)]
pub fn remove(install_dir: &Path) -> anyhow::Result<bool> {
    let bin = install_dir.join(BIN_DIR);
    edit(|path| without_entry(path, &bin.to_string_lossy()))
}

#[cfg(target_os = "macos")]
pub fn add(install_dir: &Path) -> anyhow::Result<bool> {
    Ok(link_commands(&home()?, install_dir)?)
}

#[cfg(target_os = "macos")]
pub fn remove(install_dir: &Path) -> anyhow::Result<bool> {
    Ok(unlink_commands(&home()?, install_dir)?)
}

#[cfg(unix)]
fn home() -> anyhow::Result<std::path::PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(Into::into).ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

/// 리눅스: `~/.local/bin`에 링크만 둔다 (install.sh를 거치지 않은 설치에도 `saba-chan` 명령이 생긴다). 지우는 것은 제거기가 한다
#[cfg(all(unix, not(target_os = "macos")))]
pub fn add(install_dir: &Path) -> anyhow::Result<bool> {
    Ok(link_cli(&home()?, install_dir)?)
}

#[cfg(not(any(windows, unix)))]
pub fn add(_install_dir: &Path) -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn remove(_install_dir: &Path) -> anyhow::Result<bool> {
    Ok(false)
}

/// 링크를 두는 폴더 (홈 기준). install.sh와 같은 자리
#[cfg(unix)]
const LINK_DIR: &str = ".local/bin";
/// 링크 이름: 짧은 명령과 원래 이름
#[cfg(unix)]
const LINK_NAMES: [&str; 2] = ["saba-chan", "saba-chan-cli"];
/// PATH를 넣는 셸 설정 파일 (홈 기준, 맥 기본 셸 zsh의 로그인 설정)
#[cfg(unix)]
const PROFILE: &str = ".zprofile";
/// 셸 설정에 넣는 한 줄. 끝의 표시로 우리 줄을 찾아 뺀다
#[cfg(unix)]
const PROFILE_LINE: &str = "export PATH=\"$HOME/.local/bin:$PATH\" # added by saba-chan";

/// 셸 설정에 우리 줄을 더한 내용. 이미 있으면 None
#[cfg(unix)]
fn with_profile_line(text: &str) -> Option<String> {
    if text.lines().any(|l| l.trim() == PROFILE_LINE) {
        return None;
    }
    let mut next = text.to_string();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(PROFILE_LINE);
    next.push('\n');
    Some(next)
}

/// 셸 설정에서 우리 줄만 뺀 내용. 없으면 None
#[cfg(unix)]
fn without_profile_line(text: &str) -> Option<String> {
    if !text.lines().any(|l| l.trim() == PROFILE_LINE) {
        return None;
    }
    Some(text.lines().filter(|l| l.trim() != PROFILE_LINE).map(|l| format!("{l}\n")).collect())
}

/// `home/.local/bin`에 설치 폴더 CLI로 가는 링크를 두고 셸 설정에 PATH 줄을 넣는다. 바꾼 것이 있으면 true.
/// CLI가 없으면 아무것도 하지 않는다
#[cfg(unix)]
pub fn link_commands(home: &Path, install_dir: &Path) -> std::io::Result<bool> {
    if !install_dir.join("saba-chan-cli").is_file() {
        return Ok(false);
    }
    let mut changed = link_cli(home, install_dir)?;
    let profile = home.join(PROFILE);
    // 읽지 못한 파일(UTF-8이 아님·권한)을 빈 파일로 보고 덮어쓰면 사용자의 설정이 사라진다: 없을 때만 새로 만든다
    let text = match std::fs::read_to_string(&profile) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if let Some(next) = with_profile_line(&text) {
        std::fs::write(&profile, next)?;
        changed = true;
    }
    Ok(changed)
}

/// `home/.local/bin`에 설치 폴더 CLI로 가는 링크를 둔다. 바꾼 것이 있으면 true.
/// 다른 곳을 가리키던 링크(옮기기 전 앱)는 바꾸고, 링크가 아닌 같은 이름의 파일은 건드리지 않는다. CLI가 없으면 아무것도 하지 않는다
#[cfg(unix)]
pub fn link_cli(home: &Path, install_dir: &Path) -> std::io::Result<bool> {
    let cli = install_dir.join("saba-chan-cli");
    if !cli.is_file() {
        return Ok(false);
    }
    let bin = home.join(LINK_DIR);
    std::fs::create_dir_all(&bin)?;
    let mut changed = false;
    for name in LINK_NAMES {
        let link = bin.join(name);
        match std::fs::symlink_metadata(&link) {
            Ok(m) if !m.file_type().is_symlink() => continue,
            Ok(_) if std::fs::read_link(&link).is_ok_and(|to| to == cli) => continue,
            Ok(_) => std::fs::remove_file(&link)?,
            Err(_) => {}
        }
        std::os::unix::fs::symlink(&cli, &link)?;
        changed = true;
    }
    Ok(changed)
}

/// 이 설치 폴더 CLI로 가는 링크를 지운다. 남은 링크가 없으면 셸 설정의 우리 줄도 뺀다. 바꾼 것이 있으면 true
#[cfg(unix)]
pub fn unlink_commands(home: &Path, install_dir: &Path) -> std::io::Result<bool> {
    let cli = install_dir.join("saba-chan-cli");
    let bin = home.join(LINK_DIR);
    let mut changed = false;
    for name in LINK_NAMES {
        let link = bin.join(name);
        if std::fs::read_link(&link).is_ok_and(|to| to == cli) {
            std::fs::remove_file(&link)?;
            changed = true;
        }
    }
    let ours_left = LINK_NAMES.iter().any(|n| std::fs::symlink_metadata(bin.join(n)).is_ok());
    let profile = home.join(PROFILE);
    if !ours_left {
        if let Some(next) = std::fs::read_to_string(&profile).ok().as_deref().and_then(without_profile_line) {
            std::fs::write(&profile, next)?;
            changed = true;
        }
    }
    Ok(changed)
}

/// 값의 종류(`REG_EXPAND_SZ`면 `%USERPROFILE%` 같은 항목이 그대로 남게)를 지키며 고치고, 열린 창들에 알린다
#[cfg(windows)]
fn edit(change: impl Fn(&str) -> Option<String>) -> anyhow::Result<bool> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ};
    use winreg::{RegKey, RegValue};
    let env = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
    let (current, kind) = match env.get_raw_value("Path") {
        Ok(raw) => {
            let units: Vec<u16> = raw
                .bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            let text = String::from_utf16_lossy(&units).trim_end_matches('\0').to_string();
            (text, raw.vtype)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), REG_EXPAND_SZ),
        Err(e) => return Err(e.into()),
    };
    let Some(next) = change(&current) else { return Ok(false) };
    let bytes: Vec<u8> = next
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    env.set_raw_value("Path", &RegValue { bytes, vtype: kind })?;
    broadcast();
    Ok(true)
}

/// 새로 여는 명령창이 바뀐 PATH를 받게 알린다 (이미 열린 명령창은 다시 열어야 한다)
#[cfg(windows)]
fn broadcast() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let name: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            name.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 넣을 때는 끝에 하나만, 이미 있으면(대소문자·끝 `\`가 달라도) 그대로
    #[test]
    fn adds_once_at_the_end() {
        let dir = r"C:\Saba-chan\bin";
        assert_eq!(
            with_entry(r"C:\Windows;%USERPROFILE%\x", dir).as_deref(),
            Some(r"C:\Windows;%USERPROFILE%\x;C:\Saba-chan\bin")
        );
        assert_eq!(with_entry("", dir).as_deref(), Some(dir));
        assert_eq!(
            with_entry(r"C:\Windows;", dir).as_deref(),
            Some(r"C:\Windows;C:\Saba-chan\bin")
        );
        assert_eq!(with_entry(r"c:\saba-chan\BIN\;C:\Windows", dir), None);
    }

    /// 뺄 때는 그 항목만, 다른 항목과 순서는 그대로
    #[test]
    fn removes_only_ours() {
        let dir = r"C:\Saba-chan\bin";
        assert_eq!(
            without_entry(r"C:\A;C:\Saba-chan\bin\;%X%\y", dir).as_deref(),
            Some(r"C:\A;%X%\y")
        );
        assert_eq!(without_entry(r"C:\A;C:\Saba-chan", dir), None);
        assert_eq!(without_entry(r"C:\Saba-chan\bin", dir).as_deref(), Some(""));
    }

    /// 맥: 링크는 CLI를 가리키고 셸 설정에는 PATH 줄 하나만 더한다. 앱을 옮기면 링크가 새 자리를 가리키고,
    /// 지울 때는 우리 링크와 줄만 빼서 사용자가 쓴 다른 줄은 그대로 둔다
    #[cfg(unix)]
    #[test]
    fn mac_links_the_cli_and_adds_one_profile_line() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let old_app = tmp.path().join("Old.app/Contents/MacOS");
        let app = tmp.path().join("Saba-chan.app/Contents/MacOS");
        for dir in [&old_app, &app] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("saba-chan-cli"), b"").unwrap();
        }
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zprofile"), "eval \"$(/opt/homebrew/bin/brew shellenv)\"").unwrap();

        assert!(link_commands(&home, &old_app).unwrap());
        assert!(link_commands(&home, &app).unwrap(), "a moved app relinks");
        assert!(!link_commands(&home, &app).unwrap(), "nothing changes the second time");
        for name in LINK_NAMES {
            assert_eq!(std::fs::read_link(home.join(".local/bin").join(name)).unwrap(), app.join("saba-chan-cli"));
        }
        let profile = std::fs::read_to_string(home.join(".zprofile")).unwrap();
        assert_eq!(profile, format!("eval \"$(/opt/homebrew/bin/brew shellenv)\"\n{PROFILE_LINE}\n"));

        assert!(!unlink_commands(&home, &old_app).unwrap(), "links of another app are left alone");
        assert!(unlink_commands(&home, &app).unwrap());
        assert!(std::fs::symlink_metadata(home.join(".local/bin/saba-chan")).is_err());
        assert_eq!(std::fs::read_to_string(home.join(".zprofile")).unwrap(), "eval \"$(/opt/homebrew/bin/brew shellenv)\"\n");
    }

    /// 리눅스: install.sh를 거치지 않고 `saba-chan-cli install`만 돌려도 `saba-chan` 명령이 생긴다.
    /// 셸 설정은 건드리지 않는다 (배포판 대부분이 `~/.local/bin`을 이미 PATH에 넣는다)
    #[cfg(unix)]
    #[test]
    fn linux_links_the_cli_without_touching_the_shell_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, dir) = (tmp.path().join("home"), tmp.path().join("saba-chan"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("saba-chan-cli"), b"").unwrap();
        assert!(link_cli(&home, &dir).unwrap());
        for name in LINK_NAMES {
            assert_eq!(std::fs::read_link(home.join(".local/bin").join(name)).unwrap(), dir.join("saba-chan-cli"));
        }
        assert!(!home.join(PROFILE).exists());
        assert!(!link_cli(&home, &dir).unwrap(), "nothing changes the second time");
    }

    /// 링크가 아닌 같은 이름의 파일은 사용자 것이라 덮지 않는다
    #[cfg(unix)]
    #[test]
    fn mac_never_overwrites_a_profile_it_cannot_read() {
        let home = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        std::fs::write(app.path().join("saba-chan-cli"), "").unwrap();
        let profile = home.path().join(PROFILE);
        std::fs::write(&profile, b"export A=1 # \xff\n").unwrap();
        assert!(link_commands(home.path(), app.path()).is_err());
        assert_eq!(std::fs::read(&profile).unwrap(), b"export A=1 # \xff\n");
    }

    #[cfg(unix)]
    #[test]
    fn mac_keeps_a_users_own_file() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("MacOS");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("saba-chan-cli"), b"").unwrap();
        std::fs::create_dir_all(tmp.path().join(".local/bin")).unwrap();
        std::fs::write(tmp.path().join(".local/bin/saba-chan"), b"mine").unwrap();
        link_commands(tmp.path(), &app).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(".local/bin/saba-chan")).unwrap(), b"mine");
        assert!(std::fs::read_link(tmp.path().join(".local/bin/saba-chan-cli")).is_ok());
    }
}
