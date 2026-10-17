//! `update` — 사바쨩 자체 업데이트 (확인 → 받기 → 적용)
//!
//! 모듈·익스텐션만 바뀌면 데몬이 바로 적용한다. 데몬·GUI·CLI·업데이터가 끼면
//! `requires_updater`가 오고, 데몬이 업데이터를 띄운 뒤 스스로 꺼진다.
//! 그때 CLI도 바로 끝낸다 (실행 중인 CLI 파일은 교체되지 않으므로).

use crate::cli::UpdateCmd;
use crate::cmd::{guess, split_pairs};
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::{json, Map, Value};
use std::time::Duration;

pub async fn run(ctx: &Ctx, cmd: UpdateCmd) -> Result<()> {
    match cmd {
        UpdateCmd::Status => {
            let r = ctx.api.update_status().await?;
            ctx.out.data(&r, print_status);
            Ok(())
        }
        UpdateCmd::Check => {
            let spin = crate::progress::spinner(&t!("update.checking"));
            let r = ctx.api.update_check().await;
            spin.finish_and_clear();
            let r = r?;
            ctx.out.data(&r, |r| {
                print_status(r);
                if r.get("updates_available").and_then(Value::as_u64).unwrap_or(0) > 0 {
                    println!();
                    println!(
                        "{}",
                        output::dim("saba-chan-cli update download && saba-chan-cli update apply")
                    );
                }
            });
            Ok(())
        }
        UpdateCmd::Download { components } => download(ctx, &components).await,
        UpdateCmd::Apply { components } => apply(ctx, &components).await,
        UpdateCmd::Last { clear } => {
            if clear {
                let r = ctx.api.clear_update_last_result().await?;
                ctx.out.done(&t!("update.last_cleared"), &r);
                return Ok(());
            }
            let r = ctx.api.update_last_result().await?;
            let res = r.get("result").cloned().unwrap_or(Value::Null);
            ctx.out.data(&res, |v| {
                if v.is_null() {
                    println!("{}", output::dim(&t!("update.no_last")));
                } else {
                    crate::cmd::server::print_value(v);
                }
            });
            Ok(())
        }
        UpdateCmd::Integrity => {
            let spin = crate::progress::spinner(&t!("update.verifying"));
            let r = ctx.api.update_integrity().await;
            spin.finish_and_clear();
            let r = r?;
            ctx.out.data(&r, |r| {
                let comps = r
                    .get("components")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                // 업데이터가 설치한 컴포넌트만 기록이 있다 (설치기로 처음 깐 것은 빠진다)
                if comps.is_empty() {
                    println!("{}", output::dim(&t!("update.integrity_empty")));
                    return;
                }
                let mut t = output::table(&[t!("col.name"), t!("col.state"), t!("col.what")]);
                for c in &comps {
                    let st = s(c, "status");
                    let mark = match st {
                        "Verified" => console::style(t!("update.int_verified")).green().to_string(),
                        "FileNotFound" => console::style(t!("update.int_missing")).yellow().to_string(),
                        "Tampered" => console::style(t!("update.int_changed")).red().to_string(),
                        "Error" => console::style(t!("update.int_error")).red().to_string(),
                        other => output::dim(other),
                    };
                    let mut what = s(c, "message").to_string();
                    if let Some(f) = c
                        .get("file_path")
                        .and_then(Value::as_str)
                        .filter(|f| !f.is_empty() && !what.contains(*f))
                    {
                        what = format!("{} ({})", what, f);
                    }
                    let name = if s(c, "display_name").is_empty() {
                        s(c, "component")
                    } else {
                        s(c, "display_name")
                    };
                    t.add_row(vec![name.to_string(), mark, what]);
                }
                output::print_table(t);
                println!();
                println!(
                    "{}",
                    t!(
                        "update.integrity_summary",
                        verified = output::show(r.get("verified").unwrap_or(&Value::Null)),
                        failed = output::show(r.get("failed").unwrap_or(&Value::Null)),
                        skipped = output::show(r.get("skipped").unwrap_or(&Value::Null))
                    )
                );
            });
            Ok(())
        }
        UpdateCmd::Config { pairs } => config(ctx, &pairs).await,
    }
}

fn print_status(r: &Value) {
    let comps = r
        .get("components")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut t = output::table(&[
        t!("col.name"),
        t!("update.current"),
        t!("update.latest"),
        t!("col.state"),
    ]);
    for c in &comps {
        let flag = |k: &str| c.get(k).and_then(Value::as_bool) == Some(true);
        let state = if flag("downloaded") {
            console::style(t!("update.ready")).cyan().to_string()
        } else if flag("update_available") {
            console::style(t!("common.update_available")).yellow().to_string()
        } else if !flag("installed") {
            output::dim(&t!("update.not_installed"))
        } else {
            output::dim(&t!("update.up_to_date"))
        };
        let name = if s(c, "display_name").is_empty() {
            s(c, "component")
        } else {
            s(c, "display_name")
        };
        t.add_row(vec![
            name.to_string(),
            output::show(c.get("current_version").unwrap_or(&Value::Null)),
            output::show(c.get("latest_version").unwrap_or(&Value::Null)),
            state,
        ]);
    }
    output::print_table(t);
    let err = s(r, "error");
    if !err.is_empty() {
        eprintln!("{} {}", console::style("!").yellow().bold(), err);
    }
    if let Some(at) = r.get("last_check").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        println!("{}", output::dim(&t!("update.last_check", at = at)));
    }
}

async fn download(ctx: &Ctx, components: &[String]) -> Result<()> {
    let bar = crate::progress::bytes_bar(&t!("update.downloading"));
    let req = ctx.api.update_download(components);
    tokio::pin!(req);
    let r = loop {
        tokio::select! {
            r = &mut req => break r,
            _ = tokio::time::sleep(Duration::from_millis(400)) => {
                if let Ok(p) = ctx.api.update_progress().await {
                    if let Some(total) = p.get("total_bytes").and_then(Value::as_u64).filter(|t| *t > 0) {
                        bar.set_length(total);
                    }
                    bar.set_position(p.get("bytes_received").and_then(Value::as_u64).unwrap_or(0));
                    let c = s(&p, "component");
                    if !c.is_empty() {
                        bar.set_message(c.to_string());
                    }
                }
            }
        }
    };
    bar.finish_and_clear();
    let r = r?;
    let errors: Vec<String> = r
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            if e.is_string() {
                output::show(e)
            } else {
                format!("{}: {}", s(e, "component"), s(e, "error"))
            }
        })
        .collect();
    for e in &errors {
        ctx.out.warn(e);
    }
    let n = r.get("downloaded").and_then(Value::as_array).map_or(0, |a| a.len());
    ctx.out.done(&t!("update.downloaded", n = n), &r);
    if n > 0 {
        ctx.out.hint("saba-chan-cli update apply");
    }
    Ok(())
}

async fn apply(ctx: &Ctx, components: &[String]) -> Result<()> {
    let spin = crate::progress::spinner(&t!("update.applying"));
    let r = ctx.api.update_apply(components).await;
    spin.finish_and_clear();
    let r = r?;
    for e in r.get("errors").and_then(Value::as_array).into_iter().flatten() {
        ctx.out.warn(&output::show(e));
    }
    if r.get("requires_updater").and_then(Value::as_bool) != Some(true) {
        let applied: Vec<String> = r
            .get("applied")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(output::show)
            .collect();
        if applied.is_empty() {
            ctx.out.done(&t!("update.nothing_to_apply"), &r);
        } else {
            ctx.out.done(&t!("update.applied", list = applied.join(", ")), &r);
        }
        return Ok(());
    }
    let list: Vec<String> = r
        .get("needs_updater")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(output::show)
        .collect();
    ctx.ask.ensure(&t!("update.confirm_restart", list = list.join(", ")))?;
    // 적용 뒤 데몬을 다시 띄우게 한다 (GUI 없이 쓰는 PC도 있다)
    let relaunch: Vec<String> = crate::bootstrap::daemon_binary()
        .map(|p| vec![p.display().to_string(), "--spawned".to_string()])
        .unwrap_or_default();
    let launched = ctx.api.launch_updater(&relaunch).await?;
    ctx.out.done(
        &t!("update.updater_launched"),
        &json!({ "apply": r, "launch": launched }),
    );
    ctx.out.hint(&t!("update.after_hint"));
    // 이 CLI 파일도 교체 대상일 수 있으니 셸 안에서도 여기서 끝낸다
    output::flush();
    std::process::exit(0);
}

async fn config(ctx: &Ctx, pairs: &[String]) -> Result<()> {
    if pairs.is_empty() {
        let r = ctx.api.update_config().await?;
        let cfg = r.get("config").cloned().unwrap_or(Value::Null);
        ctx.out.data(&cfg, |c| {
            let mut rows = Vec::new();
            crate::cmd::flatten(c, "", &mut rows);
            output::print_kv(
                &rows
                    .iter()
                    .map(|(k, v)| (k.clone(), output::show(v)))
                    .collect::<Vec<_>>(),
            );
        });
        return Ok(());
    }
    // 데몬이 모르는 키(예전 GitHub 설정 등)는 조용히 무시되니 저장 전에 막는다
    let current = ctx
        .api
        .update_config()
        .await
        .ok()
        .and_then(|r| r.get("config").cloned());
    let mut patch = Map::new();
    for (k, raw) in split_pairs(pairs)? {
        if let Some(known) = current
            .as_ref()
            .and_then(Value::as_object)
            .filter(|c| !c.contains_key(&k))
        {
            let keys: Vec<&str> = known.keys().map(String::as_str).collect();
            return Err(crate::error::usage(t!(
                "update.unknown_key",
                key = k,
                keys = keys.join(", ")
            )));
        }
        // 문자열 칸은 숫자처럼 보여도 문자열로 (channel 등)
        let v = match k.as_str() {
            "install_root" | "source" | "relay_url" | "channel" => Value::String(raw),
            _ => guess(&raw),
        };
        patch.insert(k, v);
    }
    let r = ctx.api.save_update_config(Value::Object(patch)).await?;
    ctx.out.done(&t!("update.config_saved"), &r);
    Ok(())
}
