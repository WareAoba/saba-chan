//! 클라우드 명령 디스패처 — 릴레이가 보낸 payload를 supervisor 호출로 변환.
//!
//! 페이로드 스키마(legacy 디스코드 봇 호환):
//! ```json
//! { "action": "raw_command", "text": "<텍스트>", "voiceChannelId": "..." }
//! ```
//!
//! 텍스트 해석 규칙(미니멀):
//! - `list` / `목록` → 등록된 인스턴스 목록
//! - `<instance> start` / `stop` / `restart` / `status` → 라이프사이클 호출
//! - `<instance> <module-command> [args...]` → `Supervisor::execute_command`
//!
//! 알 수 없는 형식은 `success: false` 결과를 돌려보내고 에러를 삼키지 않는다.

use crate::config_store::InstanceDiscordSettings;
use crate::instance::ServerInstance;
use crate::supervisor::module_loader::CommandField;
use crate::supervisor::Supervisor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 디스패치 컨텍스트 — 릴레이 poll이 전달한 요청 출처 + 인스턴스별 설정 스냅샷.
///
/// 노드/유저 권한 판정은 릴레이가 아니라 데몬(여기)이 수행한다.
/// `guild_id`/`requested_by`가 없으면(내부 호출 등) 해당 제한을 적용하지 않는다.
#[derive(Debug, Clone, Default)]
pub struct DispatchContext {
    /// 명령이 발신된 디스코드 노드(길드) ID.
    pub guild_id: Option<String>,
    /// 명령을 보낸 디스코드 유저 ID.
    pub requested_by: Option<String>,
    /// bot-config.json `instanceSettings` 스냅샷 (인스턴스 ID → 설정).
    pub instance_settings: HashMap<String, InstanceDiscordSettings>,
}

impl DispatchContext {
    /// 인스턴스가 이 컨텍스트의 노드에서 사용 가능한지.
    /// 미설정(`allowed_nodes: None`) = 모든 노드 허용 (기존 동작 호환).
    fn is_instance_allowed(&self, instance_id: &str) -> bool {
        let Some(guild) = self.guild_id.as_deref() else {
            return true;
        };
        match self
            .instance_settings
            .get(instance_id)
            .and_then(|s| s.allowed_nodes.as_ref())
        {
            None => true,
            Some(nodes) => nodes.iter().any(|g| g == guild),
        }
    }

    /// 유저가 이 인스턴스의 canonical 명령을 실행할 수 있는지 (블랙리스트 판정).
    /// `kill`/`force-stop`은 `stop` 차단 목록도 함께 적용해 우회를 막는다.
    fn is_user_allowed(&self, instance_id: &str, command: &str) -> bool {
        let Some(user) = self.requested_by.as_deref() else {
            return true;
        };
        let Some(settings) = self.instance_settings.get(instance_id) else {
            return true;
        };
        let blocked = |cmd: &str| {
            settings
                .blocked_users
                .get(cmd)
                .is_some_and(|list| list.iter().any(|u| u == user))
        };
        if blocked(command) {
            return false;
        }
        if matches!(command, "kill" | "force-stop") && blocked("stop") {
            return false;
        }
        true
    }

    /// 인스턴스 호출명 별명 오버라이드. `None` = module.toml 기본 사용.
    fn instance_aliases(&self, instance_id: &str) -> Option<&Vec<String>> {
        self.instance_settings
            .get(instance_id)
            .and_then(|s| s.aliases.as_ref())
    }

    /// 인스턴스 명령어 별명 오버라이드 (canonical 명령 → 별명 목록).
    fn instance_command_aliases(&self, instance_id: &str) -> Option<&HashMap<String, Vec<String>>> {
        self.instance_settings
            .get(instance_id)
            .map(|s| &s.command_aliases)
            .filter(|m| !m.is_empty())
    }
}

/// 사용자 입력 토큰 → 인스턴스 해석 실패 사유
#[derive(Debug, Clone)]
pub enum ResolveError {
    /// 어떤 인스턴스/모듈/별명으로도 매칭되지 않음
    NotFound { input: String },
    /// 한 토큰이 여러 인스턴스에 매칭됨 (모듈 별명이 다중 인스턴스에 적용된 경우)
    Ambiguous {
        input: String,
        candidates: Vec<String>,
    },
    /// 매칭은 되었으나 이 디스코드 노드에서 사용이 허용되지 않은 인스턴스
    NodeNotAllowed { name: String },
}

impl ResolveError {
    /// 디스패치 결과로 변환 — 릴레이 봇이 코드 기반으로 번역할 수 있게 한다.
    pub fn to_dispatch_result(&self) -> DispatchResult {
        match self {
            ResolveError::NotFound { input } => DispatchResult::err_code(
                "INSTANCE_NOT_FOUND",
                format!("instance '{}' not found", input),
                json!({ "name": input }),
            ),
            ResolveError::Ambiguous { input, candidates } => DispatchResult::err_code(
                "INSTANCE_AMBIGUOUS",
                format!(
                    "instance '{}' is ambiguous; candidates: {}",
                    input,
                    candidates.join(", ")
                ),
                json!({ "name": input, "candidates": candidates }),
            ),
            ResolveError::NodeNotAllowed { name } => DispatchResult::err_code(
                "NODE_NOT_ALLOWED",
                format!("instance '{}' is not available on this node", name),
                json!({ "name": name }),
            ),
        }
    }
}

/// 토큰을 인스턴스로 해석한다 (컨텍스트 없음 — 노드 제한/별명 오버라이드 미적용).
///
/// 기존 호출부/테스트 호환용 래퍼. 실제 매칭은 [`resolve_instance_scoped`] 참조.
pub fn resolve_instance<'a>(
    token: &str,
    instances: &'a [ServerInstance],
    module_aliases: &HashMap<String, Vec<String>>,
) -> Result<&'a ServerInstance, ResolveError> {
    resolve_instance_scoped(token, instances, module_aliases, &DispatchContext::default())
}

/// 토큰을 인스턴스로 해석한다 (노드 스코프 + 인스턴스별 별명 오버라이드 적용).
///
/// 우선순위:
/// 1. 인스턴스 이름 정확 일치 (대소문자 무시)
/// 2. 인스턴스 호출명 별명 일치 — 인스턴스별 오버라이드가 있으면 그것만,
///    없으면 module.toml `module_aliases` 기본값. 모듈 이름 자체도 별명처럼 취급.
///
/// 현재 노드에서 허용된 인스턴스만 후보로 삼는다. 허용 후보가 0개인데
/// 전체 인스턴스에서는 매칭되면 `NodeNotAllowed`를 돌려준다 —
/// "존재하지 않음"보다 "이 서버에서는 사용 불가"가 원인을 정확히 알려준다.
///
/// 매칭 후보가 여러 개면 `Ambiguous`, 0개면 `NotFound`.
pub fn resolve_instance_scoped<'a>(
    token: &str,
    instances: &'a [ServerInstance],
    module_aliases: &HashMap<String, Vec<String>>,
    ctx: &DispatchContext,
) -> Result<&'a ServerInstance, ResolveError> {
    let allowed: Vec<&ServerInstance> = instances
        .iter()
        .filter(|i| ctx.is_instance_allowed(&i.id))
        .collect();

    match resolve_among(token, &allowed, module_aliases, ctx) {
        Err(ResolveError::NotFound { input }) => {
            // 허용 셋에서 못 찾음 — 전체 셋에서 매칭되면 노드 제한 때문이다.
            let all: Vec<&ServerInstance> = instances.iter().collect();
            match resolve_among(token, &all, module_aliases, ctx) {
                Ok(inst) => Err(ResolveError::NodeNotAllowed {
                    name: inst.name.clone(),
                }),
                // 전체 셋에서 다중 매칭 → 전부 이 노드에서 차단된 상태
                Err(ResolveError::Ambiguous { input, .. }) => {
                    Err(ResolveError::NodeNotAllowed { name: input })
                }
                Err(_) => Err(ResolveError::NotFound { input }),
            }
        }
        other => other,
    }
}

/// 주어진 후보 집합에서 토큰을 매칭한다 (권한 필터링은 호출자 책임).
fn resolve_among<'a>(
    token: &str,
    candidates: &[&'a ServerInstance],
    module_aliases: &HashMap<String, Vec<String>>,
    ctx: &DispatchContext,
) -> Result<&'a ServerInstance, ResolveError> {
    let lower = token.trim().to_lowercase();
    if lower.is_empty() {
        return Err(ResolveError::NotFound {
            input: token.to_string(),
        });
    }

    // 1) 인스턴스 이름 정확 일치 (대소문자 무시)
    let by_name: Vec<&ServerInstance> = candidates
        .iter()
        .filter(|i| i.name.to_lowercase() == lower)
        .copied()
        .collect();
    if !by_name.is_empty() {
        if by_name.len() == 1 {
            return Ok(by_name[0]);
        }
        return Err(ResolveError::Ambiguous {
            input: token.to_string(),
            candidates: by_name.iter().map(|i| i.name.clone()).collect(),
        });
    }

    // 2) 호출명 별명 매칭 — 인스턴스별 오버라이드 우선, 없으면 모듈 기본.
    //    모듈 이름 자체는 오버라이드 여부와 무관하게 항상 매칭된다
    //    (별명 등록이 전혀 없어도 `!saba palworld start`가 동작해야 함).
    let matched: Vec<&ServerInstance> = candidates
        .iter()
        .filter(|i| {
            if i.module_name.to_lowercase() == lower {
                return true;
            }
            match ctx.instance_aliases(&i.id) {
                Some(overrides) => overrides.iter().any(|a| a.to_lowercase() == lower),
                None => module_aliases
                    .get(&i.module_name)
                    .is_some_and(|aliases| aliases.iter().any(|a| a.to_lowercase() == lower)),
            }
        })
        .copied()
        .collect();

    match matched.len() {
        0 => Err(ResolveError::NotFound {
            input: token.to_string(),
        }),
        1 => Ok(matched[0]),
        _ => Err(ResolveError::Ambiguous {
            input: token.to_string(),
            candidates: matched.iter().map(|i| i.name.clone()).collect(),
        }),
    }
}

/// 슈퍼바이저에서 모듈 별명 맵을 추출한다 (매칭 실패는 빈 맵으로 처리).
fn collect_module_aliases(supervisor: &Supervisor) -> HashMap<String, Vec<String>> {
    supervisor
        .module_loader
        .discover_modules()
        .map(|mods| {
            mods.into_iter()
                .map(|m| (m.metadata.name.clone(), m.metadata.module_aliases.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// 슈퍼바이저에서 모듈별 명령어 별명 맵을 추출한다 (module → command → aliases).
fn collect_command_aliases(supervisor: &Supervisor) -> HashMap<String, HashMap<String, Vec<String>>> {
    supervisor
        .module_loader
        .discover_modules()
        .map(|mods| {
            mods.into_iter()
                .map(|m| {
                    let cmds = m
                        .metadata
                        .command_aliases
                        .iter()
                        .map(|(cmd, info)| (cmd.clone(), info.aliases.clone()))
                        .collect();
                    (m.metadata.name.clone(), cmds)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// verb 토큰을 module.toml `[aliases.commands]` 별명으로 canonical 명령에 매핑한다.
/// 예: ("실행", "palworld") → "start". 매칭이 없으면 원문을 그대로 반환한다.
///
/// 릴레이 봇도 동일한 치환을 수행하지만(bot/commandAlias.ts), 릴레이 캐시가
/// 갱신되지 않았거나 사용자가 별명 설정을 저장한 적 없는 경우를 대비해
/// 데몬 측에서도 반드시 해석한다 (심층 방어 — 이 해석이 없으면
/// "Unknown REST command: 실행"으로 실패한다).
fn resolve_command_alias(
    verb: &str,
    module_name: &str,
    command_aliases: &HashMap<String, HashMap<String, Vec<String>>>,
) -> String {
    let Some(cmds) = command_aliases.get(module_name) else {
        return verb.to_string();
    };
    let verb_lower = verb.to_lowercase();
    for (canonical, aliases) in cmds {
        if canonical.to_lowercase() == verb_lower
            || aliases.iter().any(|a| a.to_lowercase() == verb_lower)
        {
            return canonical.clone();
        }
    }
    verb.to_string()
}

/// verb 토큰을 canonical 명령으로 해석한다 (인스턴스별 오버라이드 적용).
///
/// 인스턴스에 명령어 별명 오버라이드가 있으면:
/// - 오버라이드에 등록된 canonical/별명이 우선 매칭된다.
/// - 오버라이드된 canonical의 module.toml 기본 별명은 **무시**된다
///   (사용자가 그 명령의 별명을 교체했다는 의도이므로).
/// - 오버라이드되지 않은 명령은 module.toml 기본으로 폴백한다.
fn resolve_command_alias_scoped(
    verb: &str,
    inst: &ServerInstance,
    command_aliases: &HashMap<String, HashMap<String, Vec<String>>>,
    ctx: &DispatchContext,
) -> String {
    let Some(overrides) = ctx.instance_command_aliases(&inst.id) else {
        return resolve_command_alias(verb, &inst.module_name, command_aliases);
    };
    let verb_lower = verb.to_lowercase();
    for (canonical, aliases) in overrides {
        if canonical.to_lowercase() == verb_lower
            || aliases.iter().any(|a| a.to_lowercase() == verb_lower)
        {
            return canonical.clone();
        }
    }
    if let Some(cmds) = command_aliases.get(&inst.module_name) {
        for (canonical, aliases) in cmds {
            if overrides.contains_key(canonical) {
                continue;
            }
            if canonical.to_lowercase() == verb_lower
                || aliases.iter().any(|a| a.to_lowercase() == verb_lower)
            {
                return canonical.clone();
            }
        }
    }
    verb.to_string()
}

/// 디스패치 결과 — 릴레이 `/result/:id` body로 직렬화된다.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub success: bool,
    pub data: Value,
}

impl DispatchResult {
    pub fn ok(data: Value) -> Self {
        Self { success: true, data }
    }
    /// 성공 결과에 사람이 읽을 `text` 필드를 주입한다.
    /// 릴레이 봇은 `data.text`를 그대로 디스코드 메시지로 렌더링한다.
    pub fn ok_text<S: Into<String>>(text: S, data: Value) -> Self {
        let t = text.into();
        match data {
            Value::Object(mut m) => {
                m.insert("text".to_string(), json!(t));
                Self {
                    success: true,
                    data: Value::Object(m),
                }
            }
            other => Self {
                success: true,
                data: json!({ "text": t, "value": other }),
            },
        }
    }
    /// 성공 결과에 `code`/`args`와 사람이 읽을 fallback `text`를 주입한다.
    /// 릴레이 봇은 `code`가 있으면 노드 로케일로 번역하고, 없으면 `text`를 그대로 표시한다.
    pub fn ok_code<S: Into<String>>(code: &str, text: S, args: Value, data: Value) -> Self {
        let mut result = Self::ok_text(text, data);
        if let Value::Object(ref mut m) = result.data {
            m.insert("code".to_string(), json!(code));
            m.insert("args".to_string(), args);
        }
        result
    }
    /// 슈퍼바이저가 돌려준 원시 Value를 디스패치 결과로 변환한다.
    ///
    /// 슈퍼바이저/모듈은 시작·정지 실패(의존성·포트 충돌·익스텐션 필요 등)를
    /// `Err`가 아니라 `Ok(json!({ "success": false, ... }))` 또는
    /// `action_required` 필드로 돌려주기도 한다. 이 경우를 성공으로 오인하지 않고
    /// 실패 결과로 변환하며, 어느 경우든 봇이 표시할 `text`를 채운다.
    pub fn from_supervisor_value(v: Value, fallback_text: String) -> Self {
        let explicit_fail = v.get("success").and_then(Value::as_bool) == Some(false);
        let action_required = v.get("action_required").is_some();

        if explicit_fail || action_required {
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or(fallback_text);
            let mut data = v;
            if let Value::Object(ref mut m) = data {
                m.entry("error".to_string())
                    .or_insert_with(|| json!(msg.clone()));
                m.insert("text".to_string(), json!(format!("⚠️ {}", msg)));
            }
            return Self {
                success: false,
                data,
            };
        }

        let text = supervisor_text(&v).unwrap_or(fallback_text);
        Self::ok_text(text, v)
    }
    /// `from_supervisor_value`와 동일하되, 슈퍼바이저/모듈이 자체 텍스트를 제공하지
    /// 않아 fallback 텍스트를 쓸 때만 `code`/`args`를 함께 실어 릴레이 봇이
    /// 노드 로케일로 번역할 수 있게 한다. (모듈 자체 응답은 그대로 표시)
    pub fn from_supervisor_value_coded(
        v: Value,
        code: &str,
        args: Value,
        fallback_text: String,
    ) -> Self {
        let explicit_fail = v.get("success").and_then(Value::as_bool) == Some(false);
        let action_required = v.get("action_required").is_some();
        if explicit_fail || action_required {
            return Self::from_supervisor_value(v, fallback_text);
        }
        match supervisor_text(&v) {
            Some(text) => Self::ok_text(text, v),
            None => Self::ok_code(code, fallback_text, args, v),
        }
    }
    /// 성공 시 **항상** `code`/`args`를 실어 릴레이 봇이 노드 로케일로 번역하게 한다.
    ///
    /// start/stop 라이프사이클은 슈퍼바이저가 항상 내부용 영어 메시지
    /// ("Server 'X' started with PID 123 (non-managed mode)")를 돌려주는데,
    /// `from_supervisor_value_coded`는 그 텍스트를 우선해 디스코드에 영어가
    /// 그대로 노출된다. 이 함수는 그 메시지를 사용자 노출 텍스트로 쓰지 않고
    /// 코드 번역("'X' 서버가 시작되었습니다" 등)만 남긴다.
    /// 실패 응답은 `from_supervisor_value`와 동일하게 처리한다.
    pub fn from_supervisor_value_forced_code(
        v: Value,
        code: &str,
        args: Value,
        fallback_text: String,
    ) -> Self {
        let explicit_fail = v.get("success").and_then(Value::as_bool) == Some(false);
        let action_required = v.get("action_required").is_some();
        if explicit_fail || action_required {
            return Self::from_supervisor_value(v, fallback_text);
        }
        Self::ok_code(code, fallback_text, args, v)
    }
    /// 코드 기반 에러 — 릴레이 봇이 로케일별로 번역할 수 있게 `code`/`args`를 포함한다.
    pub fn err_code<S: Into<String>>(code: &str, msg: S, args: Value) -> Self {
        Self {
            success: false,
            data: json!({
                "code": code,
                "error": msg.into(),
                "args": args,
            }),
        }
    }
}

/// 모듈 커맨드 응답의 구조화 데이터(`data`)를 디스코드용 텍스트로 포매팅한다.
///
/// 구(舊) 디스코드 봇 `ipc.formatResponse`의 복원 — 플레이어 목록 같은
/// 배열/객체 데이터를 사람이 읽을 불릿 리스트로 변환한다.
/// 표시할 내용이 없으면(`null`/빈 객체/빈 문자열) `None`.
fn format_command_data(data: &Value) -> Option<String> {
    match data {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Value::Array(arr) => (!arr.is_empty()).then(|| format_array(arr)),
        Value::Object(map) => {
            if map.is_empty() {
                return None;
            }
            // 첫 번째 배열 필드를 대표 목록으로 표시 (예: { "players": [...] })
            for (key, value) in map {
                if let Value::Array(arr) = value {
                    if arr.is_empty() {
                        return Some(format!("📋 **{}**: (0)", key));
                    }
                    return Some(format!(
                        "📋 **{}** ({}):\n{}",
                        key,
                        arr.len(),
                        format_array(arr)
                    ));
                }
            }
            // 배열이 없으면 key-value 나열
            let entries: Vec<String> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| format!("• **{}**: {}", k, value_inline(v)))
                .collect();
            (!entries.is_empty()).then(|| entries.join("\n"))
        }
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => None,
    }
}

/// 배열을 불릿 리스트로 포매팅한다. 객체 항목은 `name`/`id`/`userid`를 대표값으로 쓴다.
fn format_array(arr: &[Value]) -> String {
    arr.iter()
        .map(|item| match item {
            Value::Object(map) => {
                let name = map
                    .get("name")
                    .or_else(|| map.get("id"))
                    .or_else(|| map.get("userid"))
                    .map(value_inline)
                    .unwrap_or_else(|| "?".to_string());
                let extras: Vec<String> = map
                    .iter()
                    .filter(|(k, v)| !["name", "id"].contains(&k.as_str()) && !v.is_null())
                    .map(|(k, v)| format!("{}: {}", k, value_inline(v)))
                    .collect();
                if extras.is_empty() {
                    format!("• **{}**", name)
                } else {
                    format!("• **{}** ({})", name, extras.join(", "))
                }
            }
            other => format!("• {}", value_inline(other)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// JSON 스칼라를 따옴표 없이 인라인 표기한다.
fn value_inline(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 슈퍼바이저 status 응답에서 실행 여부를 판정한다.
///
/// `online: true/false`를 우선하고, 없으면 `status` 문자열("running"/"stopped")로
/// 판정한다. 판정 불가(필드 없음/알 수 없는 값)면 `None` — 호출부는 사전 체크를
/// 건너뛰고 기존 동작을 유지한다.
fn parse_running_state(v: &Value) -> Option<bool> {
    if let Some(online) = v.get("online").and_then(Value::as_bool) {
        return Some(online);
    }
    match v.get("status").and_then(Value::as_str) {
        Some(s) if s.eq_ignore_ascii_case("running") => Some(true),
        Some(s) if s.eq_ignore_ascii_case("stopped") => Some(false),
        _ => None,
    }
}

/// 슈퍼바이저 결과 Value에서 사람이 읽을 요약 텍스트를 추출한다.
///
/// 우선순위: `text` > `message` > `output` > status 요약. 알 수 없으면 `None`.
fn supervisor_text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("text").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(s) = v.get("message").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(s) = v.get("output").and_then(Value::as_str) {
        let t = s.trim();
        return Some(if t.is_empty() {
            "(empty response)".to_string()
        } else {
            t.to_string()
        });
    }
    if let Some(status) = v.get("status").and_then(Value::as_str) {
        let server = v.get("server").and_then(Value::as_str).unwrap_or("server");
        let online = v.get("online").and_then(Value::as_bool).unwrap_or(false);
        let icon = if online { "🟢" } else { "⚪" };
        let mut s = format!("{} {} — {}", icon, server, status);
        if let Some(pid) = v.get("pid").and_then(Value::as_u64) {
            s.push_str(&format!(" (PID {})", pid));
        }
        return Some(s);
    }
    None
}

/// 페이로드 1건을 처리해 결과를 반환한다.
///
/// `ctx`는 요청 출처(길드/유저)와 인스턴스별 설정 스냅샷을 담는다 —
/// 노드/유저 권한 판정과 인스턴스별 별명 해석이 여기(데몬)서 수행된다.
pub async fn dispatch(
    supervisor: Arc<RwLock<Supervisor>>,
    payload: &Value,
    ctx: &DispatchContext,
) -> DispatchResult {
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("raw_command");

    match action {
        "raw_command" => {
            let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
            handle_raw_command(supervisor, text, ctx).await
        }
        "ping" => DispatchResult::ok(json!({ "pong": true })),
        other => DispatchResult::err_code(
            "UNSUPPORTED_ACTION",
            format!("unsupported action: {}", other),
            json!({ "action": other }),
        ),
    }
}

async fn handle_raw_command(
    supervisor: Arc<RwLock<Supervisor>>,
    text: &str,
    ctx: &DispatchContext,
) -> DispatchResult {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return DispatchResult::err_code(
            "EMPTY_COMMAND",
            "empty command text",
            json!({}),
        );
    }

    let mut tokens = trimmed.split_whitespace();
    let first = match tokens.next() {
        Some(t) => t.to_string(),
        None => {
            return DispatchResult::err_code(
                "EMPTY_COMMAND",
                "empty command text",
                json!({}),
            )
        }
    };

    let first_lower = first.to_lowercase();
    if first_lower == "list" || first_lower == "목록" {
        return list_instances(supervisor, ctx).await;
    }

    // first == 인스턴스 토큰 (이름/모듈명/별명 중 하나)
    let token = first;
    let verb = match tokens.next() {
        Some(v) => v.to_lowercase(),
        None => {
            let guard = supervisor.read().await;
            let aliases = collect_module_aliases(&guard);
            return no_verb_result(&token, guard.instance_store.list(), &aliases, ctx);
        }
    };
    let args: Vec<String> = tokens.map(|s| s.to_string()).collect();

    // 인스턴스 해석(노드 스코프 + 인스턴스별 호출명) → verb canonical화
    // (인스턴스별 명령어 별명 → module.toml 기본) → 유저 차단 판정.
    // 판정을 통과한 뒤에만 라이프사이클/커맨드 실행으로 위임한다.
    let (instance_name, verb) = {
        let guard = supervisor.read().await;
        let aliases = collect_module_aliases(&guard);
        let inst = match resolve_instance_scoped(&token, guard.instance_store.list(), &aliases, ctx)
        {
            Ok(i) => i,
            Err(e) => return e.to_dispatch_result(),
        };
        let cmd_aliases = collect_command_aliases(&guard);
        let canonical = resolve_command_alias_scoped(&verb, inst, &cmd_aliases, ctx);
        if !ctx.is_user_allowed(&inst.id, &canonical) {
            return DispatchResult::err_code(
                "USER_BLOCKED",
                format!(
                    "user is not allowed to run '{}' on '{}'",
                    canonical, inst.name
                ),
                json!({ "name": inst.name, "command": canonical }),
            );
        }
        (inst.name.clone(), canonical)
    };

    match verb.as_str() {
        "start" => lifecycle_start(supervisor, &instance_name).await,
        "stop" => lifecycle_stop(supervisor, &instance_name, false).await,
        "kill" | "force-stop" => lifecycle_stop(supervisor, &instance_name, true).await,
        "restart" => lifecycle_restart(supervisor, &instance_name).await,
        "status" => lifecycle_status(supervisor, &instance_name).await,
        // 그 외 토큰은 모듈이 정의한 커스텀 커맨드로 위임
        _ => instance_command(supervisor, &instance_name, &verb, args).await,
    }
}

/// verb(세부 명령어) 토큰이 없을 때의 응답을 결정한다.
///
/// 인스턴스 토큰이 로컬에서 해석되지 않으면 UNKNOWN_COMMAND
/// ("알 수 없는 명령어")를 돌려준다 — `!saba <오타>`처럼 단독 토큰만
/// 입력한 경우 사용자가 명령어를 잘못 쓴 것이므로 "존재하지 않는
/// 인스턴스"보다 원인이 정확하다.
/// 노드 차단이면 NODE_NOT_ALLOWED, 해석 성공/다중 매칭이면 MISSING_VERB.
fn no_verb_result(
    token: &str,
    instances: &[ServerInstance],
    module_aliases: &HashMap<String, Vec<String>>,
    ctx: &DispatchContext,
) -> DispatchResult {
    match resolve_instance_scoped(token, instances, module_aliases, ctx) {
        Err(ResolveError::NotFound { input }) => DispatchResult::err_code(
            "UNKNOWN_COMMAND",
            format!("unknown command: {}", input),
            json!({ "command": input }),
        ),
        Err(e @ ResolveError::NodeNotAllowed { .. }) => e.to_dispatch_result(),
        _ => DispatchResult::err_code(
            "MISSING_VERB",
            format!("missing verb for instance '{}'", token),
            json!({ "name": token }),
        ),
    }
}

async fn list_instances(
    supervisor: Arc<RwLock<Supervisor>>,
    ctx: &DispatchContext,
) -> DispatchResult {
    let guard = supervisor.read().await;
    // 이 노드에서 사용 가능한 인스턴스만 노출한다.
    let instances: Vec<&ServerInstance> = guard
        .instance_store
        .list()
        .iter()
        .filter(|i| ctx.is_instance_allowed(&i.id))
        .collect();
    let names: Vec<String> = instances.iter().map(|i| i.name.clone()).collect();
    let list: Vec<Value> = instances
        .iter()
        .map(|i| {
            json!({
                "id": i.id,
                "name": i.name,
                "module": i.module_name,
            })
        })
        .collect();
    let text = if names.is_empty() {
        "📭 No servers registered.".to_string()
    } else {
        format!("📋 Registered servers ({}): {}", names.len(), names.join(", "))
    };
    DispatchResult::ok_code(
        "LIST_OK",
        text,
        json!({ "names": names }),
        json!({ "instances": list }),
    )
}

/// 토큰을 인스턴스로 해석하고 (실제 인스턴스 이름, 모듈 이름, 직렬화된 cfg)를 돌려준다.
fn resolve_for_dispatch(
    supervisor: &Supervisor,
    token: &str,
) -> Result<(String, String, String, Value), ResolveError> {
    let aliases = collect_module_aliases(supervisor);
    let inst = resolve_instance(token, supervisor.instance_store.list(), &aliases)?;
    let cfg = serde_json::to_value(inst).map_err(|_| ResolveError::NotFound {
        input: token.to_string(),
    })?;
    Ok((inst.id.clone(), inst.name.clone(), inst.module_name.clone(), cfg))
}

/// `module_settings.managed_start` 오버라이드 + 모듈 기본 `interaction_mode`로
/// managed(ManagedProcess) 시작 여부를 결정한다.
///
/// GUI(`useServerActions.js`의 `handleStart`)와 동일한 규칙을 그대로 따라야 한다 —
/// 그렇지 않으면 GUI 버튼으로 시작할 때와 Discord 명령으로 시작할 때 서로 다른
/// 코드 경로(managed vs non-managed)를 타게 되어, 둘 중 하나의 트래킹 방식에서만
/// 발생하는 버그를 재현하기 어렵고 사용자에게는 "왜 방식마다 다르게 동작하는지"
/// 알 수 없는 비일관성으로 나타난다.
fn should_use_managed_start(guard: &Supervisor, module_name: &str, cfg: &Value) -> bool {
    let override_flag = cfg
        .get("module_settings")
        .and_then(|m| m.get("managed_start"))
        .and_then(|v| v.as_bool());

    match override_flag {
        Some(explicit) => explicit,
        None => {
            let module_mode = guard
                .module_loader
                .get_module(module_name)
                .ok()
                .and_then(|m| m.metadata.interaction_mode.clone());
            // GUI 기본값: `mod?.interaction_mode || 'console'` — "commands"가 명시된
            // 경우만 non-managed, 그 외(= "console" 또는 미지정)는 managed.
            module_mode.as_deref() != Some("commands")
        }
    }
}

/// 인스턴스의 현재 실행 여부를 조회한다. 조회 실패/판정 불가면 `None`.
async fn query_running_state(
    guard: &Supervisor,
    instance_name: &str,
    module_name: &str,
) -> Option<bool> {
    match guard.get_server_status(instance_name, module_name).await {
        Ok(v) => parse_running_state(&v),
        Err(_) => None,
    }
}

async fn lifecycle_start(
    supervisor: Arc<RwLock<Supervisor>>,
    token: &str,
) -> DispatchResult {
    let (instance_id, instance_name, module_name, cfg, use_managed) = {
        let guard = supervisor.read().await;
        let (instance_id, instance_name, module_name, cfg) = match resolve_for_dispatch(&guard, token) {
            Ok(v) => v,
            Err(e) => return e.to_dispatch_result(),
        };
        // 사전 상태 체크 — 이미 실행 중이면 중복 기동을 시도하지 않는다.
        // 상태 조회가 실패하면(모듈 오류 등) 기존처럼 시작을 진행한다.
        if query_running_state(&guard, &instance_name, &module_name).await == Some(true) {
            return DispatchResult::err_code(
                "ALREADY_RUNNING",
                format!("'{}' is already running", instance_name),
                json!({ "name": instance_name }),
            );
        }
        let use_managed = should_use_managed_start(&guard, &module_name, &cfg);
        (instance_id, instance_name, module_name, cfg, use_managed)
    };

    // GUI(useServerActions.js)와 동일하게 managed_start 설정/모듈 interaction_mode에
    // 따라 managed(ManagedProcess)와 non-managed(spawn_non_managed) 경로를 갈라 호출한다.
    // 이전에는 이 분기 없이 항상 non-managed로 시작했기 때문에, GUI 버튼으로 시작할 때와
    // Discord 명령으로 시작할 때 서로 다른 프로세스 트래킹 방식을 타게 되어
    // "GUI로는 정상, Discord로는 대시보드가 계속 정지중" 같은 불일치가 발생했다.
    let start_result = if use_managed {
        let mut guard = supervisor.write().await;
        guard.start_managed_server(&instance_id, &module_name, cfg).await
    } else {
        let guard = supervisor.read().await;
        guard.start_server(&instance_name, &module_name, cfg).await
    };

    match start_result {
        // forced_code: 슈퍼바이저의 영어 메시지("... (non-managed mode)")를 그대로
        // 노출하지 않고, 릴레이 봇이 노드 로케일로 번역한 간결한 문구만 표시한다.
        Ok(v) => DispatchResult::from_supervisor_value_forced_code(
            v,
            "START_OK",
            json!({ "name": instance_name }),
            format!("✅ '{}' started.", instance_name),
        ),
        Err(e) => DispatchResult::err_code(
            "START_FAILED",
            format!("start failed: {}", e),
            json!({ "name": instance_name, "detail": e.to_string() }),
        ),
    }
}

async fn lifecycle_stop(
    supervisor: Arc<RwLock<Supervisor>>,
    token: &str,
    force: bool,
) -> DispatchResult {
    let (instance_name, module_name) = {
        let guard = supervisor.read().await;
        match resolve_for_dispatch(&guard, token) {
            Ok((_id, n, m, _)) => (n, m),
            Err(e) => return e.to_dispatch_result(),
        }
    };
    // 사전 상태 체크 — 이미 정지된 인스턴스는 중복 정지를 시도하지 않는다.
    {
        let guard = supervisor.read().await;
        if query_running_state(&guard, &instance_name, &module_name).await == Some(false) {
            return DispatchResult::err_code(
                "ALREADY_STOPPED",
                format!("'{}' is already stopped", instance_name),
                json!({ "name": instance_name }),
            );
        }
    }
    let mut guard = supervisor.write().await;
    match guard.stop_server(&instance_name, &module_name, force).await {
        // forced_code: start와 동일 — 코드 번역만 노출
        Ok(v) => DispatchResult::from_supervisor_value_forced_code(
            v,
            "STOP_OK",
            json!({ "name": instance_name }),
            format!("🛑 '{}' stopped.", instance_name),
        ),
        Err(e) => DispatchResult::err_code(
            "STOP_FAILED",
            format!("stop failed: {}", e),
            json!({ "name": instance_name, "detail": e.to_string() }),
        ),
    }
}

async fn lifecycle_restart(
    supervisor: Arc<RwLock<Supervisor>>,
    token: &str,
) -> DispatchResult {
    // stop → start 순서. write 락 잠시 잡았다 풀고 read 락으로 start.
    let stop_res = lifecycle_stop(supervisor.clone(), token, false).await;
    if !stop_res.success {
        // 이미 정지된 상태라면 restart 의도상 start로 진행한다.
        let already_stopped =
            stop_res.data.get("code").and_then(Value::as_str) == Some("ALREADY_STOPPED");
        if !already_stopped {
            return stop_res; // 그 외 stop 실패면 그대로 반환
        }
    }
    lifecycle_start(supervisor, token).await
}

async fn lifecycle_status(
    supervisor: Arc<RwLock<Supervisor>>,
    token: &str,
) -> DispatchResult {
    let guard = supervisor.read().await;
    let (_instance_id, instance_name, module_name, _cfg) = match resolve_for_dispatch(&guard, token) {
        Ok(v) => v,
        Err(e) => return e.to_dispatch_result(),
    };
    match guard.get_server_status(&instance_name, &module_name).await {
        Ok(v) => DispatchResult::from_supervisor_value_coded(
            v,
            "STATUS_OK",
            json!({ "name": instance_name }),
            format!("ℹ️ Status of '{}'", instance_name),
        ),
        Err(e) => DispatchResult::err_code(
            "STATUS_FAILED",
            format!("status failed: {}", e),
            json!({ "name": instance_name, "detail": e.to_string() }),
        ),
    }
}

/// module.toml `[commands]` 정의에 따라 위치 인자를 명명 인자 객체로 변환한다.
///
/// 구(舊) 디스코드 봇이 하던 매핑의 복원 — 모듈 `command()`는 `args`를
/// dict로 기대하므로(`args.get("message")` 등) 배열을 그대로 넘기면
/// AttributeError로 모든 인자 명령이 실패한다.
///
/// 규칙: 입력 정의 순서대로 토큰 1개씩 소비하되, 마지막 입력이 text류면
/// 남은 토큰을 전부 흡수한다 (`공지 서버 점검 예정` → message="서버 점검 예정").
/// 필수 입력(기본값 없음)이 비면 `Err(입력 이름)`.
fn map_positional_args(field: &CommandField, args: &[String]) -> Result<Value, String> {
    let mut obj = serde_json::Map::new();
    let count = field.inputs.len();
    let mut idx = 0usize;
    for (i, input) in field.inputs.iter().enumerate() {
        let is_last = i + 1 == count;
        let raw: Option<String> = if is_last {
            (idx < args.len()).then(|| args[idx..].join(" "))
        } else {
            let v = args.get(idx).cloned();
            idx += 1;
            v
        };
        match raw {
            Some(s) if !s.trim().is_empty() => {
                let is_number = input.input_type.as_deref() == Some("number");
                let value = if is_number {
                    s.trim()
                        .parse::<f64>()
                        .map(|n| json!(n))
                        .unwrap_or_else(|_| json!(s))
                } else {
                    json!(s)
                };
                obj.insert(input.name.clone(), value);
            }
            _ => {
                // 기본값이 정의된 입력은 모듈 쪽 기본값에 맡긴다 (미전달).
                if input.required.unwrap_or(false) && input.default.is_none() {
                    return Err(input.name.clone());
                }
            }
        }
    }
    Ok(Value::Object(obj))
}

async fn instance_command(
    supervisor: Arc<RwLock<Supervisor>>,
    token: &str,
    command: &str,
    args: Vec<String>,
) -> DispatchResult {
    let guard = supervisor.read().await;
    let aliases = collect_module_aliases(&guard);
    let inst = match resolve_instance(token, guard.instance_store.list(), &aliases) {
        Ok(i) => i.clone(),
        Err(e) => return e.to_dispatch_result(),
    };

    // module.toml [commands] 스키마 조회 — 인자 매핑/명령 검증의 SSOT.
    let module = guard.module_loader.get_module(&inst.module_name).ok();
    let field = module.as_ref().and_then(|m| {
        m.metadata
            .commands
            .as_ref()
            .and_then(|c| c.fields.iter().find(|f| f.name.eq_ignore_ascii_case(command)))
            .cloned()
    });

    let (final_command, final_args) = match field {
        // 정의된 명령: 위치 인자 → 명명 인자 객체 (모듈은 dict를 기대한다)
        Some(f) => match map_positional_args(&f, &args) {
            Ok(named) => (command.to_string(), named),
            Err(param) => {
                return DispatchResult::err_code(
                    "MISSING_ARG",
                    format!("missing required argument '{}' for '{}'", param, command),
                    json!({ "command": command, "param": param }),
                )
            }
        },
        None => {
            let strict = module
                .as_ref()
                .and_then(|m| m.metadata.interaction_mode.as_deref())
                == Some("commands");
            if strict {
                // commands 모드 모듈: 정의되지 않은 명령은 모듈에 보내지 않고
                // 즉시 "알 수 없는 명령어"로 응답한다 (릴레이가 로케일 번역).
                return DispatchResult::err_code(
                    "UNKNOWN_COMMAND",
                    format!("unknown command: {}", command),
                    json!({ "command": command }),
                );
            }
            // console 모드 모듈: 원문 전체를 콘솔 명령으로 전달한다
            // (`!마크 xp add p 10` → "xp add p 10").
            let raw = if args.is_empty() {
                command.to_string()
            } else {
                format!("{} {}", command, args.join(" "))
            };
            (raw, json!({}))
        }
    };

    match guard
        .execute_command_value(&inst.id, &inst.module_name, &final_command, final_args)
        .await
    {
        Ok(v) => command_reply(v),
        Err(e) => DispatchResult::err_code(
            "COMMAND_FAILED",
            format!("command failed: {}", e),
            json!({ "name": inst.name, "command": command, "detail": e.to_string() }),
        ),
    }
}

/// 모듈 커맨드 성공 응답을 디스코드 표시용 결과로 변환한다.
///
/// 우선순위:
/// 1. 모듈이 제공한 `message` (모듈 i18n으로 현지화된 문구)
/// 2. `data` 포매팅 (플레이어 목록 등 구조화 데이터 — 구 봇 formatResponse 복원)
/// 3. 표시할 것이 없으면 `CMD_DONE` 코드 — 릴레이 봇이 노드 로케일로 번역
fn command_reply(v: Value) -> DispatchResult {
    let message = v
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let data_text = v.get("data").and_then(format_command_data);

    match message.or(data_text) {
        Some(text) => DispatchResult::ok_text(text, v),
        None => DispatchResult::ok_code("CMD_DONE", "✅ Done", json!({}), v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::ServerInstance;
    use crate::supervisor::module_loader::CommandInput;
    use std::collections::HashMap;

    fn mk_inst(name: &str, module: &str) -> ServerInstance {
        ServerInstance {
            id: format!("id-{}", name),
            name: name.to_string(),
            module_name: module.to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            tracked_process_path: None,
            port: None,
            rcon_port: None,
            rcon_password: None,
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: HashMap::new(),
            server_version: None,
            extension_data: HashMap::new(),
            required_extensions: Vec::new(),
            settings_import_pending: false,
        }
    }

    fn aliases_palworld_only() -> HashMap<String, Vec<String>> {
        let mut m = HashMap::new();
        m.insert(
            "palworld".to_string(),
            vec!["팰월드".to_string(), "팔월드".to_string(), "pw".to_string()],
        );
        m
    }

    #[test]
    fn dispatch_result_helpers() {
        let ok = DispatchResult::ok(json!({"a":1}));
        assert!(ok.success);
        assert_eq!(ok.data, json!({"a":1}));
    }

    #[test]
    fn err_code_contains_code_and_args() {
        let r = DispatchResult::err_code(
            "INSTANCE_NOT_FOUND",
            "instance 'x' not found",
            json!({"name": "x"}),
        );
        assert!(!r.success);
        assert_eq!(r.data["code"], "INSTANCE_NOT_FOUND");
        assert_eq!(r.data["error"], "instance 'x' not found");
        assert_eq!(r.data["args"]["name"], "x");
    }

    #[test]
    fn ok_text_injects_text_into_object() {
        let r = DispatchResult::ok_text("hello", json!({ "a": 1 }));
        assert!(r.success);
        assert_eq!(r.data["text"], "hello");
        assert_eq!(r.data["a"], 1);
    }

    #[test]
    fn ok_text_wraps_non_object() {
        let r = DispatchResult::ok_text("hi", json!("raw"));
        assert!(r.success);
        assert_eq!(r.data["text"], "hi");
        assert_eq!(r.data["value"], "raw");
    }

    #[test]
    fn supervisor_value_success_uses_message_as_text() {
        let v = json!({
            "success": true,
            "server": "mc",
            "pid": 1234,
            "message": "Server 'mc' started with PID 1234"
        });
        let r = DispatchResult::from_supervisor_value(v, "fallback".into());
        assert!(r.success);
        assert_eq!(r.data["text"], "Server 'mc' started with PID 1234");
    }

    #[test]
    fn supervisor_value_explicit_failure_becomes_error() {
        let v = json!({
            "success": false,
            "action_required": "extension_required",
            "message": "needs extension"
        });
        let r = DispatchResult::from_supervisor_value(v, "fb".into());
        assert!(!r.success);
        assert_eq!(r.data["error"], "needs extension");
        assert!(r.data["text"].as_str().unwrap().contains("needs extension"));
    }

    #[test]
    fn supervisor_value_action_required_becomes_error() {
        let v = json!({
            "action_required": "server_jar_missing",
            "message": "jar not found"
        });
        let r = DispatchResult::from_supervisor_value(v, "fb".into());
        assert!(!r.success);
        assert_eq!(r.data["error"], "jar not found");
    }

    #[test]
    fn supervisor_value_status_summary_text() {
        let v = json!({
            "server": "mc",
            "status": "running",
            "online": true,
            "pid": 42
        });
        let r = DispatchResult::from_supervisor_value(v, "fb".into());
        assert!(r.success);
        let t = r.data["text"].as_str().unwrap();
        assert!(t.contains("mc"), "text was: {}", t);
        assert!(t.contains("running"), "text was: {}", t);
    }

    #[test]
    fn supervisor_value_falls_back_when_no_known_fields() {
        let v = json!({ "weird": 1 });
        let r = DispatchResult::from_supervisor_value(v, "기본 텍스트".into());
        assert!(r.success);
        assert_eq!(r.data["text"], "기본 텍스트");
    }

    #[test]
    fn supervisor_value_empty_output_text() {
        let v = json!({ "output": "   " });
        let r = DispatchResult::from_supervisor_value(v, "fb".into());
        assert!(r.success);
        assert_eq!(r.data["text"], "(empty response)");
    }

    #[test]
    fn ok_code_injects_code_args_and_text() {
        let r = DispatchResult::ok_code(
            "LIST_OK",
            "📋 Registered servers (1): mc",
            json!({ "names": ["mc"] }),
            json!({ "instances": [] }),
        );
        assert!(r.success);
        assert_eq!(r.data["code"], "LIST_OK");
        assert_eq!(r.data["args"]["names"][0], "mc");
        assert_eq!(r.data["text"], "📋 Registered servers (1): mc");
    }

    #[test]
    fn coded_fallback_carries_code_when_no_supervisor_text() {
        // 슈퍼바이저가 text/message/output/status 없는 값을 돌려주면
        // fallback 텍스트와 함께 code/args가 실려 릴레이가 번역할 수 있다.
        let v = json!({ "pid": 42 });
        let r = DispatchResult::from_supervisor_value_coded(
            v,
            "START_OK",
            json!({ "name": "mc" }),
            "✅ Start requested for 'mc'.".into(),
        );
        assert!(r.success);
        assert_eq!(r.data["code"], "START_OK");
        assert_eq!(r.data["args"]["name"], "mc");
        assert_eq!(r.data["text"], "✅ Start requested for 'mc'.");
    }

    #[test]
    fn coded_prefers_supervisor_text_without_code() {
        // 모듈/슈퍼바이저가 자체 메시지를 제공하면 그대로 표시하고 code는 실지 않는다.
        let v = json!({ "message": "Server 'mc' started with PID 42" });
        let r = DispatchResult::from_supervisor_value_coded(
            v,
            "START_OK",
            json!({ "name": "mc" }),
            "fallback".into(),
        );
        assert!(r.success);
        assert!(r.data.get("code").is_none());
        assert_eq!(r.data["text"], "Server 'mc' started with PID 42");
    }

    /// start/stop 라이프사이클: 슈퍼바이저가 영어 메시지를 제공하더라도
    /// 코드가 항상 실려야 릴레이 봇이 노드 로케일로 번역한다.
    /// (이 보장이 없으면 디스코드에 "Server 'X' started with PID 123
    /// (non-managed mode)" 같은 영어 원문이 그대로 노출된다)
    #[test]
    fn forced_code_overrides_supervisor_english_message() {
        let v = json!({
            "success": true,
            "message": "Server 'mc' started with PID 42 (non-managed mode)"
        });
        let r = DispatchResult::from_supervisor_value_forced_code(
            v,
            "START_OK",
            json!({ "name": "mc" }),
            "✅ 'mc' started.".into(),
        );
        assert!(r.success);
        assert_eq!(r.data["code"], "START_OK");
        assert_eq!(r.data["args"]["name"], "mc");
        // 사용자 노출 텍스트(레거시 릴레이 폴백)는 내부 메시지가 아니어야 한다
        assert_eq!(r.data["text"], "✅ 'mc' started.");
    }

    #[test]
    fn forced_code_failure_still_reports_error() {
        let v = json!({ "success": false, "message": "port in use" });
        let r = DispatchResult::from_supervisor_value_forced_code(
            v,
            "START_OK",
            json!({ "name": "mc" }),
            "fb".into(),
        );
        assert!(!r.success);
        assert_eq!(r.data["error"], "port in use");
    }

    #[test]
    fn coded_failure_path_delegates_to_plain() {
        let v = json!({ "success": false, "message": "boom" });
        let r = DispatchResult::from_supervisor_value_coded(
            v,
            "START_OK",
            json!({ "name": "mc" }),
            "fb".into(),
        );
        assert!(!r.success);
        assert!(r.data.get("code").is_none());
        assert_eq!(r.data["error"], "boom");
    }

    /// `!saba <미등록 토큰>` 처럼 세부 명령어(verb) 없이 단독 토큰만 입력하면
    /// "존재하지 않는 인스턴스"가 아니라 UNKNOWN_COMMAND("알 수 없는 명령어")로
    /// 응답해야 한다.
    #[test]
    fn no_verb_unknown_token_reports_unknown_command() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = DispatchContext::default();
        let r = no_verb_result("doesnotexist", &instances, &aliases, &ctx);
        assert!(!r.success);
        assert_eq!(r.data["code"], "UNKNOWN_COMMAND");
        assert_eq!(r.data["args"]["command"], "doesnotexist");
    }

    // ── map_positional_args (위치 인자 → 명명 인자 매핑) ──

    fn mk_field(inputs: Vec<CommandInput>) -> CommandField {
        CommandField {
            name: "test".into(),
            label: "Test".into(),
            description: None,
            method: None,
            http_method: None,
            endpoint_template: None,
            rcon_template: None,
            inputs,
        }
    }

    fn mk_input(name: &str, ty: &str, required: bool, default: Option<toml::Value>) -> CommandInput {
        CommandInput {
            name: name.into(),
            label: None,
            input_type: Some(ty.into()),
            required: Some(required),
            placeholder: None,
            default,
        }
    }

    /// 마지막 text 입력은 남은 토큰을 전부 흡수한다 (공지 메시지 등).
    #[test]
    fn map_args_last_text_input_absorbs_rest() {
        let field = mk_field(vec![mk_input("message", "text", true, None)]);
        let args = vec!["서버".to_string(), "점검".to_string(), "예정".to_string()];
        let v = map_positional_args(&field, &args).unwrap();
        assert_eq!(v["message"], "서버 점검 예정");
    }

    /// 다중 입력: 앞 입력은 토큰 1개씩, 마지막이 나머지를 흡수.
    #[test]
    fn map_args_multiple_inputs_positional() {
        let field = mk_field(vec![
            mk_input("userid", "text", true, None),
            mk_input("message", "text", false, None),
        ]);
        let args = vec!["앨리스".to_string(), "규칙".to_string(), "위반".to_string()];
        let v = map_positional_args(&field, &args).unwrap();
        assert_eq!(v["userid"], "앨리스");
        assert_eq!(v["message"], "규칙 위반");
    }

    /// number 입력은 숫자로 변환된다.
    #[test]
    fn map_args_number_input_parsed() {
        let field = mk_field(vec![
            mk_input("waittime", "number", true, Some(toml::Value::Integer(30))),
            mk_input("message", "text", false, None),
        ]);
        let args = vec!["60".to_string(), "곧 꺼짐".to_string()];
        let v = map_positional_args(&field, &args).unwrap();
        assert_eq!(v["waittime"], json!(60.0));
        assert_eq!(v["message"], "곧 꺼짐");
    }

    /// 필수 입력(기본값 없음) 누락 → Err(입력 이름).
    #[test]
    fn map_args_missing_required_reports_param() {
        let field = mk_field(vec![mk_input("message", "text", true, None)]);
        let err = map_positional_args(&field, &[]).unwrap_err();
        assert_eq!(err, "message");
    }

    /// 기본값이 있는 필수 입력은 누락돼도 통과한다 (모듈 기본값 사용).
    #[test]
    fn map_args_required_with_default_ok_when_missing() {
        let field = mk_field(vec![
            mk_input("waittime", "number", true, Some(toml::Value::Integer(30))),
            mk_input("message", "text", false, None),
        ]);
        let v = map_positional_args(&field, &[]).unwrap();
        assert!(v.get("waittime").is_none());
        assert!(v.get("message").is_none());
    }

    /// 입력이 없는 명령(players 등)은 빈 객체를 돌려준다.
    #[test]
    fn map_args_no_inputs_yields_empty_object() {
        let field = mk_field(vec![]);
        let v = map_positional_args(&field, &["잉여".to_string()]).unwrap();
        assert_eq!(v, json!({}));
    }

    /// 토큰이 인스턴스로 해석되면 기존과 동일하게 MISSING_VERB.
    #[test]
    fn no_verb_known_instance_reports_missing_verb() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = DispatchContext::default();
        // 인스턴스명 / 모듈명 / 모듈 별명 모두 MISSING_VERB
        for token in ["my-palworld-1", "palworld", "팰월드"] {
            let r = no_verb_result(token, &instances, &aliases, &ctx);
            assert!(!r.success);
            assert_eq!(r.data["code"], "MISSING_VERB", "token: {}", token);
            assert_eq!(r.data["args"]["name"], token);
        }
    }

    /// 다중 매칭(Ambiguous)은 인스턴스가 존재하는 경우이므로 MISSING_VERB 유지.
    #[test]
    fn no_verb_ambiguous_token_reports_missing_verb() {
        let instances = vec![mk_inst("pal-a", "palworld"), mk_inst("pal-b", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = DispatchContext::default();
        let r = no_verb_result("팰월드", &instances, &aliases, &ctx);
        assert!(!r.success);
        assert_eq!(r.data["code"], "MISSING_VERB");
    }

    /// 노드에서 차단된 인스턴스에 verb 없이 접근하면 NODE_NOT_ALLOWED.
    #[test]
    fn no_verb_blocked_instance_reports_node_not_allowed() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = ctx_with_allowed_nodes("id-my-palworld-1", &["guild-other"], "guild-here");
        let r = no_verb_result("팰월드", &instances, &aliases, &ctx);
        assert!(!r.success);
        assert_eq!(r.data["code"], "NODE_NOT_ALLOWED");
    }

    // ── format_command_data / command_reply (구 봇 formatResponse 복원) ──

    #[test]
    fn format_data_array_of_players() {
        let data = json!([
            { "name": "Alice", "level": 12 },
            { "name": "Bob", "level": 3 }
        ]);
        let t = format_command_data(&data).unwrap();
        assert!(t.contains("• **Alice**"), "text was: {}", t);
        assert!(t.contains("level: 12"), "text was: {}", t);
        assert!(t.contains("• **Bob**"), "text was: {}", t);
    }

    #[test]
    fn format_data_object_with_array_field() {
        let data = json!({ "players": [ { "name": "Alice" } ] });
        let t = format_command_data(&data).unwrap();
        assert!(t.starts_with("📋 **players** (1):"), "text was: {}", t);
        assert!(t.contains("• **Alice**"), "text was: {}", t);
    }

    #[test]
    fn format_data_object_key_values() {
        let data = json!({ "version": "v0.3.1", "servername": "My Pal" });
        let t = format_command_data(&data).unwrap();
        assert!(t.contains("• **version**: v0.3.1"), "text was: {}", t);
        assert!(t.contains("• **servername**: My Pal"), "text was: {}", t);
    }

    #[test]
    fn format_data_empty_returns_none() {
        assert!(format_command_data(&json!(null)).is_none());
        assert!(format_command_data(&json!({})).is_none());
        assert!(format_command_data(&json!("")).is_none());
    }

    /// 모듈이 message를 주면 그대로 사용 (모듈 i18n 현지화 문구).
    #[test]
    fn command_reply_prefers_module_message() {
        let v = json!({ "success": true, "message": "🎮 접속 중 (2): a, b", "data": { "players": [] } });
        let r = command_reply(v);
        assert!(r.success);
        assert_eq!(r.data["text"], "🎮 접속 중 (2): a, b");
    }

    /// message가 없으면 data를 포매팅해 표시 — "명령어 실행됨" 회귀 방지.
    #[test]
    fn command_reply_falls_back_to_formatted_data() {
        let v = json!({ "success": true, "data": { "players": [ { "name": "Alice" } ] } });
        let r = command_reply(v);
        assert!(r.success);
        let t = r.data["text"].as_str().unwrap();
        assert!(t.contains("**Alice**"), "text was: {}", t);
    }

    /// 표시할 것이 전혀 없으면 CMD_DONE 코드 — 릴레이가 노드 로케일로 번역.
    #[test]
    fn command_reply_empty_yields_cmd_done_code() {
        let v = json!({ "success": true, "data": {} });
        let r = command_reply(v);
        assert!(r.success);
        assert_eq!(r.data["code"], "CMD_DONE");
    }

    // ── parse_running_state (start/stop 사전 체크) ──

    #[test]
    fn running_state_prefers_online_flag() {
        assert_eq!(parse_running_state(&json!({ "online": true, "status": "stopped" })), Some(true));
        assert_eq!(parse_running_state(&json!({ "online": false })), Some(false));
    }

    #[test]
    fn running_state_falls_back_to_status_string() {
        assert_eq!(parse_running_state(&json!({ "status": "running" })), Some(true));
        assert_eq!(parse_running_state(&json!({ "status": "Running" })), Some(true));
        assert_eq!(parse_running_state(&json!({ "status": "stopped" })), Some(false));
    }

    #[test]
    fn running_state_unknown_is_none() {
        assert_eq!(parse_running_state(&json!({})), None);
        assert_eq!(parse_running_state(&json!({ "status": "starting" })), None);
    }

    #[test]
    fn resolve_by_exact_instance_name() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = HashMap::new();
        let inst = resolve_instance("my-palworld-1", &instances, &aliases).unwrap();
        assert_eq!(inst.name, "my-palworld-1");
    }

    fn command_aliases_palworld() -> HashMap<String, HashMap<String, Vec<String>>> {
        let mut cmds = HashMap::new();
        cmds.insert(
            "start".to_string(),
            vec!["시작".to_string(), "실행".to_string(), "켜기".to_string()],
        );
        cmds.insert(
            "stop".to_string(),
            vec!["정지".to_string(), "끄기".to_string()],
        );
        cmds.insert(
            "players".to_string(),
            vec!["플레이어".to_string(), "접속자".to_string()],
        );
        let mut m = HashMap::new();
        m.insert("palworld".to_string(), cmds);
        m
    }

    /// Discord에서 `!팰월드 실행` 같은 한국어 별명 verb가 canonical 명령("start")으로
    /// 치환되어야 한다. 이 해석이 없으면 릴레이 캐시 미동기화 시
    /// "Unknown REST command: 실행"으로 전부 실패한다.
    #[test]
    fn resolve_command_alias_korean_to_start() {
        let aliases = command_aliases_palworld();
        assert_eq!(resolve_command_alias("실행", "palworld", &aliases), "start");
        assert_eq!(resolve_command_alias("시작", "palworld", &aliases), "start");
        assert_eq!(resolve_command_alias("끄기", "palworld", &aliases), "stop");
        assert_eq!(resolve_command_alias("접속자", "palworld", &aliases), "players");
    }

    #[test]
    fn resolve_command_alias_passes_through_canonical_and_unknown() {
        let aliases = command_aliases_palworld();
        // canonical 이름은 그대로
        assert_eq!(resolve_command_alias("start", "palworld", &aliases), "start");
        // 등록 안 된 verb는 원문 그대로 (모듈 커스텀 커맨드로 위임)
        assert_eq!(resolve_command_alias("save", "palworld", &aliases), "save");
        // 모듈에 별명 등록이 없으면 원문 그대로
        assert_eq!(resolve_command_alias("실행", "minecraft", &aliases), "실행");
    }

    #[test]
    fn resolve_command_alias_case_insensitive() {
        let aliases = command_aliases_palworld();
        assert_eq!(resolve_command_alias("START", "palworld", &aliases), "start");
    }

    #[test]
    fn resolve_by_instance_name_case_insensitive() {
        let instances = vec![mk_inst("MyServer", "minecraft")];
        let aliases = HashMap::new();
        let inst = resolve_instance("myserver", &instances, &aliases).unwrap();
        assert_eq!(inst.name, "MyServer");
    }

    #[test]
    fn resolve_by_module_name_when_single_instance() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = HashMap::new();
        let inst = resolve_instance("palworld", &instances, &aliases).unwrap();
        assert_eq!(inst.name, "my-palworld-1");
    }

    #[test]
    fn resolve_by_module_alias_korean() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let inst = resolve_instance("팰월드", &instances, &aliases).unwrap();
        assert_eq!(inst.name, "my-palworld-1");
    }

    #[test]
    fn resolve_by_module_alias_short() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let inst = resolve_instance("pw", &instances, &aliases).unwrap();
        assert_eq!(inst.name, "my-palworld-1");
    }

    #[test]
    fn resolve_not_found() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = HashMap::new();
        match resolve_instance("doesnotexist", &instances, &aliases) {
            Err(ResolveError::NotFound { input }) => assert_eq!(input, "doesnotexist"),
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    /// 회귀 테스트: Discord "start" 명령이 GUI(useServerActions.js)와 동일하게
    /// module_settings.managed_start 오버라이드를 존중해야 한다. 이 분기가 없으면
    /// managed_start=true로 설정된 인스턴스도 Discord로 시작할 때는 항상
    /// non-managed 경로를 타서, GUI로 시작할 때와 트래킹 방식이 달라진다.
    #[test]
    fn should_use_managed_start_honors_explicit_true_override() {
        let supervisor = Supervisor::new("./modules");
        let cfg = json!({ "module_settings": { "managed_start": true } });
        // 모듈이 존재하지 않아도(=="nonexistent-module") 명시적 오버라이드가 우선해야 한다
        assert!(should_use_managed_start(&supervisor, "nonexistent-module", &cfg));
    }

    #[test]
    fn should_use_managed_start_honors_explicit_false_override() {
        let supervisor = Supervisor::new("./modules");
        let cfg = json!({ "module_settings": { "managed_start": false } });
        assert!(!should_use_managed_start(&supervisor, "nonexistent-module", &cfg));
    }

    /// 오버라이드가 없고 모듈 조회도 실패하면(예: 모듈 디렉터리 미탐색) GUI의
    /// `mod?.interaction_mode || 'console'` 기본값과 동일하게 managed로 간주한다.
    #[test]
    fn should_use_managed_start_defaults_to_managed_when_no_override_or_module() {
        let supervisor = Supervisor::new("./modules");
        let cfg = json!({ "module_settings": {} });
        assert!(should_use_managed_start(&supervisor, "nonexistent-module", &cfg));
    }

    #[test]
    fn resolve_ambiguous_when_multiple_instances_of_same_module() {
        let instances = vec![
            mk_inst("pal-a", "palworld"),
            mk_inst("pal-b", "palworld"),
        ];
        let aliases = aliases_palworld_only();
        match resolve_instance("팰월드", &instances, &aliases) {
            Err(ResolveError::Ambiguous { input, candidates }) => {
                assert_eq!(input, "팰월드");
                assert_eq!(candidates.len(), 2);
                assert!(candidates.contains(&"pal-a".to_string()));
                assert!(candidates.contains(&"pal-b".to_string()));
            }
            other => panic!("expected Ambiguous, got {:?}", other),
        }
    }

    #[test]
    fn resolve_instance_name_takes_precedence_over_module() {
        // instance literally named "palworld" should match before module fallback
        let instances = vec![
            mk_inst("palworld", "minecraft"),
            mk_inst("other", "palworld"),
        ];
        let aliases = aliases_palworld_only();
        let inst = resolve_instance("palworld", &instances, &aliases).unwrap();
        assert_eq!(inst.name, "palworld");
        assert_eq!(inst.module_name, "minecraft");
    }

    // ── DispatchContext: 노드/유저 권한 + 인스턴스별 별명 ──

    use crate::config_store::InstanceDiscordSettings;

    /// instance_id에 allowed_nodes를 설정하고, ctx.guild_id를 지정한 컨텍스트.
    fn ctx_with_allowed_nodes(instance_id: &str, nodes: &[&str], guild: &str) -> DispatchContext {
        let mut settings = HashMap::new();
        settings.insert(
            instance_id.to_string(),
            InstanceDiscordSettings {
                allowed_nodes: Some(nodes.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            },
        );
        DispatchContext {
            guild_id: Some(guild.to_string()),
            requested_by: None,
            instance_settings: settings,
        }
    }

    /// 미설정 인스턴스는 어떤 노드에서든 사용 가능 (기존 동작 호환).
    #[test]
    fn scoped_resolve_unconfigured_instance_allowed_everywhere() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = DispatchContext {
            guild_id: Some("any-guild".into()),
            ..Default::default()
        };
        let inst = resolve_instance_scoped("팰월드", &instances, &aliases, &ctx).unwrap();
        assert_eq!(inst.name, "my-palworld-1");
    }

    /// 허용 목록에 없는 노드에서는 NODE_NOT_ALLOWED.
    #[test]
    fn scoped_resolve_blocked_on_other_node() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = ctx_with_allowed_nodes("id-my-palworld-1", &["guild-a"], "guild-b");
        match resolve_instance_scoped("팰월드", &instances, &aliases, &ctx) {
            Err(ResolveError::NodeNotAllowed { name }) => assert_eq!(name, "my-palworld-1"),
            other => panic!("expected NodeNotAllowed, got {:?}", other),
        }
        // 허용된 노드에서는 정상 해석
        let ctx_ok = ctx_with_allowed_nodes("id-my-palworld-1", &["guild-a"], "guild-a");
        assert!(resolve_instance_scoped("팰월드", &instances, &aliases, &ctx_ok).is_ok());
    }

    /// allowedNodes: [] (전부 체크 해제)는 모든 노드에서 차단.
    #[test]
    fn scoped_resolve_empty_allowed_nodes_denies_all() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let ctx = ctx_with_allowed_nodes("id-my-palworld-1", &[], "guild-a");
        assert!(matches!(
            resolve_instance_scoped("팰월드", &instances, &aliases, &ctx),
            Err(ResolveError::NodeNotAllowed { .. })
        ));
    }

    /// guild_id가 없으면(비-디스코드 경로) 노드 제한을 적용하지 않는다.
    #[test]
    fn scoped_resolve_no_guild_skips_node_check() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let mut ctx = ctx_with_allowed_nodes("id-my-palworld-1", &["guild-a"], "guild-b");
        ctx.guild_id = None;
        assert!(resolve_instance_scoped("팰월드", &instances, &aliases, &ctx).is_ok());
    }

    /// 같은 모듈 인스턴스 2개가 서로 다른 노드에 배정되어 있으면,
    /// 공유 별명("팰월드")이 각 노드에서 그 노드의 인스턴스로 해석된다
    /// (기존에는 Ambiguous로 실패하던 케이스).
    #[test]
    fn scoped_resolve_disambiguates_by_node() {
        let instances = vec![mk_inst("pal-a", "palworld"), mk_inst("pal-b", "palworld")];
        let aliases = aliases_palworld_only();
        let mut settings = HashMap::new();
        settings.insert(
            "id-pal-a".to_string(),
            InstanceDiscordSettings {
                allowed_nodes: Some(vec!["guild-1".into()]),
                ..Default::default()
            },
        );
        settings.insert(
            "id-pal-b".to_string(),
            InstanceDiscordSettings {
                allowed_nodes: Some(vec!["guild-2".into()]),
                ..Default::default()
            },
        );
        let ctx1 = DispatchContext {
            guild_id: Some("guild-1".into()),
            requested_by: None,
            instance_settings: settings.clone(),
        };
        let inst = resolve_instance_scoped("팰월드", &instances, &aliases, &ctx1).unwrap();
        assert_eq!(inst.name, "pal-a");

        let ctx2 = DispatchContext {
            guild_id: Some("guild-2".into()),
            requested_by: None,
            instance_settings: settings,
        };
        let inst = resolve_instance_scoped("팰월드", &instances, &aliases, &ctx2).unwrap();
        assert_eq!(inst.name, "pal-b");
    }

    /// 인스턴스 호출명 오버라이드: 지정한 별명만 매칭되고 module.toml 기본은 무시.
    #[test]
    fn scoped_resolve_instance_alias_override() {
        let instances = vec![mk_inst("my-palworld-1", "palworld")];
        let aliases = aliases_palworld_only();
        let mut settings = HashMap::new();
        settings.insert(
            "id-my-palworld-1".to_string(),
            InstanceDiscordSettings {
                aliases: Some(vec!["우리팰".to_string()]),
                ..Default::default()
            },
        );
        let ctx = DispatchContext {
            guild_id: None,
            requested_by: None,
            instance_settings: settings,
        };
        // 오버라이드 별명은 매칭
        let inst = resolve_instance_scoped("우리팰", &instances, &aliases, &ctx).unwrap();
        assert_eq!(inst.name, "my-palworld-1");
        // module.toml 기본 별명은 더 이상 매칭되지 않음 (교체 의도)
        assert!(matches!(
            resolve_instance_scoped("팰월드", &instances, &aliases, &ctx),
            Err(ResolveError::NotFound { .. })
        ));
        // 모듈 이름 자체는 항상 매칭
        assert!(resolve_instance_scoped("palworld", &instances, &aliases, &ctx).is_ok());
    }

    /// 명령어 별명 오버라이드: 오버라이드 우선, 오버라이드된 canonical의
    /// 기본 별명은 무시, 나머지 명령은 module.toml 기본으로 폴백.
    #[test]
    fn scoped_command_alias_override() {
        let inst = mk_inst("my-palworld-1", "palworld");
        let module_cmds = command_aliases_palworld();
        let mut cmd_overrides = HashMap::new();
        cmd_overrides.insert("start".to_string(), vec!["기동".to_string()]);
        let mut settings = HashMap::new();
        settings.insert(
            "id-my-palworld-1".to_string(),
            InstanceDiscordSettings {
                command_aliases: cmd_overrides,
                ..Default::default()
            },
        );
        let ctx = DispatchContext {
            guild_id: None,
            requested_by: None,
            instance_settings: settings,
        };
        // 오버라이드 별명 매칭
        assert_eq!(resolve_command_alias_scoped("기동", &inst, &module_cmds, &ctx), "start");
        // 오버라이드된 start의 기본 별명("실행")은 무시됨
        assert_eq!(resolve_command_alias_scoped("실행", &inst, &module_cmds, &ctx), "실행");
        // 오버라이드 안 된 stop은 기본 별명 유지
        assert_eq!(resolve_command_alias_scoped("끄기", &inst, &module_cmds, &ctx), "stop");
        // 오버라이드가 아예 없는 인스턴스는 기본 동작
        let ctx_empty = DispatchContext::default();
        assert_eq!(resolve_command_alias_scoped("실행", &inst, &module_cmds, &ctx_empty), "start");
    }

    /// 유저 블랙리스트: 차단 목록의 유저만 거부, 그 외 전원 허용.
    #[test]
    fn user_blacklist_blocks_only_listed_users() {
        let mut blocked = HashMap::new();
        blocked.insert("stop".to_string(), vec!["user-bad".to_string()]);
        let mut settings = HashMap::new();
        settings.insert(
            "inst-1".to_string(),
            InstanceDiscordSettings {
                blocked_users: blocked,
                ..Default::default()
            },
        );
        let mk_ctx = |user: &str| DispatchContext {
            guild_id: None,
            requested_by: Some(user.to_string()),
            instance_settings: settings.clone(),
        };
        assert!(!mk_ctx("user-bad").is_user_allowed("inst-1", "stop"));
        assert!(mk_ctx("user-good").is_user_allowed("inst-1", "stop"));
        // 차단 목록이 없는 명령은 전원 허용
        assert!(mk_ctx("user-bad").is_user_allowed("inst-1", "start"));
        // 설정이 없는 인스턴스는 전원 허용
        assert!(mk_ctx("user-bad").is_user_allowed("inst-other", "stop"));
        // requested_by가 없으면(시스템 발신) 제한 없음
        let ctx_system = DispatchContext {
            guild_id: None,
            requested_by: None,
            instance_settings: settings.clone(),
        };
        assert!(ctx_system.is_user_allowed("inst-1", "stop"));
    }

    /// stop이 차단된 유저는 kill/force-stop으로 우회할 수 없다.
    #[test]
    fn user_blacklist_stop_also_covers_kill() {
        let mut blocked = HashMap::new();
        blocked.insert("stop".to_string(), vec!["user-bad".to_string()]);
        let mut settings = HashMap::new();
        settings.insert(
            "inst-1".to_string(),
            InstanceDiscordSettings {
                blocked_users: blocked,
                ..Default::default()
            },
        );
        let ctx = DispatchContext {
            guild_id: None,
            requested_by: Some("user-bad".to_string()),
            instance_settings: settings,
        };
        assert!(!ctx.is_user_allowed("inst-1", "kill"));
        assert!(!ctx.is_user_allowed("inst-1", "force-stop"));
    }
}
