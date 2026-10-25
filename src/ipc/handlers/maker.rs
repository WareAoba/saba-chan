//! 모듈 메이커 안내 API
//!
//! - `GET /api/maker` — 에이전트가 새 모듈을 만들 때 쓸 이 PC의 경로와 데몬 계약. 읽기만 한다
//!   (메이커 스킬과 도구는 모듈 저장소 `maker/`에 있고, 앱은 시작하는 곳과 경로만 알려 준다)

use axum::{response::IntoResponse, Json};
use saba_chan_updater_lib::component_dirs::{self, Kind};
use saba_chan_updater_lib::constants;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 모듈 메이커가 있는 저장소와 사용자 안내
pub const MAKER_REPO: &str = "https://github.com/WareAoba/saba-chan-modules";
pub const MAKER_GUIDE: &str = "https://saba-chan.online/guide/module-maker";

struct Paths {
    saba_core: Option<PathBuf>,
    runner: Option<PathBuf>,
    python: Option<PathBuf>,
    shared_libs: PathBuf,
    modules_dir: PathBuf,
}

fn maker_info(paths: Paths) -> Value {
    json!({
        "ok": true,
        "saba_core": {
            "version": env!("CARGO_PKG_VERSION"),
            "path": paths.saba_core,
            "runner": paths.runner,
        },
        "python": {
            "path": paths.python.filter(|p| p.is_file()),
            "version": saba_chan_updater_lib::python_pin::PYTHON_VERSION,
        },
        "shared_libs": paths.shared_libs,
        "modules_dir": paths.modules_dir,
        "schema_versions": crate::supervisor::module_loader::supported_schema_versions(),
        "repo": MAKER_REPO,
        "guide": MAKER_GUIDE,
    })
}

fn shared_libs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("python-shared")
}

/// GET /api/maker
pub async fn get_maker() -> impl IntoResponse {
    let data_dir = constants::resolve_data_dir();
    Json(maker_info(Paths {
        saba_core: std::env::current_exe().ok(),
        runner: crate::supervisor::managed_process::runner_binary(),
        python: crate::python_env::venv_python(),
        shared_libs: shared_libs_dir(&data_dir),
        modules_dir: component_dirs::resolve(Kind::Modules).0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maker_info_reports_paths_and_the_contract() {
        let dir = std::env::temp_dir().join("saba-maker-info-test");
        let info = maker_info(Paths {
            saba_core: Some(dir.join("saba-core.exe")),
            runner: None,
            python: Some(dir.join("missing-python.exe")),
            shared_libs: shared_libs_dir(&dir),
            modules_dir: dir.join("modules"),
        });

        assert_eq!(info["saba_core"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(info["saba_core"]["runner"], Value::Null);
        // 아직 준비되지 않은 Python 경로는 알려 주지 않는다
        assert_eq!(info["python"]["path"], Value::Null);
        assert_eq!(info["shared_libs"], json!(dir.join("python-shared")));
        assert!(info["schema_versions"].as_array().is_some_and(|v| !v.is_empty()));
        assert_eq!(info["repo"], MAKER_REPO);
    }
}
