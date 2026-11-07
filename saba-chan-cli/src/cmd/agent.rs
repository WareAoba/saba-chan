//! `agent` — 이 PC의 AI 에이전트(Claude Code·Codex)에 로컬 MCP(`mcp`)를 등록·해제한다
//!
//! Claude Code: `claude mcp add --scope user --transport stdio saba-chan -- <이 CLI> mcp` (사용자 범위)
//! Codex: `~/.codex/config.toml`의 `[mcp_servers.saba-chan]` (다른 내용은 그대로 둔다)

use crate::cli::AgentCmd;
use crate::ctx::Ctx;
use crate::error::{failed, Result};
use crate::output::{self, s};
use serde_json::json;
use std::path::{Path, PathBuf};

/// 등록 이름 (두 에이전트 공통)
pub const NAME: &str = "saba-chan";

/// Codex는 도구 호출을 기본 60초만 기다린다: 설치·백업이 끝날 때까지 (mcp의 최대 대기와 같다)
const CODEX_TOOL_TIMEOUT: i64 = 3600;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn key(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
}

/// 고른 에이전트. 아무것도 고르지 않으면 둘 다
fn chosen(claude: bool, codex: bool) -> Vec<Agent> {
    match (claude, codex) {
        (false, false) | (true, true) => vec![Agent::Claude, Agent::Codex],
        (true, false) => vec![Agent::Claude],
        (false, true) => vec![Agent::Codex],
    }
}

pub async fn run(ctx: &Ctx, cmd: AgentCmd) -> Result<()> {
    let (agents, link) = match cmd {
        AgentCmd::Status => return status(ctx),
        AgentCmd::Link { claude, codex } => (chosen(claude, codex), true),
        AgentCmd::Unlink { claude, codex } => (chosen(claude, codex), false),
    };
    let exe = std::env::current_exe().map_err(|e| failed(e.to_string()))?;
    let mut rows = Vec::new();
    for agent in agents {
        let result = match (agent, link) {
            (Agent::Claude, true) => claude_link(&exe),
            (Agent::Claude, false) => claude_unlink(),
            (Agent::Codex, true) => codex_config().and_then(|f| codex_edit(&f, Some(&exe))),
            (Agent::Codex, false) => codex_config().and_then(|f| codex_edit(&f, None)),
        };
        let state = match result {
            Ok(true) => {
                if link {
                    "linked"
                } else {
                    "unlinked"
                }
            }
            Ok(false) => "not_installed",
            Err(e) => {
                rows.push(json!({ "agent": agent.key(), "state": "failed", "error": e }));
                continue;
            }
        };
        rows.push(json!({ "agent": agent.key(), "state": state }));
    }
    let failures = rows.iter().filter(|r| s(r, "state") == "failed").count();
    let v = json!({ "agents": rows, "command": [exe.to_string_lossy(), "mcp"] });
    ctx.out.data(&v, |v| {
        for r in v["agents"].as_array().into_iter().flatten() {
            let agent = if s(r, "agent") == "claude" {
                Agent::Claude
            } else {
                Agent::Codex
            };
            let line = match s(r, "state") {
                "linked" => t!("agent.linked", agent = agent.label()),
                "unlinked" => t!("agent.unlinked", agent = agent.label()),
                "not_installed" => t!("agent.not_installed", agent = agent.label()),
                _ => t!("agent.failed", agent = agent.label(), error = s(r, "error")),
            };
            say!("{line}");
        }
        if link {
            say!("{}", output::dim(&t!("agent.restart_hint")));
        }
    });
    if failures > 0 {
        return Err(failed(t!("agent.some_failed")));
    }
    Ok(())
}

fn status(ctx: &Ctx) -> Result<()> {
    let claude = which("claude").map(|c| command_ok(&c, &["mcp", "get", NAME]));
    let codex = codex_config()
        .ok()
        .filter(|f| f.parent().is_some_and(Path::is_dir))
        .map(|f| codex_linked(&f));
    let v = json!({ "claude": claude, "codex": codex });
    ctx.out.data(&v, |v| {
        for agent in [Agent::Claude, Agent::Codex] {
            let line = match v[agent.key()].as_bool() {
                Some(true) => t!("agent.is_linked", agent = agent.label()),
                Some(false) => t!("agent.is_not_linked", agent = agent.label()),
                None => t!("agent.not_installed", agent = agent.label()),
            };
            say!("{line}");
        }
    });
    Ok(())
}

/// PATH에서 실행 파일을 찾는다 (Windows는 PATHEXT 확장자도)
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path).find_map(|dir| {
        exts.iter()
            .map(|e| dir.join(format!("{name}{e}")))
            .find(|p| p.is_file())
    })
}

fn command(exe: &Path, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

fn command_ok(exe: &Path, args: &[&str]) -> bool {
    command(exe, args).status().is_ok_and(|s| s.success())
}

/// 등록했으면 true, Claude Code가 없으면 false. 같은 이름이 있으면 지우고 다시 넣는다 (CLI 경로가 바뀌었을 수 있다)
fn claude_link(exe: &Path) -> std::result::Result<bool, String> {
    let Some(claude) = which("claude") else {
        return Ok(false);
    };
    let _ = command(&claude, &["mcp", "remove", NAME, "--scope", "user"]).output();
    let me = exe.to_string_lossy();
    let out = command(
        &claude,
        &[
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "stdio",
            NAME,
            "--",
            &me,
            "mcp",
        ],
    )
    .output()
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(true)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn claude_unlink() -> std::result::Result<bool, String> {
    let Some(claude) = which("claude") else {
        return Ok(false);
    };
    // 등록이 없으면 실패로 끝난다: 지울 것이 없었던 것이므로 해제된 것으로 본다
    let _ = command(&claude, &["mcp", "remove", NAME, "--scope", "user"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// `~/.codex/config.toml`
fn codex_config() -> std::result::Result<PathBuf, String> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok_or("no home folder")?;
    Ok(PathBuf::from(home).join(".codex").join("config.toml"))
}

fn codex_linked(file: &Path) -> bool {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .is_some_and(|d| d.get("mcp_servers").and_then(|m| m.get(NAME)).is_some())
}

/// `exe`가 있으면 넣고, 없으면 뺀다. Codex 폴더가 없으면(설치 안 됨) false. 다른 설정은 그대로 둔다
fn codex_edit(file: &Path, exe: Option<&Path>) -> std::result::Result<bool, String> {
    if !file.parent().is_some_and(Path::is_dir) && which("codex").is_none() {
        return Ok(false);
    }
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let edited = codex_text(&text, exe)?;
    if edited != text {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(file, edited).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

fn codex_text(text: &str, exe: Option<&Path>) -> std::result::Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    match exe {
        Some(exe) => {
            let servers = doc.entry("mcp_servers").or_insert_with(|| {
                let mut t = toml_edit::Table::new();
                t.set_implicit(true);
                toml_edit::Item::Table(t)
            });
            let servers = servers.as_table_mut().ok_or("mcp_servers is not a table")?;
            let mut table = toml_edit::Table::new();
            table["command"] = toml_edit::value(exe.to_string_lossy().to_string());
            let mut args = toml_edit::Array::new();
            args.push("mcp");
            table["args"] = toml_edit::value(args);
            table["tool_timeout_sec"] = toml_edit::value(CODEX_TOOL_TIMEOUT);
            servers.insert(NAME, toml_edit::Item::Table(table));
        }
        None => {
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(|m| m.as_table_mut()) {
                servers.remove(NAME);
            }
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 고르지 않으면 둘 다, 하나만 고르면 그것만
    #[test]
    fn no_choice_means_both() {
        assert_eq!(chosen(false, false), vec![Agent::Claude, Agent::Codex]);
        assert_eq!(chosen(true, false), vec![Agent::Claude]);
        assert_eq!(chosen(false, true), vec![Agent::Codex]);
    }

    /// Codex 설정: 사바쨩 칸만 넣고 빼며, 사용자의 다른 설정과 주석은 그대로 둔다. 다시 넣으면 경로만 바뀐다
    #[test]
    fn codex_config_keeps_everything_else() {
        let before = "# my settings\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"npx\"\n";
        let exe = Path::new("/opt/saba/saba-chan-cli");
        let linked = codex_text(before, Some(exe)).unwrap();
        assert!(linked.starts_with("# my settings\nmodel = \"gpt-5\""), "{linked}");
        let doc: toml_edit::DocumentMut = linked.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["other"]["command"].as_str(), Some("npx"));
        assert_eq!(
            doc["mcp_servers"][NAME]["command"].as_str(),
            Some("/opt/saba/saba-chan-cli")
        );
        assert_eq!(
            doc["mcp_servers"][NAME]["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>(),
            ["mcp"]
        );
        assert_eq!(
            doc["mcp_servers"][NAME]["tool_timeout_sec"].as_integer(),
            Some(CODEX_TOOL_TIMEOUT)
        );
        let again = codex_text(&linked, Some(Path::new("/new/saba-chan-cli"))).unwrap();
        assert_eq!(again.matches("[mcp_servers.saba-chan]").count(), 1, "{again}");
        let unlinked = codex_text(&again, None).unwrap();
        assert!(
            !unlinked.contains(NAME) && unlinked.contains("[mcp_servers.other]"),
            "{unlinked}"
        );
        assert_eq!(
            codex_text("", Some(exe))
                .unwrap()
                .matches("[mcp_servers.saba-chan]")
                .count(),
            1
        );
        assert!(codex_text("model = [", Some(exe)).is_err());
    }
}
