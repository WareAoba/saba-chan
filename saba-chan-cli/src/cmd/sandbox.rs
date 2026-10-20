//! `sandbox` — 확인되지 않은 구성요소 동의: 격리 없이 돌리기(데몬 `sandbox.unisolated`),
//! 익스텐션 GUI 번들 싣기(`--gui`, 데몬 `sandbox.extension_gui`)

use crate::cli::SandboxCmd;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::{json, Value};

const UNISOLATED: &str = "unisolated";
const EXTENSION_GUI: &str = "extension-gui";

fn kind(gui: bool) -> &'static str {
    if gui {
        EXTENSION_GUI
    } else {
        UNISOLATED
    }
}

pub async fn run(ctx: &Ctx, cmd: SandboxCmd) -> Result<()> {
    match cmd {
        SandboxCmd::List => list(ctx).await,
        SandboxCmd::Allow { component, gui } => {
            // 확인되지 않은 코드에 이 PC를 여는 동의다: 무엇을 여는지 보이고 묻는다
            let question = if gui { "sandbox.confirm_gui" } else { "sandbox.confirm_allow" };
            // GUI는 네이티브 대화상자로 묻는다: CLI도 사람이 직접 답해야 한다
            ctx.ask.agreed(&t!(question, component = component))?;
            let r = ctx.api.set_consent(kind(gui), &component, true).await?;
            let key = if gui { "sandbox.gui_allowed" } else { "sandbox.allowed" };
            ctx.out.done(&t!(key, component = component), &r);
            Ok(())
        }
        SandboxCmd::Revoke { component, gui } => {
            let r = ctx.api.set_consent(kind(gui), &component, false).await?;
            let key = if gui { "sandbox.gui_revoked" } else { "sandbox.revoked" };
            ctx.out.done(&t!(key, component = component), &r);
            Ok(())
        }
    }
}

async fn list(ctx: &Ctx) -> Result<()> {
    let (unisolated, gui) = tokio::join!(ctx.api.consents(UNISOLATED), ctx.api.consents(EXTENSION_GUI));
    let v = json!({ "unisolated": unisolated?, "extension_gui": gui? });
    ctx.out.data(&v, |v| {
        let rows = consent_rows(v);
        if rows.is_empty() {
            say!("{}", output::dim(&t!("sandbox.none")));
            return;
        }
        let mut t = output::table(&[t!("sandbox.component"), t!("col.kind"), t!("col.state")]);
        for row in rows {
            t.add_row(row);
        }
        output::print_table(t);
    });
    Ok(())
}

/// 동의 표의 줄: 구성요소, 종류, 지금 파일에 맞는지 (파일이 바뀌었으면 동의가 쓰이지 않는다)
fn consent_rows(v: &Value) -> Vec<Vec<String>> {
    [("unisolated", "sandbox.kind_unisolated"), ("extension_gui", "sandbox.kind_gui")]
        .iter()
        .flat_map(|(key, label)| {
            v.pointer(&format!("/{}/components", key))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |c| {
                    let state = if c.get("current").and_then(Value::as_bool) == Some(false) {
                        console::style(t!("sandbox.stale")).yellow().to_string()
                    } else {
                        console::style(t!("sandbox.in_use")).green().to_string()
                    };
                    vec![s(&c, "component").to_string(), t!(label), state]
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 두 동의 목록을 한 표로, 파일이 바뀐 동의는 쓰이지 않는다고 표시한다 (C13)
    #[test]
    fn both_consent_lists_show_and_a_changed_component_is_marked() {
        let v = json!({
            "unisolated": { "components": [{ "component": "module-zomboid", "fingerprint": "x", "current": false }] },
            "extension_gui": { "components": [{ "component": "ext-music", "fingerprint": "y", "current": true }] }
        });
        let rows = consent_rows(&v);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0][0].as_str(), rows[0][1].clone()), ("module-zomboid", t!("sandbox.kind_unisolated")));
        assert_eq!(console::strip_ansi_codes(&rows[0][2]), t!("sandbox.stale"));
        assert_eq!((rows[1][0].as_str(), rows[1][1].clone()), ("ext-music", t!("sandbox.kind_gui")));
        assert!(consent_rows(&json!({})).is_empty());
    }
}
