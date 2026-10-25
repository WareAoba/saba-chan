//! `remote` — 원격 데몬 포트 (데몬 `/api/remote/*`, D1): 같은 주인의 다른 PC가 이 데몬에 TLS로 직접 붙는다

use crate::cli::RemoteCmd;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::{json, Map, Value};

pub async fn run(ctx: &Ctx, cmd: RemoteCmd) -> Result<()> {
    match cmd {
        RemoteCmd::Status => {
            let v = ctx.api.remote_settings().await?;
            ctx.out.data(&v, print_status);
            Ok(())
        }
        RemoteCmd::Set { on, off, port, address } => {
            let body = patch(on, off, port, address);
            if body.is_empty() {
                return status(ctx).await;
            }
            let v = ctx.api.set_remote_settings(Value::Object(body)).await?;
            ctx.out.data(&v, print_status);
            Ok(())
        }
        RemoteCmd::Clients => {
            let v = ctx.api.remote_clients().await?;
            ctx.out.data(&v, |v| {
                let clients = v.get("clients").and_then(Value::as_array).cloned().unwrap_or_default();
                if clients.is_empty() {
                    say!("{}", output::dim(&t!("remote.no_clients")));
                    return;
                }
                let mut t = output::table(&[t!("col.name"), t!("remote.host_id"), t!("remote.paired_at")]);
                for c in &clients {
                    t.add_row(vec![s(c, "name").to_string(), s(c, "hostId").to_string(), s(c, "pairedAt").to_string()]);
                }
                output::print_table(t);
            });
            Ok(())
        }
        RemoteCmd::Revoke { host_id } => {
            let r = ctx.api.revoke_remote_client(&host_id).await?;
            ctx.out.done(&t!("remote.revoked", host = host_id), &r);
            Ok(())
        }
    }
}

async fn status(ctx: &Ctx) -> Result<()> {
    let v = ctx.api.remote_settings().await?;
    ctx.out.data(&v, print_status);
    Ok(())
}

/// 준 것만 바꾼다
fn patch(on: bool, off: bool, port: Option<u16>, address: Option<String>) -> Map<String, Value> {
    let mut body = Map::new();
    if on || off {
        body.insert("enabled".into(), json!(on));
    }
    if let Some(p) = port {
        body.insert("port".into(), json!(p));
    }
    if let Some(a) = address {
        body.insert("address".into(), json!(a));
    }
    body
}

fn status_rows(v: &Value) -> Vec<[String; 2]> {
    let st = v.get("status").cloned().unwrap_or_default();
    let state = match (v.get("enabled").and_then(Value::as_bool), st.get("listening").and_then(Value::as_bool), st.get("error").and_then(Value::as_str)) {
        (Some(true), Some(true), _) => t!("remote.listening"),
        (Some(true), _, Some(code)) => t!(&format!("remote.error.{code}")),
        (Some(true), _, None) => t!("remote.not_listening"),
        _ => t!("remote.off"),
    };
    let address = match s(v, "address") {
        "" => t!("remote.no_address"),
        a => a.to_string(),
    };
    let published = if st.get("published").and_then(Value::as_bool) == Some(true) { t!("remote.yes") } else { t!("remote.no") };
    vec![
        [t!("col.state"), state],
        [t!("col.port"), v.get("port").map(|p| p.to_string()).unwrap_or_default()],
        [t!("remote.address"), address],
        [t!("remote.published"), published],
        [t!("remote.fingerprint"), st.get("certSha256").and_then(Value::as_str).unwrap_or("-").to_string()],
    ]
}

fn print_status(v: &Value) {
    let mut t = output::table(&[t!("remote.item"), t!("remote.value")]);
    for [k, val] in status_rows(v) {
        t.add_row(vec![k, val]);
    }
    output::print_table(t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_sends_only_what_was_given() {
        assert!(patch(false, false, None, None).is_empty());
        assert_eq!(Value::Object(patch(true, false, Some(7443), None)), json!({ "enabled": true, "port": 7443 }));
        assert_eq!(Value::Object(patch(false, true, None, Some(String::new()))), json!({ "enabled": false, "address": "" }));
    }

    #[test]
    fn the_status_says_why_the_port_is_not_open() {
        let v = json!({ "enabled": true, "port": 57480, "address": "", "status": { "listening": false, "published": false, "error": "port_in_use" } });
        let rows = status_rows(&v);
        assert_eq!(rows[0][1], t!("remote.error.port_in_use"));
        assert_eq!(rows[2][1], t!("remote.no_address"));
        let v = json!({ "enabled": false, "port": 57480, "address": "h.example", "status": { "listening": false, "published": false } });
        assert_eq!(status_rows(&v)[0][1], t!("remote.off"));
    }
}
