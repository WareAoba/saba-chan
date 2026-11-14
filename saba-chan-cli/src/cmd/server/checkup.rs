//! `server validate`·`diagnose`·`agree`·`reset`·`reorder`: 점검·약관·초기화·순서

use super::*;

/// 실패여도 issues가 있으면 결과로 보여 준다 (validate는 치명 문제가 있으면 success:false)
pub(super) async fn checkup(ctx: &Ctx, r: Result<Value>, ok_msg: &str, server: &str) -> Result<()> {
    let (value, failed_flag) = match r {
        Ok(v) => (v, false),
        Err(e) => match e.downcast_ref::<CliError>() {
            // JSON이면 데몬 오류를 그대로 (error_code·issues·solution이 다 들어 있다)
            Some(CliError::Api { body, .. }) if body.get("issues").is_some() && !ctx.out.json => (body.clone(), true),
            _ => return Err(e),
        },
    };
    ctx.out.data(&value, |v| {
        if !crate::cmd::print_issues(v) {
            say!("{} {}", console::style("✓").green().bold(), ok_msg);
        }
    });
    // 동의하지 않은 모듈 약관이 있으면 바로 풀 수 있는 명령을 알려 준다
    if value.get("error_code").and_then(Value::as_str) == Some("agreement_required") {
        ctx.out.hint(&format!(
            "saba-chan-cli server agree {}",
            shlex::try_quote(server).unwrap_or_else(|_| server.into())
        ));
    }
    if failed_flag {
        return Err(CliError::Failed(t!("validate.blocked")).into());
    }
    Ok(())
}

pub(super) async fn validate(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.validate(&sv.id).await;
    checkup(ctx, r, &t!("validate.ok", name = sv.name), &sv.name).await
}

pub(super) async fn diagnose(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.diagnose(&sv.id).await;
    checkup(ctx, r, &t!("diagnose.ok", name = sv.name), &sv.name).await
}

/// 데몬이 실제로 띄울 줄 (서버는 켜지 않는다). 형식이 틀린 인자는 경고로
pub(super) async fn launch_preview(ctx: &Ctx, query: Option<&str>, extra: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.launch_preview(&sv.id, extra).await?;
    ctx.out.data(&r, |r| {
        if r.get("has_launch_args").and_then(Value::as_bool) == Some(false) {
            say!("{}", dim(&t!("launch_preview.no_args", name = sv.name)));
        }
        match r.get("command_line").and_then(Value::as_str) {
            Some(line) if r.get("success").and_then(Value::as_bool) == Some(true) => say!("{}", line),
            _ => say!("{} {}", console::style("✗").red(), s(r, "message")),
        }
        for e in r.get("errors").and_then(Value::as_array).into_iter().flatten() {
            let what = e.get("arg").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| show(&e["got"]));
            say!("{} {}", console::style("!").yellow(), t!("launch_preview.invalid_format", arg = what, field = s(e, "field")));
        }
    });
    Ok(())
}

/// 모듈이 선언한 약관 중 아직 동의하지 않은 것을 원문 주소와 함께 보여 주고, 확인을 받아 동의한다
pub(super) async fn agree(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let inst = ctx.api.instance(&sv.id).await?;
    let pending = pending_agreements(&inst);
    if pending.is_empty() {
        ctx.out.done(&t!("agree.none", name = sv.name), &json!({ "success": true, "agreements": inst["agreements"] }));
        return Ok(());
    }
    show_agreements(ctx, &pending);
    let names: Vec<&str> = pending.iter().map(|a| s(a, "label")).collect();
    ctx.ask.agreed(&t!("agree.confirm", name = sv.name, names = names.join(", ")))?;
    let r = accept(ctx, &sv.id, &pending).await?;
    ctx.out.done(&t!("agree.done", name = sv.name), &r);
    Ok(())
}

/// 만들 때 고를 수 있는 값: 사바쨩이 설치할 수 있는 값(`install_options`), 없으면 `options` 전부 (GUI와 같다)
pub(super) fn install_choices(field: &Value) -> Vec<String> {
    let list = field.get("install_options").filter(|v| v.is_array()).or_else(|| field.get("options"));
    list.and_then(Value::as_array).map(|o| o.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// 설정 이름: 모듈 locales `settings.<칸>.label` → `label` → `name`
pub(super) fn setting_label(locales: Option<&Value>, field: &Value) -> String {
    let localized = locales.and_then(|l| l.pointer(&format!("/settings/{}/label", s(field, "name")))).and_then(Value::as_str);
    let label = localized.filter(|l| !l.is_empty()).unwrap_or(s(field, "label"));
    if label.is_empty() { s(field, "name") } else { label }.to_string()
}

/// 선택지 이름: 모듈 locales `settings.<칸>.options.<값>`, 없으면 값 그대로
pub(super) fn option_label(locales: Option<&Value>, field: &Value, value: &str) -> String {
    let pointer = format!("/settings/{}/options/{}", s(field, "name"), value);
    locales.and_then(|l| l.pointer(&pointer)).and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or(value).to_string()
}

/// 모듈이 선언한 약관 (모듈 목록 `agreements`)을 locales 이름과 함께. 만들기 전에 묻는다 (GUI와 같다)
pub(super) fn module_agreements(module: &Value, locales: Option<&Value>) -> Vec<Value> {
    module["agreements"].as_array().into_iter().flatten().map(|a| {
        let id = s(a, "id");
        let label = locales.and_then(|l| l.pointer(&format!("/agreements/{}", id))).and_then(Value::as_str).unwrap_or(id);
        json!({ "id": id, "label": label, "url": a.get("url").cloned().unwrap_or(Value::Null) })
    }).collect()
}

/// 이 데몬의 OS·CPU에서 돌지 않는 모듈이면 그 이유 (`platform_supported`, `unsupported_reason`). 돌면 None
pub(super) fn platform_refusal(module: &Value) -> Option<String> {
    if module.get("platform_supported").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let code = if s(module, "unsupported_reason") == "arch" { "module_arch_unsupported" } else { "module_platform_unsupported" };
    crate::i18n::try_t(&format!("module_errors.{code}")).or_else(|| crate::i18n::try_t("module_errors.module_platform_unsupported"))
}

/// 예/아니오 칸 (익스텐션 AddServer.options, 만들 때 고르는 설정). `--yes`는 확인을 넘길 뿐 값을 정하지 않으므로 기본값을 쓴다
pub(super) fn bool_option(ask: &crate::prompt::Prompt, label: &str, default: &Value) -> Result<Value> {
    if ask.enabled && !ask.yes {
        Ok(Value::Bool(ask.confirm(label, default.as_bool().unwrap_or(false))?))
    } else {
        Ok(default.clone())
    }
}

/// 모듈이 선언했지만 아직 동의하지 않은 약관 (인스턴스 응답의 `agreements`)
pub(super) fn pending_agreements(inst: &Value) -> Vec<Value> {
    inst["agreements"].as_array().into_iter().flatten().filter(|a| a["accepted"] == json!(false)).cloned().collect()
}

/// `--accept`로 댔지만 이 서버의 약관에 없는 id (오타)
pub(super) fn unknown_accepts<'a>(inst: &Value, named: &'a [String]) -> Vec<&'a str> {
    let agreements = inst["agreements"].as_array().cloned().unwrap_or_default();
    named.iter().map(String::as_str).filter(|n| !agreements.iter().any(|a| s(a, "id") == *n)).collect()
}

pub(super) fn show_agreements(ctx: &Ctx, pending: &[Value]) {
    if !ctx.out.json {
        for a in pending {
            say!("  {}  {}", console::style(s(a, "label")).bold(), s(a, "url"));
        }
    }
}

pub(super) async fn accept(ctx: &Ctx, id: &str, pending: &[Value]) -> Result<Value> {
    let ids: Vec<String> = pending.iter().map(|a| s(a, "id").to_string()).collect();
    ctx.api.accept_agreements(id, &ids).await
}

/// 동의할 약관을 나눈다: `--accept`로 id를 댄 것(바로 동의)과 사람에게 물을 것.
/// 물을 수 없으면(`--json`, 터미널 아님) 묻지 않는다. `--yes`는 약관 동의로 치지 않는다
pub(super) fn split_agreements(pending: &[Value], named: &[String], can_ask: bool) -> (Vec<Value>, Vec<Value>) {
    let (by_name, rest): (Vec<Value>, Vec<Value>) = pending.iter().cloned().partition(|a| named.iter().any(|n| n == s(a, "id")));
    (by_name, if can_ask { rest } else { Vec::new() })
}

/// 만든 서버에 동의할 약관이 있으면 GUI처럼 만들 때 묻는다. 동의하지 않았거나 실패해도 서버 만들기는
/// 이어 가고 `server agree`를 안내한다 (첫 start가 약관 때문에 막히지 않게, 만든 서버의 남은 단계는 그대로)
pub(super) async fn offer_agreements(ctx: &Ctx, id: &str, name: &str, named: &[String]) {
    let Ok(inst) = ctx.api.instance(id).await else {
        return;
    };
    let unknown = unknown_accepts(&inst, named);
    if !unknown.is_empty() {
        let known: Vec<&str> = inst["agreements"].as_array().into_iter().flatten().map(|a| s(a, "id")).collect();
        ctx.out.warn(&t!("agree.unknown_id", ids = unknown.join(", "), known = known.join(", ")));
    }
    let pending = pending_agreements(&inst);
    if pending.is_empty() {
        return;
    }
    let (by_name, ask) = split_agreements(&pending, named, ctx.ask.enabled);
    let mut chosen = by_name;
    if !ask.is_empty() {
        show_agreements(ctx, &ask);
        let names: Vec<&str> = ask.iter().map(|a| s(a, "label")).collect();
        if ctx.ask.consent(&t!("agree.confirm", name = name, names = names.join(", "))) {
            chosen.extend(ask);
        }
    }
    let accepted = if chosen.is_empty() {
        false
    } else {
        match accept(ctx, id, &chosen).await {
            Ok(_) => true,
            Err(e) => {
                ctx.out.warn(&e.to_string());
                false
            }
        }
    };
    if accepted && !ctx.out.json {
        say!("{} {}", console::style("✓").green().bold(), t!("agree.done", name = name));
    }
    if !accepted || chosen.len() < pending.len() {
        ctx.out.hint(&format!("saba-chan-cli server agree {}", shell_word(name)));
    }
}

pub(super) async fn reset(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server_exact(ctx, query).await?;
    if status_of(&runtime(ctx, &sv).await?) == "running" {
        return Err(failed(t!("err.hint.server_running")));
    }
    if let Ok(m) = module_info(ctx, &sv.module).await {
        crate::cmd::module::ensure_supported(&m, "reset_server")?;
    }
    let locales = module_locales(ctx, &sv.module).await;
    ctx.ask.ensure(&reset_question(&sv.name, locales.as_ref()))?;
    let r = ctx.api.reset_server(&sv.id).await?;
    ctx.out.done(&t!("reset.done", name = sv.name), &r);
    Ok(())
}

/// 초기화 확인 문구. 무엇이 지워지는지는 게임마다 달라 모듈 locales의 `reset.description`을 덧붙인다 (GUI와 같다)
pub(super) fn reset_question(name: &str, locales: Option<&Value>) -> String {
    let question = t!("reset.confirm", name = name);
    match locales.and_then(|l| l.pointer("/reset/description")).and_then(Value::as_str).filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("{}\n{}", question, d),
        None => question,
    }
}

pub(super) async fn reorder(ctx: &Ctx, names: &[String]) -> Result<()> {
    let all = resolve::servers(ctx).await?;
    let mut order = Vec::new();
    for n in names {
        let sv = resolve::server(ctx, Some(n)).await?;
        if !order.contains(&sv.id) {
            order.push(sv.id);
        }
    }
    // 적지 않은 서버는 원래 순서대로 뒤에
    let rest: Vec<String> = all
        .iter()
        .map(|s| s.id.clone())
        .filter(|id| !order.contains(id))
        .collect();
    order.extend(rest);
    let r = ctx.api.reorder(&order).await?;
    ctx.out.done(&t!("reorder.done"), &r);
    Ok(())
}
