//! 사용자 PATH에 설치 폴더의 `bin`을 넣고 뺀다 (Windows `HKCU\Environment\Path`)
//!
//! `bin`에는 짧은 명령 실행기 `saba-chan.exe`만 있다(saba-chan-cli `launcher.rs`). 설치 폴더 자체를 넣지 않는 것은
//! 데몬·설치기 같은 다른 실행 파일까지 이름으로 불리지 않게 하려는 것이다.
//! 리눅스·맥은 install.sh가 `~/.local/bin` 링크를 만든다.

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

#[cfg(not(windows))]
pub fn add(_install_dir: &Path) -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn remove(_install_dir: &Path) -> anyhow::Result<bool> {
    Ok(false)
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
}
