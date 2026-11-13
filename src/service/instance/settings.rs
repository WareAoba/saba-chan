//! 인스턴스 설정 저장(PATCH)과 그 값 검사. 핸들러 `PATCH /api/instance/:id`·`/settings/write`가 부른다

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::ipc::IPCServer;
use crate::service::{Failure, Outcome};

/// 설정 값 하나가 잘못됐다는 400 응답 `invalid_format`: 이유는 `error`와 `message`에
fn invalid_field_message(field: &str, message: impl Into<String>) -> Failure {
    let message = message.into();
    Failure::new(StatusCode::BAD_REQUEST, json!({ "success": false, "error": message, "error_code": "invalid_format", "message": message, "fields": [{"field": field}] }))
}

/// PATCH의 `stop_timeout`(정지 대기 초): 비움(null·""·0)은 모듈 값으로 되돌리기, 아니면 1~3600 정수 (숫자 글자도 받는다)
pub(crate) fn stop_timeout_from(value: &Value) -> Result<Option<u64>, ()> {
    let n = match value {
        Value::Null => return Ok(None),
        Value::String(s) if s.trim().is_empty() => return Ok(None),
        Value::String(s) => s.trim().parse::<u64>().map_err(|_| ())?,
        Value::Number(n) => n.as_u64().ok_or(())?,
        _ => return Err(()),
    };
    match n {
        0 => Ok(None),
        1..=3600 => Ok(Some(n)),
        _ => Err(()),
    }
}

/// 이보다 작은 메모리 상한은 서버가 뜨지도 못하므로 받지 않는다
const MIN_MEMORY_LIMIT_MB: u64 = 256;

/// PATCH의 `memory_limit_mb`: 비움(null·""·0)은 제한 없음, 아니면 `MIN_MEMORY_LIMIT_MB` 이상의 MiB 정수.
/// 글자는 숫자(MiB)나 `4G`·`4096M`(`GB`·`MB`도, 대소문자 무관)
pub(crate) fn memory_limit_from(value: &Value) -> Result<Option<u64>, ()> {
    let n = match value {
        Value::Null => return Ok(None),
        Value::String(s) if s.trim().is_empty() => return Ok(None),
        Value::String(s) => {
            let t = s.trim().to_ascii_uppercase();
            let t = t.strip_suffix('B').unwrap_or(&t);
            let (digits, scale) = match t.strip_suffix('G') {
                Some(d) => (d, 1024),
                None => (t.strip_suffix('M').unwrap_or(t), 1),
            };
            digits.trim().parse::<u64>().ok().and_then(|n| n.checked_mul(scale)).ok_or(())?
        }
        Value::Number(n) => n.as_u64().ok_or(())?,
        _ => return Err(()),
    };
    match n {
        0 => Ok(None),
        n if n >= MIN_MEMORY_LIMIT_MB => Ok(Some(n)),
        _ => Err(()),
    }
}

/// 추가 실행 인자 입력: 문자열이면 `chat::split_quoted`로 나누고, 배열이면 문자열만. null은 비우기.
/// 줄바꿈·NUL이 든 인자는 받지 않는다
pub(crate) fn extra_args_from(value: &Value) -> Option<Vec<String>> {
    let args: Vec<String> = match value {
        Value::Null => Vec::new(),
        Value::String(s) => crate::chat::split_quoted(s),
        Value::Array(items) => items.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<_>>()?,
        _ => return None,
    };
    (!args.iter().any(|a| a.contains(['\r', '\n', '\0']))).then_some(args)
}

/// PATCH가 모듈 설정이 아니라 인스턴스 필드로 받는 이름 (그 밖의 이름은 `module_settings`로):
/// 공통 칸(`constants::INSTANCE_FIELDS`)과 익스텐션 값·전용 멤버 명단·자동 강퇴 닉네임, 받지만 쓰지 않는 이름
fn is_known_field(key: &str) -> bool {
    saba_chan_updater_lib::constants::INSTANCE_FIELDS.iter().any(|(k, _)| *k == key) || matches!(key, "extension_data" | "controllers" | "kick_players") || IGNORED_FIELDS.contains(&key)
}

/// 받지만 쓰지 않는 이름 (응답 `ignored`).
/// RCON은 보안상 쓰지 않는다: 예전 GUI·설정이 보내도 모듈 설정이나 게임 파일로 넘기지 않는다
const IGNORED_FIELDS: &[&str] = &["rcon_port", "rcon_password", "enable_rcon"];

/// 모듈이 선언하지 않아도 데몬이 `module_settings`에서 읽는 설정
fn is_daemon_setting(key: &str) -> bool {
    saba_chan_updater_lib::constants::DAEMON_SETTINGS.iter().any(|(k, _)| *k == key)
}

/// 포트 값: 1~65535의 수 또는 그런 숫자 문자열. 다른 값은 `None`
fn port_value(value: &Value) -> Option<u16> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
    .and_then(|p| u16::try_from(p).ok())
    .filter(|&p| p > 0)
}

/// 모듈 설정 변경을 PATCH 규칙으로 본다: 선언한 타입·범위·선택지·형식(400 `validation_failed`·`invalid_format`)과
/// 모르는 키(400 `unknown_setting`). 통과하면 `None`. 게임 설정 파일 쓰기(`PUT /properties`)도 같은 검사를 지난다
pub(crate) fn settings_rejection(supervisor: &crate::supervisor::Supervisor, instance: &crate::instance::ServerInstance, id: &str, settings: &Value) -> Option<Failure> {
    // ── 설정값 타입/범위 스키마 검증 ──
    if let Some(settings_obj) = settings.as_object() {
        if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
            if let Some(ref settings_meta) = module.metadata.settings {
                // 부분 변경이므로 보낸 필드만 검증한다
                // 숨겨진 설정은 required를 보지 않는다 (06 §8.2). 숨김은 저장된 값에 이번 요청을 얹어 계산한다
                let view = serde_json::to_value(instance).unwrap_or_default();
                let mut hidden = crate::validator::hidden_fields(&settings_meta.fields, |k| settings_obj.get(k).cloned().or_else(|| crate::instance::stored_setting(&view, instance, k)));
                // 설치된 버전에 없는 설정도 숨긴 설정이다 (이번 요청이 버전을 바꾸면 그 버전으로)
                let version = settings_obj.get("server_version").and_then(Value::as_str).or(instance.server_version.as_deref());
                hidden.extend(crate::validator::version_hidden(&settings_meta.fields, version));
                let errors: Vec<_> = crate::validator::validate_settings_patch(
                    &settings_meta.fields,
                    settings_obj,
                )
                .into_iter()
                .filter(|e| !(e.error_type == crate::validator::ValidationErrorType::Required && hidden.contains(&e.field)))
                .collect();
                if !errors.is_empty() {
                    let error_details: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                    let fields: Vec<Value> = errors
                        .iter()
                        .map(|e| json!({ "field": e.field, "type": e.error_type, "message": e.message }))
                        .collect();
                    tracing::warn!(
                        "Settings validation failed for instance {}: {:?}",
                        id, error_details
                    );
                    // 형식(`format`·`pattern`)이 틀렸으면 GUI가 "잘못된 형식입니다"를 띄운다
                    let code = if errors.iter().any(|e| e.error_type == crate::validator::ValidationErrorType::InvalidFormat) {
                        "invalid_format"
                    } else {
                        "validation_failed"
                    };
                    let error = json!({
                        "success": false,
                        "error": "Settings validation failed",
                        "error_code": code,
                        "message": "Settings validation failed",
                        "details": error_details,
                        "fields": fields,
                    });
                    return Some(Failure::new(StatusCode::BAD_REQUEST, error));
                }
            }
        }
    }

    // ── 모르는 키 거절 (오타가 조용히 module_settings에 쌓이지 않게) ──
    // 모듈이 설정 목록을 선언했을 때만 본다. 공통 필드와 데몬이 읽는 설정(`constants::DAEMON_SETTINGS`)은 받는다
    if let (Some(settings_obj), Ok(module)) = (settings.as_object(), supervisor.module_loader.get_module(&instance.module_name)) {
        if let Some(ref settings_meta) = module.metadata.settings {
            let unknown: Vec<&String> = settings_obj
                .keys()
                .filter(|k| !is_known_field(k) && !is_daemon_setting(k))
                .filter(|k| !settings_meta.fields.iter().any(|f| &f.name == *k))
                .collect();
            if !unknown.is_empty() {
                let fields: Vec<Value> = unknown.iter().map(|k| json!({ "field": k })).collect();
                let error = json!({
                    "success": false,
                    "error": format!("Unknown setting: {}", unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")),
                    "error_code": "unknown_setting",
                    "fields": fields,
                });
                return Some(Failure::new(StatusCode::BAD_REQUEST, error));
            }
        }
    }
    None
}

/// `PATCH /api/instance/:id` 본체: 보낸 키만 저장하고 게임 설정 파일에 옮긴다. `revision`(GET에서 받은 값)을 주면
/// 그사이 다른 곳에서 바뀐 인스턴스에는 쓰지 않고 409 `settings_changed`로 거절한다.
/// 답은 `{success, synced, restart_required, revision, sync_error?, ignored?}`
pub(crate) async fn update(state: &IPCServer, id: &str, mut settings: Value) -> Outcome {
    let revision = crate::config_store::patch::take_revision(&mut settings);
    let (task, _saving) = lock_for_save(state, id, &settings).await?;
    let mut supervisor = state.supervisor.write().await;

    let Some(instance) = supervisor.instance_store.get(id) else {
        return Err(Failure::coded(StatusCode::NOT_FOUND, "not_found", "Instance not found"));
    };
    let now = instance.settings_revision();
    if revision.is_some_and(|seen| seen != now) {
        let error = json!({ "success": false, "error": "The server settings were changed elsewhere; read them again", "error_code": "settings_changed", "revision": now });
        return Err(Failure::new(StatusCode::CONFLICT, error));
    }

    // 잠근 뒤 다시 본다: 폴더를 옮기는 다른 저장·작업은 위 잠금을 지나야 한다
    let moves_folder = settings.get("working_dir").is_some_and(|v| v.as_str() != instance.working_dir.as_deref());
    if moves_folder && task.is_none() {
        return Err(Failure::busy());
    }
    // 다른 인스턴스가 쓰는 폴더(서버 폴더나 인스턴스 폴더, 그 안이나 바깥)로는 옮기지 않는다: 만들 때처럼 한 폴더에 서버 둘을 두지 않는다.
    // 그 인스턴스의 서버·설치·업데이트가 옮긴 서버의 파일을 덮어쓴다
    if let Some(dir) = settings.get("working_dir").and_then(|v| v.as_str()).filter(|d| moves_folder && !d.is_empty()) {
        if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(dir), id) {
            let error = json!({
                "success": false,
                "error": format!("Server '{}' already uses this folder", other.name),
                "error_code": "working_dir_in_use",
                "instance": other.name,
            });
            return Err(Failure::new(StatusCode::CONFLICT, error));
        }
    }

    if let Some(rejected) = settings_rejection(&supervisor, instance, id, &settings) {
        return Err(rejected);
    }

    // 설정값 업데이트
    let mut updated = instance.clone();
    let stop_timeout = apply_fields(&mut updated, &settings)?;
    // 06 §8.2: 켜져 있을 때 값이 바뀐 restart_required 모듈 설정. 데몬은 스스로 재시작하지 않는다
    let restart_required = if supervisor.is_running(id).await { changed_restart_settings(&supervisor, instance, &updated, &settings) } else { Vec::new() };
    store_module_settings(&mut updated, &settings, stop_timeout);

    tracing::info!(
        "Updating instance {} with settings: port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}",
        id,
        updated.port,
        updated.executable_path,
        updated.protocol_mode,
        updated.module_settings.len()
    );
    let props_sync = game_file_values(&supervisor, &updated.module_name, &settings);

    // 저장
    let updated_clone = updated.clone();
    let new_revision = updated_clone.settings_revision();
    // 모드·자동 강퇴·명단이 바뀌었으면 이미 들어와 있는 사람도 다시 본다
    let sweep = crate::controllers::sweep_needed(instance, &updated_clone);
    if let Err(e) = supervisor.instance_store.update(id, updated) {
        return Err(Failure::coded(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", format!("Failed to update instance: {}", e)));
    }
    // 켜 둔 서버의 게임 채팅도 바로 따른다
    crate::supervisor::game_chat::remember(id, &updated_clone.game_chat());

    // 익스텐션 hook에 넘길 값을 챙기고 락을 놓는다. 아래 모듈·익스텐션 호출은 락 밖에서 (C12)
    let instance_dir = supervisor.instance_store.instance_dir(id);
    // 모듈의 모든 익스텐션 설정을 범용으로 전달 (특정 익스텐션 이름 참조 없음)
    let module = supervisor.module_loader.get_module(&updated_clone.module_name).ok();
    if let Some(m) = module.as_ref().filter(|m| m.metadata.runtime_settings().iter().any(|(name, _)| settings.get(*name).is_some())) {
        super::open_user_runtime(m, &updated_clone);
    }
    let mut hook_ctx = crate::extension::hook_context(&updated_clone, &instance_dir, module.as_ref().map(|m| &m.metadata));
    hook_ctx["settings"] = crate::extension::redact_for_hooks(module.as_ref().map(|m| &m.metadata), &settings);
    hook_ctx["module_extensions"] = module
        .map(|m| serde_json::to_value(&m.metadata.extensions).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!({}));
    let runtime = supervisor.runtime.clone();
    drop(supervisor);
    if sweep {
        crate::controllers::sweep(state, &runtime, Some(id));
    }

    let sync_error = sync_game_files(state, id, props_sync).await;

    // 인스턴스 설정 변경 시 Extension hook 디스패치 (extension_data가 있는 경우)
    // instance_dir + 모듈 익스텐션 설정을 범용으로 전달 → 각 extension이 자체 판단
    if !updated_clone.extension_data.is_empty() {
        let ext_mgr = state.extension_manager.clone();
        let _results = crate::plugin::with_instance(Some(state.host_context(id)), crate::extension::run_hook(&ext_mgr, "server.settings_changed", hook_ctx)).await;
    }

    // 게임 설정 파일의 비밀번호·포트가 바뀌었을 수 있다
    state.credentials.invalidate(id);
    let mut body = json!({ "success": true, "synced": sync_error.is_none(), "restart_required": restart_required, "revision": new_revision });
    if let Some(e) = sync_error {
        body["sync_error"] = e.get("message").or_else(|| e.get("error")).cloned().unwrap_or(Value::Null);
        // 설정은 이미 저장했다: 게임 파일 동기화 실패의 칸(오류 코드 등)만 덧붙이고 성공 표시는 덮지 않는다 (Codex 리뷰 #550)
        if let (Some(body), Some(fields)) = (body.as_object_mut(), e.as_object()) {
            body.extend(fields.iter().filter(|(k, _)| k.as_str() != "success").map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    // RCON 키처럼 받되 쓰지 않는 것
    let ignored: Vec<&String> = settings.as_object().into_iter().flat_map(|o| o.keys()).filter(|k| IGNORED_FIELDS.contains(&k.as_str())).collect();
    if !ignored.is_empty() {
        body["ignored"] = json!(ignored);
    }
    Ok(body)
}

/// 저장이 설 줄: 서버 폴더를 옮기는 저장은 설치·업데이트·백업·복원이 그 폴더를 쓰지 않을 때만 (업데이트는 시작할 때
/// 계산한 폴더로 다른 인스턴스를 잠그고 받는다). 같은 값을 다시 보내는 저장은 막지 않는다.
/// 그 밖의 저장은 설정 파일 줄에 선다: 겹치면 늦게 끝난 게임 파일 쓰기가 나중 저장을 덮는다. 서버 파일을 바꾸는 작업 중이면 거절한다.
/// 돌려주는 것: (작업 잠금, 설정 파일 잠금) 중 쥔 것
async fn lock_for_save(
    state: &IPCServer,
    id: &str,
    settings: &Value,
) -> Result<(Option<crate::instance::lock::InstanceLock>, Option<crate::instance::lock::FilesGuard>), Failure> {
    let moves_folder = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.get(id).is_some_and(|i| settings.get("working_dir").is_some_and(|v| v.as_str() != i.working_dir.as_deref()))
    };
    if moves_folder {
        crate::instance::lock::InstanceLock::take_for_writes(id).await.map(|task| (Some(task), None)).ok_or_else(Failure::busy)
    } else {
        crate::instance::lock::InstanceLock::files(id).await.map(|files| (None, Some(files))).ok_or_else(Failure::busy)
    }
}

/// 인스턴스 칸(포트·실행 인자·메모리 상한·게임 채팅·사용 모드·명단·자동 업데이트·실행 파일·폴더·익스텐션 값)을 얹는다.
/// 돌려주는 것: 정지 대기 시간 (`None`: 보내지 않음, `Some(None)`: 모듈 값으로 되돌림)
fn apply_fields(updated: &mut crate::instance::ServerInstance, settings: &Value) -> Result<Option<Option<u64>>, Failure> {
    // working_dir이 null인데 executable_path가 있으면 자동 보정 (이번 요청이 executable_path를 지우면 하지 않는다)
    if updated.working_dir.is_none() && !settings.get("executable_path").is_some_and(Value::is_null) {
        if let Some(exe_path) = &updated.executable_path {
            if let Some(parent) = std::path::Path::new(exe_path).parent() {
                updated.working_dir = Some(parent.to_string_lossy().to_string());
                tracing::info!(
                    "Auto-inferred working_dir to {} from existing executable_path",
                    parent.display()
                );
            }
        }
    }


    // common settings
    // port: 1~65535의 숫자 또는 숫자 문자열 (범위 밖 값을 다른 숫자로 저장하지 않는다). 빈 문자열(바꾸지 않음)은 예전처럼 받는다
    let given_port = settings.get("port").filter(|v| !v.is_null() && v.as_str() != Some(""));
    if given_port.is_some_and(|v| port_value(v).is_none()) {
        return Err(super::invalid_field("port", "port must be a number from 1 to 65535"));
    }
    if let Some(port) = settings.get("port").and_then(port_value) {
        updated.port = Some(port);
    }

    // NOTE: REST 비밀번호 자동 생성은 인스턴스 실행 시점에 supervisor에서 처리합니다.
    // REST 접속값은 모듈 설정이다 (`[protocols.rest]`): 아래 모듈 설정 저장이 다룬다
    // executable_path는 null이면 지정을 지워 기본값으로 되돌린다
    let cleared = |key: &str| settings.get(key).is_some_and(Value::is_null);
    // 설정 필드에 없는 추가 실행 인자: 문자열(따옴표로 묶기)이나 문자열 배열
    if let Some(extra) = settings.get("extra_args") {
        match extra_args_from(extra) {
            Some(args) => updated.extra_args = args,
            None => {
                return Err(invalid_field_message("extra_args", "extra_args must be a string or an array of strings"));
            }
        }
    }
    // 메모리 상한(MiB): 비우거나(null·"") 0이면 제한 없음
    if let Some(limit) = settings.get("memory_limit_mb") {
        match memory_limit_from(limit) {
            // 러너가 상한을 걸 수 없는 OS에 저장하면 걸린 줄 알고 그냥 돈다
            Ok(Some(_)) if !crate::instance::MEMORY_LIMIT_ENFORCED => {
                let message = "memory_limit_mb is only enforced on Windows; this daemon's OS cannot apply it";
                let error = json!({ "success": false, "error": message, "error_code": "memory_limit_unsupported", "message": message, "fields": [{"field": "memory_limit_mb"}] });
                return Err(Failure::new(StatusCode::UNPROCESSABLE_ENTITY, error));
            }
            Ok(v) => updated.memory_limit_mb = v,
            Err(()) => {
                return Err(invalid_field_message("memory_limit_mb", format!("memory_limit_mb must be empty or a whole number of MiB of at least {}", MIN_MEMORY_LIMIT_MB)));
            }
        }
    }
    // 게임 채팅 전달: 모드와 감지 단어를 함께 본다 (감지 단어 모드에는 단어가 있어야 한다)
    let game_chat_error = |message: &str, field: &str| {
        invalid_field_message(field, message)
    };
    if let Some(mode) = settings.get("game_chat") {
        match crate::supervisor::game_chat::Mode::parse(mode) {
            Ok(mode) => updated.game_chat = mode,
            Err(e) => return Err(game_chat_error(e, "game_chat")),
        }
    }
    if let Some(keyword) = settings.get("game_chat_keyword") {
        match crate::supervisor::game_chat::parse_keyword(keyword) {
            Ok(keyword) => updated.game_chat_keyword = keyword,
            Err(e) => return Err(game_chat_error(e, "game_chat_keyword")),
        }
    }
    if let Err(e) = updated.game_chat().check() {
        return Err(game_chat_error(e, "game_chat_keyword"));
    }
    // 사용 모드와 명단: 전용 멤버 명단·그 자동 강퇴(명단이 비면 켤 수 없다), 서버 공용의 자동 강퇴 닉네임
    if let Some(access) = settings.get("access") {
        match crate::controllers::Access::parse(access) {
            Ok(access) => updated.access = access,
            Err(e) => return Err(game_chat_error(e, "access")),
        }
    }
    if let Some(list) = settings.get("kick_players") {
        match crate::controllers::parse_kick_list(list) {
            Ok(list) => updated.kick_players = list,
            Err(e) => return Err(game_chat_error(e, "kick_players")),
        }
    }
    if let Some(list) = settings.get("controllers") {
        match crate::controllers::parse(list) {
            Ok(list) => updated.controllers = list,
            Err(e) => return Err(game_chat_error(e, "controllers")),
        }
    }
    // 게임 서버 자동 업데이트 (원격에서는 바꾸지 못한다: actions LAUNCH_KEYS)
    if let Some(auto) = settings.get("auto_update") {
        match auto.as_bool() {
            Some(auto) => updated.auto_update = auto,
            None => return Err(game_chat_error("auto_update must be true or false", "auto_update")),
        }
    }
    if let Some(only) = settings.get("controllers_only") {
        match only.as_bool() {
            Some(only) => updated.controllers_only = only,
            None => return Err(game_chat_error("controllers_only must be true or false", "controllers_only")),
        }
    }
    // 명단 밖 자동 강퇴는 전용 멤버 모드의 스위치다: 서버 공용이면 끈다. 꺼 두어야 access가 없는 예전 파일(켠 채 저장된 것)과
    // 가려진다 (`InstanceStore::load_instance_from_dir`의 옮기기, 리뷰 P1)
    if updated.access.is_shared() {
        updated.controllers_only = false;
    }
    if let Err(e) = crate::controllers::check(&updated.controllers, updated.controllers_only) {
        return Err(game_chat_error(e, "controllers_only"));
    }
    // 정지 대기 시간(초): 비우거나(null·"") 0이면 모듈 값을 쓴다
    let stop_timeout = match settings.get("stop_timeout").map(stop_timeout_from) {
        Some(Err(())) => {
            return Err(invalid_field_message("stop_timeout", "stop_timeout must be empty or a whole number of seconds from 1 to 3600"));
        }
        other => other.map(Result::unwrap_or_default),
    };
    if cleared("executable_path") {
        updated.executable_path = None;
    }
    if let Some(executable_path) = settings.get("executable_path").and_then(|v| v.as_str()) {
        // executable_path는 보통 모듈의 server_executable + working_dir에서 자동 해석되지만,
        // 명시적으로 설정하면 override로 저장합니다.
        updated.executable_path = Some(executable_path.to_string());
        // working_dir이 미설정이면 executable_path의 부모 디렉토리로 자동 설정
        if updated.working_dir.is_none() {
            if let Some(parent) = std::path::Path::new(executable_path).parent() {
                updated.working_dir = Some(parent.to_string_lossy().to_string());
                tracing::info!(
                    "Auto-set working_dir to {} from executable_path",
                    parent.display()
                );
            }
        }
    }

    // 작업 폴더 (예: GUI가 서버 파일을 설치한 뒤 그 폴더로)
    if let Some(dir) = settings.get("working_dir").and_then(|v| v.as_str()).filter(|d| !d.trim().is_empty()) {
        updated.working_dir = Some(dir.to_string());
    }

    // protocol_mode 설정 (rest 또는 rcon)
    if let Some(protocol_mode) = settings.get("protocol_mode").and_then(|v| v.as_str()) {
        updated.protocol_mode = protocol_mode.to_string();
    }

    // server_version 설정
    if let Some(version) = settings.get("server_version").and_then(|v| v.as_str()) {
        updated.server_version = Some(version.to_string());
    }

    // ── 범용 extension_data 갱신 ──
    // 프론트엔드에서 { "extension_data": { "key": value, ... } } 형태로 전달
    // null 값은 해당 키 삭제, 빈 문자열도 삭제 처리
    if let Some(ext_updates) = settings.get("extension_data") {
        if let Some(obj) = ext_updates.as_object() {
            for (key, value) in obj {
                match value {
                    serde_json::Value::Null => {
                        updated.extension_data.remove(key);
                    }
                    serde_json::Value::String(s) if s.is_empty() => {
                        updated.extension_data.remove(key);
                    }
                    serde_json::Value::String(s) => {
                        // 숫자로 변환 가능하면 Number로 저장 (CPU 제한 등)
                        if let Ok(n) = s.parse::<f64>() {
                            updated.extension_data.insert(key.clone(), json!(n));
                        } else {
                            updated.extension_data.insert(key.clone(), value.clone());
                        }
                    }
                    _ => {
                        updated.extension_data.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }
    Ok(stop_timeout)
}

/// 켜져 있을 때 값이 바뀐 `restart_required` 모듈 설정. 공통 칸(port 등)은 PATCH 전 인스턴스 칸의 값과, 그 밖에는 module_settings와 비교한다
fn changed_restart_settings(
    supervisor: &crate::supervisor::Supervisor,
    instance: &crate::instance::ServerInstance,
    updated: &crate::instance::ServerInstance,
    settings: &Value,
) -> Vec<String> {
    let before = serde_json::to_value(instance).unwrap_or_default();
    supervisor.module_loader.get_module(&updated.module_name).ok()
        .and_then(|m| m.metadata.settings.clone())
        .map(|s| s.fields)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.restart_required && settings.get(&f.name).is_some_and(|v| crate::instance::stored_setting(&before, instance, &f.name).as_ref() != Some(v)))
        .map(|f| f.name)
        .collect()
}

/// 동적 모듈 설정 저장 (하드코딩 필드 이외의 모든 설정을 module_settings에 저장)과 정지 대기 시간
fn store_module_settings(updated: &mut crate::instance::ServerInstance, settings: &Value, stop_timeout: Option<Option<u64>>) {
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if !is_known_field(key) {
                // 설정 화면은 고치지 않은 값도 함께 보낸다: 값이 바뀐 키만 사용자가 정한 것으로 기록한다
                if updated.module_settings.get(key) != Some(value) {
                    updated.user_settings.insert(key.clone());
                    updated.imported_settings.remove(key);
                }
                updated.module_settings.insert(key.clone(), value.clone());
            }
        }
    }
    match stop_timeout {
        Some(Some(n)) => {
            updated.module_settings.insert("stop_timeout".into(), json!(n));
        }
        Some(None) => {
            updated.module_settings.remove("stop_timeout");
            updated.user_settings.remove("stop_timeout");
        }
        None => {}
    }
}

/// 게임 설정 파일로 넘길 값 (configure lifecycle 호출). 데몬 필드, 데몬이 읽는 설정(DAEMON_SETTINGS),
/// 모듈이 사바쨩 설정으로 선언한 값은 뺀다
fn game_file_values(supervisor: &crate::supervisor::Supervisor, module: &str, settings: &Value) -> serde_json::Map<String, Value> {
    const DAEMON_FIELDS: &[&str] = &[
        "protocol_mode", "executable_path", "working_dir", "server_version", "extension_data", "extra_args", "memory_limit_mb",
        "game_chat", "game_chat_keyword", "access", "controllers", "controllers_only", "kick_players", "rcon_port", "rcon_password", "enable_rcon", "auto_update",
    ];
    let saba_fields: Vec<String> = supervisor.module_loader.get_module(module).ok()
        .and_then(|m| m.metadata.settings.as_ref().map(|s| s.saba_fields().into_iter().map(str::to_string).collect()))
        .unwrap_or_default();
    let mut props_sync = serde_json::Map::new();
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if DAEMON_FIELDS.contains(&key.as_str()) || is_daemon_setting(key) || saba_fields.contains(key) {
                continue;
            }
            props_sync.insert(key.clone(), value.clone());
        }
    }
    props_sync
}

/// 게임 설정 파일 동기화 (변경된 항목이 있을 때만). 실패해도 저장은 남기고 그 본문을 돌려준다 (답에 싣는다)
async fn sync_game_files(state: &IPCServer, id: &str, props_sync: serde_json::Map<String, Value>) -> Option<Value> {
    if props_sync.is_empty() {
        return None;
    }
    // 값에는 비밀번호가 들 수 있어 키만 남긴다
    tracing::info!(
        "Syncing settings to the game config for instance {}: {:?}",
        id,
        props_sync.keys().collect::<Vec<_>>()
    );
    let props_value = Value::Object(props_sync);
    match crate::supervisor::Supervisor::manage_properties(&state.supervisor, id, "write", Some(props_value)).await {
        // 모듈에 configure가 없으면 쓸 게임 설정 파일도 없다
        Ok(r) if r.get("success").and_then(|v| v.as_bool()) == Some(false)
            && r.get("error_code").and_then(|v| v.as_str()) != Some("not_supported")
            && !r.get("message").and_then(|v| v.as_str()).is_some_and(|m| m.starts_with("Unknown function")) => {
            let message = r.get("message").or(r.get("error")).and_then(|v| v.as_str()).unwrap_or("configure failed");
            tracing::warn!("Module could not write game settings for instance {}: {}", id, message);
            Some(r)
        }
        Ok(_) => {
            tracing::info!("Game config synced for instance {}", id);
            None
        }
        Err(e) => {
            tracing::warn!("Failed to sync the game config for instance {}: {}", id, e);
            Some(crate::plugin::error_response(&e))
        }
    }
}
