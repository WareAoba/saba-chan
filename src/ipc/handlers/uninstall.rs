//! 이 PC에서 사바쨩을 지운다 (짝 맺은 PC가 원격으로 부른다: 앱의 PC 카드 "그 PC에서 사바쨩도 지우기")
//!
//!   POST /api/system/uninstall {keepSettings, keepInstances} → 202 {ok:true}
//!
//! 데몬이 CLI의 `uninstall --yes`를 따로 띄우고 답한다. 제거가 데몬을 끄므로, 리눅스에서 systemd 사용자 서비스로 돈 데몬은
//! 서비스 밖(`systemd-run --user`)에서 띄워 서비스가 멈출 때 함께 끝나지 않게 한다.
//! Windows는 프로그램 폴더를 지우려면 관리자 권한이 필요해 받지 않는다 (`unsupported`)

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UninstallBody {
    #[serde(default = "yes")]
    keep_settings: bool,
    #[serde(default = "yes")]
    keep_instances: bool,
}

fn yes() -> bool {
    true
}

/// 답이 부른 쪽에 닿은 뒤 지우기 시작하도록 기다리는 시간
const START_DELAY_SECS: u32 = 3;

/// 띄울 명령 (프로그램, 인자). systemd 사용자 세션이 있으면 그 밖에서 돌린다
pub fn uninstall_command(cli: &std::path::Path, keep_settings: bool, keep_instances: bool, systemd_run: Option<&std::path::Path>) -> (String, Vec<String>) {
    let mut cli_args = vec![cli.to_string_lossy().into_owned(), "uninstall".into(), "--yes".into()];
    if !keep_settings {
        cli_args.push("--remove-settings".into());
    }
    if !keep_instances {
        cli_args.push("--remove-instances".into());
    }
    // 경로는 인자로 넘기고 셸 글자에 넣지 않는다 (`"$@"`)
    let mut sh = vec!["-c".into(), format!("sleep {START_DELAY_SECS}; exec \"$@\""), "saba-uninstall".into()];
    sh.extend(cli_args);
    match systemd_run {
        Some(run) => {
            let mut args = vec!["--user".into(), "--collect".into(), "--quiet".into(), "/bin/sh".into()];
            args.extend(sh);
            (run.to_string_lossy().into_owned(), args)
        }
        None => ("/bin/sh".into(), sh),
    }
}

fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

pub async fn uninstall(body: Option<Json<UninstallBody>>) -> axum::response::Response {
    let body = body.map(|Json(b)| b).unwrap_or(UninstallBody { keep_settings: true, keep_instances: true });
    if cfg!(windows) {
        return super::error(StatusCode::NOT_IMPLEMENTED, "unsupported", "Remote uninstall is not available on Windows");
    }
    let cli = std::env::current_exe().ok().and_then(|me| me.parent().map(|d| d.join("saba-chan-cli"))).filter(|p| p.is_file());
    let Some(cli) = cli else {
        return super::error(StatusCode::CONFLICT, "cli_missing", "saba-chan-cli is not next to the daemon");
    };
    let systemd_run = cfg!(target_os = "linux").then(|| find_in_path("systemd-run")).flatten().filter(|_| std::env::var_os("XDG_RUNTIME_DIR").is_some());
    let (program, args) = uninstall_command(&cli, body.keep_settings, body.keep_instances, systemd_run.as_deref());
    let spawned = std::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => {
            tracing::warn!("Uninstall requested by a paired PC (keep settings: {}, keep servers: {})", body.keep_settings, body.keep_instances);
            (StatusCode::ACCEPTED, Json(json!({ "success": true }))).into_response()
        }
        Err(e) => super::error(StatusCode::INTERNAL_SERVER_ERROR, "spawn_failed", e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uninstall_runs_outside_the_service_and_keeps_what_was_asked() {
        let cli = std::path::Path::new("/home/a b/.local/share/saba-chan/saba-chan-cli");
        let (program, args) = uninstall_command(cli, true, false, Some(std::path::Path::new("/usr/bin/systemd-run")));
        assert_eq!(program, "/usr/bin/systemd-run");
        assert_eq!(&args[..4], ["--user", "--collect", "--quiet", "/bin/sh"]);
        assert_eq!(args[5], "sleep 3; exec \"$@\"");
        assert_eq!(&args[7..], ["/home/a b/.local/share/saba-chan/saba-chan-cli", "uninstall", "--yes", "--remove-instances"]);

        let (program, args) = uninstall_command(cli, false, true, None);
        assert_eq!(program, "/bin/sh");
        assert_eq!(&args[3..], ["/home/a b/.local/share/saba-chan/saba-chan-cli", "uninstall", "--yes", "--remove-settings"]);
    }
}
