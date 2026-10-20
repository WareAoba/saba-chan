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
                    say!();
                    say!(
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
            let r = ctx.api.update_last_result(crate::i18n::lang()).await?;
            ctx.out.data(&r, |r| print_last(ctx, r));
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
                    say!("{}", output::dim(&t!("update.integrity_empty")));
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
                say!();
                say!(
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
    if comps.is_empty() {
        say!("{}", output::dim(&t!("update.not_checked")));
    }
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
    if !comps.is_empty() {
        output::print_table(t);
    }
    let err = s(r, "error");
    if !err.is_empty() {
        tell!("{} {}", console::style("!").yellow().bold(), err);
    }
    if let Some(at) = r.get("last_check").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        say!("{}", output::dim(&t!("update.last_check", at = at)));
    }
}

/// 받기 결과의 실패들. 구성요소를 고르지 않은 받기는 실패해도 `errors` 없이 `{ok:false, error}`만 온다
fn download_errors(r: &Value) -> Vec<String> {
    let mut errors: Vec<String> = r
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
    if errors.is_empty() && r.get("ok").and_then(Value::as_bool) == Some(false) {
        errors.push(match s(r, "error") {
            "" => t!("update.download_failed"),
            e => e.to_string(),
        });
    }
    errors
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
    let r = crate::client::partial(r)?;
    let errors = download_errors(&r);
    for e in &errors {
        ctx.out.warn(e);
    }
    let n = r.get("downloaded").and_then(Value::as_array).map_or(0, |a| a.len());
    if !errors.is_empty() {
        // 일부만 받았으면 받은 것은 사람에게 알리고 실패로 끝낸다 (--json은 오류 문서 하나에 결과를 싣는다)
        if n > 0 && !ctx.out.json {
            ctx.out.done(&t!("update.downloaded", n = n), &r);
        }
        return Err(crate::error::CliError::Partial { message: t!("update.download_failed"), data: r }.into());
    }
    ctx.out.done(&t!("update.downloaded", n = n), &r);
    if n > 0 {
        ctx.out.hint("saba-chan-cli update apply");
    }
    Ok(())
}

/// 업데이터 없이 끝난 적용의 결과: (알릴 문구, 실패 여부). 오류가 있으면 일부만 적용됐어도 실패다
fn apply_outcome(r: &Value) -> (Option<String>, bool) {
    let applied: Vec<String> = r
        .get("applied")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(output::show)
        .collect();
    let failed = r.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty());
    let done = if !applied.is_empty() {
        Some(t!("update.applied", list = applied.join(", ")))
    } else if failed {
        None
    } else {
        Some(t!("update.nothing_to_apply"))
    };
    (done, failed)
}

async fn apply(ctx: &Ctx, components: &[String]) -> Result<()> {
    let spin = crate::progress::spinner(&t!("update.applying"));
    let r = ctx.api.update_apply(components).await;
    spin.finish_and_clear();
    let r = crate::client::partial(r)?;
    for e in r.get("errors").and_then(Value::as_array).into_iter().flatten() {
        ctx.out.warn(&output::show(e));
    }
    if r.get("requires_updater").and_then(Value::as_bool) != Some(true) {
        let (done, failed) = apply_outcome(&r);
        if failed {
            // 일부 적용은 사람에게만 따로 알리고, --json은 오류 문서 하나에 결과를 싣는다
            if let Some(msg) = done.filter(|_| !ctx.out.json) {
                ctx.out.done(&msg, &r);
            }
            return Err(crate::error::CliError::Partial { message: t!("update.apply_failed"), data: r }.into());
        }
        if let Some(msg) = done {
            ctx.out.done(&msg, &r);
        }
        return Ok(());
    }
    // 실패한 구성요소가 있어도 코어 업데이트는 막지 않는다 (GUI와 같다): 알리고 종료 코드로 실패를 남긴다
    let (_, failed) = apply_outcome(&r);
    if failed {
        ctx.out.warn(&t!("update.apply_failed"));
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
    // 적용 전 결과를 기억해 두어야 업데이터가 남긴 새 결과를 가려낸다
    let before = ctx.api.update_last_result(crate::i18n::lang()).await.ok().and_then(|l| result_stamp(&l));
    let launched = ctx.api.launch_updater(&relaunch).await?;
    if !ctx.out.json {
        ctx.out.done(&t!("update.updater_launched"), &Value::Null);
    }
    // 데몬이 다시 뜨면 결과와 패치노트를 바로 보인다 (GUI 없는 PC). 실행 중인 CLI 파일은 업데이터가 비켜 두고 바꾼다
    let code = match wait_for_result(ctx, before.as_deref()).await {
        Some(last) => {
            let ok = last.pointer("/result/success").and_then(Value::as_bool) == Some(true);
            ctx.out.data(&json!({ "apply": r, "launch": launched, "last": last }), |_| print_last(ctx, &last));
            i32::from(failed || !ok)
        }
        None => {
            ctx.out.data(&json!({ "apply": r, "launch": launched, "last": null }), |_| ctx.out.warn(&t!("update.result_timeout")));
            ctx.out.hint(&t!("update.after_hint"));
            1
        }
    };
    // 이 셸은 옛 CLI 파일로 떠 있으니 셸 안에서도 여기서 끝낸다
    output::flush();
    std::process::exit(code);
}

/// 업데이터가 적용하고 데몬을 다시 띄우기까지 기다리는 최대 시간
const APPLY_WAIT: Duration = Duration::from_secs(15 * 60);

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

/// `update last`와 적용 뒤 기다리기가 함께 쓰는 결과 표시: 줄 + 앱 언어 패치노트
fn print_last(ctx: &Ctx, r: &Value) {
    let rows = last_rows(r);
    if rows.is_empty() {
        say!("{}", output::dim(&t!("update.no_last")));
        return;
    }
    output::print_kv(&rows);
    // 패치노트는 GUI처럼 앱 언어로 (없으면 데몬이 영어·한국어 순으로 고른다)
    if let Some(notes) = r.get("notes").and_then(Value::as_str).filter(|n| !n.trim().is_empty()) {
        say!();
        output::heading(&t!("update.notes"));
        say!("{}", notes.trim_end());
    }
    // 이미 확인한 결과(GUI가 패치노트를 열었다)는 확인 안내를 하지 않는다
    if r.get("acknowledged").and_then(Value::as_bool) != Some(true) {
        ctx.out.hint("saba-chan-cli update last --clear");
    }
}

/// 결과의 시각. 업데이터가 남긴 결과마다 다르므로 적용 전 결과와 새 결과를 가른다
fn result_stamp(r: &Value) -> Option<String> {
    r.get("result").and_then(|v| v.get("timestamp")).and_then(Value::as_str).map(str::to_string)
}

fn is_new_result(before: Option<&str>, r: &Value) -> bool {
    result_stamp(r).is_some_and(|t| before != Some(t.as_str()))
}

/// 업데이터가 끝나고 데몬이 다시 떠서 새 결과를 줄 때까지 기다린다. 그 사이 데몬이 꺼져 있는 오류는 기다림의 일부다
async fn wait_for_result(ctx: &Ctx, before: Option<&str>) -> Option<Value> {
    let spin = crate::progress::spinner(&t!("update.waiting_result"));
    let deadline = tokio::time::Instant::now() + APPLY_WAIT;
    let mut found = None;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Ok(r) = ctx.api.update_last_result(crate::i18n::lang()).await {
            if is_new_result(before, &r) {
                found = Some(r);
                break;
            }
        }
    }
    spin.finish_and_clear();
    found
}

/// 마지막 적용 결과 (`{result: {timestamp, success, updated_components, failed, message}, version}`). 결과가 없으면 빈 목록
fn last_rows(r: &Value) -> Vec<(String, String)> {
    let Some(res) = r.get("result").filter(|v| v.is_object()) else {
        return Vec::new();
    };
    let list = |key: &str| -> Vec<String> {
        res.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
    };
    let ok = res.get("success").and_then(Value::as_bool) == Some(true);
    let mut rows = Vec::new();
    if let Some(v) = r.get("version").and_then(Value::as_str) {
        rows.push((t!("col.version"), v.to_string()));
    }
    rows.push((
        t!("update.result"),
        if ok { console::style(t!("update.result_ok")).green().to_string() } else { console::style(t!("update.result_failed")).red().to_string() },
    ));
    rows.push((t!("update.when"), s(res, "timestamp").to_string()));
    let updated = list("updated_components");
    if !updated.is_empty() {
        rows.push((t!("update.updated"), updated.join(", ")));
    }
    for f in list("failed") {
        rows.push((t!("update.failed_one"), console::style(f).red().to_string()));
    }
    if let Some(m) = res.get("message").and_then(Value::as_str).filter(|m| !m.is_empty()) {
        rows.push((t!("col.what"), m.to_string()));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 고르지 않은 받기가 실패하면 `errors` 없이 `{ok:false, error}`만 온다: 0개 받음으로 끝내지 않는다 (cloud-review-426 P2)
    #[test]
    fn a_failed_full_download_is_a_failure() {
        assert_eq!(download_errors(&json!({ "ok": false, "error": "net down" })), vec!["net down".to_string()]);
        assert_eq!(download_errors(&json!({ "ok": false })).len(), 1);
        assert!(download_errors(&json!({ "ok": true, "downloaded": [], "count": 0 })).is_empty());
        assert_eq!(
            download_errors(&json!({ "ok": false, "errors": [{ "component": "gui", "error": "x" }] })),
            vec!["gui: x".to_string()]
        );
    }

    /// 데몬이 오류를 돌려준 적용은 "적용할 게 없어요"로 성공 처리하지 않는다 (cloud-review-300 P3-4)
    #[test]
    fn an_apply_with_errors_fails() {
        let (done, failed) = apply_outcome(&json!({ "applied": [], "errors": ["boom"] }));
        assert!(done.is_none() && failed);
        let (done, failed) = apply_outcome(&json!({ "applied": ["gui"], "errors": ["boom"] }));
        assert!(done.unwrap().contains("gui") && failed);
        let (done, failed) = apply_outcome(&json!({ "applied": [], "errors": [] }));
        assert!(done.is_some() && !failed);
    }

    /// 적용 뒤 기다리기는 업데이터가 남긴 새 결과에서만 끝난다: 적용 전 결과나 결과 없음은 아직이다
    #[test]
    fn waiting_ends_only_on_a_result_newer_than_the_one_before_apply() {
        let old = json!({ "result": { "timestamp": "2026-10-20T10:00:00Z", "success": true } });
        let new = json!({ "result": { "timestamp": "2026-10-20T13:40:00Z", "success": false } });
        let before = result_stamp(&old);
        assert_eq!(before.as_deref(), Some("2026-10-20T10:00:00Z"));
        assert!(!is_new_result(before.as_deref(), &old));
        assert!(!is_new_result(before.as_deref(), &json!({ "result": null })));
        assert!(is_new_result(before.as_deref(), &new));
        // 적용 전에 결과가 없었으면 처음 생긴 결과가 새 결과다
        assert!(is_new_result(None, &new));
        assert!(!is_new_result(None, &json!({})));
    }

    /// update last는 원시 객체 대신 버전·결과·구성요소·실패를 줄로 보인다 (C16)
    #[test]
    fn the_last_result_reads_as_rows() {
        let r = json!({ "ok": true, "version": "0.1.7", "notes": "고친 것", "result": {
            "timestamp": "2026-10-19T13:05:00Z", "success": false, "updated_components": ["saba-core", "cli"],
            "failed": ["gui: 파일이 쓰기 중"], "message": null
        } });
        let rows: Vec<(String, String)> = last_rows(&r).into_iter().map(|(k, v)| (k, console::strip_ansi_codes(&v).to_string())).collect();
        assert_eq!(rows[0], (t!("col.version"), "0.1.7".to_string()));
        assert_eq!(rows[1].1, t!("update.result_failed"));
        assert!(rows.contains(&(t!("update.updated"), "saba-core, cli".to_string())));
        assert!(rows.contains(&(t!("update.failed_one"), "gui: 파일이 쓰기 중".to_string())));
        assert!(last_rows(&json!({ "ok": true, "result": null })).is_empty());
    }
}
