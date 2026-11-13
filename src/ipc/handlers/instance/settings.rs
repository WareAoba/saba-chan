//! 인스턴스 설정 저장(PATCH)과 그 값 검사

use super::*;

/// 설정 값 하나가 잘못됐다는 400 응답: `error`는 "invalid_format", 이유는 `message`에
fn invalid_field_message(field: &str, message: impl Into<String>) -> axum::response::Response {
    let error = json!({ "error": "invalid_format", "error_code": "invalid_format", "message": message.into(), "fields": [{"field": field}] });
    (StatusCode::BAD_REQUEST, Json(error)).into_response()
}

/// PATCH의 `stop_timeout`(정지 대기 초): 비움(null·""·0)은 모듈 값으로 되돌리기, 아니면 1~3600 정수 (숫자 글자도 받는다)
fn stop_timeout_from(value: &Value) -> Result<Option<u64>, ()> {
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
fn memory_limit_from(value: &Value) -> Result<Option<u64>, ()> {
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
pub(crate) fn settings_rejection(supervisor: &crate::supervisor::Supervisor, instance: &crate::instance::ServerInstance, id: &str, settings: &Value) -> Option<axum::response::Response> {
    // ── 설정값 타입/범위 스키마 검증 ──
    if let Some(settings_obj) = settings.as_object() {
        if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
            if let Some(ref settings_meta) = module.metadata.settings {
                // 부분 변경이므로 보낸 필드만 검증한다
                // 숨겨진 설정은 required를 보지 않는다 (06 §8.2). 숨김은 저장된 값에 이번 요청을 얹어 계산한다
                let view = serde_json::to_value(instance).unwrap_or_default();
                let mut hidden = crate::validator::hidden_fields(&settings_meta.fields, |k| settings_obj.get(k).cloned().or_else(|| stored_setting(&view, instance, k)));
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
                        "error": code,
                        "error_code": code,
                        "message": "Settings validation failed",
                        "details": error_details,
                        "fields": fields,
                    });
                    return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
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
                    "error": format!("Unknown setting: {}", unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")),
                    "error_code": "unknown_setting",
                    "fields": fields,
                });
                return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
            }
        }
    }
    None
}

/// PATCH /api/instance/:id - 인스턴스 설정 업데이트 (보낸 키만). `revision`(GET에서 받은 값)을 주면
/// 그사이 다른 곳에서 바뀐 인스턴스에는 쓰지 않고 409 `settings_changed`로 거절한다
pub async fn update_instance_settings(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(mut settings): Json<serde_json::Value>,
) -> impl IntoResponse {
    let revision = crate::config_store::patch::take_revision(&mut settings);
    // 서버 폴더를 옮기는 저장은 설치·업데이트·백업·복원이 그 폴더를 쓰지 않을 때만 (업데이트는 시작할 때
    // 계산한 폴더로 다른 인스턴스를 잠그고 받는다). 같은 값을 다시 보내는 저장은 막지 않는다.
    // 그 밖의 저장은 설정 파일 줄에 선다: 겹치면 늦게 끝난 게임 파일 쓰기가 나중 저장을 덮는다. 서버 파일을 바꾸는 작업 중이면 거절한다
    let moves_folder = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.get(&id).is_some_and(|i| settings.get("working_dir").is_some_and(|v| v.as_str() != i.working_dir.as_deref()))
    };
    let (_task, _saving) = if moves_folder {
        match super::super::backup::InstanceLock::take_for_writes(&id).await {
            Some(task) => (Some(task), None),
            None => return super::super::backup::instance_busy(),
        }
    } else {
        match super::super::backup::InstanceLock::files(&id).await {
            Some(files) => (None, Some(files)),
            None => return super::super::backup::instance_busy(),
        }
    };
    let mut supervisor = state.supervisor.write().await;

    // 인스턴스 찾기
    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst,
        None => {
            let error = json!({ "error": "Instance not found" });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    let now = instance.settings_revision();
    if revision.is_some_and(|seen| seen != now) {
        let error = json!({ "success": false, "error": "The server settings were changed elsewhere; read them again", "error_code": "settings_changed", "revision": now });
        return (StatusCode::CONFLICT, Json(error)).into_response();
    }

    // 잠근 뒤 다시 본다: 폴더를 옮기는 다른 저장·작업은 위 잠금을 지나야 한다
    let moves_folder = settings.get("working_dir").is_some_and(|v| v.as_str() != instance.working_dir.as_deref());
    if moves_folder && _task.is_none() {
        return super::super::backup::instance_busy();
    }
    // 다른 인스턴스가 쓰는 폴더(서버 폴더나 인스턴스 폴더, 그 안이나 바깥)로는 옮기지 않는다: 만들 때처럼 한 폴더에 서버 둘을 두지 않는다.
    // 그 인스턴스의 서버·설치·업데이트가 옮긴 서버의 파일을 덮어쓴다
    if let Some(dir) = settings.get("working_dir").and_then(|v| v.as_str()).filter(|d| moves_folder && !d.is_empty()) {
        if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(dir), &id) {
            let error = json!({
                "success": false,
                "error": format!("Server '{}' already uses this folder", other.name),
                "error_code": "working_dir_in_use",
                "instance": other.name,
            });
            return (StatusCode::CONFLICT, Json(error)).into_response();
        }
    }

    if let Some(rejected) = settings_rejection(&supervisor, instance, &id, &settings) {
        return rejected;
    }

    // 설정값 업데이트
    let mut updated = instance.clone();

    // working_dir이 null인데 executable_path가 있으면 자동 보정 (이번 요청이 executable_path를 지우면 하지 않는다)
    if updated.working_dir.is_none() && !settings.get("executable_path").is_some_and(Value::is_null) {
        if let Some(ref exe_path) = updated.executable_path {
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
        return invalid_field("port", "port must be a number from 1 to 65535");
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
                return invalid_field_message("extra_args", "extra_args must be a string or an array of strings");
            }
        }
    }
    // 메모리 상한(MiB): 비우거나(null·"") 0이면 제한 없음
    if let Some(limit) = settings.get("memory_limit_mb") {
        match memory_limit_from(limit) {
            // 러너가 상한을 걸 수 없는 OS에 저장하면 걸린 줄 알고 그냥 돈다
            Ok(Some(_)) if !crate::instance::MEMORY_LIMIT_ENFORCED => {
                let message = "memory_limit_mb is only enforced on Windows; this daemon's OS cannot apply it";
                let error = json!({ "error": message, "error_code": "memory_limit_unsupported", "message": message, "fields": [{"field": "memory_limit_mb"}] });
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
            }
            Ok(v) => updated.memory_limit_mb = v,
            Err(()) => {
                return invalid_field_message("memory_limit_mb", format!("memory_limit_mb must be empty or a whole number of MiB of at least {}", MIN_MEMORY_LIMIT_MB));
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
            Err(e) => return game_chat_error(e, "game_chat"),
        }
    }
    if let Some(keyword) = settings.get("game_chat_keyword") {
        match crate::supervisor::game_chat::parse_keyword(keyword) {
            Ok(keyword) => updated.game_chat_keyword = keyword,
            Err(e) => return game_chat_error(e, "game_chat_keyword"),
        }
    }
    if let Err(e) = updated.game_chat().check() {
        return game_chat_error(e, "game_chat_keyword");
    }
    // 사용 모드와 명단: 전용 멤버 명단·그 자동 강퇴(명단이 비면 켤 수 없다), 서버 공용의 자동 강퇴 닉네임
    if let Some(access) = settings.get("access") {
        match crate::controllers::Access::parse(access) {
            Ok(access) => updated.access = access,
            Err(e) => return game_chat_error(e, "access"),
        }
    }
    if let Some(list) = settings.get("kick_players") {
        match crate::controllers::parse_kick_list(list) {
            Ok(list) => updated.kick_players = list,
            Err(e) => return game_chat_error(e, "kick_players"),
        }
    }
    if let Some(list) = settings.get("controllers") {
        match crate::controllers::parse(list) {
            Ok(list) => updated.controllers = list,
            Err(e) => return game_chat_error(e, "controllers"),
        }
    }
    // 게임 서버 자동 업데이트 (원격에서는 바꾸지 못한다: actions LAUNCH_KEYS)
    if let Some(auto) = settings.get("auto_update") {
        match auto.as_bool() {
            Some(auto) => updated.auto_update = auto,
            None => return game_chat_error("auto_update must be true or false", "auto_update"),
        }
    }
    if let Some(only) = settings.get("controllers_only") {
        match only.as_bool() {
            Some(only) => updated.controllers_only = only,
            None => return game_chat_error("controllers_only must be true or false", "controllers_only"),
        }
    }
    // 명단 밖 자동 강퇴는 전용 멤버 모드의 스위치다: 서버 공용이면 끈다. 꺼 두어야 access가 없는 예전 파일(켠 채 저장된 것)과
    // 가려진다 (`InstanceStore::load_instance_from_dir`의 옮기기, 리뷰 P1)
    if updated.access.is_shared() {
        updated.controllers_only = false;
    }
    if let Err(e) = crate::controllers::check(&updated.controllers, updated.controllers_only) {
        return game_chat_error(e, "controllers_only");
    }
    // 정지 대기 시간(초): 비우거나(null·"") 0이면 모듈 값을 쓴다
    let stop_timeout = match settings.get("stop_timeout").map(stop_timeout_from) {
        Some(Err(())) => {
            return invalid_field_message("stop_timeout", "stop_timeout must be empty or a whole number of seconds from 1 to 3600");
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

    // 06 §8.2: 켜져 있을 때 값이 바뀐 restart_required 모듈 설정. 데몬은 스스로 재시작하지 않는다
    let running = supervisor.is_running(&id).await;
    let restart_required: Vec<String> = if running {
        // 공통 칸(port 등)은 PATCH 전 인스턴스 칸의 값과, 그 밖에는 module_settings와 비교한다
        let before = serde_json::to_value(instance).unwrap_or_default();
        supervisor.module_loader.get_module(&updated.module_name).ok()
            .and_then(|m| m.metadata.settings.clone())
            .map(|s| s.fields)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.restart_required && settings.get(&f.name).is_some_and(|v| stored_setting(&before, instance, &f.name).as_ref() != Some(v)))
            .map(|f| f.name)
            .collect()
    } else {
        Vec::new()
    };

    // 동적 모듈 설정 저장 (하드코딩 필드 이외의 모든 설정을 module_settings에 저장)
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

    tracing::info!(
        "Updating instance {} with settings: port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}",
        id,
        updated.port,
        updated.executable_path,
        updated.protocol_mode,
        updated.module_settings.len()
    );

    // 게임 설정 파일로 넘길 값 (configure lifecycle 호출). 데몬 필드, 데몬이 읽는 설정(DAEMON_SETTINGS),
    // 모듈이 사바쨩 설정으로 선언한 값은 뺀다
    const DAEMON_FIELDS: &[&str] = &[
        "protocol_mode", "executable_path", "working_dir", "server_version", "extension_data", "extra_args", "memory_limit_mb",
        "game_chat", "game_chat_keyword", "access", "controllers", "controllers_only", "kick_players", "rcon_port", "rcon_password", "enable_rcon", "auto_update",
    ];
    let saba_fields: Vec<String> = supervisor.module_loader.get_module(&updated.module_name).ok()
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

    // 저장
    let updated_clone = updated.clone();
    let new_revision = updated_clone.settings_revision();
    // 모드·자동 강퇴·명단이 바뀌었으면 이미 들어와 있는 사람도 다시 본다
    let sweep = crate::controllers::sweep_needed(instance, &updated_clone);
    if let Err(e) = supervisor.instance_store.update(&id, updated) {
        let error = json!({ "error": format!("Failed to update instance: {}", e) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }
    // 켜 둔 서버의 게임 채팅도 바로 따른다
    crate::supervisor::game_chat::remember(&id, &updated_clone.game_chat());

    // 익스텐션 hook에 넘길 값을 챙기고 락을 놓는다. 아래 모듈·익스텐션 호출은 락 밖에서 (C12)
    let instance_dir = supervisor.instance_store.instance_dir(&id);
    // 모듈의 모든 익스텐션 설정을 범용으로 전달 (특정 익스텐션 이름 참조 없음)
    let module = supervisor.module_loader.get_module(&updated_clone.module_name).ok();
    if let Some(m) = module.as_ref().filter(|m| m.metadata.runtime_settings().iter().any(|(name, _)| settings.get(*name).is_some())) {
        open_user_runtime(m, &updated_clone);
    }
    let mut hook_ctx = crate::extension::hook_context(&updated_clone, &instance_dir, module.as_ref().map(|m| &m.metadata));
    hook_ctx["settings"] = crate::extension::redact_for_hooks(module.as_ref().map(|m| &m.metadata), &settings);
    hook_ctx["module_extensions"] = module
        .map(|m| serde_json::to_value(&m.metadata.extensions).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!({}));
    let runtime = supervisor.runtime.clone();
    drop(supervisor);
    if sweep {
        crate::controllers::sweep(&state, &runtime, Some(&id));
    }

    // 게임 설정 파일 동기화 (변경된 항목이 있을 때만). 실패해도 저장은 남기고 응답에 싣는다
    let mut sync_error: Option<Value> = None;
    if !props_sync.is_empty() {
        // 값에는 비밀번호가 들 수 있어 키만 남긴다
        tracing::info!(
            "Syncing settings to the game config for instance {}: {:?}",
            id,
            props_sync.keys().collect::<Vec<_>>()
        );
        let props_value = Value::Object(props_sync);
        match crate::supervisor::Supervisor::manage_properties(&state.supervisor, &id, "write", Some(props_value)).await {
            // 모듈에 configure가 없으면 쓸 게임 설정 파일도 없다
            Ok(r) if r.get("success").and_then(|v| v.as_bool()) == Some(false)
                && r.get("error_code").and_then(|v| v.as_str()) != Some("not_supported")
                && !r.get("message").and_then(|v| v.as_str()).is_some_and(|m| m.starts_with("Unknown function")) => {
                let message = r.get("message").or(r.get("error")).and_then(|v| v.as_str()).unwrap_or("configure failed");
                tracing::warn!("Module could not write game settings for instance {}: {}", id, message);
                sync_error = Some(r);
            }
            Ok(_) => tracing::info!("Game config synced for instance {}", id),
            Err(e) => {
                tracing::warn!("Failed to sync the game config for instance {}: {}", id, e);
                sync_error = Some(crate::plugin::error_response(&e));
            }
        }
    }

    // 인스턴스 설정 변경 시 Extension hook 디스패치 (extension_data가 있는 경우)
    // instance_dir + 모듈 익스텐션 설정을 범용으로 전달 → 각 extension이 자체 판단
    if !updated_clone.extension_data.is_empty() {
        let ext_mgr = state.extension_manager.clone();
        let _results = crate::plugin::with_instance(Some(state.host_context(&id)), crate::extension::run_hook(&ext_mgr, "server.settings_changed", hook_ctx)).await;
    }

    // 게임 설정 파일의 비밀번호·포트가 바뀌었을 수 있다
    state.credentials.invalidate(&id);
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
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::{memory_limit_from, stop_timeout_from};
    use serde_json::json;

    /// 외부 감사 10-21: 같은 서버의 저장 둘이 겹치면 늦게 끝난 게임 파일 쓰기가 나중 저장을 덮었다(데몬과 파일이 달라짐).
    /// 저장은 인스턴스마다 하나씩, 게임 파일 쓰기까지 마친 뒤 다음 저장이 시작된다. 범위 밖 포트는 다른 숫자로 저장하지 않고 거절한다
    /// 저장이 게임 파일을 쓰는 동안 설정 창이 실제 값을 읽으면(`settings/live`) 저장이 끝난 뒤 읽는다:
    /// 아직 쓰지 않은 옛 파일 값으로 캐시를 덮지 않게
    #[tokio::test]
    async fn a_live_read_during_a_save_waits_for_the_game_file_write() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 인스턴스 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("live-read-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n"),
        )
        .unwrap();
        let file = tmp.path().join("motd.txt");
        std::fs::write(&file, "old").unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, sys, time\nconfig = json.load(sys.stdin)\nf = {:?}\n\
                 if sys.argv[1] == 'functions':\n    print(json.dumps({{'success': True, 'functions': ['configure', 'import_settings']}}))\n\
                 elif sys.argv[1] == 'import_settings':\n    print(json.dumps({{'success': True, 'settings': {{'motd': open(f).read()}}}}))\n\
                 else:\n    time.sleep(1.0)\n    open(f, 'w').write((config.get('properties') or config.get('settings') or {{}}).get('motd', ''))\n    print(json.dumps({{'success': True}}))\n",
                file.to_string_lossy()
            ),
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", &name);
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let save = {
            let (state, id) = (state.clone(), id.clone());
            tokio::spawn(async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(json!({ "motd": "new" }))).await).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let live = crate::ipc::internal::into_internal(super::super::super::managed::read_live_settings_handler(Path(id.clone()), State(state.clone())).await).await;
        let saved = save.await.unwrap();
        assert!(saved.status.is_success(), "{}", saved.body);
        assert_eq!(saved.body["synced"], true, "{}", saved.body);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(live.body["settings"]["motd"], "new", "{}", live.body);
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.module_settings["motd"], "new");
        // 읽기가 캐시를 맞춘 뒤의 revision을 준다: 설정 창은 이 값으로 저장한다 (연 때의 값이면 409가 된다)
        assert_eq!(live.body["revision"], json!(inst.settings_revision()), "{}", live.body);
            }

    /// 설정 저장은 게임 파일을 쓴다: 업데이트·복원처럼 서버 파일을 바꾸는 작업 중에는 거절하고(409),
    /// 파일을 읽기만 하는 백업 중에는 저장한다. 약관 동의도 같은 줄에 선다
    #[tokio::test]
    async fn settings_writes_are_refused_while_server_files_change() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\njson.load(sys.stdin)\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let save = |motd: &'static str| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(json!({ "motd": motd }))).await).await }
        };

        let update = super::super::super::backup::InstanceLock::take_for_writes(&id).await.unwrap();
        let refused = save("during update").await;
        assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(refused.body["error_code"], "instance_busy");
        let agreement = crate::ipc::internal::into_internal(
            super::super::super::managed::accept_agreements_handler(Path(id.clone()), State(state.clone()), Json(json!({ "accept": ["eula"] }))).await,
        )
        .await;
        assert_eq!(agreement.status, axum::http::StatusCode::CONFLICT);
        drop(update);

        let backup = super::super::super::backup::InstanceLock::take(&id).unwrap();
        assert!(save("during backup").await.status.is_success());
        drop(backup);
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings["motd"], "during backup");
    }

    #[tokio::test]
    async fn saves_of_one_server_run_one_after_another_and_ports_stay_in_range() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        // 첫 저장의 게임 파일 쓰기는 늦게 끝난다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\nconfig = json.load(sys.stdin)\n\
             if (config.get('properties') or config.get('settings') or {}).get('motd') == 'first':\n    time.sleep(1.0)\n\
             print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };

        let done = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let save = |motd: &'static str| {
            let (done, call) = (done.clone(), patch(json!({ "motd": motd })));
            tokio::spawn(async move {
                let r = call.await;
                assert!(r.status.is_success(), "{:?}", r.body);
                done.lock().unwrap().push(motd);
            })
        };
        let first = save("first");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let second = save("second");
        first.await.unwrap();
        second.await.unwrap();
        assert_eq!(*done.lock().unwrap(), ["first", "second"], "the second save waited for the first one's game file write");
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings["motd"], "second");

        for (key, value) in [("port", json!(65537)), ("port", json!(0)), ("port", json!("70000"))] {
            let bad = patch(json!({ key: value.clone() })).await;
            assert_eq!((bad.status.as_u16(), bad.body["error_code"].as_str(), bad.body["fields"][0]["field"].as_str()), (400, Some("invalid_format"), Some(key)), "{key}={value}");
        }
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_ne!(inst.port, Some(1));
        assert!(patch(json!({ "port": "25570" })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.port, Some(25570));
    }

    #[tokio::test]
    async fn settings_save_rejects_unknown_keys_and_reports_a_failed_game_file_write() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nprint(json.dumps({'success': False, 'message': 'server.properties is read-only'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };

        let typo = patch(json!({ "motdd": "hi" })).await;
        assert_eq!((typo.status.as_u16(), typo.body["error_code"].as_str()), (400, Some("unknown_setting")));
        assert_eq!(typo.body["fields"][0]["field"], "motdd");

        let saved = patch(json!({ "motd": "hi", "auto_restart": false, "enable_rcon": true })).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        assert_eq!(saved.body["synced"], false);
        // Codex 리뷰 #550: 모듈 실패 칸을 합치면서 저장 성공 표시를 false로 덮어 GUI가 저장 실패로 보였다
        assert_eq!(saved.body["success"], true, "the settings were saved");
        assert_eq!(saved.body["sync_error"], "server.properties is read-only");
        assert_eq!(saved.body["ignored"], json!(["enable_rcon"]));
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["motd"], json!("hi"), "the setting is kept even when the game file write fails");
    }

    /// 설정 저장에 GET의 revision을 실으면, 그사이 다른 곳에서 바뀐 인스턴스에는 쓰지 않는다 (0.1.10 결정 1)
    #[tokio::test]
    async fn a_settings_save_with_a_stale_revision_is_refused() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("a", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let read = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await.body }
        };

        let seen = read().await["revision"].as_str().unwrap().to_string();
        let first = patch(json!({ "motd": "one", "revision": seen })).await;
        assert!(first.status.is_success(), "{:?}", first.body);
        assert_eq!(first.body["revision"], read().await["revision"], "the reply carries the new revision");
        let stale = patch(json!({ "motd": "two", "revision": seen })).await;
        assert_eq!((stale.status.as_u16(), stale.body["error_code"].as_str()), (409, Some("settings_changed")));
        assert_eq!(stale.body["revision"], first.body["revision"]);
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["motd"], json!("one"));
        // revision 없이 보내면 예전처럼 받는다
        assert!(patch(json!({ "motd": "three" })).await.status.is_success());
    }

    /// 메모리 상한은 러너가 Windows Job으로만 건다: 다른 OS의 데몬은 상한을 저장하지 않고 적용할 수 없다고 답한다
    /// (저장되면 사용자는 걸린 줄 안다). 비우기는 어디서나 받는다
    #[tokio::test]
    async fn a_memory_limit_is_only_saved_where_it_is_enforced() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("a", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let stored = || {
            let (state, id) = (state.clone(), id.clone());
            async move { state.supervisor.read().await.instance_store.get(&id).map(|i| i.memory_limit_mb) }
        };

        let saved = patch(json!({ "memory_limit_mb": "4G" })).await;
        if cfg!(windows) {
            assert!(saved.status.is_success(), "{:?}", saved.body);
            assert_eq!(stored().await, Some(Some(4096)));
        } else {
            assert_eq!((saved.status.as_u16(), saved.body["error_code"].as_str()), (422, Some("memory_limit_unsupported")), "{:?}", saved.body);
            assert_eq!(saved.body["fields"], json!([{ "field": "memory_limit_mb" }]));
            assert_eq!(stored().await, Some(None));
        }
        assert!(patch(json!({ "memory_limit_mb": null })).await.status.is_success());
        assert_eq!(stored().await, Some(None));

        // 0.1.14까지 저장된 상한: 인스턴스 응답이 이 OS에서 걸리지 않는다고 알린다
        let read = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await.body }
        };
        assert_eq!(read().await.get("memory_limit_unsupported"), None);
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).unwrap().clone();
            inst.memory_limit_mb = Some(2048);
            sup.instance_store.update(&id, inst).unwrap();
        }
        let body = read().await;
        assert_eq!(body["memory_limit_mb"], json!(2048));
        assert_eq!(body.get("memory_limit_unsupported"), (!cfg!(windows)).then_some(&json!(true)));

        // 다른 PC의 데몬을 보는 클라이언트는 자기 OS로 짐작하지 않는다: 단건 조회와 서버 목록이 이 데몬의 판정을 싣는다
        assert_eq!(body["memory_limit_supported"], json!(cfg!(windows)));
        let list = crate::ipc::internal::into_internal(crate::ipc::handlers::server::list_servers_internal(&state).await).await.body;
        assert_eq!(list["servers"][0]["memory_limit_supported"], json!(cfg!(windows)));
    }

    /// 06 §8.1·8.2 (스키마 2): 옛 문자열 값은 목록으로 읽고, 숨긴 설정은 required를 보지 않으며 `hidden`에 실린다.
    /// 켜져 있을 때 바꾼 restart_required 설정은 응답 `restart_required`에 실린다
    #[tokio::test]
    async fn schema_v2_settings_migrate_hide_and_ask_for_a_restart() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\ndefault = false\n\n\
             [[settings.fields]]\nname = \"damage\"\ntype = \"text\"\nrequired = true\nvisible_when = { field = \"pvp\", equals = true }\n\n\
             [[settings.fields]]\nname = \"admins\"\ntype = \"list\"\nrestart_required = true\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nrestart_required = true\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"multiline\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.module_settings.insert("admins".into(), json!("Steve, Alex"));
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        sup.migrate_settings();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let get = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await }
        };
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };

        let read = get().await;
        assert_eq!(read.body["module_settings"]["admins"], json!(["Steve", "Alex"]), "an old string value is read as a list");
        assert_eq!(read.body["hidden"], json!(["damage"]));

        let saved = patch(json!({ "damage": "", "motd": "line one\nline two", "admins": ["Steve"] })).await;
        assert!(saved.status.is_success(), "a hidden setting is not required: {:?}", saved.body);
        assert_eq!(saved.body["restart_required"], json!([]), "the server is off");
        let shown = patch(json!({ "pvp": true, "damage": "" })).await;
        assert_eq!(shown.status.as_u16(), 400, "shown again, it is required: {:?}", shown.body);

        state.supervisor.read().await.tracker.track(&id, std::process::id()).unwrap();
        let running = patch(json!({ "admins": ["Steve"], "motd": "x" })).await;
        assert_eq!(running.body["restart_required"], json!([]), "unchanged values ask for nothing");
        let running = patch(json!({ "admins": ["Alex"], "motd": "y" })).await;
        assert_eq!(running.body["restart_required"], json!(["admins"]));
        // 인스턴스 칸에 저장되는 설정(port)은 그 칸의 지금 값과 비교한다
        let running = patch(json!({ "port": 25570 })).await;
        assert_eq!(running.body["restart_required"], json!(["port"]));
        let running = patch(json!({ "port": 25570 })).await;
        assert_eq!(running.body["restart_required"], json!([]), "saving the same value again asks for nothing");
        state.supervisor.read().await.tracker.untrack(&id).unwrap();
        let stored = std::fs::read_to_string(state.supervisor.read().await.instance_store.instance_dir(&id).join("settings.json")).unwrap();
        assert!(stored.contains("\"Alex\""), "{stored}");
    }

    /// 설치된 버전에 없는 설정(`versions` 밖)은 인스턴스 조회와 서버 목록이 `version_hidden`으로 알리고,
    /// 숨긴 설정이라 `required`를 보지 않는다. 버전을 모르는 서버는 모든 설정을 보인다
    #[tokio::test]
    async fn settings_the_installed_version_does_not_have_are_reported_as_hidden() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"pvp\"\ntype = \"text\"\nrequired = true\nversions = \"<1.21.9\"\n\n\
             [[settings.fields]]\nname = \"difficulty\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let get = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await.body }
        };
        let listed = || {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(crate::ipc::handlers::server::list_servers_internal(&state).await).await.body["servers"][0].clone() }
        };
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        assert_eq!(get().await["version_hidden"], json!([]), "the version is not known yet: every setting stays");
        assert_eq!(listed().await.get("version_hidden"), None);
        let empty = patch(json!({ "pvp": "" })).await;
        assert_eq!(empty.status.as_u16(), 400, "still a required setting: {:?}", empty.body);

        assert!(patch(json!({ "server_version": "1.21.10" })).await.status.is_success());
        assert_eq!(get().await["version_hidden"], json!(["pvp"]));
        assert_eq!(get().await["hidden"], json!(["pvp"]), "the CLI leaves it out with the other hidden settings");
        assert_eq!(listed().await["version_hidden"], json!(["pvp"]));
        let empty = patch(json!({ "pvp": "" })).await;
        assert!(empty.status.is_success(), "a setting this version does not have is not required: {:?}", empty.body);

        assert!(patch(json!({ "server_version": "1.21.8" })).await.status.is_success());
        assert_eq!(get().await["version_hidden"], json!([]));
    }

    /// 리뷰 554: 숨김 조건은 인스턴스 칸에 저장되는 설정(port 등)의 지금 값을 보고, list 기본값은 만들 때 채우며,
    /// list에 빈 문자열은 받지 않는다. 모듈을 다시 읽어 문자열 설정이 list가 되면 그때 옮긴다
    #[tokio::test]
    async fn list_defaults_instance_field_conditions_and_refresh_follow_the_contract() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let toml = |admins_type: &str| {
            format!(
                "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
                 [[settings.fields]]\nname = \"port\"\ntype = \"number\"\ndefault = 25565\n\n\
                 [[settings.fields]]\nname = \"note\"\ntype = \"text\"\nvisible_when = {{ field = \"port\", equals = 25565 }}\n\n\
                 [[settings.fields]]\nname = \"ops\"\ntype = \"list\"\ndefault = [\"a\", \"b\"]\n\n\
                 [[settings.fields]]\nname = \"admins\"\ntype = \"{admins_type}\"\n"
            )
        };
        std::fs::write(dir.join("module.toml"), toml("text")).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game", "port": 25565 }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let get = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await }
        };
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        assert_eq!(get().await.body["module_settings"]["ops"], json!(["a", "b"]), "a list default is filled in");
        assert_eq!(get().await.body["hidden"], json!([]));
        assert!(patch(json!({ "port": 26000 })).await.status.is_success());
        assert_eq!(get().await.body["hidden"], json!(["note"]), "the condition reads the port the instance has now");
        let empty = patch(json!({ "ops": "" })).await;
        assert_eq!(empty.status.as_u16(), 400, "a list is never a string: {:?}", empty.body);

        patch(json!({ "admins": "Steve, Alex" })).await;
        std::fs::write(dir.join("module.toml"), toml("list")).unwrap();
        let refreshed = crate::ipc::internal::into_internal(crate::ipc::handlers::server::refresh_modules(State(state.clone())).await).await;
        assert!(refreshed.status.is_success(), "{:?}", refreshed.body);
        assert_eq!(get().await.body["module_settings"]["admins"], json!(["Steve", "Alex"]), "moved when the modules are read again");
    }

    /// 게임 설정 파일로 넘기는 값(configure)에서 사바쨩 설정(`group = "saba-chan"`)과 데몬 필드는 뺀다.
    /// 어느 키가 사바쨩 설정인지는 모듈 선언이 정한다: 데몬은 게임별 키 이름을 모른다 (MK6)
    #[tokio::test]
    async fn only_game_settings_are_passed_to_configure() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n\n\
             [[settings.fields]]\nname = \"heap\"\ntype = \"text\"\ngroup = \"saba-chan\"\n\n\
             [[settings.fields]]\nname = \"ram\"\ntype = \"text\"\ngroup = \"advanced\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nconfig = json.load(sys.stdin)\n\
             print(json.dumps({'success': False, 'message': json.dumps(sorted(config.get('settings', {})))}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        // 데몬이 읽는 설정(DAEMON_SETTINGS)과 실행 칸도 게임 설정 파일로 가지 않는다: GUI는 저장마다 auto_restart를 함께 보낸다
        let body = json!({
            "motd": "hi", "heap": "4G", "ram": "8", "protocol_mode": "rest",
            "auto_restart": true, "restart_on_unknown_exit": false, "managed_start": true, "log_sessions_keep": 5,
            "stop_timeout": 90, "memory_limit_mb": "", "extra_args": "-Dx=1", "game_chat": "all",
        });
        let saved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(body)).await).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        // 받은 키는 실패 메시지로 돌려받는다: Windows에서 확인되지 않은 모듈은 격리되어 모듈 폴더에 쓸 수 없다
        let configured: Vec<String> = serde_json::from_str(saved.body["sync_error"].as_str().unwrap()).unwrap();
        assert_eq!(configured, vec!["motd", "ram"], "a saba-chan setting and daemon fields stay out; a game setting named like another game's launcher setting goes in");
        assert_eq!(crate::supervisor::game_chat::settings(&id).mode, crate::supervisor::game_chat::Mode::All, "a running server follows at once");

        // 게임 채팅 감지 단어 모드에는 단어가 있어야 한다
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        assert_eq!(patch(json!({ "game_chat": "keyword" })).await.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(patch(json!({ "game_chat": "loud" })).await.status, axum::http::StatusCode::BAD_REQUEST);
        patch(json!({ "game_chat": "keyword", "game_chat_keyword": " !d " })).await;
        let stored = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((stored.game_chat, stored.game_chat_keyword.as_str()), (crate::supervisor::game_chat::Mode::Keyword, "!d"));

        // 전용 멤버 명단: 빈 명단으로는 명단 밖 자동 강퇴를 켤 수 없고, 겹친 닉네임은 거절한다
        assert_eq!(patch(json!({ "access": "members", "controllers_only": true })).await.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(patch(json!({ "controllers": [{ "name": "a" }, { "name": "A" }] })).await.status, axum::http::StatusCode::BAD_REQUEST);
        let saved = patch(json!({ "access": "members", "controllers": [{ "name": " Steve ", "discord_id": "332007306403250178" }], "controllers_only": true })).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        let stored = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((stored.controllers[0].name.as_str(), stored.controllers_only), ("Steve", true));
        assert_eq!(patch(json!({ "controllers": [] })).await.status, axum::http::StatusCode::BAD_REQUEST, "emptying the list keeps the gate on");
        // 서버 공용으로 바꾸면 명단 밖 자동 강퇴(전용 멤버의 스위치)는 꺼진다 (리뷰 P1: access 없는 예전 파일과 가려진다)
        assert!(patch(json!({ "access": "shared" })).await.status.is_success());
        let stored = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((stored.access, stored.controllers_only), (crate::controllers::Access::Shared, false));
    }

    /// 설치·업데이트·백업·복원이 그 폴더를 쓰는 동안 서버 폴더(`working_dir`)를 옮기지 않는다:
    /// 업데이트는 시작할 때 계산한 폴더를 기준으로 다른 인스턴스를 잠그고 받는다
    #[tokio::test]
    async fn the_server_folder_does_not_move_while_a_task_holds_the_instance() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(modules.join("game")).unwrap();
        std::fs::write(
            modules.join("game").join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(modules.join("game").join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        let here = tmp.path().join("here").to_string_lossy().into_owned();
        inst.working_dir = Some(here.clone());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let busy = super::super::super::backup::InstanceLock::take(&id).unwrap();
        let elsewhere = tmp.path().join("elsewhere").to_string_lossy().into_owned();
        let moved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": elsewhere }))).await).await;
        assert_eq!(moved.status, 409, "{}", moved.body);
        assert_eq!(moved.body["error_code"], "instance_busy");
        // 같은 값을 다시 보내는 저장(GUI는 설정 전체를 보낸다)은 막지 않는다
        let same = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": here }))).await).await;
        assert_eq!(same.status, 200, "{}", same.body);
        drop(busy);
        let moved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": elsewhere }))).await).await;
        assert_eq!(moved.status, 200, "{}", moved.body);

        // 다른 인스턴스가 쓰는 폴더로는 옮기지 않는다: 만들 때처럼 한 폴더에 서버 둘을 두지 않는다
        // (켜져 있거나 작업 중이면 그 서버가 옮긴 서버의 파일을 덮어쓴다)
        let mut other = crate::instance::ServerInstance::new("other", "game");
        let theirs = tmp.path().join("theirs").to_string_lossy().into_owned();
        other.working_dir = Some(theirs.clone());
        let other_id = other.id.clone();
        let their_instance_dir = {
            let mut sup = state.supervisor.write().await;
            sup.instance_store.add(other).unwrap();
            sup.instance_store.instance_dir(&other_id).to_string_lossy().into_owned()
        };
        // 그 폴더의 안이나 바깥도 같다: 한쪽의 복원·삭제가 다른 쪽 서버 파일까지 치운다
        let inside = tmp.path().join("theirs").join("sub").to_string_lossy().into_owned();
        let outside = tmp.path().to_string_lossy().into_owned();
        for dir in [&theirs, &their_instance_dir, &inside, &outside] {
            let taken = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": dir }))).await).await;
            assert_eq!(taken.status, 409, "{dir}: {}", taken.body);
            assert_eq!(taken.body["error_code"], "working_dir_in_use");
            assert_eq!(taken.body["instance"], "other");
            let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "new", "module_name": "game", "working_dir": dir }))).await).await;
            assert_eq!(made.status, 409, "{dir}: {}", made.body);
            assert_eq!(made.body["error_code"], "working_dir_in_use");
        }
        // 이름만 비슷한 옆 폴더는 다른 폴더다
        let beside = tmp.path().join("theirs2").to_string_lossy().into_owned();
        let moved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": beside }))).await).await;
        assert_eq!(moved.status, 200, "{}", moved.body);
    }

    #[tokio::test]
    async fn only_values_the_user_changed_count_as_user_settings() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"java_path\"\ntype = \"text\"\ndefault = \"{instance_dir}/jdk/bin/java\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let user_settings = |id: String| {
            let state = state.clone();
            async move { state.supervisor.read().await.instance_store.get(&id).unwrap().user_settings.clone() }
        };

        // 기본값을 채워 보내도, 아무것도 안 보내도 모듈이 정한 값이다
        let filled = create(json!({ "name": "a", "module_name": "game", "module_settings": { "java_path": "{instance_dir}/jdk/bin/java" } })).await;
        let id = filled.body["id"].as_str().unwrap().to_string();
        assert!(user_settings(id.clone()).await.is_empty());
        let typed = create(json!({ "name": "b", "module_name": "game", "module_settings": { "java_path": "/opt/jdk/bin/java" } })).await;
        assert!(user_settings(typed.body["id"].as_str().unwrap().to_string()).await.contains("java_path"));

        // 저장할 때 고치지 않은 값을 함께 보내도 사용자가 정한 것이 아니다. 바꾼 값만 기록한다
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let current = state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings["java_path"].clone();
        assert!(patch(json!({ "java_path": current })).await.status.is_success());
        assert!(user_settings(id.clone()).await.is_empty());
        assert!(patch(json!({ "java_path": "/opt/jdk/bin/java" })).await.status.is_success());
        assert!(user_settings(id.clone()).await.contains("java_path"));
    }

    /// 정지 대기 시간은 잘못된 값을 저장하지 않고 400으로 알린다. 숫자 글자는 숫자로, 빈 값은 모듈 값으로 되돌린다
    #[tokio::test]
    async fn a_stop_timeout_patch_is_checked_before_it_is_saved() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = made.body["id"].as_str().unwrap().to_string();
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let saved = || async { state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.get("stop_timeout").cloned() };

        assert!(patch(json!({ "stop_timeout": "60" })).await.status.is_success());
        assert_eq!(saved().await, Some(json!(60)));
        let bad = patch(json!({ "stop_timeout": -5 })).await;
        assert_eq!(bad.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(bad.body["error_code"], "invalid_format");
        assert_eq!(bad.body["fields"][0]["field"], "stop_timeout");
        assert_eq!(saved().await, Some(json!(60)), "a rejected value changes nothing");
        assert!(patch(json!({ "stop_timeout": null })).await.status.is_success());
        assert_eq!(saved().await, None);
    }

    #[tokio::test]
    async fn null_clears_the_executable_override() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = made.body["id"].as_str().unwrap().to_string();
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let exe = tmp.path().join("server").join("run.sh");
        assert!(patch(json!({ "executable_path": exe.to_str().unwrap() })).await.status.is_success());
        assert!(patch(json!({ "executable_path": null })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.executable_path, None);
        assert!(!inst.module_settings.contains_key("executable_path"));

        // 작업 폴더 없이 실행 파일만 정했던 인스턴스: 지울 때 옛 실행 파일 폴더로 작업 폴더를 채우지 않는다
        {
            let mut sup = state.supervisor.write().await;
            let mut legacy = sup.instance_store.get(&id).cloned().unwrap();
            legacy.executable_path = Some(exe.to_string_lossy().into_owned());
            legacy.working_dir = None;
            sup.instance_store.update(&id, legacy).unwrap();
        }
        assert!(patch(json!({ "executable_path": null })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((inst.executable_path, inst.working_dir), (None, None));

        // 비울 수 없는 칸은 null이어도 지금 값을 둔다 (CLI가 이 표로 빈 값을 거절한다)
        let set = json!({ "port": 25000, "working_dir": "/srv/game", "protocol_mode": "rest", "server_version": "1.2" });
        assert!(patch(set.clone()).await.status.is_success());
        for key in saba_chan_updater_lib::constants::UNCLEARABLE_INSTANCE_FIELDS {
            assert!(patch(json!({ *key: null })).await.status.is_success());
        }
        let after = serde_json::to_value(state.supervisor.read().await.instance_store.get(&id).cloned().unwrap()).unwrap();
        for key in saba_chan_updater_lib::constants::UNCLEARABLE_INSTANCE_FIELDS {
            assert_eq!(after[*key], set[*key], "{key}");
        }
    }

    #[test]
    fn stop_timeout_is_empty_or_whole_seconds_up_to_an_hour() {
        for empty in [json!(null), json!(""), json!(0), json!(" ")] {
            assert_eq!(stop_timeout_from(&empty), Ok(None), "{empty}");
        }
        assert_eq!(stop_timeout_from(&json!(60)), Ok(Some(60)));
        assert_eq!(stop_timeout_from(&json!(" 90 ")), Ok(Some(90)));
        assert_eq!(stop_timeout_from(&json!(3600)), Ok(Some(3600)));
        for bad in [json!(-5), json!("abc"), json!(1.5), json!(true), json!(99999), json!("3601")] {
            assert!(stop_timeout_from(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn memory_limit_is_empty_or_a_whole_number_of_mib() {
        assert_eq!(memory_limit_from(&json!(null)), Ok(None));
        assert_eq!(memory_limit_from(&json!("")), Ok(None));
        assert_eq!(memory_limit_from(&json!(0)), Ok(None));
        assert_eq!(memory_limit_from(&json!(6144)), Ok(Some(6144)));
        assert_eq!(memory_limit_from(&json!(" 4096 ")), Ok(Some(4096)));
        assert!(memory_limit_from(&json!(100)).is_err());
        assert!(memory_limit_from(&json!(4.5)).is_err());
        assert!(memory_limit_from(&json!(-1)).is_err());
        assert_eq!(memory_limit_from(&json!("4G")), Ok(Some(4096)));
        assert_eq!(memory_limit_from(&json!("6gb")), Ok(Some(6144)));
        assert_eq!(memory_limit_from(&json!("1024M")), Ok(Some(1024)));
        assert!(memory_limit_from(&json!("4T")).is_err());
        assert!(memory_limit_from(&json!("B")).is_err());
        assert!(memory_limit_from(&json!(true)).is_err());
    }
}
