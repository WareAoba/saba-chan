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
        RemoteCmd::Peers => {
            let v = ctx.api.remote_peers().await?;
            ctx.out.data(&v, |v| {
                let peers = v.get("peers").and_then(Value::as_array).cloned().unwrap_or_default();
                if peers.is_empty() {
                    say!("{}", output::dim(&t!("remote.no_peers")));
                    return;
                }
                let mut t = output::table(&[t!("col.name"), t!("remote.host_id"), t!("col.state"), t!("remote.reachable"), t!("remote.saved")]);
                for p in &peers {
                    t.add_row(vec![s(p, "name").to_string(), s(p, "hostId").to_string(), s(p, "status").to_string(), yes_no(p, "hasEndpoint"), yes_no(p, "saved")]);
                }
                output::print_table(t);
            });
            Ok(())
        }
        RemoteCmd::Connect { host_id } => {
            let r = ctx.api.connect_remote_daemon(&host_id).await?;
            ctx.out.done(&t!("remote.connected", name = s(&r, "name")), &r);
            Ok(())
        }
        RemoteCmd::Daemons => {
            let v = ctx.api.remote_daemons().await?;
            ctx.out.data(&v, |v| {
                let daemons = v.get("daemons").and_then(Value::as_array).cloned().unwrap_or_default();
                if daemons.is_empty() {
                    say!("{}", output::dim(&t!("remote.no_daemons")));
                    return;
                }
                let mut t = output::table(&[t!("col.name"), t!("remote.host_id"), t!("remote.address"), t!("remote.paired_at")]);
                for d in &daemons {
                    t.add_row(vec![s(d, "name").to_string(), s(d, "hostId").to_string(), daemon_address(d), s(d, "pairedAt").to_string()]);
                }
                output::print_table(t);
            });
            Ok(())
        }
        RemoteCmd::Servers => {
            let v = ctx.api.remote_servers().await?;
            ctx.out.data(&v, |v| {
                let mut t = output::table(&[t!("remote.daemon"), t!("col.name"), t!("col.game"), t!("col.state")]);
                for row in server_rows(v) {
                    t.add_row(row.to_vec());
                }
                output::print_table(t);
            });
            Ok(())
        }
        RemoteCmd::Forget { host_id } => {
            let r = ctx.api.forget_remote_daemon(&host_id).await?;
            ctx.out.done(&t!("remote.forgotten", host = host_id), &r);
            Ok(())
        }
    }
}

async fn status(ctx: &Ctx) -> Result<()> {
    let v = ctx.api.remote_settings().await?;
    ctx.out.data(&v, print_status);
    Ok(())
}

fn state_label(status: &str) -> String {
    crate::i18n::try_t(&format!("status.{status}")).unwrap_or_else(|| status.to_string())
}

/// 합쳐 보기 (G1c): 데몬마다 서버 한 줄씩. 서버가 없거나 닿지 않는 데몬도 한 줄로 이유를 남긴다
fn server_rows(v: &Value) -> Vec<[String; 4]> {
    let mut rows = Vec::new();
    for d in v.get("daemons").and_then(Value::as_array).into_iter().flatten() {
        let daemon = match (d.get("hostId").and_then(Value::as_str), s(d, "name")) {
            (None, _) => t!("remote.this_pc"),
            (Some(host), "") => host.to_string(),
            (Some(_), name) => name.to_string(),
        };
        let servers = d.get("servers").and_then(Value::as_array).cloned().unwrap_or_default();
        if d.get("ok").and_then(Value::as_bool) != Some(true) {
            let why = t!(&format!("remote.connect_error.{}", s(d, "error_code")));
            rows.push([daemon, "-".into(), String::new(), why]);
        } else if servers.is_empty() {
            rows.push([daemon, "-".into(), String::new(), t!("remote.no_servers")]);
        } else {
            for srv in &servers {
                rows.push([daemon.clone(), s(srv, "name").to_string(), s(srv, "module").to_string(), state_label(s(srv, "status"))]);
            }
        }
    }
    rows
}

fn yes_no(v: &Value, key: &str) -> String {
    if v.get(key).and_then(Value::as_bool) == Some(true) { t!("remote.yes") } else { t!("remote.no") }
}

/// 준 것만 바꾼다
/// 붙는 원격 데몬의 주소. 공인 주소가 없는 PC는 릴레이 터널로 붙는다 (T1)
fn daemon_address(d: &Value) -> String {
    match s(d, "address") {
        "" => t!("remote.via_relay"),
        address => format!("{}:{}", address, d.get("port").map(|p| p.to_string()).unwrap_or_default()),
    }
}

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
    fn every_daemons_servers_are_one_table_and_an_unreachable_daemon_says_why() {
        let v = json!({ "daemons": [
            { "hostId": null, "name": "", "ok": true, "servers": [{ "id": "a", "name": "test-pal", "module": "palworld", "status": "running" }] },
            { "hostId": "host-b", "name": "게임 PC", "ok": true, "servers": [] },
            { "hostId": "host-c", "name": "", "ok": false, "error_code": "remote_unreachable", "servers": [] },
        ] });
        let rows = server_rows(&v);
        assert_eq!(rows[0], [t!("remote.this_pc"), "test-pal".into(), "palworld".into(), state_label("running")]);
        assert_eq!(rows[1], ["게임 PC".to_string(), "-".into(), String::new(), t!("remote.no_servers")]);
        assert_eq!(rows[2], ["host-c".to_string(), "-".into(), String::new(), t!("remote.connect_error.remote_unreachable")]);
    }

    #[test]
    fn a_daemon_without_a_public_address_is_reached_through_the_relay() {
        assert_eq!(daemon_address(&json!({ "address": "", "port": 57480 })), t!("remote.via_relay"));
        assert_eq!(daemon_address(&json!({ "address": "h.example", "port": 57480 })), "h.example:57480");
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
