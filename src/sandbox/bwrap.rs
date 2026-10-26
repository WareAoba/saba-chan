//! 확인되지 않은 모듈·익스텐션의 Python 격리 (리눅스, bubblewrap)
//!
//! AppContainer처럼 준 폴더만 보인다: 시스템 폴더(`SYSTEM_DIRS`)는 읽기 전용으로 붙이고, 그 밖(홈·데이터 폴더·다른 인스턴스·
//! `/run`의 사용자 소켓)은 보이지 않는다. `/tmp`는 빈 폴더다. 그 위에 `Grants`의 폴더만 붙인다: 읽기는 읽기 전용, 쓰기는 쓰기
//! (목록 권한은 필요 없다: 붙인 폴더의 위 폴더는 bwrap이 만든다). 네트워크는 연다(설치가 내려받는다). 그래서 Windows와 달리
//! 127.0.0.1에도 닿는다: 데몬 API는 토큰이 데이터 폴더에 있어 쓰지 못한다.
//! 프로세스·IPC·UTS 네임스페이스는 따로 두고, bwrap이 끝나면(`--die-with-parent`) 안의 프로세스도 모두 끝난다.

use super::appcontainer::{Access, Grants};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// bwrap을 찾는 곳 (PATH를 따르지 않는다)
const CANDIDATES: &[&str] = &["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"];

/// 읽기 전용으로 보이는 시스템 폴더 (없는 것은 건너뛴다). `/run/systemd/resolve`는 `/etc/resolv.conf`가 가리키는 곳이다
const SYSTEM_DIRS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/etc",
    "/nix/store",
    "/run/systemd/resolve",
];

/// 쓸 수 있는 bwrap. 처음 한 번 빈 격리를 띄워 본다 (사용자 네임스페이스를 막은 시스템은 실패한다)
pub fn program() -> Option<&'static Path> {
    // 다른 시험은 리눅스가 동의를 받아 격리 없이 돈다고 본다: bwrap 시험만 켠다
    #[cfg(test)]
    if !USE_IN_TEST.with(std::cell::Cell::get) {
        return None;
    }
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            if !cfg!(target_os = "linux") {
                return None;
            }
            let bwrap = CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file())?;
            let mut probe = std::process::Command::new(&bwrap);
            probe.args(args(&Grants::default(), None)).args(["--", "true"]);
            probe
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            match probe.output() {
                Ok(out) if out.status.success() => Some(bwrap),
                Ok(out) => {
                    tracing::warn!(
                        "[Sandbox] bubblewrap cannot isolate here: {}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    );
                    None
                }
                Err(e) => {
                    tracing::warn!("[Sandbox] cannot run {}: {}", bwrap.display(), e);
                    None
                }
            }
        })
        .as_deref()
}

#[cfg(test)]
thread_local! {
    static USE_IN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 이 시험 스레드에서 bwrap을 쓴다
#[cfg(test)]
pub fn use_in_this_test() {
    USE_IN_TEST.with(|c| c.set(true));
}

/// bwrap 인자 (`--` 앞까지). 붙이는 폴더는 위 폴더부터 붙여, 안쪽 폴더의 권한이 바깥 폴더에 덮이지 않게 한다
pub fn args(grants: &Grants, cwd: Option<&Path>) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    let mut push = |items: &[&std::ffi::OsStr]| out.extend(items.iter().map(|s| s.to_os_string()));
    for dir in SYSTEM_DIRS.iter().map(Path::new) {
        match std::fs::symlink_metadata(dir) {
            // 합쳐진 /usr (`/bin -> usr/bin`): 링크는 링크로 둔다
            Ok(m) if m.file_type().is_symlink() => {
                if let Ok(target) = std::fs::read_link(dir) {
                    push(&["--symlink".as_ref(), target.as_os_str(), dir.as_os_str()]);
                }
            }
            Ok(m) if m.is_dir() => push(&["--ro-bind".as_ref(), dir.as_os_str(), dir.as_os_str()]),
            _ => {}
        }
    }
    for flag in [
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--unshare-all",
        "--share-net",
        "--die-with-parent",
        "--new-session",
    ] {
        push(&[flag.as_ref()]);
    }
    let mut entries: Vec<&(PathBuf, Access)> = grants.entries.iter().filter(|(_, a)| *a != Access::List).collect();
    entries.sort_by_key(|(p, a)| (p.components().count(), *a == Access::Write));
    for (path, access) in entries {
        let flag = if *access == Access::Write {
            "--bind"
        } else {
            "--ro-bind"
        };
        push(&[flag.as_ref(), path.as_os_str(), path.as_os_str()]);
    }
    if let Some(cwd) = cwd {
        push(&["--chdir".as_ref(), cwd.as_os_str()]);
    }
    out
}

#[cfg_attr(windows, allow(dead_code))] // Windows는 AppContainer로 띄운다
/// `cmd`를 bwrap 안에서 띄우는 명령. 환경 변수 변경과 작업 폴더는 그대로 옮긴다 (입출력은 부르는 쪽이 정한다)
pub fn wrap(bwrap: &Path, cmd: &std::process::Command, grants: &Grants) -> std::process::Command {
    let mut wrapped = std::process::Command::new(bwrap);
    wrapped
        .args(args(grants, cmd.get_current_dir()))
        .arg("--")
        .arg(cmd.get_program())
        .args(cmd.get_args());
    for (key, value) in cmd.get_envs() {
        match value {
            Some(v) => wrapped.env(key, v),
            None => wrapped.env_remove(key),
        };
    }
    // 세션 버스는 보이지 않지만 주소도 넘기지 않는다
    wrapped.env_remove("DBUS_SESSION_BUS_ADDRESS");
    if let Some(cwd) = cmd.get_current_dir() {
        wrapped.current_dir(cwd);
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(args: &[OsString], seq: &[&str]) -> Option<usize> {
        args.windows(seq.len())
            .position(|w| w.iter().zip(seq).all(|(a, b)| a == b))
    }

    #[test]
    fn only_granted_folders_are_mounted_and_inner_ones_come_last() {
        let grants = Grants {
            entries: vec![
                (PathBuf::from("/srv/game/world"), Access::Write),
                (PathBuf::from("/srv/game"), Access::Read),
                (PathBuf::from("/srv"), Access::List),
            ],
            ..Default::default()
        };
        let args = args(&grants, Some(Path::new("/srv/mods/x")));
        let outer = pos(&args, &["--ro-bind", "/srv/game", "/srv/game"]).expect("read grant is read-only");
        let inner = pos(&args, &["--bind", "/srv/game/world", "/srv/game/world"]).expect("write grant is writable");
        assert!(
            outer < inner,
            "the inner writable folder must be mounted over the outer read-only one"
        );
        assert!(pos(&args, &["/srv", "/srv"]).is_none(), "a list grant mounts nothing");
        assert!(
            pos(&args, &["--ro-bind", "/", "/"]).is_none(),
            "the whole file system is never visible"
        );
        for flag in ["--unshare-all", "--share-net", "--die-with-parent", "--new-session"] {
            assert!(pos(&args, &[flag]).is_some(), "{flag}");
        }
        assert!(
            pos(&args, &["--tmpfs", "/tmp"]).unwrap() < outer,
            "grants under /tmp must land on top of the empty /tmp"
        );
        assert!(pos(&args, &["--chdir", "/srv/mods/x"]).is_some());
    }

    #[test]
    fn an_isolated_run_sees_only_its_grants() {
        use_in_this_test();
        let Some(bwrap) = program() else {
            eprintln!("bubblewrap is not usable here; skipping");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (granted, readonly, hidden) = (root.join("granted"), root.join("readonly"), root.join("hidden"));
        for d in [&granted, &readonly, &hidden] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(readonly.join("a"), "read").unwrap();
        std::fs::write(hidden.join("secret"), "token").unwrap();
        let grants = Grants {
            entries: vec![(granted.clone(), Access::Write), (readonly.clone(), Access::Read)],
            ..Default::default()
        };
        let run = |script: &str| {
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.args(["-c", script]).current_dir(&granted);
            wrap(bwrap, &cmd, &grants).output().unwrap()
        };
        let ok = |script: &str| run(script).status.success();
        assert!(ok("echo x > out"), "writes inside a write grant");
        assert_eq!(std::fs::read_to_string(granted.join("out")).unwrap(), "x\n");
        assert!(ok(&format!("cat {}/a", readonly.display())), "reads a read grant");
        assert!(
            !ok(&format!("echo x > {}/b", readonly.display())),
            "cannot write a read grant"
        );
        assert!(
            !ok(&format!("cat {}/secret", hidden.display())),
            "cannot read an ungranted folder"
        );
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        assert!(!ok(&format!("ls {home}")), "the home folder is not visible");
        assert!(!ok("echo x > /usr/saba-bwrap-test"), "system folders are read-only");
    }
}
