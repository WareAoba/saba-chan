//! `sandbox` — 확인되지 않은 구성요소를 격리 없이 돌리기 동의 (데몬 `sandbox.unisolated`)

use crate::cli::SandboxCmd;
use crate::ctx::Ctx;
use crate::error::Result;

pub async fn run(ctx: &Ctx, cmd: SandboxCmd) -> Result<()> {
    let (component, allow) = match cmd {
        SandboxCmd::Allow { component } => (component, true),
        SandboxCmd::Revoke { component } => (component, false),
    };
    let r = ctx.api.set_unisolated(&component, allow).await?;
    let key = if allow { "sandbox.allowed" } else { "sandbox.revoked" };
    ctx.out.done(&t!(key, component = component), &r);
    Ok(())
}
