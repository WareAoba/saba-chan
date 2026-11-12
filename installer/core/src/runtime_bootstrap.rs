//! 런타임 부트스트랩 — 포터블 Python 자동 설치
//!
//! 인스톨러가 설치 과정에서 **엔드유저가 Python을 별도로 설치할 필요 없이**
//! 포터블 런타임을 받고 venv를 만든다. 자리·받기·검사·venv 만들기는 데몬과 같은
//! `saba_chan_updater_lib::python_runtime`을 쓴다. 설치기는 시스템 Python을 쓰지 않고 늘 포터블 Python으로 만든다

use saba_chan_updater_lib::python_runtime::{self as runtime, Plain};
use std::path::{Path, PathBuf};

/// 포터블 Python과 venv를 준비한다. 둘 중 검사(`python_runtime::verify`, 최소 판 포함)에 실패한 것은 지우고 다시 만든다
///
/// `data_dir`: 런타임 폴더 (`resolve_runtime_data_dir`). 반환: venv 내 Python 실행 파일 경로
pub async fn setup_python_with_repair(data_dir: &Path, on_download: &(dyn Fn(u64, Option<u64>) + Sync)) -> Result<PathBuf, String> {
    let portable = runtime::portable_python(data_dir);
    if runtime::verify(&Plain, &portable).await {
        tracing::info!("포터블 Python 이미 존재: {}", portable.display());
    } else {
        runtime::install_portable(&Plain, data_dir, on_download).await.map_err(|e| format!("Python 준비 실패: {e:#}"))?;
    }
    let venv = runtime::venv_python(data_dir);
    if runtime::verify(&Plain, &venv).await {
        tracing::info!("Python venv 이미 유효: {}", venv.display());
        return Ok(venv);
    }
    let venv = runtime::create_venv(&Plain, &portable, data_dir).await.map_err(|e| format!("{e:#}"))?;
    tracing::info!("Python venv 준비 완료: {}", venv.display());
    Ok(venv)
}

/// 런타임 폴더: 데몬과 같은 규칙(`python_runtime::runtime_dir`)에 설치 폴더를 넘긴다 (데몬은 설치 폴더에서 돈다).
/// 정하지 못하면 설치 폴더
pub fn resolve_runtime_data_dir(install_dir: &Path) -> PathBuf {
    runtime::runtime_dir(Some(install_dir)).unwrap_or_else(|_| install_dir.to_path_buf())
}

/// 데몬이 다시 만들 옛 venv(최소 판 미만)를 설치기도 쓰지 않는다: 같은 검사로 보고 포터블 Python으로 다시 만든다
#[cfg(all(test, unix))]
#[tokio::test]
async fn the_installer_rebuilds_a_venv_the_daemon_would_reject() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let script = |path: PathBuf, body: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let portable = tmp.path().join("python-standalone/python/bin/python3");
    script(portable, "[ \"$1\" = -m ] || { echo 3.12.8; exit 0; }\nmkdir -p \"$3/bin\"\nprintf '#!/bin/sh\\necho 3.12.8\\n' > \"$3/bin/python\"\nchmod +x \"$3/bin/python\"");
    let venv = tmp.path().join("python-env/bin/python");
    script(venv.clone(), "echo 3.10.12");

    let python = setup_python_with_repair(tmp.path(), &|_, _| {}).await.unwrap();

    assert_eq!(python, venv);
    let out = std::process::Command::new(&python).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3.12.8", "the old venv was rebuilt");
}
