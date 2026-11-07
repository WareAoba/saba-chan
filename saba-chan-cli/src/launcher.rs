//! `saba-chan` — 명령 이름이 짧은 실행기. 같은 설치의 `saba-chan-cli`를 그대로 실행한다
//!
//! Windows는 설치 폴더의 `bin\saba-chan.exe`로 놓고 그 `bin`만 사용자 PATH에 넣는다(설치기). 설치 폴더 바로 아래에
//! 두지 않는 것은 0.1.1 GUI 이름 `Saba-chan.exe`(대소문자만 다르다)와 겹쳐 업데이터가 옛 GUI로 보고 치우기 때문이다.
//! 리눅스·맥은 이 실행기 없이 `saba-chan` 링크가 CLI를 가리킨다.

use std::path::{Path, PathBuf};

/// 실행기 자신의 pid. CLI가 이 값을 보면 자기를 띄운 실행기를 안다 (명령창 제거가 실행기를 끄지 않게)
const LAUNCHER_PID_ENV: &str = "SABA_LAUNCHER_PID";

/// 실행기 옆, 없으면 한 단계 위 폴더의 CLI
fn cli_next_to(launcher: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "saba-chan-cli.exe"
    } else {
        "saba-chan-cli"
    };
    let dir = launcher.parent()?;
    [dir.join(name), dir.parent()?.join(name)]
        .into_iter()
        .find(|p| p.is_file())
}

fn main() {
    let me = std::env::current_exe().ok();
    let Some(cli) = me.as_deref().and_then(cli_next_to) else {
        eprintln!("saba-chan: saba-chan-cli was not found next to this launcher. Reinstall Saba-chan.");
        std::process::exit(127);
    };
    let mut cmd = std::process::Command::new(&cli);
    cmd.args(std::env::args_os().skip(1))
        .env(LAUNCHER_PID_ENV, std::process::id().to_string());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = cmd.exec();
        eprintln!("saba-chan: {e}");
        std::process::exit(126);
    }
    #[cfg(not(unix))]
    {
        // Ctrl+C는 같은 콘솔의 CLI가 받아 처리한다: 실행기는 끝나지 않고 CLI가 끝날 때까지 기다린다.
        // `SetConsoleCtrlHandler(None, TRUE)`의 무시 속성은 자식에게 상속되어 CLI도 Ctrl+C를 못 받으므로,
        // 상속되지 않는 실행기만의 처리기를 둔다
        #[cfg(windows)]
        unsafe {
            unsafe extern "system" fn swallow(_ctrl_type: u32) -> windows_sys::core::BOOL {
                1
            }
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(swallow), 1);
        }
        match cmd.status() {
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("saba-chan: {e}");
                std::process::exit(126);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `bin\saba-chan.exe`는 한 단계 위의 CLI를, 같은 폴더에 있으면 그것을 쓴다
    #[test]
    fn finds_the_cli_beside_or_above() {
        let root = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "saba-chan-cli.exe"
        } else {
            "saba-chan-cli"
        };
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        assert_eq!(cli_next_to(&bin.join("saba-chan")), None);
        std::fs::write(root.path().join(name), b"").unwrap();
        assert_eq!(cli_next_to(&bin.join("saba-chan")), Some(root.path().join(name)));
        std::fs::write(bin.join(name), b"").unwrap();
        assert_eq!(cli_next_to(&bin.join("saba-chan")), Some(bin.join(name)));
    }
}
