//! `backup` — 서버 월드 백업 목록·만들기·지우기·되돌리기
//!
//! 무엇을 백업할지는 모듈이 정한다 (데몬이 지원하지 않으면 `backup_unsupported`).
//! 되돌리기는 서버가 꺼져 있어야 한다. 켜져 있으면 끄고 되돌린 뒤 다시 켤지 묻는다.
//! 되돌리기 직전 상태는 데몬이 `pre-restore` 백업으로 남긴다.

use crate::cli::{BackupCmd, StartArgs};
use crate::cmd::server::{runtime, shell_word, status_of, stop_server};
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use crate::resolve::{self, Server};
use serde_json::Value;

pub async fn run(ctx: &Ctx, cmd: BackupCmd) -> Result<()> {
    match cmd {
        BackupCmd::List { server } => list(ctx, server.as_deref()).await,
        BackupCmd::Create { server } => create(ctx, server.as_deref()).await,
        BackupCmd::Delete { server, backup } => {
            let sv = resolve::server(ctx, Some(&server)).await?;
            let name = pick(ctx, &sv, backup.as_deref()).await?;
            ctx.ask
                .ensure(&t!("backup.confirm_delete", backup = name, name = sv.name))?;
            let r = ctx.api.delete_backup(&sv.id, &name).await?;
            ctx.out.done(&t!("backup.deleted", backup = name), &r);
            Ok(())
        }
        BackupCmd::Restore { server, backup } => restore(ctx, &server, backup.as_deref()).await,
    }
}

async fn backups(ctx: &Ctx, sv: &Server) -> Result<Vec<Value>> {
    let r = ctx.api.backups(&sv.id).await?;
    Ok(r.get("backups").and_then(Value::as_array).cloned().unwrap_or_default())
}

async fn list(ctx: &Ctx, server: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    let r = ctx.api.backups(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let items = r.get("backups").and_then(Value::as_array).cloned().unwrap_or_default();
        if items.is_empty() {
            say!("{}", output::dim(&t!("backup.none", name = sv.name)));
            ctx.out
                .hint(&format!("saba-chan-cli backup create {}", shell_word(&sv.name)));
            return;
        }
        let mut t = output::table(&[
            t!("backup.col.name"),
            t!("backup.col.kind"),
            t!("backup.col.when"),
            t!("backup.col.size"),
        ]);
        for b in &items {
            t.add_row(vec![
                s(b, "name").to_string(),
                kind_label(s(b, "kind")),
                b.get("created")
                    .and_then(Value::as_u64)
                    .map(output::ago)
                    .unwrap_or_default(),
                size_cell(b, |stored| t!("backup.stored", size = stored)),
            ]);
        }
        output::print_table(t);
    });
    Ok(())
}

async fn create(ctx: &Ctx, server: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    let spin = crate::progress::spinner(&t!("backup.creating", name = sv.name));
    let r = ctx.api.create_backup(&sv.id).await;
    spin.finish_and_clear();
    let r = r?;
    ctx.out
        .done(&t!("backup.created", backup = name_of(r.get("backup"))), &r);
    Ok(())
}

async fn restore(ctx: &Ctx, server: &str, backup: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, Some(server)).await?;
    let name = pick(ctx, &sv, backup).await?;
    let was_running = status_of(&runtime(ctx, &sv).await?) != "stopped";
    if was_running {
        ctx.ask
            .ensure(&t!("backup.confirm_restore_running", backup = name, name = sv.name))?;
        stop_server(ctx, &sv, false, false).await?;
    } else {
        ctx.ask
            .ensure(&t!("backup.confirm_restore", backup = name, name = sv.name))?;
    }
    let spin = crate::progress::spinner(&t!("backup.restoring", backup = name));
    let r = ctx.api.restore_backup(&sv.id, &name).await;
    spin.finish_and_clear();
    let r = r?;
    ctx.out.done(&t!("backup.restored", backup = name, name = sv.name), &r);
    let pre = name_of(r.get("pre_restore_backup"));
    if !pre.is_empty() {
        ctx.out.note(&t!("backup.pre_restore", backup = pre));
    }
    if let Some(msg) = links_missed(&r) {
        ctx.out.warn(&msg);
    }
    if was_running
        && ctx
            .ask
            .confirm(&t!("backup.start_again", name = sv.name), true)
            .unwrap_or(false)
    {
        return crate::cmd::server::start(
            ctx,
            StartArgs {
                server: Some(sv.id.clone()),
                ..Default::default()
            },
        )
        .await;
    }
    Ok(())
}

/// 백업 이름: 정확히, 앞부분만, 또는 `latest`(가장 최근). 이름이 없으면 목록에서 고른다(최근 것부터)
async fn pick(ctx: &Ctx, sv: &Server, query: Option<&str>) -> Result<String> {
    let mut items = backups(ctx, sv).await?;
    let what = t!("noun.backup");
    let label = |b: &Value| format!("{} ({})", s(b, "name"), kind_label(s(b, "kind")));
    let Some(query) = query.filter(|q| !q.trim().is_empty()) else {
        if items.is_empty() {
            return Err(crate::error::failed(t!("backup.none", name = sv.name)));
        }
        newest_first(&mut items);
        // 하나뿐이어도 터미널이 아니면 고르지 않는다: 되돌리기·지우기는 이름을 적어야 한다
        if !ctx.ask.enabled {
            return Err(crate::error::usage(t!("resolve.name_required", what = what)));
        }
        let labels: Vec<String> = items.iter().map(label).collect();
        return match ctx.ask.select(&t!("resolve.pick", what = what), &labels, 0)? {
            Some(i) => Ok(s(&items[i], "name").to_string()),
            None => Err(crate::error::CliError::Cancelled.into()),
        };
    };
    if query.eq_ignore_ascii_case("latest") {
        if let Some(b) = items
            .iter()
            .max_by_key(|b| b.get("created").and_then(Value::as_u64).unwrap_or(0))
        {
            return Ok(s(b, "name").to_string());
        }
    }
    let found = resolve::find(&items, query, |b| vec![s(b, "name").to_string()], label);
    let b = resolve::settle(ctx, &what, query, found, label)?;
    Ok(s(&b, "name").to_string())
}

/// 되살리지 못한 링크가 있으면 그 수 (경로는 데몬 로그에만 남는다)
fn links_missed(r: &Value) -> Option<String> {
    let n = r.get("links_not_restored").and_then(Value::as_u64).filter(|n| *n > 0)?;
    Some(t!("backup.links_not_restored", count = n))
}

fn newest_first(items: &mut [Value]) {
    items.sort_by_key(|b| std::cmp::Reverse(b.get("created").and_then(Value::as_u64).unwrap_or(0)));
}

/// 데몬은 백업을 `{name, ...}`로 준다 (이름 문자열만 와도 받는다)
fn name_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(n)) => n.clone(),
        Some(b) => s(b, "name").to_string(),
        None => String::new(),
    }
}

fn kind_label(kind: &str) -> String {
    crate::i18n::try_t(&format!("backup.kind.{}", kind)).unwrap_or_else(|| kind.to_string())
}

/// 목록의 크기 칸. 증분 백업은 담은 크기(`size`)와 이번에 새로 저장한 크기(`stored`)가 다르면 둘 다
fn size_cell(b: &Value, stored_label: impl Fn(String) -> String) -> String {
    let Some(total) = b.get("size").and_then(Value::as_u64) else {
        return String::new();
    };
    match b.get("stored").and_then(Value::as_u64) {
        Some(stored) if stored != total => format!("{} · {}", size(total), stored_label(size(stored))),
        _ => size(total),
    }
}

/// 바이트 → 사람이 읽는 크기
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 복원은 끝났어도 되살리지 못한 링크가 있으면 그 수를 알린다. 경로는 데몬 로그에만 있다 (D-B14)
    #[test]
    fn a_restore_that_missed_links_says_how_many() {
        crate::i18n::init("ko");
        assert!(links_missed(&json!({ "links_not_restored": 2 })).is_some_and(|m| m.contains('2')));
        assert_eq!(links_missed(&json!({ "links_not_restored": 0 })), None);
        assert_eq!(links_missed(&json!({})), None, "an older daemon does not report it");
    }

    /// 증분 백업(MK16)은 담은 크기와 새로 저장한 크기가 다르면 둘 다 보인다. 0.1.8 백업이나 옛 데몬은 크기만
    #[test]
    fn an_incremental_backup_shows_what_it_newly_stored() {
        let ko = |b: &Value| {
            size_cell(b, |stored| {
                crate::i18n::try_t_in("ko", "backup.stored")
                    .unwrap()
                    .replace("{{size}}", &stored)
            })
        };
        let mb = 1024 * 1024;
        let cell = ko(&json!({ "size": 120 * mb, "stored": 2 * mb }));
        assert!(
            cell.starts_with("120.0 MB") && cell.contains("2.0 MB") && cell.contains("새로"),
            "{cell}"
        );
        assert_eq!(ko(&json!({ "size": 120 * mb, "stored": 120 * mb })), "120.0 MB");
        assert_eq!(
            ko(&json!({ "size": 120 * mb })),
            "120.0 MB",
            "an older daemon does not send stored"
        );
        assert_eq!(ko(&json!({ "size": 120 * mb, "stored": "2" })), "120.0 MB");
        assert_eq!(ko(&json!({})), "");
    }
}
