//! ═══════════════════════════════════════════════════════════════════
//! IPC 스키마 계약 테스트 — 크로스 컴포넌트 드리프트 방지
//! ═══════════════════════════════════════════════════════════════════
//!
//! ## 목적
//! daemon(Rust) ↔ GUI(React/JS) ↔ 클라우드 봇(relay-server, Node/JS) 간 JSON 메시지의
//! 필드 드리프트를 CI 단계에서 즉시 감지한다.
//!
//! ## 검증 전략
//! 1. 각 주요 응답 타입(`ServerInfo`, `ExtensionInfo`, ...)의 "최대 채움"
//!    대표 인스턴스를 만든다. (모든 Optional 필드에 `Some`, 모든 Vec/Map에 값 하나)
//! 2. `serde_json::to_value`로 직렬화해 실제 wire payload를 얻는다.
//! 3. JS 소비자(GUI / bot)가 접근하는 필드 목록 상수를 payload에 모두 존재하는지 검사.
//!    소비자 목록은 `grep "server\\." saba-chan-gui/src server-chan/relay-server/`로 수집한다.
//!
//! ## 드리프트 감지 시나리오
//! - Rust가 필드 이름을 변경  → 직렬화 키가 사라짐  → 테스트 실패
//! - Rust가 필드를 제거        → 컴파일 실패 (대표 인스턴스 생성 불가)
//! - JS가 새 필드를 읽기 시작  → 소비자 목록에 추가  → Rust에 없으면 테스트 실패
//! - `#[serde(rename = ...)]` 오타 → 키 불일치로 테스트 실패
//!
//! 이 테스트는 런타임(데몬/네트워크) 없이 순수 직렬화만 검증한다.

use saba_core::ipc::{
    CommandResponse, ExtensionInfo, ExtensionListResponse, PortConflictInfo, ProtocolsInfo,
    ServerInfo, ServerListResponse,
};
use saba_core::supervisor::module_loader::{
    ModuleCommands, ModuleSettings, SyntaxHighlight,
};
use saba_core::supervisor::PortConflictStopEvent;
use serde_json::Value;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// 소비자 필드 리스트 — SSOT
// ═══════════════════════════════════════════════════════════════
//
// 이 목록은 `saba-chan-gui/src/**/*.js`와 클라우드 봇(relay-server)에서
// `server.<field>` 형태로 접근하는 모든 필드를 포함한다. 필드가 추가되면
// 여기에 등록하고, Rust 구조체에도 반영되어 있어야 한다.

/// GUI가 ServerInfo에서 직접 접근하는 필드 (`server.<field>`)
const GUI_SERVER_INFO_FIELDS: &[&str] = &[
    "id",
    "name",
    "module",
    "status",
    "pid",
    "port",
    "rcon_port",
    "rest_host",
    "rest_port",
    "protocol_mode",
    "module_settings",
    "server_version",
    "last_api_action",
    "provisioning",
    "extension_data",
    "port_conflicts",
];

/// 클라우드 봇(relay-server)이 ServerInfo에서 접근하는 필드
const BOT_SERVER_INFO_FIELDS: &[&str] = &[
    "id",
    "name",
    "module",
    "status",
    "pid",
    "module_settings",
];

/// GUI가 ExtensionInfo(modules[])에서 접근하는 필드
const GUI_EXTENSION_INFO_FIELDS: &[&str] = &[
    "name",
    "interaction_mode",
    "syntax_highlight",
    "dir_signatures",
];

/// PortConflictInfo에서 GUI가 접근하는 필드
const GUI_PORT_CONFLICT_FIELDS: &[&str] = &["port", "conflict_name"];

/// CommandResponse는 양쪽 모두 success/message 읽음
const SHARED_COMMAND_RESPONSE_FIELDS: &[&str] = &["success", "message"];

/// ServerListResponse 최상위 필드
const SERVER_LIST_RESPONSE_FIELDS: &[&str] = &["servers"];

/// ExtensionListResponse 최상위 필드
const EXTENSION_LIST_RESPONSE_FIELDS: &[&str] = &["modules"];

// ═══════════════════════════════════════════════════════════════
// 공통: 소비자가 기대하는 필드가 직렬화 결과에 전부 존재하는지 검사
// ═══════════════════════════════════════════════════════════════

fn assert_fields_present(value: &Value, required: &[&str], consumer: &str, type_name: &str) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{} was not serialized as JSON object", type_name));
    let mut missing: Vec<&&str> = required
        .iter()
        .filter(|f| !obj.contains_key(**f))
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "{consumer} expects these fields on {type_name} but they are MISSING from JSON: {missing:?}\n\
         Actual fields present: {:?}\n\
         → Rust struct may have been renamed/removed OR serde rename is wrong.",
        obj.keys().collect::<Vec<_>>(),
    );
}

// ═══════════════════════════════════════════════════════════════
// 대표 인스턴스 팩토리 (모든 필드를 채움)
// ═══════════════════════════════════════════════════════════════

fn representative_port_conflict() -> PortConflictInfo {
    PortConflictInfo {
        port: 25565,
        port_type: "game".to_string(),
        conflict_name: "other-server".to_string(),
        conflict_id: "srv-002".to_string(),
        conflict_port_type: "rcon".to_string(),
    }
}

fn representative_server_info() -> ServerInfo {
    let mut module_settings = HashMap::new();
    module_settings.insert("managed_start".to_string(), Value::Bool(true));
    module_settings.insert("server_version".to_string(), Value::String("1.21".into()));

    let mut extension_status = HashMap::new();
    extension_status.insert(
        "docker".to_string(),
        serde_json::json!({"memory_percent": 25.5, "cpu_percent": 10.0}),
    );

    let mut extension_data = HashMap::new();
    extension_data.insert("docker_enabled".to_string(), Value::Bool(true));
    extension_data.insert(
        "install_method_steamcmd".to_string(),
        Value::Bool(false),
    );

    ServerInfo {
        id: "srv-001".to_string(),
        name: "my-minecraft".to_string(),
        module: "minecraft".to_string(),
        status: "running".to_string(),
        pid: Some(12345),
        start_time: Some(1_700_000_000),
        executable_path: Some("/opt/saba/mc/server.jar".to_string()),
        working_dir: Some("/opt/saba/mc".to_string()),
        port: Some(25565),
        rcon_port: Some(25575),
        rcon_password: Some("secret".to_string()),
        rest_host: Some("127.0.0.1".to_string()),
        rest_port: Some(8080),
        rest_username: Some("admin".to_string()),
        rest_password: Some("pw".to_string()),
        protocol_mode: "rcon".to_string(),
        module_settings,
        server_version: Some("1.21".to_string()),
        last_api_action: Some(1_700_000_100_000),
        provisioning: false,
        extension_status,
        extension_data,
        port_conflicts: vec![representative_port_conflict()],
    }
}

fn representative_extension_info() -> ExtensionInfo {
    ExtensionInfo {
        name: "minecraft".to_string(),
        version: "1.0.0".to_string(),
        description: Some("Minecraft server module".to_string()),
        path: "/opt/saba/modules/minecraft".to_string(),
        executable_path: Some("server.jar".to_string()),
        icon: Some("icon.png".to_string()),
        interaction_mode: Some("console".to_string()),
        protocols: Some(ProtocolsInfo {
            supported: vec!["rcon".to_string(), "rest".to_string()],
            default: Some("rcon".to_string()),
        }),
        settings: Some(ModuleSettings { fields: vec![] }),
        commands: Some(ModuleCommands { fields: vec![] }),
        syntax_highlight: Some(SyntaxHighlight { rules: vec![] }),
        dir_signatures: vec!["server.properties".to_string()],
        install: None,
        module_aliases: vec!["mc".to_string()],
        command_aliases: HashMap::new(),
    }
}

// ═══════════════════════════════════════════════════════════════
// 테스트
// ═══════════════════════════════════════════════════════════════

#[test]
fn server_info_contains_all_fields_gui_consumes() {
    let info = representative_server_info();
    let v = serde_json::to_value(&info).expect("ServerInfo serialization");
    assert_fields_present(&v, GUI_SERVER_INFO_FIELDS, "saba-chan-gui", "ServerInfo");
}

#[test]
fn server_info_contains_all_fields_bot_consumes() {
    let info = representative_server_info();
    let v = serde_json::to_value(&info).expect("ServerInfo serialization");
    assert_fields_present(&v, BOT_SERVER_INFO_FIELDS, "cloud bot (relay-server)", "ServerInfo");
}

#[test]
fn server_info_status_uses_snake_case_string_values() {
    // GUI/bot 양쪽이 'running'/'stopped' 문자열을 리터럴 비교한다.
    // status가 enum으로 바뀌거나 직렬화 형식이 바뀌면 즉시 깨진다.
    let info = representative_server_info();
    let v = serde_json::to_value(&info).unwrap();
    let status = v.get("status").and_then(|x| x.as_str()).unwrap();
    assert_eq!(status, "running");
}

#[test]
fn server_info_module_settings_is_object_not_array() {
    // GUI가 `latestServer.module_settings[field.name]` 형태로 key 접근.
    // Vec로 직렬화되면 런타임 실패.
    let info = representative_server_info();
    let v = serde_json::to_value(&info).unwrap();
    let ms = v.get("module_settings").expect("module_settings missing");
    assert!(
        ms.is_object(),
        "module_settings must serialize as JSON object, got: {ms}"
    );
}

#[test]
fn server_info_extension_data_is_object() {
    // GUI가 `server.extension_data?.docker_enabled` key 접근.
    let info = representative_server_info();
    let v = serde_json::to_value(&info).unwrap();
    let ed = v.get("extension_data").expect("extension_data missing");
    assert!(
        ed.is_object(),
        "extension_data must be a JSON object, got: {ed}"
    );
    assert_eq!(
        ed.get("docker_enabled"),
        Some(&Value::Bool(true)),
        "extension_data should preserve nested bool values"
    );
}

#[test]
fn server_info_port_conflicts_element_has_gui_fields() {
    let info = representative_server_info();
    let v = serde_json::to_value(&info).unwrap();
    let arr = v
        .get("port_conflicts")
        .and_then(|x| x.as_array())
        .expect("port_conflicts must be array");
    let first = arr.first().expect("representative instance has 1 conflict");
    assert_fields_present(
        first,
        GUI_PORT_CONFLICT_FIELDS,
        "saba-chan-gui",
        "PortConflictInfo",
    );
}

#[test]
fn server_list_response_shape() {
    let resp = ServerListResponse {
        servers: vec![representative_server_info()],
        port_conflict_stops: vec![],
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_fields_present(
        &v,
        SERVER_LIST_RESPONSE_FIELDS,
        "saba-chan-gui + cloud bot (relay-server)",
        "ServerListResponse",
    );
    assert!(
        v.get("servers").and_then(|x| x.as_array()).is_some(),
        "ServerListResponse.servers must be JSON array"
    );
}

#[test]
fn server_list_response_omits_port_conflict_stops_when_empty() {
    // GUI는 이 필드를 optional로 처리하므로 빈 배열은 생략되어야 payload 크기가 줄어든다.
    let resp = ServerListResponse {
        servers: vec![],
        port_conflict_stops: vec![],
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert!(
        !v.as_object().unwrap().contains_key("port_conflict_stops"),
        "empty port_conflict_stops should be skipped (skip_serializing_if)"
    );
}

#[test]
fn server_list_response_includes_port_conflict_stops_when_populated() {
    let resp = ServerListResponse {
        servers: vec![],
        port_conflict_stops: vec![PortConflictStopEvent {
            stopped_name: "mc".to_string(),
            stopped_id: "srv-001".to_string(),
            existing_name: "other".to_string(),
            port: 25565,
            timestamp_ms: 1_700_000_000_000,
        }],
    };
    let v = serde_json::to_value(&resp).unwrap();
    let stops = v
        .get("port_conflict_stops")
        .and_then(|x| x.as_array())
        .expect("non-empty port_conflict_stops must serialize");
    assert_eq!(stops.len(), 1);
}

#[test]
fn extension_info_contains_all_fields_gui_consumes() {
    let ext = representative_extension_info();
    let v = serde_json::to_value(&ext).expect("ExtensionInfo serialization");
    assert_fields_present(
        &v,
        GUI_EXTENSION_INFO_FIELDS,
        "saba-chan-gui",
        "ExtensionInfo",
    );
}

#[test]
fn extension_info_syntax_highlight_has_rules_subfield() {
    // GUI가 `mod?.syntax_highlight?.rules` 접근.
    let ext = representative_extension_info();
    let v = serde_json::to_value(&ext).unwrap();
    let sh = v.get("syntax_highlight").expect("syntax_highlight missing");
    assert!(
        sh.get("rules").and_then(|r| r.as_array()).is_some(),
        "syntax_highlight.rules must be array. Got: {sh}"
    );
}

#[test]
fn extension_list_response_shape() {
    let resp = ExtensionListResponse {
        modules: vec![representative_extension_info()],
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_fields_present(
        &v,
        EXTENSION_LIST_RESPONSE_FIELDS,
        "saba-chan-gui",
        "ExtensionListResponse",
    );
}

#[test]
fn command_response_contains_shared_fields() {
    let r = CommandResponse {
        success: true,
        message: "ok".to_string(),
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_fields_present(
        &v,
        SHARED_COMMAND_RESPONSE_FIELDS,
        "saba-chan-gui + cloud bot (relay-server)",
        "CommandResponse",
    );
    // bot/GUI는 success를 bool로 가정한다.
    assert!(v.get("success").unwrap().is_boolean());
}

#[test]
fn port_conflict_info_has_all_five_fields() {
    // 내부 구조체의 모든 필드가 직렬화되는지 확인 (skip_serializing_if 실수 방지).
    let c = representative_port_conflict();
    let v = serde_json::to_value(&c).unwrap();
    for key in [
        "port",
        "port_type",
        "conflict_name",
        "conflict_id",
        "conflict_port_type",
    ] {
        assert!(
            v.get(key).is_some(),
            "PortConflictInfo.{key} must always serialize (no skip_serializing_if). Actual: {v}"
        );
    }
}

#[test]
fn server_info_id_and_name_are_distinct_fields() {
    // bot processor에서 `s.id === targetId`와 `s.name.toLowerCase()`가 모두 쓰인다.
    // 과거 버그 방지: 하나의 alias로 합쳐지면 안 된다.
    let info = representative_server_info();
    let v = serde_json::to_value(&info).unwrap();
    assert_ne!(v.get("id"), v.get("name"));
    assert!(v.get("id").unwrap().is_string());
    assert!(v.get("name").unwrap().is_string());
}
