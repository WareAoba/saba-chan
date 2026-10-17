//! 데몬 자체 부트스트랩 (인스톨러/업데이터 미사용 시에도 동작 보장)
//!
//! 인스톨러나 업데이터가 수행하던 "환경 의존 설정" 중 일부는, 빌드 산출물을
//! 단순 복사한 환경(개발 빌드 → `C:\Program Files\Saba-chan\` 등)에서도
//! 데몬 시작 시점에 자동으로 보강되어야 한다.
//!
//! 현재 처리하는 항목:
//!  - 공유 Python 유틸리티(`i18n.py`, `daemon_rcon.py`)를
//!    `%APPDATA%/saba-chan/modules/` 에 보장. 컴파일 타임에 임베드된
//!    원본을 SoT 로 사용하여, 설치/업데이트 경로와 무관하게 항상 최신본을 둔다.
//!
//! 모든 경로는 `saba_chan_updater_lib::constants` 의 SoT 함수로 해석한다.
//! `SABA_DATA_DIR` / `SABA_MODULES_PATH` 등 환경 변수 오버라이드와 호환된다.

use std::path::Path;

use saba_chan_updater_lib::constants;

/// 컴파일 타임에 임베드된 공유 Python 유틸리티.
///
/// 새 항목을 추가할 때는 반드시 두 위치(saba-core 임베드, 인스톨러
/// `install_shared_modules`)를 동시에 갱신할 것. 인스톨러 측은 zip 산출물에서
/// 복사하지만, 데몬 측은 임베드 원본으로 항상 최신을 보장한다.
const EMBEDDED_SHARED_MODULES: &[(&str, &str)] = &[
    ("i18n.py", include_str!("../modules/i18n.py")),
    ("daemon_rcon.py", include_str!("../modules/daemon_rcon.py")),
];

/// 데몬 시작 시 호출. 실패해도 데몬은 계속 동작하지만 경고 로그를 남긴다.
pub fn run() {
    if let Err(e) = ensure_shared_python_modules(&constants::resolve_modules_dir()) {
        tracing::warn!("[Bootstrap] 공유 Python 모듈 보강 실패: {}", e);
    }
}

/// `modules_dir` 안에 임베드된 공유 모듈을 보장한다.
///
/// - 파일이 없거나 내용이 다르면 덮어쓴다.
/// - 내용이 같으면 아무 것도 하지 않는다(ctime/mtime 보존).
/// - 디렉터리 자체가 없으면 생성한다.
pub fn ensure_shared_python_modules(modules_dir: &Path) -> std::io::Result<()> {
    if !modules_dir.exists() {
        std::fs::create_dir_all(modules_dir)?;
    }

    for (name, embedded) in EMBEDDED_SHARED_MODULES {
        let dst = modules_dir.join(name);
        let needs_write = match std::fs::read_to_string(&dst) {
            Ok(existing) => existing != *embedded,
            Err(_) => true, // 파일 없음 또는 읽기 실패 → 새로 쓴다
        };
        if needs_write {
            std::fs::write(&dst, embedded.as_bytes())?;
            tracing::info!(
                "[Bootstrap] 공유 모듈 갱신: {} ({} bytes)",
                dst.display(),
                embedded.len()
            );
        } else {
            tracing::debug!("[Bootstrap] 공유 모듈 최신 상태: {}", dst.display());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_modules_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_shared_python_modules(tmp.path()).unwrap();
        for (name, expected) in EMBEDDED_SHARED_MODULES {
            let p = tmp.path().join(name);
            assert!(p.exists(), "{} 파일이 생성되어야 함", name);
            let got = std::fs::read_to_string(&p).unwrap();
            assert_eq!(&got, expected, "{} 내용이 임베드본과 동일해야 함", name);
        }
    }

    #[test]
    fn overwrites_when_outdated() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("i18n.py");
        std::fs::write(&target, b"# stale content\n").unwrap();

        ensure_shared_python_modules(tmp.path()).unwrap();

        let got = std::fs::read_to_string(&target).unwrap();
        assert_eq!(got, EMBEDDED_SHARED_MODULES[0].1);
    }

    #[test]
    fn preserves_when_up_to_date() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_shared_python_modules(tmp.path()).unwrap();
        let target = tmp.path().join("i18n.py");
        let mtime1 = std::fs::metadata(&target).unwrap().modified().unwrap();

        // 같은 내용으로 재호출 → 덮어쓰지 않아야 함 (mtime 변화 없음)
        std::thread::sleep(std::time::Duration::from_millis(50));
        ensure_shared_python_modules(tmp.path()).unwrap();
        let mtime2 = std::fs::metadata(&target).unwrap().modified().unwrap();

        assert_eq!(mtime1, mtime2, "내용이 같으면 덮어쓰지 않아야 함");
    }

    #[test]
    fn creates_directory_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("nested").join("modules");
        assert!(!nested.exists());

        ensure_shared_python_modules(&nested).unwrap();

        assert!(nested.exists());
        assert!(nested.join("i18n.py").exists());
        assert!(nested.join("daemon_rcon.py").exists());
    }
}
