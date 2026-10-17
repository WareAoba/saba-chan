//! ═══════════════════════════════════════════════════════════════════
//! P0-4: 실제 모듈(lifecycle.py) contract 검증
//! ═══════════════════════════════════════════════════════════════════
//!
//! 분류: **E2E** — 실제 saba-chan-modules 저장소의 lifecycle.py를 시스템 Python으로
//! 별도 프로세스로 spawn하여 데몬 호출 규약(stdin JSON in / stdout JSON out)을 검증.
//!
//! ## 검증 대상
//! - `minecraft/lifecycle.py validate`
//! - `palworld/lifecycle.py validate`
//! - `zomboid/lifecycle.py validate`
//!
//! ## 데몬 contract (saba-core::supervisor::validate_instance 기반)
//! - 입력: stdin에 JSON config (working_dir / port / extension_data 등)
//! - 출력: stdout에 단일 JSON 객체. `issues` 키가 list 형태로 존재해야 한다.
//! - 각 issue 항목에는 `code`, `severity`, `message` 필드 권장.
//! - 종료 코드는 0 (실패하더라도 issues로 표현, exit code 비0은 fatal).
//!
//! ## 테스트 전략
//! - 빈/최소 config로 호출 → 모듈은 의존성 누락(Java/SteamCMD/Java64bit 등) 이슈를
//!   issues 배열에 담아 반환해야 함.
//! - 표준 출력이 **단일 JSON**이어야 한다 (여러 줄 print → 데몬 파싱 실패).
//! - exit code = 0.
//!
//! 모듈 저장소가 없거나 Python 실행이 불가하면 SKIP.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

fn find_python() -> Option<String> {
    let candidates = if cfg!(windows) {
        vec!["python", "python3"]
    } else {
        vec!["python3", "python"]
    };
    for c in candidates {
        if let Ok(out) = std::process::Command::new(c).arg("--version").output() {
            if out.status.success() {
                return Some(c.to_string());
            }
        }
    }
    None
}

fn modules_repo_root() -> Option<PathBuf> {
    // saba-chan crate 루트는 cargo가 CARGO_MANIFEST_DIR로 제공
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir.parent()?.join("saba-chan-modules");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

async fn invoke_lifecycle(
    python: &str,
    lifecycle_py: &PathBuf,
    verb: &str,
    config: Value,
) -> Result<(i32, String, String), Box<dyn std::error::Error>> {
    // 데몬과 동일하게 PYTHONPATH에 saba-chan/modules (공용 i18n.py 등) 주입
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let common_modules_dir = manifest_dir.join("modules");
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut pypath = common_modules_dir.to_string_lossy().to_string();
    if let Ok(existing) = std::env::var("PYTHONPATH") {
        pypath = format!("{pypath}{sep}{existing}");
    }

    let mut child = tokio::process::Command::new(python)
        .arg(lifecycle_py)
        .arg(verb)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONPATH", &pypath)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        let stdin = child.stdin.as_mut().ok_or("no stdin")?;
        stdin.write_all(&serde_json::to_vec(&config)?).await?;
        stdin.shutdown().await?;
    }

    let output = child.wait_with_output().await?;
    let exit = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((exit, stdout, stderr))
}

fn assert_validate_contract(module_name: &str, exit: i32, stdout: &str, stderr: &str) {
    assert_eq!(
        exit, 0,
        "[{module_name}] validate exit code must be 0 (issues로 표현). stdout={stdout}, stderr={stderr}"
    );

    // stdout은 단일 JSON 객체
    let parsed: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("[{module_name}] validate stdout must be JSON: {e}; raw=<<{stdout}>>")
    });

    let obj = parsed.as_object().unwrap_or_else(|| {
        panic!("[{module_name}] validate response must be JSON object, got: {parsed}")
    });

    let issues = obj.get("issues").unwrap_or_else(|| {
        panic!("[{module_name}] validate response must have 'issues' key; got: {parsed}")
    });

    let issues_arr = issues.as_array().unwrap_or_else(|| {
        panic!("[{module_name}] 'issues' must be array, got: {issues}")
    });

    // 각 issue 항목 형식 검증
    for (i, item) in issues_arr.iter().enumerate() {
        let item_obj = item.as_object().unwrap_or_else(|| {
            panic!("[{module_name}] issues[{i}] must be object: {item}")
        });
        for required_key in ["code", "severity", "message"] {
            assert!(
                item_obj.contains_key(required_key),
                "[{module_name}] issues[{i}] missing key '{required_key}': {item}"
            );
        }
    }
}

#[tokio::test]
async fn minecraft_lifecycle_validate_contract() {
    let Some(python) = find_python() else {
        eprintln!("SKIP: Python not found");
        return;
    };
    let Some(modules_root) = modules_repo_root() else {
        eprintln!("SKIP: ../saba-chan-modules not found");
        return;
    };

    let lifecycle = modules_root.join("minecraft").join("lifecycle.py");
    if !lifecycle.exists() {
        eprintln!("SKIP: minecraft/lifecycle.py not found at {}", lifecycle.display());
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let config = json!({
        "working_dir": tmp.path().to_string_lossy(),
        "port": 25565,
        "extension_data": {},
    });

    let (exit, stdout, stderr) = invoke_lifecycle(&python, &lifecycle, "validate", config)
        .await
        .expect("invoke minecraft validate");

    assert_validate_contract("minecraft", exit, &stdout, &stderr);
}

/// palworld는 `from extensions.ue4_ini import ...`로 saba-chan-extensions에 cross-repo 의존.
/// 데몬은 `%APPDATA%/saba-chan` 디렉토리에 `extensions/`가 배포된 형태로 PYTHONPATH를 구성하지만,
/// 테스트 환경(개발 트리)에는 동일 구조가 없다. AppData 의존을 강제하면 CI에서 fragile하므로
/// 본 테스트는 ignored — 발견된 cross-repo 결합도를 P1 이슈로 문서화.
#[tokio::test]
#[ignore = "P1: palworld lifecycle.py가 extensions.ue4_ini에 cross-repo 의존 — AppData 또는 추가 PYTHONPATH 셋업 필요"]
async fn palworld_lifecycle_validate_contract() {
    let Some(python) = find_python() else {
        eprintln!("SKIP: Python not found");
        return;
    };
    let Some(modules_root) = modules_repo_root() else {
        eprintln!("SKIP: ../saba-chan-modules not found");
        return;
    };

    let lifecycle = modules_root.join("palworld").join("lifecycle.py");
    if !lifecycle.exists() {
        eprintln!("SKIP: palworld/lifecycle.py not found");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let config = json!({
        "working_dir": tmp.path().to_string_lossy(),
        "port": 8211,
        "extension_data": {},
    });

    let (exit, stdout, stderr) = invoke_lifecycle(&python, &lifecycle, "validate", config)
        .await
        .expect("invoke palworld validate");

    assert_validate_contract("palworld", exit, &stdout, &stderr);
}

#[tokio::test]
async fn zomboid_lifecycle_validate_contract() {
    let Some(python) = find_python() else {
        eprintln!("SKIP: Python not found");
        return;
    };
    let Some(modules_root) = modules_repo_root() else {
        eprintln!("SKIP: ../saba-chan-modules not found");
        return;
    };

    let lifecycle = modules_root.join("zomboid").join("lifecycle.py");
    if !lifecycle.exists() {
        eprintln!("SKIP: zomboid/lifecycle.py not found");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let config = json!({
        "working_dir": tmp.path().to_string_lossy(),
        "port": 16261,
        "extension_data": {},
    });

    let (exit, stdout, stderr) = invoke_lifecycle(&python, &lifecycle, "validate", config)
        .await
        .expect("invoke zomboid validate");

    assert_validate_contract("zomboid", exit, &stdout, &stderr);
}

#[tokio::test]
async fn unknown_verb_returns_error_object_not_panic() {
    // 알 수 없는 verb를 호출해도 데몬 측 파싱이 실패하지 않도록 JSON 응답이어야 함.
    let Some(python) = find_python() else {
        eprintln!("SKIP: Python not found");
        return;
    };
    let Some(modules_root) = modules_repo_root() else {
        eprintln!("SKIP: ../saba-chan-modules not found");
        return;
    };

    let lifecycle = modules_root.join("minecraft").join("lifecycle.py");
    if !lifecycle.exists() {
        eprintln!("SKIP: minecraft/lifecycle.py not found");
        return;
    }

    let (exit, stdout, _stderr) = invoke_lifecycle(
        &python,
        &lifecycle,
        "no_such_verb_xyz",
        json!({}),
    )
    .await
    .expect("invoke unknown verb");

    // exit 0이든 비0이든, stdout은 JSON이어야 한다 (데몬이 안전하게 파싱 가능)
    let parsed: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("Unknown verb stdout must still be JSON: {e}; raw=<<{stdout}>>; exit={exit}")
    });
    assert!(parsed.is_object(), "Unknown verb response must be object: {parsed}");
    let obj = parsed.as_object().unwrap();
    // 일부 모듈은 success: false / message만, 일부는 추가 필드. 최소: success 또는 message 키 보유.
    assert!(
        obj.contains_key("success") || obj.contains_key("message"),
        "Unknown verb response must have 'success' or 'message': {parsed}"
    );
}
