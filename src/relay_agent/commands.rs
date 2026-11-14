//! 릴레이 명령 실행: 액션 카탈로그 호출, 자동 실행, 원격 MCP 호출자, 권한 명령

use super::*;

/// 릴레이의 액션 요청 한 건
pub(super) async fn action_request(server: &crate::ipc::IPCServer, action: &str, payload: &Value, caller: &crate::actions::Caller) -> Result<Value, crate::actions::ActionError> {
    let id = payload.get("id").and_then(Value::as_str).unwrap_or_default();
    let instance = payload.get("instance").and_then(Value::as_str);
    let input = payload.get("input").cloned().unwrap_or(Value::Null);
    match action {
        // 여러 단계 계획 (I1)
        "plan" if payload.get("steps").is_some() => crate::actions::plan_steps(server, &payload["steps"], caller).await,
        // 문장 명령 ("terra-a 공지 안녕"): 데몬이 해석한다
        "plan" if payload.get("text").is_some() => {
            let view = crate::actions::plan_text(server, payload["text"].as_str().unwrap_or_default(), caller).await?;
            auto_run(server, payload, view, caller).await
        }
        "plan" => {
            let view = crate::actions::plan(server, id, instance, &input, caller).await?;
            auto_run(server, payload, view, caller).await
        }
        // 확인을 묻기 전의 권한 검사: 계획을 남기지 않는다 (`check_first`)
        "check" if payload.get("text").is_some() => crate::actions::check_text(server, payload["text"].as_str().unwrap_or_default(), caller).await,
        "check" => crate::actions::check(server, id, instance, &input, caller).await,
        "run_plan" => {
            let plan_id = payload.get("planId").and_then(Value::as_str).unwrap_or_default();
            crate::actions::run(server, id, instance, &input, Some(plan_id), caller).await
        }
        _ => crate::actions::query(server, id, instance, &input, caller).await,
    }
}

/// 계획 뒤 바로 실행 (`autoRun: {except?: [액션 id]}`): 확인이 필요 없고 `except`에 없는 액션이면 `run_plan`을 따로 받지 않고
/// 이어서 실행한다. 답은 `run_plan`의 답에 `autoRun: true`와 계획의 `planId`·`actionId`·`instance`를 붙인 것이다.
/// 확인이 필요하거나 실행할 것이 없으면 계획을 그대로 준다 (릴레이가 지금처럼 묻거나 `run_plan`을 보낸다)
async fn auto_run(server: &crate::ipc::IPCServer, payload: &Value, view: Value, caller: &crate::actions::Caller) -> Result<Value, crate::actions::ActionError> {
    let Some(plan_id) = auto_run_plan(payload, &view).map(str::to_string) else { return Ok(view) };
    let tag = |data: &mut Value| {
        if !data.is_object() {
            *data = json!({});
        }
        data["autoRun"] = json!(true);
        for key in ["planId", "actionId", "instance"] {
            if data.get(key).is_none() {
                data[key] = view[key].clone();
            }
        }
    };
    match crate::actions::run(server, "", None, &Value::Null, Some(&plan_id), caller).await {
        Ok(mut done) => {
            tag(&mut done);
            Ok(done)
        }
        Err(mut e) => {
            tag(&mut e.data);
            Err(e)
        }
    }
}

/// 이어서 실행할 계획의 planId: `autoRun`을 받았고, 확인이 필요 없고(`needsConfirm: false`), 실행할 액션이 `except`에 없을 때만
pub(super) fn auto_run_plan<'a>(payload: &Value, view: &'a Value) -> Option<&'a str> {
    let opts = payload.get("autoRun").filter(|v| v.is_object())?;
    let (plan_id, action_id) = (view["planId"].as_str()?, view["actionId"].as_str()?);
    let excepted = opts["except"].as_array().is_some_and(|list| list.iter().any(|v| v.as_str() == Some(action_id)));
    (view["needsConfirm"] == json!(false) && !excepted).then_some(plan_id)
}

/// 원격 MCP 요청자. 릴레이가 확인한 이 PC의 주인(`hostOwnerDiscordId`)과 요청자가 같을 때만 받는다.
/// 길드가 실려 오면 디스코드 요청이므로 MCP로 받지 않는다
pub(super) fn mcp_caller(cmd: &Value, payload: &Value, locale: String) -> Result<crate::actions::Caller, crate::actions::ActionError> {
    let id = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let user = id(cmd.get("requestedBy"));
    let owner = id(cmd.get("hostOwnerDiscordId"));
    let guild = id(cmd.get("guildId"));
    match (user, owner, guild) {
        (Some(user), Some(owner), None) if user == owner => {
            let user_name = id(payload.get("userName"));
            Ok(crate::actions::Caller::mcp(user, owner, id(payload.get("confirmedBy")), user_name, locale))
        }
        _ => Err(crate::actions::ActionError { status: 403, code: "PERMISSION_DENIED", message: "Permission denied".into(), data: Value::Null }),
    }
}

/// 릴레이 결과 모양: 성공 `{success:true, data}`, 실패 `{success:false, data:{code, error, args?, field?, reason}}`.
/// `reason`은 그 실패 코드의 짧은 이유(`Text`)이고, 릴레이에 그 문구가 없으면 `error`를 보인다
pub(super) fn action_result(result: Result<Value, crate::actions::ActionError>) -> Value {
    let mut out = match result {
        Ok(v) => json!({ "success": v.get("success").and_then(Value::as_bool).unwrap_or(true), "data": v }),
        Err(e) => {
            let mut data = json!({ "code": e.code, "error": e.message });
            data["reason"] = crate::chat::reason_or(e.code, &e.data, e.message.clone()).to_json();
            if let Value::Object(extra) = &e.data {
                for (k, v) in extra {
                    data[k] = v.clone();
                }
            }
            json!({ "success": false, "data": data })
        }
    };
    crate::actions::strip_local_details(&mut out);
    out
}

/// 디스코드 `/사바쨩 권한 부여·해제·기본`과 `/설정` 명령별 레벨: 노드 주인만 바꿀 수 있다. 챗봇 경로는 받지 않는다
pub(super) async fn permission_action(server: &crate::ipc::IPCServer, who: &crate::chat::permissions::Requester, payload: &Value) -> Value {
    use crate::chat::permissions::Change;
    let fail = |code: &str| json!({ "success": false, "data": { "code": code } });
    let Some(guild) = who.guild_id.as_deref() else { return fail("MISSING_GUILD") };
    if who.via_chatbot || who.owner_id.is_none() || who.owner_id != who.user_id {
        return fail("NOT_OWNER");
    }
    let discord_id = payload.get("discordId").and_then(Value::as_str).unwrap_or("").to_string();
    let level = payload.get("level").and_then(Value::as_i64);
    let change = match payload.get("op").and_then(Value::as_str) {
        Some("grant") => match level {
            Some(l) => Change::Member { discord_id, level: Some(l) },
            None => return fail("INVALID_LEVEL"),
        },
        Some("revoke") => Change::Member { discord_id, level: None },
        Some("default_level") => match level {
            Some(l) => Change::DefaultLevel(l),
            None => return fail("INVALID_LEVEL"),
        },
        // `/설정` 명령별 레벨: `{key: "<인스턴스>/<명령>", level: 레벨|null}` (null은 덮어쓰기를 지운다)
        Some("command_level") => {
            let key = payload.get("key").and_then(Value::as_str).unwrap_or("").to_string();
            match payload.get("level") {
                Some(Value::Null) | None => Change::CommandLevel { key, level: None },
                Some(_) if level.is_none() => return fail("INVALID_LEVEL"),
                Some(_) => Change::CommandLevel { key, level },
            }
        }
        _ => return fail("UNKNOWN_OP"),
    };
    let owner = who.owner_id.clone();
    let result = server
        .config_store
        .update_node_settings(guild, |node| match crate::chat::permissions::apply(node, &change, owner.as_deref()) {
            Ok(()) => (Ok(node.clone()), true),
            Err(code) => (Err(code), false),
        })
        .await;
    match result {
        Ok(Ok(node)) => {
            // 저장을 마친 뒤의 시각: 릴레이는 이보다 먼저 읽은 heartbeat 권한을 버린다
            let mut data = permission_entry(&Value::Object(node), &remote_targets(server).await);
            data["permissionsAt"] = json!(now_ms());
            json!({ "success": true, "data": data })
        }
        Ok(Err(code)) => fail(&code.to_uppercase()),
        Err(e) => json!({ "success": false, "data": { "code": "SAVE_FAILED", "error": e.to_string() } }),
    }
}
