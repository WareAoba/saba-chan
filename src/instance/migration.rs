//! 가져온(마이그레이션) 서버의 원래 게임 설정 파일
//!
//! 가져올 때 모듈이 `config_files`로 밝힌 파일(사바쨩이 고칠 수 있는 게임 설정)을 인스턴스 폴더의
//! `migration/`에 그대로 복사해 둔다. 가져오기를 해제(인스턴스 삭제)할 때 이 사본으로 되돌려,
//! 사바쨩이 바꾼 설정(RCON 끄기, REST 켜기, 포트 등)을 남기지 않는다. 서버 폴더 자체는 지우지 않는다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 인스턴스 폴더 안의 보관 폴더 (`DAEMON_ENTRIES`: 격리된 모듈이 쓰지 못한다)
pub const DIR: &str = "migration";
const MANIFEST: &str = "originals.json";

#[derive(Debug, Serialize, Deserialize)]
struct Original {
    /// 원래 자리 (가져올 때의 절대 경로)
    path: PathBuf,
    /// `migration/` 안의 사본 이름
    copy: String,
}

/// 되돌리기 결과
#[derive(Debug, Default, PartialEq)]
pub struct Restored {
    pub restored: usize,
    /// 되돌리지 못한 원래 자리 (경로는 데몬 로그에만)
    pub failed: Vec<PathBuf>,
}

/// 원래 파일을 보관한다. 없는 파일은 건너뛴다(가져오기 전에 없던 파일은 되돌릴 내용이 없다).
/// 인스턴스 폴더 안의 파일(옮겨 온 서버)은 인스턴스와 함께 지워지므로 건너뛴다. 보관한 수
pub fn save(instance_dir: &Path, files: &[PathBuf]) -> std::io::Result<usize> {
    let dir = instance_dir.join(DIR);
    let mut originals = Vec::new();
    for path in files.iter().filter(|p| p.is_file() && !inside_instance(p, instance_dir)) {
        if originals.iter().any(|o: &Original| o.path == *path) {
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        let copy = originals.len().to_string();
        std::fs::copy(path, dir.join(&copy))?;
        originals.push(Original { path: path.clone(), copy });
    }
    if !originals.is_empty() {
        let body = serde_json::to_vec_pretty(&originals).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(MANIFEST), body)?;
    }
    Ok(originals.len())
}

/// 보관한 원래 파일이 있는가 (해제할 때 되돌리기를 물을지)
pub fn saved(instance_dir: &Path) -> bool {
    instance_dir.join(DIR).join(MANIFEST).is_file()
}

/// 보관한 내용으로 원래 자리를 되돌린다. `allowed`가 거절한 자리(지금 그 서버의 폴더 밖)와
/// 상위 폴더가 사라진 자리는 건드리지 않고 실패로 센다
pub fn restore(instance_dir: &Path, allowed: impl Fn(&Path) -> bool) -> Restored {
    let dir = instance_dir.join(DIR);
    let mut out = Restored::default();
    let Ok(body) = std::fs::read(dir.join(MANIFEST)) else { return out };
    let Ok(originals) = serde_json::from_slice::<Vec<Original>>(&body) else {
        tracing::warn!("Migration originals list is unreadable: {}", dir.display());
        return out;
    };
    for original in originals {
        let copy = dir.join(&original.copy);
        let ok = crate::utils::is_single_path_component(&original.copy)
            && copy.is_file()
            && original.path.parent().is_some_and(Path::is_dir)
            && allowed(&original.path)
            && std::fs::copy(&copy, &original.path).is_ok();
        if ok {
            out.restored += 1;
        } else {
            tracing::warn!("Could not restore {} to its state before the import", original.path.display());
            out.failed.push(original.path);
        }
    }
    out
}

/// 인스턴스 폴더 밖에 있는 서버 폴더 (가져온 서버 등). 인스턴스를 지워도 남는다
pub fn linked_folder(working_dir: Option<&str>, instance_dir: &Path) -> Option<String> {
    let dir = working_dir.map(str::trim).filter(|d| !d.is_empty())?;
    (!inside_instance(Path::new(dir), instance_dir)).then(|| dir.to_string())
}

/// 인스턴스 폴더 안인가. 모르면 안으로 본다: 링크(정션 등) 아래 데이터 폴더처럼 `is_inside`가 판정하지 못하는 경로도
/// 글자 그대로나 실제 경로로 안이면 안이다 (밖으로 잘못 보면 인스턴스와 함께 지울 서버 폴더를 "남는다"고 안내한다)
fn inside_instance(path: &Path, instance_dir: &Path) -> bool {
    let lexical = || std::path::absolute(path).ok().zip(std::path::absolute(instance_dir).ok()).is_some_and(|(p, i)| p.starts_with(i));
    let real = || std::fs::canonicalize(path).ok().zip(std::fs::canonicalize(instance_dir).ok()).is_some_and(|(p, i)| p.starts_with(i));
    crate::module_paths::is_inside(path, &[instance_dir.to_path_buf()]) || lexical() || real()
}

/// 옮겨 온 서버 폴더가 놓일 자리: 인스턴스 폴더 안의 `subdir`(모듈 `install_subdir`). 상대 경로의 보통 이름만 받고,
/// 첫 이름이 데몬 항목(`logs`·`migration` 등)이면 거절한다 (모듈이 정한 값으로 서버 폴더를 밖이나 데몬 자리로 옮기지 않게)
pub fn move_target(instance_dir: &Path, subdir: &str) -> Option<PathBuf> {
    let rel = Path::new(subdir);
    let mut parts = rel.components().peekable();
    let first = match parts.peek() {
        Some(std::path::Component::Normal(name)) => name.to_str()?,
        _ => return None,
    };
    let plain = rel.components().all(|c| matches!(c, std::path::Component::Normal(n) if n.to_str().is_some_and(|n| !n.contains(':') && !n.contains('\\'))));
    (plain && !crate::supervisor::is_daemon_entry(first)).then(|| instance_dir.join(rel))
}

/// 가져오기 "옮기기": 서버 폴더(`src`)를 인스턴스 폴더 안(`dest`)으로 옮겨 이 인스턴스에 전속시킨다.
/// 같은 드라이브 안의 이름 바꾸기만 한다(바로 끝나고, 실패하면 아무것도 바뀌지 않는다). 다른 드라이브로 복사하지 않는다.
/// 실패하면 까닭 코드: `source_missing`, `destination_exists`, `move_cross_volume`(다른 드라이브),
/// `move_in_use`(켜진 서버·열린 파일, 권한), `move_failed`
pub fn move_into(src: &Path, dest: &Path) -> Result<(), &'static str> {
    if !src.is_dir() {
        return Err("source_missing");
    }
    if dest.exists() {
        return Err("destination_exists");
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "move_failed")?;
    }
    std::fs::rename(src, dest).map_err(|e| {
        tracing::warn!("Migration: could not move {} to {}: {}", src.display(), dest.display(), e);
        match e.kind() {
            std::io::ErrorKind::CrossesDevices => "move_cross_volume",
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy => "move_in_use",
            // Windows: 다른 프로세스가 쓰는 중(ERROR_SHARING_VIOLATION 32), 다른 드라이브(ERROR_NOT_SAME_DEVICE 17)
            _ if cfg!(windows) && e.raw_os_error() == Some(32) => "move_in_use",
            _ if cfg!(windows) && e.raw_os_error() == Some(17) => "move_cross_volume",
            _ => "move_failed",
        }
    })
}

/// 가져온 폴더에서 사용자에게 미리 알릴 것 (만들기 응답 `notices`):
/// `folder_not_writable` 쓸 수 없는 폴더(Program Files 등, 설정 저장·서버 실행이 실패한다),
/// `server_running` 그 폴더에서 이미 도는 프로그램(사바쨩 밖에서 켠 서버)
pub fn notices(folder: &Path) -> Vec<&'static str> {
    let mut out = Vec::new();
    if folder.is_dir() && !writable(folder) {
        out.push("folder_not_writable");
    }
    let me = std::process::id();
    if crate::process_monitor::get_running_processes().iter().any(|p| p.pid != me && crate::process_monitor::runs_from(p, folder)) {
        out.push("server_running");
    }
    out
}

/// 그 폴더에 파일을 만들 수 있는가 (만든 시험 파일은 바로 지운다)
fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".saba-write-check-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        // 지난 시험 파일이 남았으면 그것도 치운다
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => std::fs::remove_file(&probe).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlinking_puts_back_the_files_as_they_were_before_the_import() {
        let tmp = tempfile::tempdir().unwrap();
        let server = tmp.path().join("기존 서버");
        let instance = tmp.path().join("instance");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::create_dir_all(&instance).unwrap();
        let props = server.join("server.properties");
        std::fs::write(&props, "enable-rcon=true\nmotd=mine\n").unwrap();
        let absent = server.join("eula.txt");

        assert_eq!(save(&instance, &[props.clone(), absent.clone(), props.clone()]).unwrap(), 1);
        assert!(saved(&instance));
        std::fs::write(&props, "enable-rcon=false\nmotd=mine\n").unwrap();
        std::fs::write(&absent, "eula=true\n").unwrap();

        assert_eq!(restore(&instance, |_| true), Restored { restored: 1, failed: vec![] });
        assert_eq!(std::fs::read_to_string(&props).unwrap(), "enable-rcon=true\nmotd=mine\n");
        // 가져오기 전에 없던 파일은 보관하지 않았다: 서버가 쓰는 파일일 수 있어 지우지 않는다
        assert!(absent.exists());
    }

    /// 인스턴스 폴더 안의 파일(옮겨 온 서버)은 인스턴스와 함께 지워지므로 보관하지 않고, 밖의 파일만 보관한다
    #[test]
    fn only_files_outside_the_instance_folder_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = tmp.path().join("instance");
        std::fs::create_dir_all(instance.join("server")).unwrap();
        let inside = instance.join("server").join("server.properties");
        std::fs::write(&inside, "motd=mine\n").unwrap();
        let data = tmp.path().join("Zomboid");
        std::fs::create_dir_all(&data).unwrap();
        let outside = data.join("pvp.ini");
        std::fs::write(&outside, "PVP=true\n").unwrap();

        assert_eq!(save(&instance, &[inside, outside.clone()]).unwrap(), 1);
        std::fs::write(&outside, "PVP=false\n").unwrap();
        assert_eq!(restore(&instance, |_| true), Restored { restored: 1, failed: vec![] });
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "PVP=true\n");
    }

    #[test]
    fn nothing_is_saved_when_no_file_existed() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(save(tmp.path(), &[tmp.path().join("missing.ini")]).unwrap(), 0);
        assert!(!saved(tmp.path()));
        assert_eq!(restore(tmp.path(), |_| true), Restored::default());
    }

    #[test]
    fn a_place_outside_the_server_now_is_not_written() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.ini");
        std::fs::write(&file, "old").unwrap();
        save(&tmp.path().join("i"), std::slice::from_ref(&file)).unwrap();
        std::fs::write(&file, "new").unwrap();

        let out = restore(&tmp.path().join("i"), |_| false);

        assert_eq!(out, Restored { restored: 0, failed: vec![file.clone()] });
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
    }

    #[test]
    fn moving_takes_the_whole_folder_into_the_instance_and_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("기존 서버");
        std::fs::create_dir_all(src.join("world")).unwrap();
        std::fs::write(src.join("server.properties"), "motd=mine\n").unwrap();
        let dest = tmp.path().join("instances").join("id").join("server");

        move_into(&src, &dest).unwrap();

        assert!(!src.exists());
        assert!(dest.join("world").is_dir());
        assert_eq!(std::fs::read_to_string(dest.join("server.properties")).unwrap(), "motd=mine\n");
        assert_eq!(move_into(&src, &tmp.path().join("other")), Err("source_missing"));
        std::fs::create_dir_all(&src).unwrap();
        assert_eq!(move_into(&src, &dest), Err("destination_exists"));
        assert!(src.is_dir(), "a refused move changes nothing");
    }

    /// 옮길 자리는 인스턴스 폴더 안의 하위 폴더만 (모듈의 install_subdir가 절대 경로·`..`이면 거절)
    #[test]
    fn a_move_target_stays_inside_the_instance_folder() {
        let instance = Path::new("/data/instances/id");
        assert_eq!(move_target(instance, "server"), Some(instance.join("server")));
        assert_eq!(move_target(instance, "game/server"), Some(instance.join("game/server")));
        for bad in ["", ".", "..", "../outside", "a/../../b", "/abs", "C:\\x", "C:x", "\\\\host\\share", "logs", "migration"] {
            assert_eq!(move_target(instance, bad), None, "{bad}");
        }
    }

    #[test]
    fn a_writable_folder_raises_no_notice_and_keeps_no_probe_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(notices(tmp.path()).is_empty());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_is_noticed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        // root는 권한과 관계없이 쓸 수 있다
        let root = std::fs::write(tmp.path().join("x"), "").is_ok();
        let got = notices(tmp.path());
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        if !root {
            assert_eq!(got, vec!["folder_not_writable"]);
        }
    }

    #[test]
    fn only_a_server_folder_outside_the_instance_folder_is_linked() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = tmp.path().join("instance");
        std::fs::create_dir_all(instance.join("server")).unwrap();
        let outside = tmp.path().join("server");
        std::fs::create_dir_all(&outside).unwrap();
        let outside = outside.to_string_lossy().into_owned();
        assert_eq!(linked_folder(Some(&outside), &instance), Some(outside.clone()));
        assert_eq!(linked_folder(Some(&instance.join("server").to_string_lossy()), &instance), None);
        assert_eq!(linked_folder(None, &instance), None);
        assert_eq!(linked_folder(Some(" "), &instance), None);
    }

    /// 데이터 폴더가 링크(정션·심볼릭 링크) 아래 있어도 인스턴스 폴더 안의 서버는 연결된 폴더가 아니다
    /// (연결로 잘못 보면 "서버 파일은 남는다"고 안내하고 인스턴스와 함께 지운다)
    #[cfg(unix)]
    #[test]
    fn a_server_folder_inside_an_instance_under_a_link_is_not_linked() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(real.join("instance").join("server")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let instance = link.join("instance");
        assert_eq!(linked_folder(Some(&instance.join("server").to_string_lossy()), &instance), None);
        let through_real = real.join("instance").join("server");
        assert_eq!(linked_folder(Some(&through_real.to_string_lossy()), &instance), None);
    }
}
