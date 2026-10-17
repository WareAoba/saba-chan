//! 모듈 Python 코드 정적 검사 (M12)
//!
//! 검사 자체는 `modules/saba_audit.py`(AST)가 하고, 여기서는 그 결과를 module.toml
//! `[permissions]` 선언과 대조한다. 쓰는 곳: `saba-core module audit <폴더>`,
//! `GET /api/module/:name/audit`. 정적 검사는 악성 코드를 다 막지 못한다: 선언과 다른 곳을
//! 사용자에게 보여 주는 한 겹이다.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    // -I: 검사할 폴더의 파일이 import되지 않게 (현재 폴더·PYTHONPATH 무시)
    let out = std::process::Command::new(python)
        .arg("-I")
        .arg("-c")
        .arg(SCRIPT)
        .arg(dir)
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .with_context(|| format!("run {}", python.display()))?;
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

    #[tokio::test]
    async fn the_scanner_finds_capabilities_in_module_code() {
        let python = tool_python();
        if std::process::Command::new(&python).arg("--version").output().is_err() {
            return; // Python 없는 환경
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lifecycle.py"),
            "import subprocess\nimport json\nfrom urllib.request import urlopen\nimport os\n\
             def f(c):\n    os.system('x')\n    exec(c)\n    __import__('json')\n    return dict(os.environ)\n",
        )
        .unwrap();
        // 데몬이 함께 배포하는 도우미는 모듈 코드로 세지 않는다
        std::fs::write(dir.path().join("daemon_rcon.py"), "import socket\n").unwrap();
        let found = scan(&python, dir.path()).await.unwrap();
        let mut perms: Vec<&str> = found.iter().map(|f| f.permission.as_str()).collect();
        perms.sort_unstable();
        assert_eq!(perms, vec!["dynamic_code", "env_secrets", "network", "spawn_process", "spawn_process"], "{:?}", found);
    }
}
