//! Shared utility functions for the saba-chan core daemon.

use tokio::process::Command;

/// Apply platform-specific flags to hide the console window on Windows.
/// On non-Windows platforms, this is a no-op.
#[cfg(target_os = "windows")]
pub fn apply_creation_flags(cmd: &mut Command) -> &mut Command {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
pub fn apply_creation_flags(cmd: &mut Command) -> &mut Command {
    cmd
}

/// 새 콘솔 창에 띄울 실행 파일 (non-managed 시작). 경로 없는 이름은 `env`의 PATH(없으면 데몬 PATH)에서 `.exe`로 찾고,
/// 상대 경로는 작업 폴더(`cwd`) 기준이다. `.exe`·`.com`만 돌려준다: 배치 파일은 인자를 cmd가 다시 해석하므로
/// `CreateProcessW`로 바로 띄우지 않는다 (None이면 호출자가 `std::process::Command`로 띄운다)
#[cfg_attr(not(windows), allow(dead_code))]
pub fn new_console_program(program: &str, cwd: &std::path::Path, env: &[(String, String)]) -> Option<std::path::PathBuf> {
    let is_exe = |p: &std::path::Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com"));
    let path = std::path::Path::new(program.trim());
    if path.components().count() > 1 || path.is_absolute() {
        let full = cwd.join(path);
        return (is_exe(&full) && full.is_file()).then_some(full);
    }
    let search = env.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case("PATH")).map(|(_, v)| std::ffi::OsString::from(v)).or_else(|| std::env::var_os("PATH"))?;
    let name = if is_exe(path) { path.to_path_buf() } else { path.with_extension("exe") };
    std::env::split_paths(&search).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

/// 데몬 환경 위에 `overrides`를 얹은 환경. 이름은 대소문자를 가리지 않고, 대소문자 무시 순서로 정렬한다 (Windows 환경 블록 규칙)
#[cfg_attr(not(windows), allow(dead_code))]
pub fn merged_env(base: impl IntoIterator<Item = (String, String)>, overrides: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in base.into_iter().chain(overrides.iter().cloned()) {
        out.retain(|(old, _)| !old.eq_ignore_ascii_case(&k));
        out.push((k, v));
    }
    out.sort_by_key(|(k, _)| k.to_uppercase());
    out
}

/// Windows: 프로그램을 새 콘솔 창에 띄운다 ("터미널로 열기" CLI, non-managed 게임 서버). 데몬의 표준 스트림(파이프·NUL·숨긴 콘솔)을
/// 넘기지 않아야 프로그램이 새 창의 콘솔을 자기 입출력으로 쓴다. `std::process::Command`는 표준 핸들을 넘기므로
/// `CreateProcessW`를 직접 부른다 (`STARTF_USESTDHANDLES` 없이, 핸들 상속 없이). `env`가 비면 데몬 환경을 물려준다. pid를 돌려준다
#[cfg(windows)]
pub fn spawn_in_new_console(program: &std::path::Path, args: &[String], dir: Option<&std::path::Path>, env: &[(String, String)]) -> std::io::Result<u32> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW};
    use winapi::um::winbase::{CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT};

    let wide = |s: &std::ffi::OsStr| s.encode_wide().collect::<Vec<u16>>();
    let mut line = Vec::new();
    crate::sandbox::appcontainer::push_arg(&wide(program.as_os_str()), &mut line);
    for a in args {
        crate::sandbox::appcontainer::push_arg(&wide(std::ffi::OsStr::new(a)), &mut line);
    }
    line.push(0);
    let mut app = wide(program.as_os_str());
    app.push(0);
    let mut dir = dir.map(|d| wide(d.as_os_str())).unwrap_or_default();
    dir.push(0);
    // 환경 블록: `이름=값\0`을 잇고 끝에 \0 하나 더
    let block: Option<Vec<u16>> = (!env.is_empty()).then(|| {
        let base = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
        let mut b: Vec<u16> = merged_env(base, env).iter().flat_map(|(k, v)| format!("{k}={v}\0").encode_utf16().collect::<Vec<_>>()).collect();
        b.push(0);
        b
    });
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
            block.as_ref().map_or(std::ptr::null_mut(), |b| b.as_ptr() as *mut _),
            if dir.len() > 1 { dir.as_ptr() } else { std::ptr::null() },
            &mut si,
            &mut pi,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        Ok(pi.dwProcessId)
    }
}

/// 경로 한 칸으로만 쓸 수 있는 이름인지 (인스턴스 id, 모듈 이름, 로케일 등).
/// 구분자·`.`/`..`·드라이브 표기(`C:`)·NUL이 있으면 다른 폴더를 가리킬 수 있어 거부한다.
/// 끝의 `.`·공백도 거부한다: Windows가 버려서 `x.`가 `x` 폴더를 가리킨다
pub fn is_single_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.contains(['/', '\\', ':', '\0'])
}

/// 구성요소 폴더 안을 가리키는 상대 경로인가 (`module.entry`, 익스텐션 `python_modules` 값).
/// 비었거나 절대 경로이거나 `..`·`.`·드라이브 같은 성분이 있으면 아니다
pub fn is_inner_relative_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !path.is_empty() && !p.is_absolute() && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// 준비해 둔 `staged` 폴더를 `dest` 자리에 둔다. 옛 `dest`는 `.old-<이름>`으로 옆에 두었다가 새 폴더가 자리를 잡은 뒤 지우고,
/// 바꾸다 실패하면 되돌린다. `.old-`는 점으로 시작해 탐색이 건너뛰고, 중간에 끝나면 다음 기동의 `remove_install_leftovers`가 되돌린다
pub fn swap_dir(staged: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context;
    let mut old_name = std::ffi::OsString::from(".old-");
    old_name.push(dest.file_name().unwrap_or_default());
    let old = dest.with_file_name(old_name);
    if old.exists() {
        std::fs::remove_dir_all(&old).with_context(|| format!("Failed to clear {}", old.display()))?;
    }
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &old).with_context(|| format!("Failed to move old {} aside", dest.display()))?;
    }
    if let Err(e) = std::fs::rename(staged, dest) {
        if had_old {
            let _ = std::fs::rename(&old, dest);
        }
        return Err(anyhow::Error::new(e).context(format!("Failed to put {} in place", dest.display())));
    }
    if had_old {
        if let Err(e) = std::fs::remove_dir_all(&old) {
            tracing::warn!("Failed to remove the old files {}: {}", old.display(), e);
        }
    }
    Ok(())
}

/// 설치 도중 데몬이 끝나 남은 준비·옛 폴더와 받다 만 파일 (`.staging-*`·`.old-*`·`.tmp-*`)을 지운다.
/// 옛 폴더를 옆으로 옮긴 뒤 새 폴더를 두기 전에 끝났으면(제자리가 비었으면) 옛 폴더를 되돌린다.
/// 설치가 돌 수 없는 기동 때만 부른다
pub fn remove_install_leftovers(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut leftovers: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    // 되돌리기를 먼저: 되돌린 폴더는 지우지 않는다
    leftovers.retain(|path| {
        let Some(original) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix(".old-")) else { return true };
        let target = dir.join(original);
        if original.is_empty() || !path.is_dir() || target.exists() {
            return true;
        }
        match std::fs::rename(path, &target) {
            Ok(()) => {
                tracing::warn!("Restored {} after an interrupted install", target.display());
                false
            }
            Err(e) => {
                // 지우면 마지막 정상 설치까지 잃는다: 그대로 둔다
                tracing::warn!("Could not restore {} after an interrupted install: {}", target.display(), e);
                false
            }
        }
    });
    for path in leftovers {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if ![".staging-", ".old-", ".tmp-"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => tracing::info!("Removed leftover of an interrupted install: {}", path.display()),
            Err(e) => tracing::warn!("Could not remove leftover {}: {}", path.display(), e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_console_runs_only_an_exe_found_like_the_shell_would() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("java.exe"), "").unwrap();
        std::fs::write(bin.join("start.bat"), "").unwrap();
        let path = std::env::join_paths([&bin]).unwrap().into_string().unwrap();
        let env = vec![("Path".to_string(), path)];
        // 경로 없는 이름은 모듈이 준 PATH(없으면 데몬 PATH)에서 .exe로 찾는다
        assert_eq!(new_console_program("java", tmp.path(), &env), Some(bin.join("java.exe")));
        assert_eq!(new_console_program(&bin.join("java.exe").to_string_lossy(), tmp.path(), &[]), Some(bin.join("java.exe")));
        // 상대 경로는 작업 폴더 기준
        assert_eq!(new_console_program("bin/java.exe", tmp.path(), &[]), Some(bin.join("java.exe")));
        // 배치 파일·못 찾은 이름은 새 콘솔로 띄우지 않는다 (호출자가 예전 방식으로 띄운다)
        assert_eq!(new_console_program(&bin.join("start.bat").to_string_lossy(), tmp.path(), &env), None);
        assert_eq!(new_console_program("missing", tmp.path(), &env), None);
    }

    #[test]
    fn the_new_console_environment_is_the_daemons_with_the_modules_values_on_top() {
        let base = [("PATH", "a"), ("Temp", "t"), ("b", "1")].map(|(k, v)| (k.to_string(), v.to_string()));
        let merged = merged_env(base, &[("path".to_string(), "x".to_string()), ("NEW".to_string(), "n".to_string())]);
        // 이름은 대소문자를 가리지 않고 덮고, 대소문자 무시 순서로 정렬한다 (Windows 환경 블록)
        assert_eq!(merged, [("b", "1"), ("NEW", "n"), ("path", "x"), ("Temp", "t")].map(|(k, v)| (k.to_string(), v.to_string())));
    }

    #[test]
    fn only_plain_names_are_single_components() {
        for ok in ["minecraft", "ko", "zh-TW", "a1b2-c3", "my.server"] {
            assert!(is_single_path_component(ok), "{ok}");
        }
        // Windows는 끝의 `.`·공백을 버려 `x.`와 `x `가 `x` 폴더를 가리킨다 (백로그 L17c)
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:", "C:x", "a\0b", "x.", "x ", "x. "] {
            assert!(!is_single_path_component(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_install_cut_off_mid_swap_gets_its_old_folder_back() {
        let tmp = tempfile::tempdir().unwrap();
        // 옛 폴더를 옆으로 옮긴 뒤, 새 폴더를 제자리에 두기 전에 데몬이 끝났다
        for dir in [".old-box", ".staging-box"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        std::fs::write(tmp.path().join(".old-box").join("module.toml"), "kept").unwrap();
        remove_install_leftovers(tmp.path());
        let left: Vec<String> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["box"]);
        assert_eq!(std::fs::read_to_string(tmp.path().join("box").join("module.toml")).unwrap(), "kept");
    }

    #[test]
    fn leftovers_of_an_interrupted_install_are_removed_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in [".staging-box", ".staging-box-1a2b", ".old-box", "box", ".hidden"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        std::fs::write(tmp.path().join(".tmp-module-box.zip"), "partial").unwrap();
        std::fs::write(tmp.path().join("box.zip"), "kept").unwrap();
        remove_install_leftovers(tmp.path());
        let mut left: Vec<String> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [".hidden", "box", "box.zip"]);
    }
}
