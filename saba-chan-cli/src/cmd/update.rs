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
                say!("{}", integrity_summary(r));
            });
            Ok(())
        }
        UpdateCmd::Repair => repair(ctx).await,
        UpdateCmd::Rollback => rollback(ctx).await,
        UpdateCmd::Config { pairs } => config(ctx, &pairs).await,
    }
}

fn names(items: &[Value]) -> String {
    items.iter().map(component_name).collect::<Vec<_>>().join(", ")
}

/// `update repair`: 데몬이 점검하고, 바뀌었거나 없어진 구성요소만 지금 판 그대로 다시 받아 덮는다.
/// 데몬·GUI·업데이터가 끼면 업데이트와 같은 길(다시 시작을 묻고 업데이터)로 간다
async fn repair(ctx: &Ctx) -> Result<()> {
    let spin = crate::progress::spinner(&t!("update.repair_checking"));
    let plan = ctx.api.update_repair_plan().await;
    spin.finish_and_clear();
    let plan = plan?;
    let list = |key: &str| plan.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let (todo, unavailable) = (list("repair"), list("unavailable"));
    if !ctx.out.json {
        say!("{}", integrity_summary(&plan));
    }
    for u in &unavailable {
        ctx.out.warn(&t!("update.repair_unavailable", name = component_name(u)));
    }
    if todo.is_empty() {
        if !unavailable.is_empty() {
            return Err(crate::error::CliError::Partial { message: t!("update.repair_failed"), data: plan }.into());
        }
        ctx.out.data(&plan, |_| say!("{}", t!("update.nothing_to_repair")));
        return Ok(());
    }
    if !ctx.out.json {
        say!("{}", t!("update.repair_list", list = names(&todo)));
    }
    // 받는 동안 받기 시작한 구성요소를 한 줄씩 알린다 (진행률에 잡히지 않을 만큼 빨리 받은 것은 끝난 뒤에)
    let mut shown: Vec<String> = Vec::new();
    let show = |key: &str, shown: &mut Vec<String>| {
        if let Some(item) = todo.iter().find(|i| s(i, "component") == key).filter(|_| !shown.iter().any(|k| k == key)) {
            shown.push(key.to_string());
            if !ctx.out.json {
                say!("{}  {} {}", t!("update.downloading"), component_name(item), s(item, "installed"));
            }
        }
    };
    let req = ctx.api.update_repair();
    tokio::pin!(req);
    let r = loop {
        tokio::select! {
            r = &mut req => break r,
            _ = tokio::time::sleep(Duration::from_millis(400)) => {
                if let Ok(p) = ctx.api.update_progress().await {
                    show(s(&p, "component"), &mut shown);
                }
            }
        }
    };
    let r = crate::client::partial(r)?;
    for key in r.get("repaired").and_then(Value::as_array).into_iter().flatten().chain(r.get("needs_updater").and_then(Value::as_array).into_iter().flatten()) {
        show(key.as_str().unwrap_or_default(), &mut shown);
    }
    for e in r.get("errors").and_then(Value::as_array).into_iter().flatten() {
        ctx.out.warn(&output::show(e));
    }
    if r.get("requires_updater").and_then(Value::as_bool) == Some(true) {
        return hand_to_updater(ctx, r).await;
    }
    let repaired: Vec<Value> = r
        .get("repaired")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|k| todo.iter().find(|i| i.get("component") == Some(k)).cloned())
        .collect();
    let failed = r.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty()) || !unavailable.is_empty();
    if !repaired.is_empty() && !(failed && ctx.out.json) {
        ctx.out.done(&t!("update.repaired", list = names(&repaired)), &r);
    }
    if failed {
        return Err(crate::error::CliError::Partial { message: t!("update.repair_failed"), data: r }.into());
    }
    Ok(())
}

/// `정상 9 · 바뀜/읽기 실패 1 · 파일 없음 1` (`update integrity`와 같은 줄)
fn integrity_summary(r: &Value) -> String {
    t!(
        "update.integrity_summary",
        verified = output::show(r.get("verified").unwrap_or(&Value::Null)),
        failed = output::show(r.get("failed").unwrap_or(&Value::Null)),
        skipped = output::show(r.get("skipped").unwrap_or(&Value::Null))
    )
}

/// 되돌릴 수 있는 판을 알리는 줄. 남긴 판이 없으면 None
fn rollback_offer(r: &Value) -> Option<String> {
    (r.get("available").and_then(Value::as_bool) == Some(true)).then(|| {
        t!("update.rollback_available", version = s(r, "version"), current = s(r, "current"), at = local_time(s(r, "updated_at")))
    })
}

/// `update rollback`: 데몬이 업데이터를 띄워 직전 판으로 되돌린다 (데몬은 꺼지고 되돌린 뒤 다시 켜진다)
async fn rollback(ctx: &Ctx) -> Result<()> {
    let info = ctx.api.update_rollback_info().await?;
    let Some(offer) = rollback_offer(&info) else {
        ctx.out.data(&info, |_| say!("{}", t!("update.no_rollback")));
        return Ok(());
    };
    if !ctx.out.json {
        say!("{}", offer);
    }
    ctx.ask.ensure(&t!("update.rollback_confirm", version = s(&info, "version")))?;
    // 되돌린 뒤 데몬을 다시 띄우게 한다 (GUI 없이 쓰는 PC도 있다)
    let relaunch: Vec<String> = crate::bootstrap::daemon_binary()
        .map(|p| vec![p.display().to_string(), "--spawned".to_string()])
        .unwrap_or_default();
    let spin = crate::progress::spinner(&t!("update.rolling_back"));
    let r = ctx.api.update_rollback(&relaunch).await;
    spin.finish_and_clear();
    let r = r?;
    ctx.out.done(&t!("update.rollback_launched"), &r);
    ctx.out.hint(&t!("update.rollback_after"));
    // 이 셸은 되돌리는 CLI 파일로 떠 있고 데몬도 내려간다: 셸 안에서도 여기서 끝낸다
    output::flush();
    std::process::exit(0);
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
        let state = match component_state(c) {
            ComponentState::Ready => console::style(t!("update.ready")).cyan().to_string(),
            ComponentState::Update => console::style(t!("common.update_available")).yellow().to_string(),
            ComponentState::Reinstall => console::style(t!("update.reinstall_official")).yellow().to_string(),
            ComponentState::NotInstalled => output::dim(&t!("update.not_installed")),
            ComponentState::Latest => output::dim(&t!("update.up_to_date")),
        };
        t.add_row(vec![
            component_name(c),
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
        say!("{}", output::dim(&t!("update.last_check", at = local_time(at))));
    }
}

#[derive(Debug, PartialEq)]
enum ComponentState {
    Ready,
    Update,
    /// 버전은 같지만 공식 카탈로그 판이 아니라 다시 받는다 (업데이터 `reinstall_from_catalog`)
    Reinstall,
    NotInstalled,
    Latest,
}

fn component_state(c: &Value) -> ComponentState {
    let flag = |k: &str| c.get(k).and_then(Value::as_bool) == Some(true);
    if flag("downloaded") {
        ComponentState::Ready
    } else if !flag("installed") {
        ComponentState::NotInstalled
    } else if flag("update_available") && !s(c, "current_version").is_empty() && s(c, "current_version") == s(c, "latest_version") {
        ComponentState::Reinstall
    } else if flag("update_available") {
        ComponentState::Update
    } else {
        ComponentState::Latest
    }
}

/// 모듈·익스텐션은 사용자 언어로 "palworld 모듈", 나머지는 데몬이 준 이름
fn component_name(c: &Value) -> String {
    let key = s(c, "component");
    if let Some(id) = key.strip_prefix("module-") {
        return t!("update.name_module", name = id);
    }
    if let Some(id) = key.strip_prefix("ext-") {
        return t!("update.name_extension", name = id);
    }
    if s(c, "display_name").is_empty() { key } else { s(c, "display_name") }.to_string()
}

/// RFC 3339 시각을 이 PC 시간대의 `2026-11-08 18:03`으로. 읽지 못하면 그대로
fn local_time(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| at.to_string())
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
    hand_to_updater(ctx, r).await
}

/// 데몬·GUI·업데이터가 낀 적용(`requires_updater`): 다시 시작을 묻고 데몬에 업데이터를 띄우게 한 뒤, 다시 뜬 데몬의 결과를 보이고 끝낸다
async fn hand_to_updater(ctx: &Ctx, r: Value) -> Result<()> {
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
    let last = wait_for_result(ctx, before.as_deref()).await;
    if let Some(l) = last.as_ref().filter(|_| !ctx.out.json) {
        print_last(ctx, l);
    }
    let code = match apply_result(&r, &launched, last.as_ref(), failed) {
        Ok(v) => {
            if ctx.out.json {
                ctx.out.data(&v, |_| {});
            }
            0
        }
        Err(e) => {
            ctx.out.error(&e);
            if last.is_none() {
                ctx.out.hint(&t!("update.after_hint"));
            }
            e.exit_code()
        }
    };
    // 이 셸은 옛 CLI 파일로 떠 있으니 셸 안에서도 여기서 끝낸다
    output::flush();
    std::process::exit(code);
}

/// 적용·실행·새 결과를 하나로. 구성요소 실패, 업데이터 실패, 시간 초과는 그 내용을 실은 오류다
fn apply_result(apply: &Value, launch: &Value, last: Option<&Value>, failed: bool) -> std::result::Result<Value, crate::error::CliError> {
    let data = json!({ "apply": apply, "launch": launch, "last": last });
    let message = match last {
        None => t!("update.result_timeout"),
        Some(l) if failed || l.pointer("/result/success").and_then(Value::as_bool) != Some(true) => t!("update.apply_failed"),
        Some(_) => return Ok(data),
    };
    Err(crate::error::CliError::Partial { message, data })
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
    // 사용자 되돌리기는 릴리즈가 아니라 돌아간 판(`version`)과 그 전 판(`replaced_by`)을 보인다
    let reverted = s(res, "result") == "reverted";
    if let Some(v) = if reverted { res.get("version") } else { r.get("version") }.and_then(Value::as_str) {
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
    if reverted {
        rows.push((t!("col.what"), t!("update.last_reverted", version = s(res, "version"), from = s(res, "replaced_by"))));
    }
    // 다 되돌리지 못했으면 원문에 되살리는 차례가 있다
    if let Some(m) = res.get("message").and_then(Value::as_str).filter(|m| !m.is_empty() && !(reverted && ok)) {
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

    #[test]
    fn same_version_updates_read_as_official_reinstall_and_missing_parts_as_not_installed() {
        let c = |installed: bool, avail: bool, cur: &str, latest: &str| {
            json!({"installed": installed, "update_available": avail, "current_version": cur, "latest_version": latest})
        };
        assert_eq!(component_state(&c(true, true, "0.1.12", "0.1.12")), ComponentState::Reinstall);
        assert_eq!(component_state(&c(true, true, "0.1.11", "0.1.12")), ComponentState::Update);
        assert_eq!(component_state(&c(false, true, "", "0.1.13")), ComponentState::NotInstalled);
        assert_eq!(component_state(&c(true, false, "0.1.13", "0.1.13")), ComponentState::Latest);
        assert_eq!(local_time("not a time"), "not a time");
        assert!(!local_time("2026-11-08T09:03:53Z").contains('T'));
    }

    #[test]
    fn modules_and_extensions_are_named_in_the_users_language() {
        crate::i18n::init("en");
        assert!(component_name(&json!({"component": "module-palworld", "display_name": "Module: palworld"})).contains("palworld"));
        assert!(!component_name(&json!({"component": "module-palworld", "display_name": "Module: palworld"})).contains("Module:"));
        assert_eq!(component_name(&json!({"component": "cli", "display_name": "CLI"})), "CLI");
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

    /// 적용 뒤 실패나 시간 초과는 --json에서도 오류 문서({ok:false, error_code})로 끝나고, 적용 내용을 함께 싣는다
    #[test]
    fn a_failed_or_timed_out_apply_ends_as_an_error_document() {
        let apply = json!({ "applied": ["saba-core"], "errors": [] });
        let launch = json!({ "ok": true });
        let good = json!({ "result": { "timestamp": "t", "success": true } });
        let bad = json!({ "result": { "timestamp": "t", "success": false } });

        let v = apply_result(&apply, &launch, Some(&good), false).unwrap();
        assert_eq!(v["last"], good);

        for (last, failed) in [(Some(&bad), false), (Some(&good), true), (None, false)] {
            let doc = apply_result(&apply, &launch, last, failed).unwrap_err().to_json();
            assert_eq!(doc["ok"], json!(false));
            assert_eq!(doc["error_code"], json!("partial"));
            assert_eq!(doc["apply"], apply);
            assert_eq!(doc["last"], last.cloned().unwrap_or(Value::Null));
        }
    }

    /// 되돌릴 판이 있으면 그 판·지금 판·업데이트한 때를 한 줄로 알리고, 없으면 알리지 않는다
    #[test]
    fn the_rollback_offer_names_both_versions() {
        crate::i18n::init("en");
        let offer = rollback_offer(&json!({ "available": true, "version": "0.1.13", "current": "0.1.14", "updated_at": "2026-11-12T03:09:00Z" })).unwrap();
        assert!(offer.contains("0.1.13") && offer.contains("0.1.14") && offer.contains("2026-11-12 "), "{offer}");
        assert!(rollback_offer(&json!({ "available": false, "current": "0.1.14" })).is_none());
        let confirm = crate::i18n::try_t_in("ko", "update.rollback_confirm").unwrap();
        assert_eq!(crate::i18n::fill(&confirm, &[("version", "0.1.13".into())]), "0.1.13으로 되돌리면 사바쨩이 다시 시작돼요. 계속할까요?");
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

    /// 사용자 되돌리기 결과는 돌아간 판과 그 전 판을 사용자 언어로 보인다 (마커의 영어 원문이 아니라)
    #[test]
    fn a_rollback_result_reads_in_the_user_language() {
        let r = json!({ "ok": true, "version": null, "result": {
            "timestamp": "2026-11-12T03:09:00Z", "success": true, "updated_components": [], "result": "reverted",
            "version": "0.1.13", "replaced_by": "0.1.14", "message": "Rolled back from 0.1.14 to 0.1.13"
        } });
        let rows: Vec<(String, String)> = last_rows(&r).into_iter().map(|(k, v)| (k, console::strip_ansi_codes(&v).to_string())).collect();
        assert_eq!(rows[0], (t!("col.version"), "0.1.13".to_string()));
        let what = &rows.iter().find(|(k, _)| *k == t!("col.what")).unwrap().1;
        assert_eq!(what, &t!("update.last_reverted", version = "0.1.13", from = "0.1.14"));
        assert!(what.contains("0.1.14") && what.contains("0.1.13"), "{what}");
    }
}
