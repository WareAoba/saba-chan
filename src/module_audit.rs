//! 모듈 Python 코드 정적 검사 (M12)
//!
//! 검사 자체는 `modules/saba_audit.py`(AST)가 하고, 여기서는 그 결과를 module.toml
//! `[permissions]` 선언과 대조한다. 쓰는 곳: `saba-core module audit <폴더>`,
//! `GET /api/module/:name/audit`. 정적 검사는 악성 코드를 다 막지 못한다: 선언과 다른 곳을
//! 사용자에게 보여 주는 한 겹이다.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use crate::supervisor::module_loader::ModulePermissions;

/// 검사기 원본. 데몬이 공용 유틸리티와 함께 배포한다
pub const SCRIPT: &str = include_str!("../modules/saba_audit.py");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub permission: String,
    pub file: String,
    pub line: u32,
    pub detail: String,
}

/// 선언하지 않은 능력을 쓰는 곳. 모르는 권한(`unparsable` 등)도 여기에 든다
pub fn undeclared(permissions: &ModulePermissions, findings: &[Finding]) -> Vec<Finding> {
    findings
        .iter()
        .filter(|f| permissions.declares(&f.permission) != Some(true))
        .cloned()
        .collect()
}

/// `python`으로 `dir`의 .py 파일을 검사한다 (막히는 호출: 데몬 안에서는 `scan`)
pub fn scan_blocking(python: &Path, dir: &Path) -> Result<Vec<Finding>> {
    let manifest_path = dir.join("module.toml");
    let sdk_minimum = if manifest_path.is_file() {
        let source = std::fs::read_to_string(&manifest_path).with_context(|| format!("read {}", manifest_path.display()))?;
        let manifest: toml::Value = toml::from_str(source.trim_start_matches('\u{feff}'))
            .with_context(|| format!("parse {}", manifest_path.display()))?;
        Some(manifest.get("module").and_then(|value| value.get("dependencies"))
            .and_then(|value| value.get("saba-core")).and_then(toml::Value::as_str).unwrap_or("missing").to_string())
    } else {
        None
    };
    let metadata = serde_json::to_vec(&serde_json::json!({"sdk_source": include_str!("../modules/daemon_rcon.py"), "sdk_minimum": sdk_minimum}))?;
    // -I: 검사할 폴더의 파일이 import되지 않게 (현재 폴더·PYTHONPATH 무시)
    let mut child = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new(python))
        .arg("-I")
        .arg("-c")
        .arg(SCRIPT)
        .arg(dir)
        .arg("--sdk-stdin")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run {}", python.display()))?;
    child.stdin.take().context("audit stdin")?.write_all(&metadata).context("write SDK audit metadata")?;
    let out = child.wait_with_output().context("wait for module audit")?;
    #[derive(Deserialize)]
    struct Output {
        success: bool,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        findings: Vec<Finding>,
    }
    let parsed: Output = serde_json::from_slice(&out.stdout)
        .with_context(|| format!("audit output: {}", String::from_utf8_lossy(&out.stderr)))?;
    if !parsed.success {
        bail!("{}", parsed.message.unwrap_or_else(|| "audit failed".into()));
    }
    Ok(parsed.findings)
}

pub async fn scan(python: &Path, dir: &Path) -> Result<Vec<Finding>> {
    let (python, dir) = (python.to_path_buf(), dir.to_path_buf());
    tokio::task::spawn_blocking(move || scan_blocking(&python, &dir)).await?
}

/// 데몬 없이 도는 도구(`saba-core module audit`)가 쓸 Python: `SABA_PYTHON`, 없으면 PATH의 것
pub fn tool_python() -> std::path::PathBuf {
    std::env::var_os("SABA_PYTHON")
        .map(Into::into)
        .unwrap_or_else(|| if cfg!(windows) { "python".into() } else { "python3".into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제로 도는 Python이 있는지. Windows의 `python` 앱 실행 별칭(스토어 스텁)은 실행은 되지만 실패한다
    fn python_runs(python: &Path) -> bool {
        std::process::Command::new(python).arg("--version").output().is_ok_and(|o| o.status.success())
    }

    // L25: PATH의 첫 python이 스토어 스텁이면 검사 시험이 건너뛰지 않고 실패했다
    #[cfg(unix)]
    #[test]
    fn a_python_that_only_prints_an_install_hint_is_not_used() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("python");
        std::fs::write(&stub, "#!/bin/sh\necho 'Python was not found' >&2\nexit 9009\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!python_runs(&stub));
        assert!(!python_runs(&dir.path().join("missing")));
    }

    fn finding(permission: &str) -> Finding {
        Finding { permission: permission.into(), file: "lifecycle.py".into(), line: 1, detail: String::new() }
    }

    #[test]
    fn only_undeclared_or_unknown_capabilities_are_reported() {
        let perms = ModulePermissions { network: true, ..Default::default() };
        let found = vec![finding("network"), finding("spawn_process"), finding("unparsable")];
        let left: Vec<String> = undeclared(&perms, &found).into_iter().map(|f| f.permission).collect();
        assert_eq!(left, vec!["spawn_process", "unparsable"]);
    }

    #[test]
    fn sdk_imports_are_checked_against_the_embedded_table() {
        let python = tool_python();
        if !python_runs(&python) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lifecycle.py"), "from daemon_rcon import safe_join as join\nfrom i18n import I18n\n").unwrap();
        let found = scan_blocking(&python, dir.path()).unwrap();
        assert_eq!(found.len(), 2, "{:?}", found);
        assert!(found.iter().all(|finding| finding.permission == "sdk_version"));
        assert_eq!(undeclared(&ModulePermissions::default(), &found), found);
        std::fs::write(dir.path().join("module.toml"), "[module.dependencies]\nsaba-core = \">=0.1.9\"\n").unwrap();
        let found = scan_blocking(&python, dir.path()).unwrap();
        assert_eq!(found.len(), 1, "{:?}", found);
        assert!(found[0].detail.contains("daemon_rcon.safe_join"));
        assert!(found[0].detail.contains(">=0.1.9"));
        assert!(found[0].detail.contains("0.1.10"));
        std::fs::write(dir.path().join("module.toml"), "[module.dependencies]\nsaba-core = \">=0.1.10\"\n").unwrap();
        assert!(scan_blocking(&python, dir.path()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_scanner_finds_capabilities_in_module_code() {
        let python = tool_python();
        if !python_runs(&python) {
            return; // Python 없는 환경
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lifecycle.py"),
            "import subprocess\nimport json\nfrom urllib.request import urlopen\nimport os\n\
             def f(c):\n    os.system('x')\n    exec(c)\n    __import__('json')\n    return dict(os.environ)\n",
        )
        .unwrap();
        // 실행 코드가 import하지 않는 테스트는 세지 않는다
        std::fs::write(dir.path().join("test_lifecycle.py"), "import socket\n").unwrap();
        let found = scan(&python, dir.path()).await.unwrap();
        let mut perms: Vec<&str> = found.iter().map(|f| f.permission.as_str()).collect();
        perms.sort_unstable();
        assert_eq!(perms, vec!["dynamic_code", "env_secrets", "network", "spawn_process", "spawn_process"], "{:?}", found);
    }

    /// 소스 없는 바이트코드와 네이티브 라이브러리도 보고한다 (cloud review 4)
    #[tokio::test]
    async fn sourceless_bytecode_and_native_libraries_are_reported() {
        let python = tool_python();
        if !python_runs(&python) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("lifecycle.py"), "import helper\nimport fast\n").unwrap();
        std::fs::write(p.join("helper.pyc"), b"\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();
        std::fs::write(p.join("fast.pyd"), b"MZ").unwrap();
        std::fs::create_dir(p.join("__pycache__")).unwrap();
        // 소스 확인을 거치는 보통 캐시는 괜찮고, unchecked-hash 캐시는 보고한다
        std::fs::write(p.join("__pycache__/lifecycle.cpython-312.pyc"), b"\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();
        std::fs::write(p.join("__pycache__/other.cpython-312.pyc"), b"\x00\x00\x00\x00\x01\x00\x00\x00").unwrap();
        let found = scan(&python, p).await.unwrap();
        let mut got: Vec<String> = found.iter().map(|f| format!("{} {}", f.permission, f.file)).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            vec!["native_code fast.pyd", "opaque_code __pycache__/other.cpython-312.pyc", "opaque_code helper.pyc"],
            "{:?}",
            found
        );
        // 표준 모듈 이름을 가리는 파일·패키지 (cloud review 5)
        std::fs::write(p.join("json.py"), "x = 1\n").unwrap();
        std::fs::create_dir(p.join("socket")).unwrap();
        std::fs::write(p.join("socket/__init__.py"), "").unwrap();
        let found = scan(&python, p).await.unwrap();
        let shadows: Vec<&str> = found.iter().filter(|f| f.permission == "stdlib_shadow").map(|f| f.file.as_str()).collect();
        assert_eq!(shadows, vec!["json.py", "socket"], "{:?}", found);
        let found: Vec<Finding> = found.into_iter().filter(|f| f.permission != "stdlib_shadow").collect();
        assert_eq!(found.len(), 3, "{:?}", found);
        // opaque_code는 선언할 수 없는 권한이라 늘 남는다
        let all = ModulePermissions { native_code: true, ..Default::default() };
        assert_eq!(undeclared(&all, &found).len(), 2);
    }

    /// 공용 도우미 이름이나 테스트 파일 뒤에 숨긴 코드도 잡는다 (cloud review 3)
    #[tokio::test]
    async fn code_hidden_behind_helper_or_test_names_is_still_found() {
        let python = tool_python();
        if !python_runs(&python) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        write("lifecycle.py", "import test_helper\nimport i18n\nfrom tests import util\nfrom os import system\n");
        write("test_helper.py", "import test_deeper\n");
        write("test_deeper.py", "import socket\n");
        write("i18n.py", "import ctypes\n");
        write("tests/util.py", "import pickle\n");
        let found = scan(&python, dir.path()).await.unwrap();
        let mut got: Vec<String> = found.iter().map(|f| format!("{} {}", f.permission, f.file)).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            vec!["dynamic_code tests/util.py", "native_code i18n.py", "network test_deeper.py", "spawn_process lifecycle.py"],
            "{:?}",
            found
        );
    }
}
