//! `mcp` — 이 PC의 AI 에이전트(Claude Code·Codex)가 CLI 전체를 쓰는 로컬 MCP 서버 (stdin/stdout)
//!
//! 원격 MCP(릴레이, 액션 카탈로그만)와 달리 이 PC에서 CLI를 실행할 수 있는 에이전트는 모든 명령을 쓴다.
//! 명령은 이 CLI를 자식으로 다시 실행한다(`--json`, 입력 없음): 출력·종료 코드·확인 규칙이 사람이 쓰는 CLI와 같다.
//!
//!   saba_help  명령 트리·인자·종료 코드 (`help --json`)
//!   saba_run   명령 하나 실행. 확인이 필요하면 실행하지 않고 묻던 말을 돌려준다 → 사람에게 묻고 `confirm: true`로 다시
//!
//! 확인은 사람이 한다: `--yes`는 인자로 받지 않고 `confirm`으로만 붙인다. 약관 동의는 `--yes`로도 대신하지 않는다(CLI 규칙).

use crate::error::{failed, Result};
use clap::Parser;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};
use std::time::Duration;

/// 자식 CLI에 주는 표시: 데몬 작업 기록의 출처가 `agent`가 된다 (`client.rs`)
pub const AGENT_ENV: &str = "SABA_AGENT";

/// 명령 하나의 기본·최대 대기 (설치·백업·서버 업데이트는 오래 걸린다)
const DEFAULT_TIMEOUT: u64 = 300;
const MAX_TIMEOUT: u64 = 3600;

/// 에이전트에게 돌려줄 출력의 최대 길이 (글자). 넘으면 앞부분만 싣고 잘렸다고 적는다
const MAX_OUTPUT: usize = 200_000;

/// 에이전트 안에서 쓸 수 없는 명령: 이 서버 자신, 사람이 키보드로 쓰는 화면
const REFUSED: &[&str] = &["mcp", "shell", "tui"];

/// 값을 받는 전역 옵션 (명령 이름을 찾을 때 그 값을 건너뛴다)
const VALUE_FLAGS: &[&str] = &["--lang", "--port", "--daemon"];

const INSTRUCTIONS: &str = "Saba-chan runs game servers on this PC. These tools run the saba-chan command line with \
full access, as the PC owner.\n\
- Call saba_help first (no arguments for the command tree, or a path like [\"server\", \"add\"]) to find commands, \
arguments and exit codes.\n\
- saba_run takes the arguments after the program name, e.g. [\"server\", \"list\"] or [\"start\", \"survival\"]. \
Output is JSON.\n\
- Never pass --yes. When a command needs the user's OK the result says confirm_required with the question: ask the \
user in plain words, and only if they agree call saba_run again with the same args and confirm: true.\n\
- Exit code 3 means the Saba-chan daemon is not running: start it with saba_run [\"daemon\", \"start\"].\n\
- Game agreements (EULA) are accepted only by the user in a terminal or the app; do not try to accept them.";

#[derive(Clone, Default)]
struct Local {
    /// 띄울 때 고른 대상 ([`base_args`])
    base: Vec<String>,
}

pub async fn run() -> Result<()> {
    let server = Local {
        base: base_args(std::env::args_os()),
    }
    .serve(rmcp::transport::stdio())
    .await
    .map_err(|e| failed(e.to_string()))?;
    server.waiting().await.map_err(|e| failed(e.to_string()))?;
    Ok(())
}

fn schema(v: Value) -> std::sync::Arc<Map<String, Value>> {
    std::sync::Arc::new(v.as_object().cloned().unwrap_or_default())
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "saba_help",
            "Saba-chan command tree with arguments and exit codes (`help --json`). Pass a command path to see one command.",
            schema(json!({
                "type": "object",
                "properties": { "command": { "type": "array", "items": { "type": "string" }, "description": "Command path, e.g. [\"server\", \"add\"]. Empty for every command" } }
            })),
        )
        .with_title("Saba-chan commands")
        .with_annotations(ToolAnnotations::new().read_only(true).destructive(false).open_world(false)),
        Tool::new(
            "saba_run",
            "Run one saba-chan command and get its JSON output and exit code (0 ok, 1 failed, 2 usage or needs confirm, 3 daemon unreachable, 4 not found).",
            schema(json!({
                "type": "object",
                "properties": {
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments after the program name, e.g. [\"server\", \"list\"]" },
                    "confirm": { "type": "boolean", "description": "Only after the user said yes to the question a previous call returned" },
                    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT, "description": "How long to wait (default 300)" }
                },
                "required": ["args"]
            })),
        )
        .with_title("Run a Saba-chan command")
        .with_annotations(ToolAnnotations::new().read_only(false).destructive(true).open_world(true)),
    ]
}

impl ServerHandler for Local {
    #[allow(deprecated)]
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("saba-chan", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let input = request.arguments.unwrap_or_default();
        let result = match request.name.as_ref() {
            "saba_help" => help(&strings(input.get("command"))),
            "saba_run" => {
                let confirm = input.get("confirm").and_then(Value::as_bool) == Some(true);
                let wait = input
                    .get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_TIMEOUT)
                    .clamp(1, MAX_TIMEOUT);
                match child_args(&strings(input.get("args")), confirm, &self.base) {
                    Ok(args) => reply(run_cli(&args, Duration::from_secs(wait)).await),
                    Err(why) => CallToolResult::error(vec![ContentBlock::text(why)]),
                }
            }
            other => CallToolResult::error(vec![ContentBlock::text(format!("unknown tool: {other}"))]),
        };
        Ok(result.into())
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

/// `help --json`과 같은 명령 트리 (자식 없이)
fn help(path: &[String]) -> CallToolResult {
    let mut root = crate::cli::localized();
    root.build();
    match crate::cli::schema(&root, path) {
        Some(v) => CallToolResult::structured(v),
        None => CallToolResult::error(vec![ContentBlock::text(format!("no such command: {}", path.join(" ")))]),
    }
}

/// 이 서버를 띄울 때 고른 대상(`--daemon`, `--port`, `--lang`): 호출마다 기본으로 자식에 붙인다
fn base_args<I, T>(argv: I) -> Vec<String>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let Ok(cli) = crate::cli::Cli::try_parse_from(argv) else {
        return Vec::new();
    };
    let g = cli.global;
    let mut out = Vec::new();
    for (flag, value) in [
        ("--daemon", g.daemon),
        ("--port", g.port.map(|p| p.to_string())),
        ("--lang", g.lang),
    ] {
        if let Some(v) = value {
            out.extend([flag.to_string(), v]);
        }
    }
    out
}

/// 에이전트가 준 인자 → 자식 CLI 인자. 늘 `--json`, 확인을 받았을 때만 `--yes`.
/// `base`(띄울 때 고른 대상)는 호출이 같은 옵션을 직접 주지 않았을 때만 붙인다
fn child_args(args: &[String], confirm: bool, base: &[String]) -> std::result::Result<Vec<String>, String> {
    // 자식과 같은 파서로 읽는다: `-ys`처럼 묶은 짧은 옵션 안의 `-y`도 잡는다
    let parsed_yes =
        crate::cli::Cli::try_parse_from(std::iter::once("saba-chan-cli").chain(args.iter().map(String::as_str)))
            .is_ok_and(|c| c.global.yes);
    if parsed_yes
        || args
            .iter()
            .any(|a| a == "--yes" || a == "-y" || a.starts_with("--yes="))
    {
        return Err("Do not pass --yes: ask the user, then call again with confirm: true".into());
    }
    // 약관(EULA) 동의는 사람이 직접 한다: 에이전트가 `--accept <ID>`로 대신 동의하지 못하게 막는다
    if args.iter().any(|a| a == "--accept" || a.starts_with("--accept=")) {
        return Err(
            "Do not pass --accept: game agreements are accepted only by the user in a terminal or the app".into(),
        );
    }
    let mut words = args.iter();
    let command = loop {
        match words.next() {
            Some(a) if VALUE_FLAGS.contains(&a.as_str()) => {
                words.next();
            }
            Some(a) if a.starts_with('-') => {}
            other => break other,
        }
    };
    let Some(command) = command else {
        return Err("args needs a command, e.g. [\"status\"] (see saba_help)".into());
    };
    if REFUSED.contains(&command.as_str()) {
        return Err(format!("`{command}` is for a person at a keyboard and cannot run here"));
    }
    let mut out = vec!["--json".to_string()];
    if confirm {
        out.push("--yes".into());
    }
    for pair in base.chunks(2) {
        let flag = &pair[0];
        let given = args.iter().any(|a| a == flag || a.starts_with(&format!("{flag}=")));
        if !given {
            out.extend(pair.iter().cloned());
        }
    }
    out.extend(args.iter().cloned());
    Ok(out)
}

/// 자식 CLI 한 번의 결과
#[derive(Debug, Default)]
struct Outcome {
    /// 끝나지 않았으면(시간 초과) None
    exit: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_cli(args: &[String], wait: Duration) -> Outcome {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return Outcome {
                exit: Some(1),
                stderr: e.to_string(),
                ..Default::default()
            }
        }
    };
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .env(AGENT_ENV, "1")
        .env("SABA_NO_TUI", "1")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // 에이전트가 창 없이 띄운 서버다: 자식이 콘솔 창을 새로 열지 않게
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                exit: Some(1),
                stderr: e.to_string(),
                ..Default::default()
            }
        }
    };
    match tokio::time::timeout(wait, child.wait_with_output()).await {
        Ok(Ok(o)) => Outcome {
            exit: Some(o.status.code().unwrap_or(1)),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Ok(Err(e)) => Outcome {
            exit: Some(1),
            stderr: e.to_string(),
            ..Default::default()
        },
        // 기다림을 버리면 `kill_on_drop`이 자식을 끝낸다
        Err(_) => Outcome {
            exit: None,
            ..Default::default()
        },
    }
}

/// 결과를 에이전트가 읽는 꼴로: 확인이 필요하면 묻던 말과 다시 부르는 법, 아니면 종료 코드와 출력
fn reply(o: Outcome) -> CallToolResult {
    let Some(exit) = o.exit else {
        return CallToolResult::error(vec![ContentBlock::text(
            "timed out and was stopped. The daemon may still be working on it: check with saba_run [\"status\"] before trying again",
        )]);
    };
    // `--json`의 결과는 마지막 줄 (`--json-stream`이면 앞 줄들은 진행)
    let parsed: Option<Value> = serde_json::from_str(o.stdout.trim()).ok().or_else(|| {
        o.stdout
            .trim()
            .lines()
            .last()
            .and_then(|l| serde_json::from_str(l).ok())
    });
    if let Some(v) = parsed
        .as_ref()
        .filter(|v| v.get("error_code").and_then(Value::as_str) == Some("needs_confirm"))
    {
        let question = v.get("question").and_then(Value::as_str).unwrap_or_default();
        return CallToolResult::error(vec![ContentBlock::text(
            json!({
                "confirm_required": true,
                "question": question,
                "next": "Not run yet. Ask the user this question in plain words. Only if they agree, call saba_run again with the same args and confirm: true.",
            })
            .to_string(),
        )]);
    }
    let output = parsed.unwrap_or_else(|| Value::String(cut(&o.stdout)));
    let mut body = json!({ "exit_code": exit, "output": output });
    if exit != 0 && !o.stderr.trim().is_empty() {
        body["stderr"] = json!(cut(o.stderr.trim()));
    }
    let text = cut(&body.to_string());
    if exit == 0 {
        CallToolResult::success(vec![ContentBlock::text(text)])
    } else {
        CallToolResult::error(vec![ContentBlock::text(text)])
    }
}

fn cut(s: &str) -> String {
    match s.char_indices().nth(MAX_OUTPUT) {
        Some((i, _)) => format!("{}\n[output cut at {MAX_OUTPUT} characters]", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    /// 늘 JSON으로, 확인은 사람이 동의한 뒤(`confirm`)에만 `--yes`로 붙는다
    #[test]
    fn json_always_and_yes_only_after_confirm() {
        assert_eq!(
            child_args(&a(&["server", "list"]), false, &[]).unwrap(),
            a(&["--json", "server", "list"])
        );
        assert_eq!(
            child_args(&a(&["stop", "survival"]), true, &[]).unwrap(),
            a(&["--json", "--yes", "stop", "survival"])
        );
        assert!(child_args(&a(&["stop", "survival", "--yes"]), false, &[]).is_err());
        assert!(child_args(&a(&["-y", "stop", "x"]), false, &[]).is_err());
        // 묶은 짧은 옵션(`-ys` = `-y -s`)도 CLI 파서로 읽어 막는다
        assert!(child_args(&a(&["do", "server.stop", "-ys", "survival"]), false, &[]).is_err());
        // `-sy`는 `-s y`(서버 이름 y)라 확인 생략이 아니다
        assert!(child_args(&a(&["do", "server.stop", "-sy"]), false, &[]).is_ok());
        // 약관 동의는 confirm이 있어도 넘기지 않는다
        assert!(child_args(&a(&["server", "add", "mc", "--accept", "eula"]), true, &[]).is_err());
        assert!(child_args(&a(&["server", "add", "mc", "--accept=eula"]), false, &[]).is_err());
    }

    /// `saba-chan --daemon X mcp`처럼 띄울 때 고른 대상은 호출마다 기본으로 붙고, 호출이 고른 대상이 이긴다
    #[test]
    fn the_target_chosen_at_start_is_the_default() {
        let base = a(&["--daemon", "prod", "--port", "5000"]);
        assert_eq!(
            child_args(&a(&["status"]), false, &base).unwrap(),
            a(&["--json", "--daemon", "prod", "--port", "5000", "status"])
        );
        assert_eq!(
            child_args(&a(&["--daemon=home", "status"]), false, &base).unwrap(),
            a(&["--json", "--port", "5000", "--daemon=home", "status"])
        );
        assert_eq!(
            base_args(["saba-chan-cli", "--daemon", "prod", "mcp"]),
            a(&["--daemon", "prod"])
        );
        assert_eq!(
            base_args(["saba-chan-cli", "mcp", "--port", "5000"]),
            a(&["--port", "5000"])
        );
    }

    /// 사람이 키보드로 쓰는 화면과 이 서버 자신은 돌리지 않는다. 전역 옵션의 값은 명령으로 보지 않는다
    #[test]
    fn interactive_screens_are_refused() {
        assert!(child_args(&[], false, &[]).is_err());
        assert!(child_args(&a(&["--lang", "ko"]), false, &[]).is_err());
        for c in ["mcp", "shell", "tui"] {
            assert!(child_args(&a(&[c]), false, &[]).is_err(), "{c}");
            assert!(child_args(&a(&["--daemon", "home", c]), false, &[]).is_err(), "{c}");
        }
        assert!(child_args(&a(&["--daemon", "mcp", "status"]), false, &[]).is_ok());
    }

    fn text(r: &CallToolResult) -> String {
        serde_json::to_value(&r.content).unwrap().to_string()
    }

    /// 확인이 필요하다는 CLI 오류는 실행하지 않았다는 것과 묻던 말, 다시 부르는 법으로 바뀐다
    #[test]
    fn needs_confirm_becomes_a_question() {
        let stdout = json!({ "ok": false, "error": "x", "error_code": "needs_confirm", "question": "Stop survival?" })
            .to_string();
        let r = reply(Outcome {
            exit: Some(2),
            stdout,
            stderr: String::new(),
        });
        assert_eq!(r.is_error, Some(true));
        let t = text(&r);
        assert!(
            t.contains("confirm_required") && t.contains("Stop survival?") && t.contains("confirm: true"),
            "{t}"
        );
    }

    /// 성공은 출력 JSON 그대로, 실패는 오류로, 시간 초과는 상태를 먼저 보라고 한다
    #[test]
    fn exit_codes_carry_through() {
        let ok = reply(Outcome {
            exit: Some(0),
            stdout: "{\"servers\":[]}\n".into(),
            stderr: String::new(),
        });
        assert_eq!(ok.is_error, Some(false));
        assert!(text(&ok).contains("servers"));
        let bad = reply(Outcome {
            exit: Some(4),
            stdout: "{\"ok\":false,\"error_code\":\"not_found\"}".into(),
            stderr: "x".into(),
        });
        assert_eq!(bad.is_error, Some(true));
        assert!(text(&bad).contains("not_found"));
        assert!(text(&reply(Outcome::default())).contains("timed out"));
    }

    /// 도구 이름과 명령 트리가 맞물린다: 모르는 명령은 오류
    #[test]
    fn help_tool_reads_the_command_tree() {
        crate::i18n::init("en");
        assert_eq!(help(&a(&["server", "start"])).is_error, Some(false));
        assert_eq!(help(&a(&["nope"])).is_error, Some(true));
        assert_eq!(
            tools().iter().map(|t| t.name.to_string()).collect::<Vec<_>>(),
            ["saba_help", "saba_run"]
        );
    }
}
