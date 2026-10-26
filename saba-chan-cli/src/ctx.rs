//! 명령 하나를 실행하는 데 필요한 것 묶음

use crate::bootstrap;
use crate::client::Api;
use crate::error::{CliError, Result};
use crate::output::Out;
use crate::prompt::Prompt;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Ctx {
    pub api: Api,
    /// 데몬을 새로 띄울 포트 (`--port` 또는 설정). 옛 포트에 남은 데몬에 붙어 있으면 `api.port`와 다르다
    pub spawn_port: u16,
    /// `--port`로 포트를 정했다 (셸이 줄마다 포트를 다시 찾지 않는다)
    pub port_fixed: bool,
    pub out: Out,
    pub ask: Prompt,
    /// 대화형 셸 안에서 실행 중
    pub in_shell: bool,
    /// 콘솔에 붙어 있음 (콘솔로 온 답은 이미 흐르므로 다시 보여 주지 않는다)
    pub attached: bool,
}

impl Ctx {
    /// 이 포트의 데몬에 붙고, 데몬을 띄울 때도 이 포트에 띄운다 (`--port`)
    pub fn on_port(&self, port: u16) -> Ctx {
        Ctx {
            api: self.api.on_port(port),
            spawn_port: port,
            port_fixed: true,
            ..self.clone()
        }
    }

    /// 셸 한 줄 전에 포트를 다시 찾는다: 셸 안에서 설정 포트를 바꾸거나 데몬을 다시 켰을 수 있다 (L62)
    pub fn follow_daemon_port(&mut self) {
        if self.port_fixed {
            return;
        }
        self.spawn_port = bootstrap::ipc_port();
        let port = bootstrap::live_port(self.spawn_port);
        if port != self.api.port {
            self.api = self.api.on_port(port);
        }
    }

    /// 데몬이 꺼져 있으면, 물을 수 있을 때 켤지 묻는다
    pub async fn ensure_daemon(&self) -> Result<()> {
        match self.api.health().await {
            Ok(_) => Ok(()),
            Err(e) if matches!(e.downcast_ref::<CliError>(), Some(CliError::DaemonDown { .. })) => {
                if self.ask.enabled
                    && !self.ask.yes
                    && bootstrap::daemon_binary().is_some()
                    && self.ask.confirm(&t!("daemon.offer_start"), true)?
                {
                    let h = self.start_daemon().await?;
                    self.out
                        .note(&t!("daemon.started", version = crate::output::s(&h, "version")));
                    if let Some(w) = crate::cmd::daemon::launcher_warning(&h) {
                        self.out.warn(&w);
                    }
                    return Ok(());
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// 데몬을 띄우고 응답할 때까지 기다린다. 이미 떠 있으면 그대로. 결과 출력은 부른 쪽이 한다.
    pub async fn start_daemon(&self) -> Result<serde_json::Value> {
        if let Ok(h) = self.api.health().await {
            return Ok(h);
        }
        let mut child = bootstrap::spawn_daemon(self.spawn_port)?;
        let api = if self.spawn_port == self.api.port {
            self.api.clone()
        } else {
            self.api.on_port(self.spawn_port)
        };
        let spin = crate::progress::spinner(&t!("daemon.starting"));
        let deadline = Instant::now() + bootstrap::START_TIMEOUT;
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Ok(h) = api.health().await {
                spin.finish_and_clear();
                return Ok(h);
            }
            // 곧바로 끝난 데몬(다른 데몬이 잠금을 쥔 경우 등)은 시간 초과까지 기다리지 않는다
            if let Ok(Some(status)) = child.try_wait() {
                spin.finish_and_clear();
                let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "-".into());
                return Err(crate::error::failed(t!("daemon.exited_early", code = code)));
            }
        }
        spin.finish_and_clear();
        Err(crate::error::failed(t!("daemon.start_timeout")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 셸 한 줄에 준 `--port`는 붙을 곳과 새로 띄울 곳 모두에 쓴다 (`daemon start --port N`이 N에 띄운다)
    #[test]
    fn a_port_given_on_one_line_is_used_to_attach_and_to_spawn() {
        let ctx = Ctx {
            api: Api::new(57474, None),
            spawn_port: 58000,
            port_fixed: false,
            out: crate::output::Out::new(true, false),
            ask: crate::prompt::Prompt {
                enabled: false,
                yes: false,
            },
            in_shell: true,
            attached: false,
        };
        let line = ctx.on_port(57495);
        assert_eq!((line.api.port, line.spawn_port, line.port_fixed), (57495, 57495, true));
    }
}
