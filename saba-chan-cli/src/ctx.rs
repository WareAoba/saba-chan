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
    pub out: Out,
    pub ask: Prompt,
    /// 대화형 셸 안에서 실행 중
    pub in_shell: bool,
    /// 콘솔에 붙어 있음 (새 줄이 이미 흐르므로 명령 답을 따로 보여 주지 않는다)
    pub attached: bool,
}

impl Ctx {
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
        bootstrap::spawn_daemon(self.api.port)?;
        let spin = crate::progress::spinner(&t!("daemon.starting"));
        let deadline = Instant::now() + bootstrap::START_TIMEOUT;
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Ok(h) = self.api.health().await {
                spin.finish_and_clear();
                return Ok(h);
            }
        }
        spin.finish_and_clear();
        Err(crate::error::failed(t!("daemon.start_timeout")))
    }
}
