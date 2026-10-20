//! 설정값 타입 검증 및 포트 충돌 검사 모듈
//!
//! - `validate_setting_value`: module.toml의 SettingField 스키마를 기반으로
//!   개별 설정값의 타입·범위·필수여부를 검증합니다.
//! - `validate_settings_patch`: 부분 변경 요청에 들어 있는 설정 필드만 검증합니다.
//! - `check_port_conflicts`: 실행 중인 인스턴스와 포트 충돌을 검사합니다.
//! - `shift_ports_clear_of`: 새 인스턴스의 포트를 다른 인스턴스와 겹치지 않게 옮깁니다.

use crate::instance::ServerInstance;
use crate::supervisor::module_loader::SettingField;
use serde_json::Value;

/// 개별 설정 필드 검증 에러
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
    pub error_type: ValidationErrorType,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorType {
    Required,
    TypeMismatch,
    OutOfRange,
    InvalidOption,
    /// 모듈이 정한 형식(`format`·`pattern`)과 다르다
    InvalidFormat,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// 포트 충돌 정보
#[derive(Debug, Clone)]
pub struct PortConflict {
    /// 충돌하는 포트 번호
    pub port: u16,
    /// 포트 종류 ("port", "rest_port", 모듈 포트 설정 이름)
    pub port_type: String,
    /// 충돌하는 상대 인스턴스 이름
    pub conflicting_instance_name: String,
    /// 충돌하는 상대 인스턴스 ID
    pub conflicting_instance_id: String,
    /// 상대 인스턴스의 포트 종류
    pub conflicting_port_type: String,
}

impl std::fmt::Display for PortConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Port {} ({}) conflicts with instance '{}' ({})",
            self.port, self.port_type, self.conflicting_instance_name, self.conflicting_port_type
        )
    }
}

/// 단일 설정값의 타입·범위·필수여부를 검증합니다.
///
/// # Arguments
/// * `field` - module.toml에서 정의된 필드 스키마
/// * `value` - 사용자가 입력한 설정값 (JSON)
///
/// # Returns
/// * `Ok(())` - 유효한 값
/// * `Err(ValidationError)` - 검증 실패
pub fn validate_setting_value(
    field: &SettingField,
    value: Option<&Value>,
) -> Result<(), ValidationError> {
    let field_name = &field.name;

    // 1. 필수 필드 검사
    match value {
        None | Some(Value::Null) => {
            if field.required == Some(true) {
                return Err(ValidationError {
                    field: field_name.clone(),
                    message: format!("Required field '{}' is missing", field_name),
                    error_type: ValidationErrorType::Required,
                });
            }
            // 필수가 아니면 값이 없어도 OK
            return Ok(());
        }
        Some(Value::Array(items)) if items.is_empty() && field.field_type == "list" => {
            if field.required == Some(true) {
                return Err(ValidationError {
                    field: field_name.clone(),
                    message: format!("Required field '{}' is empty", field_name),
                    error_type: ValidationErrorType::Required,
                });
            }
            return validate_list(field, &[]);
        }
        Some(Value::String(s)) if s.is_empty() => {
            if field.required == Some(true) {
                return Err(ValidationError {
                    field: field_name.clone(),
                    message: format!("Required field '{}' is empty", field_name),
                    error_type: ValidationErrorType::Required,
                });
            }
            return Ok(());
        }
        _ => {}
    }

    let val = value.unwrap();

    // 2. 타입별 검증
    match field.field_type.as_str() {
        "number" => validate_number(field, val),
        "boolean" => validate_boolean(field, val),
        "select" => validate_select(field, val),
        "text" | "password" | "file" => validate_string(field, val),
        // 목록은 항목마다 형식을 보므로 여기서 끝난다
        "list" => return validate_list_value(field, val),
        "multiline" => validate_multiline(field, val),
        _ => Ok(()), // 알 수 없는 타입은 통과
    }?;

    // 3. 형식 (`format`·`pattern`)
    validate_format(field, val)
}

/// 이 필드가 받는 값의 종류 (오류의 `expected`). 형식이 있으면 그 이름
pub fn expected_of(field: &SettingField) -> &str {
    if let Some(format) = field.format.as_deref() {
        return format;
    }
    if field.pattern.is_some() {
        return "pattern";
    }
    match field.field_type.as_str() {
        "number" if field.step.is_some_and(|s| s.fract() == 0.0) => "integer",
        "number" => "number",
        "boolean" => "boolean",
        "select" => "enum",
        "file" => "path",
        _ => "text",
    }
}

fn format_ok(format: &str, s: &str) -> bool {
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    match format {
        // 4G, 1024M, 512k: 숫자 뒤에 단위 한 글자. 4096·4GB·"4 G"는 아니다
        "memory_size" => s.strip_suffix(['K', 'M', 'G', 'k', 'm', 'g']).is_some_and(digits),
        "port" => digits(s) && s.parse::<u32>().is_ok_and(|p| (1..=65535).contains(&p)),
        "integer" => digits(s.strip_prefix('-').unwrap_or(s)),
        "path" => !s.trim().is_empty() && !s.contains(['\r', '\n', '\0']),
        _ => true,
    }
}

/// 모듈이 정한 형식: 이름 붙은 형식과 정규식. 둘 다 값 전체가 맞아야 한다
fn validate_format(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    let s = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return Ok(()),
    };
    let format_bad = field.format.as_deref().is_some_and(|f| !format_ok(f, &s));
    let pattern_bad = field
        .pattern
        .as_deref()
        .and_then(|p| regex::Regex::new(&format!("^(?:{})$", p)).ok())
        .is_some_and(|re| !re.is_match(&s));
    if format_bad || pattern_bad {
        return Err(ValidationError {
            field: field.name.clone(),
            message: format!("'{}' value '{}' is not in the expected format ({})", field.name, s, expected_of(field)),
            error_type: ValidationErrorType::InvalidFormat,
        });
    }
    Ok(())
}

/// 숫자 타입 검증: 파싱 가능 여부, min/max 범위
fn validate_number(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    let num = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    };

    let num = match num {
        Some(n) => n,
        None => {
            return Err(ValidationError {
                field: field.name.clone(),
                message: format!(
                    "'{}' must be a number, got: {}",
                    field.name,
                    value
                ),
                error_type: ValidationErrorType::TypeMismatch,
            });
        }
    };

    // 범위 검사
    if let Some(min) = field.min {
        if num < min {
            return Err(ValidationError {
                field: field.name.clone(),
                message: format!(
                    "'{}' value {} is below minimum {}",
                    field.name, num, min
                ),
                error_type: ValidationErrorType::OutOfRange,
            });
        }
    }

    if let Some(max) = field.max {
        if num > max {
            return Err(ValidationError {
                field: field.name.clone(),
                message: format!(
                    "'{}' value {} exceeds maximum {}",
                    field.name, num, max
                ),
                error_type: ValidationErrorType::OutOfRange,
            });
        }
    }

    Ok(())
}

/// 불리언 타입 검증
fn validate_boolean(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    match value {
        Value::Bool(_) => Ok(()),
        Value::String(s) if s == "true" || s == "false" => Ok(()),
        _ => Err(ValidationError {
            field: field.name.clone(),
            message: format!(
                "'{}' must be a boolean (true/false), got: {}",
                field.name, value
            ),
            error_type: ValidationErrorType::TypeMismatch,
        }),
    }
}

/// 프리셋(select) 타입 검증: 허용된 옵션 목록에 포함 여부
fn validate_select(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    let val_str = match value {
        Value::String(s) => s.clone(),
        _ => value.to_string().trim_matches('"').to_string(),
    };

    if let Some(ref options) = field.options {
        if !options.iter().any(|opt| opt == &val_str) {
            return Err(ValidationError {
                field: field.name.clone(),
                message: format!(
                    "'{}' value '{}' is not a valid option. Valid options: {:?}",
                    field.name, val_str, options
                ),
                error_type: ValidationErrorType::InvalidOption,
            });
        }
    }

    Ok(())
}

/// 문자열 타입 검증 (text, password, file)
fn validate_string(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    match value {
        // 값은 설정 파일의 한 줄·실행 인자 하나가 된다: 줄바꿈은 다른 키를 덮어쓸 수 있다
        Value::String(s) if s.contains(['\r', '\n', '\0']) => Err(ValidationError {
            field: field.name.clone(),
            message: format!("'{}' cannot contain line breaks", field.name),
            error_type: ValidationErrorType::InvalidFormat,
        }),
        Value::String(_) => Ok(()),
        Value::Number(_) | Value::Bool(_) => Ok(()), // 숫자/불리언도 문자열로 변환 가능
        _ => Err(ValidationError {
            field: field.name.clone(),
            message: format!(
                "'{}' must be a string, got: {}",
                field.name, value
            ),
            error_type: ValidationErrorType::TypeMismatch,
        }),
    }
}

/// 목록 타입 (06 §8.1): 문자열 배열. 항목마다 `options`·`format`·`pattern`을 보고, 빈 항목·같은 값 두 번은 형식 오류다
fn validate_list_value(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    let items: Option<Vec<&str>> = value.as_array().and_then(|a| a.iter().map(Value::as_str).collect());
    let Some(items) = items else {
        return Err(ValidationError {
            field: field.name.clone(),
            message: format!("'{}' must be a list of strings, got: {}", field.name, value),
            error_type: ValidationErrorType::TypeMismatch,
        });
    };
    validate_list(field, &items)
}

fn validate_list(field: &SettingField, items: &[&str]) -> Result<(), ValidationError> {
    let error = |message: String, error_type| Err(ValidationError { field: field.name.clone(), message, error_type });
    let count = items.len() as u32;
    if field.min_items.is_some_and(|min| count < min) || field.max_items.is_some_and(|max| count > max) {
        return error(format!("'{}' has {} items, outside {:?}..={:?}", field.name, count, field.min_items, field.max_items), ValidationErrorType::OutOfRange);
    }
    for (i, item) in items.iter().enumerate() {
        if item.is_empty() || item.contains(['\r', '\n', '\0']) || items[..i].contains(item) {
            return error(format!("'{}' items must be non-empty, single-line and different", field.name), ValidationErrorType::InvalidFormat);
        }
        if let Some(options) = field.options.as_ref().filter(|o| !o.iter().any(|x| x == item)) {
            return error(format!("'{}' item '{}' is not a valid option. Valid options: {:?}", field.name, item, options), ValidationErrorType::InvalidOption);
        }
        validate_format(field, &Value::from(*item))?;
    }
    Ok(())
}

/// 여러 줄 타입 (06 §8.1): 줄바꿈을 받는 문자열. `max_length`는 글자 수
fn validate_multiline(field: &SettingField, value: &Value) -> Result<(), ValidationError> {
    let error = |message: String, error_type| Err(ValidationError { field: field.name.clone(), message, error_type });
    let Some(s) = value.as_str() else {
        return error(format!("'{}' must be a string, got: {}", field.name, value), ValidationErrorType::TypeMismatch);
    };
    if s.contains('\0') {
        return error(format!("'{}' cannot contain a NUL character", field.name), ValidationErrorType::InvalidFormat);
    }
    if field.max_length.is_some_and(|max| s.chars().count() > max as usize) {
        return error(format!("'{}' is longer than {:?} characters", field.name, field.max_length), ValidationErrorType::OutOfRange);
    }
    Ok(())
}

/// 06 §8.1 옮기기: 문자열로 저장된 값이 모듈 업데이트로 `list` 설정이 됐으면 쉼표로 나눠 배열로 바꾼다
/// (앞뒤 공백·빈 항목은 버린다). 바꾼 것이 있으면 true. 파일에는 다음 저장 때 쓴다
pub fn migrate_list_values(fields: &[SettingField], settings: &mut std::collections::HashMap<String, Value>) -> bool {
    let mut changed = false;
    for field in fields.iter().filter(|f| f.field_type == "list") {
        if let Some(Value::String(text)) = settings.get(&field.name) {
            let items: Vec<Value> = text.split(',').map(str::trim).filter(|i| !i.is_empty()).map(Value::from).collect();
            settings.insert(field.name.clone(), Value::Array(items));
            changed = true;
        }
    }
    changed
}

/// 06 §8.2: `visible_when`에 따라 지금 숨겨진 설정 이름. 값이 없으면 그 설정의 `default`로 비교한다
pub fn hidden_fields(fields: &[SettingField], value_of: impl Fn(&str) -> Option<Value>) -> Vec<String> {
    fields
        .iter()
        .filter(|f| {
            let Some(cond) = &f.visible_when else { return false };
            let Some(target) = fields.iter().find(|t| t.name == cond.field) else { return false };
            let current = value_of(&target.name).filter(|v| !v.is_null()).or_else(|| target.default.as_ref().and_then(|d| serde_json::to_value(d).ok()));
            let Some(current) = current else { return true };
            !cond.values().into_iter().any(|want| same_setting_value(&target.field_type, &current, want))
        })
        .map(|f| f.name.clone())
        .collect()
}

/// 설정 타입으로 바꾼 값끼리 비교한다: boolean은 참·거짓, number는 수 값("25565"와 25565는 같다), 나머지는 문자열
fn same_setting_value(field_type: &str, a: &Value, b: &Value) -> bool {
    let text = |v: &Value| match v {
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    match field_type {
        "boolean" => {
            let flag = |v: &Value| v.as_bool().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
            flag(a).is_some() && flag(a) == flag(b)
        }
        "number" => {
            let num = |v: &Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
            num(a).is_some() && num(a) == num(b)
        }
        _ => text(a) == text(b),
    }
}

/// 부분 변경(PATCH) 검증: 요청에 들어 있는 필드만 검사한다.
/// 보내지 않은 필수 필드는 기존 값을 유지하므로 오류가 아니다.
/// 필수 필드를 `null`·빈 문자열로 지우려는 요청은 여전히 오류다.
pub fn validate_settings_patch(
    fields: &[SettingField],
    settings: &serde_json::Map<String, Value>,
) -> Vec<ValidationError> {
    fields
        .iter()
        .filter_map(|field| {
            let value = settings.get(&field.name)?;
            validate_setting_value(field, Some(value)).err()
        })
        .collect()
}

/// 인스턴스의 포트가 다른 실행 중인 인스턴스와 충돌하는지 검사합니다.
///
/// 모듈의 지원 프로토콜 정보(`module_protocols`)가 제공되면,
/// 해당 모듈이 실제로 사용하는 프로토콜의 포트만 비교합니다.
/// 예: REST를 지원하지 않는 모듈의 `rest_port`는 충돌 검사에서 제외됩니다.
///
/// # Arguments
/// * `target` - 검사할 인스턴스
/// * `all_instances` - 모든 인스턴스 목록
/// * `running_ids` - 현재 실행 중인 인스턴스 ID 집합
/// * `module_protocols` - 모듈별 지원 프로토콜 맵 (module_name → ["rcon", "rest", ...]).
///   `None`이면 모든 포트를 비교합니다 (하위 호환).
///
/// # Returns
/// 충돌 목록 (비어있으면 충돌 없음)
pub fn check_port_conflicts(
    target: &ServerInstance,
    all_instances: &[ServerInstance],
    running_ids: &std::collections::HashSet<String>,
    module_protocols: Option<&std::collections::HashMap<String, Vec<String>>>,
) -> Vec<PortConflict> {
    let mut conflicts = Vec::new();

    // 대상 인스턴스에서 실제 사용하는 포트만 수집
    let target_ports = collect_active_ports(target, module_protocols);

    if target_ports.is_empty() {
        return conflicts;
    }

    for other in all_instances {
        // 자기 자신은 제외
        if other.id == target.id {
            continue;
        }

        // 실행 중인 인스턴스만 검사
        if !running_ids.contains(&other.id) {
            continue;
        }

        // 상대 인스턴스에서 실제 사용하는 포트만 수집
        let other_ports = collect_active_ports(other, module_protocols);

        for &(target_port, target_type) in &target_ports {
            for &(other_port, other_type) in &other_ports {
                if target_port == other_port {
                    conflicts.push(PortConflict {
                        port: target_port,
                        port_type: target_type.to_string(),
                        conflicting_instance_name: other.name.clone(),
                        conflicting_instance_id: other.id.clone(),
                        conflicting_port_type: other_type.to_string(),
                    });
                }
            }
        }
    }

    conflicts
}

/// 인스턴스가 쓰는 포트 `(설정 이름, 값)`: 인스턴스 필드(`port`, `rest_port`)와
/// 모듈 설정 중 `port = true`로 표시된 값
pub fn instance_ports(instance: &ServerInstance, port_fields: &[&str]) -> Vec<(String, u16)> {
    let mut ports: Vec<(String, u16)> = [("port", instance.port), ("rest_port", instance.rest_port)]
        .into_iter()
        .filter_map(|(name, port)| port.map(|p| (name.to_string(), p)))
        .collect();
    for name in port_fields {
        if crate::supervisor::module_loader::CORE_PORT_FIELDS.contains(name) {
            continue;
        }
        let value = instance.module_settings.get(*name).and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        });
        if let Some(port) = value.and_then(|p| u16::try_from(p).ok()) {
            ports.push((name.to_string(), port));
        }
    }
    ports
}

/// 포트를 옮기는 폭의 단위. 게임이 몰래 쓰는 옆 포트(예: 좀보이드 port+1)와도 겹치지 않게 넉넉히 둔다
pub const PORT_SHIFT_STEP: u16 = 10;

/// `taken`인 포트가 있으면 인스턴스의 모든 포트를 같은 폭(`PORT_SHIFT_STEP`의 배수)만큼 옮긴다.
/// 같은 게임의 포트 묶음이 함께 움직이므로 서로의 간격이 유지된다. 옮긴 폭을 돌려준다(안 옮겼으면 None).
pub fn shift_ports_clear_of(instance: &mut ServerInstance, port_fields: &[&str], taken: impl Fn(u16) -> bool) -> Option<u16> {
    let ports = instance_ports(instance, port_fields);
    let delta = (0..=u16::MAX / PORT_SHIFT_STEP).map(|k| k * PORT_SHIFT_STEP).find(|&delta| {
        ports.iter().all(|(_, p)| p.checked_add(delta).is_some_and(|q| !taken(q)))
    })?;
    if delta == 0 {
        return None;
    }
    for (name, port) in ports {
        let moved = port + delta;
        match name.as_str() {
            "port" => instance.port = Some(moved),
            "rest_port" => instance.rest_port = Some(moved),
            _ => {}
        }
        // 인스턴스 필드는 같은 이름의 모듈 설정이 있을 때만 그것도 맞춘다
        let core = crate::supervisor::module_loader::CORE_PORT_FIELDS.contains(&name.as_str());
        if !core || instance.module_settings.contains_key(&name) {
            instance.module_settings.insert(name, serde_json::json!(moved));
        }
    }
    Some(delta)
}

/// 시작 전 검사에 쓰는 포트 `(설정 이름, 값)`: 인스턴스 간 검사와 같은 포트에
/// 모듈 설정 중 `port = true`로 표시된 값을 더한다
pub fn start_check_ports(
    instance: &ServerInstance,
    module_protocols: Option<&std::collections::HashMap<String, Vec<String>>>,
    port_fields: &[&str],
) -> Vec<(String, u16)> {
    let mut ports: Vec<(String, u16)> = collect_active_ports(instance, module_protocols)
        .into_iter()
        .map(|(port, name)| (name.to_string(), port))
        .collect();
    for (name, port) in instance_ports(instance, port_fields) {
        if !crate::supervisor::module_loader::CORE_PORT_FIELDS.contains(&name.as_str()) && !ports.iter().any(|(_, p)| *p == port) {
            ports.push((name, port));
        }
    }
    ports
}

/// 다른 프로그램이 이미 잡고 있는 포트를 찾는다. 게임마다 TCP·UDP가 달라 둘 다 보고,
/// 어느 쪽이든 사용 중이면 그 포트를 돌려준다
pub fn ports_held_elsewhere(ports: &[(String, u16)]) -> Vec<(String, u16)> {
    #[cfg(windows)]
    {
        // Windows는 0.0.0.0에 포트를 열면 방화벽 허용 알림을 띄우므로 묶어 보지 않고 OS의 사용 중 포트 목록을 읽는다
        let held = held_ports::read();
        ports.iter().filter(|(_, port)| *port != 0 && held.contains(port)).cloned().collect()
    }
    #[cfg(not(windows))]
    ports_held_by_binding(ports)
}

/// Windows가 예약한(제외 범위) 포트를 찾는다. Hyper-V·WinNAT 등이 부팅마다 범위를 잡아 두며, 그 안의 포트는
/// 어떤 프로그램도 묶지 못한다(게임 서버가 조용히 그 포트를 열지 못한다). 사용 중 포트 목록에는 나오지 않는다.
/// 범위는 TCP·UDP마다 따로 잡히므로 시작을 막을 때는 그 포트가 쓰는 프로토콜이 막혔을 때만 예약으로 본다
pub fn ports_reserved_by_windows(ports: &[(String, u16)]) -> Vec<(String, u16)> {
    ports.iter().filter(|(name, port)| {
        let (tcp, udp) = reserved_protocols(*port);
        blocks_start(name, tcp, udp)
    }).cloned().collect()
}

/// 시작을 막을 예약인가. REST·RCON은 TCP만 쓰므로 TCP가 막히면 막힌 것이고, 게임 포트처럼 프로토콜을
/// 모르는 포트는 둘 다 막혔을 때만 막힌 것으로 본다 (UDP만 쓰는 팰월드 8211이 TCP 범위에 걸려도 돈다)
fn blocks_start(name: &str, tcp_denied: bool, udp_denied: bool) -> bool {
    if matches!(name, "rest_port" | "rcon_port") {
        tcp_denied
    } else {
        tcp_denied && udp_denied
    }
}

/// 루프백에 묶어 보아 TCP나 UDP가 권한 거부면 예약된 것이다. 새 인스턴스의 포트를 고를 때 쓴다(어느 쪽이든 피한다)
pub fn port_reserved_by_windows(port: u16) -> bool {
    let (tcp, udp) = reserved_protocols(port);
    tcp || udp
}

/// (TCP 거부, UDP 거부). 루프백은 방화벽 알림을 띄우지 않는다. Windows 밖에서는 늘 (false, false)
fn reserved_protocols(port: u16) -> (bool, bool) {
    #[cfg(windows)]
    {
        if port == 0 {
            return (false, false);
        }
        let addr = (std::net::Ipv4Addr::LOCALHOST, port);
        let denied = |e: std::io::Error| e.kind() == std::io::ErrorKind::PermissionDenied;
        (std::net::TcpListener::bind(addr).err().is_some_and(denied), std::net::UdpSocket::bind(addr).err().is_some_and(denied))
    }
    #[cfg(not(windows))]
    {
        let _ = port;
        (false, false)
    }
}

/// TCP 포트(REST·RCON)가 Windows 예약 범위에 들었는가
pub fn tcp_port_reserved_by_windows(port: u16) -> bool {
    reserved_protocols(port).0
}

/// 기본 REST 포트가 막혔을 때 옮길 곳: `PORT_SHIFT_STEP` 간격으로 올라가며 `blocked`가 아닌 첫 포트
pub fn next_free_port(from: u16, blocked: impl Fn(u16) -> bool) -> Option<u16> {
    (1..=u16::MAX / PORT_SHIFT_STEP)
        .filter_map(|k| from.checked_add(k * PORT_SHIFT_STEP))
        .find(|&p| !blocked(p))
}

#[cfg(not(windows))]
fn ports_held_by_binding(ports: &[(String, u16)]) -> Vec<(String, u16)> {
    let in_use = |e: std::io::Error| {
        // Windows는 다른 프로그램이 독점(SO_EXCLUSIVEADDRUSE)으로 잡은 포트나 예약 범위에 PermissionDenied를 준다
        e.kind() == std::io::ErrorKind::AddrInUse || (cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied)
    };
    ports
        .iter()
        .filter(|(_, port)| *port != 0)
        .filter(|(_, port)| {
            let addr = (std::net::Ipv4Addr::UNSPECIFIED, *port);
            std::net::TcpListener::bind(addr).err().is_some_and(in_use)
                || std::net::UdpSocket::bind(addr).err().is_some_and(in_use)
        })
        .cloned()
        .collect()
}

/// TCP는 듣고 있는 포트, UDP는 묶인 포트. 목록을 못 읽으면 빈 집합 (검사를 건너뛴다).
/// IPv4는 주소와 상관없이 센다(예전 0.0.0.0 바인딩 검사도 막혔다). IPv6는 `[::]`에 묶인 것만 센다:
/// 자바 서버처럼 dual-stack으로 듣는 소켓은 IPv6 표에만 나오고, 특정 IPv6 주소만 쓰는 소켓은 IPv4 포트와 겹치지 않는다
#[cfg(windows)]
pub(crate) use held_ports::tcp_listener_pids;

#[cfg(windows)]
mod held_ports {
    use std::collections::HashSet;
    use winapi::shared::tcpmib::{MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID};
    use winapi::shared::udpmib::{MIB_UDP6TABLE_OWNER_PID, MIB_UDPTABLE_OWNER_PID};
    use winapi::shared::ws2def::{AF_INET, AF_INET6};
    use winapi::um::iphlpapi::{GetExtendedTcpTable, GetExtendedUdpTable};
    use winapi::shared::iprtrmib::{TCP_TABLE_OWNER_PID_LISTENER, UDP_TABLE_OWNER_PID};

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    /// 표를 받을 버퍼를 크기가 맞을 때까지 다시 잡는다 (호출 사이에 표가 커질 수 있다)
    fn table(read: impl Fn(*mut winapi::ctypes::c_void, &mut u32) -> u32) -> Option<Vec<u64>> {
        let mut size = 0u32;
        for _ in 0..4 {
            // u64로 잡아 표 구조체 정렬을 맞춘다
            let mut buf = vec![0u64; (size as usize).div_ceil(8).max(1)];
            size = (buf.len() * 8) as u32;
            match read(buf.as_mut_ptr().cast(), &mut size) {
                0 => return Some(buf),
                ERROR_INSUFFICIENT_BUFFER => continue,
                _ => return None,
            }
        }
        None
    }

    /// 표의 행 수와 행들을 읽는다. 표는 `dwNumEntries` 다음에 행 배열이 오는 모양이다
    unsafe fn rows<T, R>(buf: &[u64], table: *const T, first: *const R) -> &[R] {
        let count = *(table as *const u32) as usize;
        let start = first as usize - buf.as_ptr() as usize;
        let fits = (buf.len() * 8).saturating_sub(start) / std::mem::size_of::<R>();
        std::slice::from_raw_parts(first, count.min(fits))
    }

    /// TCP로 그 포트를 듣는 프로세스 (IPv4·IPv6 모두. 루프백 중계가 듣는 쪽이 인스턴스 서버인지 본다)
    pub fn tcp_listener_pids(want: u16) -> Option<Vec<u32>> {
        let port = |raw: u32| u16::from_be(raw as u16);
        let mut pids = Vec::new();
        let mut read_any = false;
        unsafe {
            if let Some(buf) = table(|p, n| GetExtendedTcpTable(p, n, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)) {
                let t = buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
                pids.extend(rows(&buf, t, (*t).table.as_ptr()).iter().filter(|r| port(r.dwLocalPort) == want).map(|r| r.dwOwningPid));
                read_any = true;
            }
            if let Some(buf) = table(|p, n| GetExtendedTcpTable(p, n, 0, AF_INET6 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)) {
                let t = buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
                pids.extend(rows(&buf, t, (*t).table.as_ptr()).iter().filter(|r| port(r.dwLocalPort) == want).map(|r| r.dwOwningPid));
                read_any = true;
            }
        }
        read_any.then_some(pids)
    }

    pub fn read() -> HashSet<u16> {
        let mut held = HashSet::new();
        let port = |raw: u32| u16::from_be(raw as u16);
        unsafe {
            if let Some(buf) = table(|p, n| GetExtendedTcpTable(p, n, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)) {
                let t = buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
                held.extend(rows(&buf, t, (*t).table.as_ptr()).iter().map(|r| port(r.dwLocalPort)));
            }
            if let Some(buf) = table(|p, n| GetExtendedTcpTable(p, n, 0, AF_INET6 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)) {
                let t = buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
                held.extend(rows(&buf, t, (*t).table.as_ptr()).iter().filter(|r| r.ucLocalAddr == [0; 16]).map(|r| port(r.dwLocalPort)));
            }
            if let Some(buf) = table(|p, n| GetExtendedUdpTable(p, n, 0, AF_INET as u32, UDP_TABLE_OWNER_PID, 0)) {
                let t = buf.as_ptr() as *const MIB_UDPTABLE_OWNER_PID;
                held.extend(rows(&buf, t, (*t).table.as_ptr()).iter().map(|r| port(r.dwLocalPort)));
            }
            if let Some(buf) = table(|p, n| GetExtendedUdpTable(p, n, 0, AF_INET6 as u32, UDP_TABLE_OWNER_PID, 0)) {
                let t = buf.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID;
                held.extend(rows(&buf, t, (*t).table.as_ptr()).iter().filter(|r| r.ucLocalAddr == [0; 16]).map(|r| port(r.dwLocalPort)));
            }
        }
        held
    }
}

/// 인스턴스에서 모듈이 실제 사용하는 포트만 수집합니다.
///
/// `module_protocols`가 제공되면 해당 모듈의 `protocols_supported`를 참조하여
/// - game port: 항상 포함
/// - rest_port: 모듈이 "rest"를 지원할 때만 포함
fn collect_active_ports<'a>(
    instance: &'a ServerInstance,
    module_protocols: Option<&std::collections::HashMap<String, Vec<String>>>,
) -> Vec<(u16, &'a str)> {
    let protocols = module_protocols.and_then(|mp| mp.get(&instance.module_name));
    // 프로토콜 정보가 없으면 (하위 호환) 모든 포트를 포함
    let supports_rest = protocols.is_none_or(|p| p.iter().any(|s| s == "rest"));

    let mut ports = Vec::new();
    if let Some(port) = instance.port {
        ports.push((port, "port"));
    }
    if supports_rest {
        if let Some(rest_port) = instance.rest_port {
            ports.push((rest_port, "rest_port"));
        }
    }
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    // 리뷰: TCP 범위 8135-8234가 잡힌 PC에서 UDP만 쓰는 팰월드 8211까지 예약으로 막아 0.1.6 인스턴스가 못 떴다
    #[test]
    fn a_game_port_is_blocked_only_when_both_protocols_are_reserved() {
        assert!(!blocks_start("port", true, false));
        assert!(blocks_start("port", true, true));
        assert!(blocks_start("rest_port", true, false));
        assert!(!blocks_start("rest_port", false, true));
    }

    // 리뷰: 게임 포트를 직접 정한 팰월드를 여럿 만들면 REST가 모두 같은 첫 비예약 포트로 옮겨져 둘째가 못 떴다
    #[test]
    fn the_rest_port_moves_past_ports_other_instances_hold() {
        let taken = [8222u16, 8232];
        assert_eq!(next_free_port(8212, |p| taken.contains(&p) || (8135..=8234).contains(&p)), Some(8242));
        assert_eq!(next_free_port(8212, |p| p == 8222 || p == 8242), Some(8232));
    }

    fn make_field(name: &str, field_type: &str) -> SettingField {
        SettingField {
            name: name.to_string(),
            field_type: field_type.to_string(),
            label: name.to_string(),
            description: None,
            required: None,
            default: None,
            min: None,
            max: None,
            step: None,
            options: None,
            group: None,
            port: false,
            ..Default::default()
        }
    }

    /// 06 §8.1: 목록은 문자열 배열이고 항목마다 검사한다. 여러 줄은 줄바꿈을 받는다
    #[test]
    fn list_and_multiline_values_follow_the_contract() {
        use serde_json::json;
        let mut admins = make_field("admins", "list");
        admins.pattern = Some("[A-Za-z0-9_]+".into());
        admins.max_items = Some(2);
        let check = |f: &SettingField, v: Value| validate_setting_value(f, Some(&v)).map_err(|e| e.error_type);
        assert_eq!(check(&admins, json!(["Steve", "Alex"])), Ok(()));
        assert_eq!(check(&admins, json!([])), Ok(()));
        assert_eq!(check(&admins, json!("Steve")), Err(ValidationErrorType::TypeMismatch));
        assert_eq!(check(&admins, json!(["Steve", 3])), Err(ValidationErrorType::TypeMismatch));
        for bad in [json!(["Steve", ""]), json!(["Steve", "Steve"]), json!(["a b"]), json!(["a\nb"])] {
            assert_eq!(check(&admins, bad.clone()), Err(ValidationErrorType::InvalidFormat), "{bad}");
        }
        assert_eq!(check(&admins, json!(["a", "b", "c"])), Err(ValidationErrorType::OutOfRange));
        let mut maps = make_field("maps", "list");
        maps.options = Some(vec!["forest".into(), "desert".into()]);
        maps.min_items = Some(1);
        maps.required = Some(true);
        assert_eq!(check(&maps, json!(["desert", "forest"])), Ok(()));
        assert_eq!(check(&maps, json!(["sea"])), Err(ValidationErrorType::InvalidOption));
        assert_eq!(check(&maps, json!([])), Err(ValidationErrorType::Required));

        let mut motd = make_field("motd", "multiline");
        motd.max_length = Some(5);
        assert_eq!(check(&motd, json!("가나\n다")), Ok(()), "five characters, not bytes");
        assert_eq!(check(&motd, json!("123456")), Err(ValidationErrorType::OutOfRange));
        assert_eq!(check(&motd, json!("a\u{0}")), Err(ValidationErrorType::InvalidFormat));
        assert_eq!(check(&motd, json!(["a"])), Err(ValidationErrorType::TypeMismatch));
    }

    /// 06 §8.1 옮기기와 §8.2 숨김 계산
    #[test]
    fn list_values_migrate_and_hidden_follows_typed_values() {
        use serde_json::json;
        let fields = vec![make_field("admins", "list"), make_field("motd", "text")];
        let mut settings = std::collections::HashMap::from([("admins".to_string(), json!(" Steve, ,Alex,")), ("motd".to_string(), json!("a,b"))]);
        assert!(migrate_list_values(&fields, &mut settings));
        assert_eq!((settings["admins"].clone(), settings["motd"].clone()), (json!(["Steve", "Alex"]), json!("a,b")));
        assert!(!migrate_list_values(&fields, &mut settings), "already a list");

        let mut pvp = make_field("pvp", "boolean");
        pvp.default = Some(toml::Value::Boolean(false));
        let port = make_field("port", "number");
        let mut damage = make_field("damage", "text");
        damage.visible_when = Some(crate::supervisor::module_loader::VisibleWhen { field: "pvp".into(), equals: Some(json!(true)), any_of: None });
        let mut bind = make_field("bind", "text");
        bind.visible_when = Some(crate::supervisor::module_loader::VisibleWhen { field: "port".into(), equals: None, any_of: Some(vec![json!(25565)]) });
        let fields = vec![pvp, port, damage, bind];
        let hidden = |values: Value| hidden_fields(&fields, |k| values.get(k).cloned());
        assert_eq!(hidden(json!({})), vec!["damage", "bind"], "pvp falls back to its default; port has none");
        assert_eq!(hidden(json!({"pvp": "true", "port": "25565"})), Vec::<String>::new(), "typed comparison");
        assert_eq!(hidden(json!({"pvp": true, "port": 25566})), vec!["bind"]);
    }

    #[test]
    fn values_must_match_the_declared_format() {
        let mut ram = make_field("ram", "text");
        ram.format = Some("memory_size".into());
        for ok in ["4G", "1024M", "512k"] {
            assert!(validate_setting_value(&ram, Some(&Value::from(ok))).is_ok(), "{}", ok);
        }
        for bad in ["4096", "4GB", "4 G", "G", "-4G", "4기", "가"] {
            let err = validate_setting_value(&ram, Some(&Value::from(bad))).unwrap_err();
            assert_eq!(err.error_type, ValidationErrorType::InvalidFormat, "{}", bad);
        }
        let mut port = make_field("port", "number");
        port.format = Some("port".into());
        assert!(validate_setting_value(&port, Some(&serde_json::json!(8211))).is_ok());
        assert!(validate_setting_value(&port, Some(&serde_json::json!(70000))).is_err());
        assert!(validate_setting_value(&port, Some(&serde_json::json!(1.5))).is_err());
        let mut name = make_field("server_name", "text");
        name.pattern = Some("[A-Za-z0-9_]+".into());
        assert!(validate_setting_value(&name, Some(&Value::from("my_srv"))).is_ok());
        assert!(validate_setting_value(&name, Some(&Value::from("my srv"))).is_err());
        assert_eq!(expected_of(&name), "pattern");
        assert_eq!(expected_of(&ram), "memory_size");
    }

    /// 설정 값은 설정 파일의 한 줄·실행 인자 하나가 된다. 줄바꿈이 있으면 다른 키를 덮어쓸 수 있다
    /// (예: 서버 설명에 "x\nPassword=" → 원격에서 바꿀 수 없는 서버 비밀번호가 비워진다)
    #[test]
    fn text_values_cannot_carry_line_breaks() {
        for field_type in ["text", "password"] {
            let field = make_field("public_description", field_type);
            assert!(validate_setting_value(&field, Some(&Value::from("우리 서버, 환영합니다"))).is_ok());
            for bad in ["x\nPassword=", "x\r\nOpen=true", "x\0"] {
                let err = validate_setting_value(&field, Some(&Value::from(bad))).unwrap_err();
                assert_eq!(err.error_type, ValidationErrorType::InvalidFormat, "{field_type} {bad:?}");
            }
        }
    }

    #[test]
    fn test_required_field_missing() {
        let mut field = make_field("port", "number");
        field.required = Some(true);
        let result = validate_setting_value(&field, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::Required);
    }

    #[test]
    fn test_number_in_range() {
        let mut field = make_field("port", "number");
        field.min = Some(1024.0);
        field.max = Some(65535.0);

        let val = Value::Number(serde_json::Number::from(25565));
        assert!(validate_setting_value(&field, Some(&val)).is_ok());

        let val_low = Value::Number(serde_json::Number::from(80));
        assert!(validate_setting_value(&field, Some(&val_low)).is_err());

        let val_high = Value::Number(serde_json::Number::from(70000));
        assert!(validate_setting_value(&field, Some(&val_high)).is_err());
    }

    #[test]
    fn test_number_string_accepted() {
        let field = make_field("port", "number");
        let val = Value::String("25565".to_string());
        assert!(validate_setting_value(&field, Some(&val)).is_ok());
    }

    #[test]
    fn test_boolean_values() {
        let field = make_field("is_pvp", "boolean");
        assert!(validate_setting_value(&field, Some(&Value::Bool(true))).is_ok());
        assert!(validate_setting_value(&field, Some(&Value::String("true".into()))).is_ok());
        assert!(validate_setting_value(&field, Some(&Value::String("yes".into()))).is_err());
    }

    #[test]
    fn test_select_valid_option() {
        let mut field = make_field("difficulty", "select");
        field.options = Some(vec![
            "peaceful".into(), "easy".into(), "normal".into(), "hard".into(),
        ]);

        let val = Value::String("normal".into());
        assert!(validate_setting_value(&field, Some(&val)).is_ok());

        let val_invalid = Value::String("impossible".into());
        assert!(validate_setting_value(&field, Some(&val_invalid)).is_err());
    }

    #[test]
    fn new_instance_ports_move_together_past_taken_ones() {
        let mut inst = ServerInstance::new("b", "zomboid");
        inst.port = Some(16261);
        inst.rest_port = Some(27015);
        inst.module_settings.insert("port".into(), serde_json::json!(16261));
        inst.module_settings.insert("steam_port1".into(), serde_json::json!(8766));
        let fields = ["port", "steam_port1"];

        let free: std::collections::HashSet<u16> = [25565].into();
        assert_eq!(shift_ports_clear_of(&mut inst, &fields, |p| free.contains(&p)), None, "nothing taken, nothing moves");

        let taken: std::collections::HashSet<u16> = [16261, 8766, 27025].into();
        assert_eq!(shift_ports_clear_of(&mut inst, &fields, |p| taken.contains(&p)), Some(20), "+10 would hit rest 27025");
        assert_eq!(inst.port, Some(16281));
        assert_eq!(inst.rest_port, Some(27035));
        assert_eq!(inst.module_settings["port"], serde_json::json!(16281));
        assert_eq!(inst.module_settings["steam_port1"], serde_json::json!(8786));
        assert!(!inst.module_settings.contains_key("rest_port"), "no module setting is invented for core fields");
    }

    #[test]
    fn test_port_conflict_detection() {
        let instance_a = ServerInstance::new("server-a", "minecraft");
        let mut instance_a = instance_a;
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(25565);

        let mut instance_b = ServerInstance::new("server-b", "minecraft");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(25565); // 같은 포트!

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![instance_a.clone(), instance_b];

        let conflicts = check_port_conflicts(&instance_a, &all, &running, None);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].port, 25565);
    }

    #[test]
    fn test_no_conflict_when_not_running() {
        let mut instance_a = ServerInstance::new("server-a", "minecraft");
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(25565);

        let mut instance_b = ServerInstance::new("server-b", "minecraft");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(25565);

        let running: std::collections::HashSet<String> = std::collections::HashSet::new();
        let all = vec![instance_a.clone(), instance_b];

        let conflicts = check_port_conflicts(&instance_a, &all, &running, None);
        assert!(conflicts.is_empty());
    }

    /// REST를 지원하지 않는 모듈의 rest_port는 충돌 검사에서 무시되어야 합니다.
    #[test]
    fn test_rest_port_ignored_for_non_rest_module() {
        // 좀보이드 2개: 둘 다 rest_port=8212 이지만, 모듈이 REST를 지원하지 않음
        let mut instance_a = ServerInstance::new("zomboid-1", "zomboid");
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(16261);
        instance_a.rest_port = Some(8212);  // 잘못 설정된 기본값

        let mut instance_b = ServerInstance::new("zomboid-2", "zomboid");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(16262);
        instance_b.rest_port = Some(8212);  // 같은 rest_port (그러나 미사용)

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![instance_a.clone(), instance_b];

        // 프로토콜 맵: zomboid는 stdin만 지원 (rest 없음)
        let mut module_protocols = std::collections::HashMap::new();
        module_protocols.insert("zomboid".to_string(), vec!["stdin".to_string()]);

        // rest_port 충돌이 무시되어야 함
        let conflicts = check_port_conflicts(&instance_a, &all, &running, Some(&module_protocols));
        assert!(conflicts.is_empty(), "REST를 지원하지 않는 모듈의 rest_port 충돌은 무시되어야 합니다: {:?}", conflicts);
    }

    /// REST를 지원하는 모듈의 rest_port 충돌은 정상 검출되어야 합니다.
    #[test]
    fn test_rest_port_conflict_detected_for_rest_module() {
        let mut instance_a = ServerInstance::new("palworld-1", "palworld");
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(8211);
        instance_a.rest_port = Some(8212);

        let mut instance_b = ServerInstance::new("palworld-2", "palworld");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(8213);
        instance_b.rest_port = Some(8212);  // 같은 rest_port — 실제 충돌

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![instance_a.clone(), instance_b];

        let mut module_protocols = std::collections::HashMap::new();
        module_protocols.insert("palworld".to_string(), vec!["rest".to_string()]);

        let conflicts = check_port_conflicts(&instance_a, &all, &running, Some(&module_protocols));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].port, 8212);
        assert_eq!(conflicts[0].port_type, "rest_port");
    }

    /// 서로 다른 모듈 간 — REST 미지원 모듈의 rest_port가 REST 지원 모듈과 겹쳐도 무시
    #[test]
    fn test_cross_module_rest_port_no_false_conflict() {
        let mut zomboid = ServerInstance::new("zomboid-1", "zomboid");
        zomboid.id = "aaa".to_string();
        zomboid.port = Some(16261);
        zomboid.rest_port = Some(8212);  // 잘못 설정된 기본값

        let mut palworld = ServerInstance::new("palworld-1", "palworld");
        palworld.id = "bbb".to_string();
        palworld.port = Some(8211);
        palworld.rest_port = Some(8212);  // REST 실제 사용

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![zomboid.clone(), palworld];

        let mut module_protocols = std::collections::HashMap::new();
        module_protocols.insert("zomboid".to_string(), vec!["stdin".to_string()]);
        module_protocols.insert("palworld".to_string(), vec!["rest".to_string()]);

        // zomboid가 대상: rest_port를 사용하지 않으므로 충돌 없어야 함
        let conflicts = check_port_conflicts(&zomboid, &all, &running, Some(&module_protocols));
        assert!(conflicts.is_empty(), "대상 모듈이 REST 미지원이면 rest_port 충돌 없어야 합니다: {:?}", conflicts);
    }

    // ═══════════════════════════════════════════════════════
    // 추가 심층 테스트
    // ═══════════════════════════════════════════════════════

    /// 비필수 필드에 None 전달 → Ok
    #[test]
    fn test_optional_field_none_is_ok() {
        let field = make_field("description", "text");
        assert!(validate_setting_value(&field, None).is_ok());
    }

    /// 비필수 필드에 Null 전달 → Ok
    #[test]
    fn test_optional_field_null_is_ok() {
        let field = make_field("description", "text");
        assert!(validate_setting_value(&field, Some(&Value::Null)).is_ok());
    }

    /// 필수 필드에 빈 문자열 → Required 에러
    #[test]
    fn test_required_field_empty_string_is_error() {
        let mut field = make_field("name", "text");
        field.required = Some(true);
        let result = validate_setting_value(&field, Some(&Value::String("".into())));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::Required);
    }

    /// 비필수 필드에 빈 문자열 → Ok
    #[test]
    fn test_optional_field_empty_string_is_ok() {
        let field = make_field("description", "text");
        assert!(validate_setting_value(&field, Some(&Value::String("".into()))).is_ok());
    }

    /// 숫자 — 정확히 min에서 Ok, 정확히 max에서 Ok
    #[test]
    fn test_number_at_exact_boundaries() {
        let mut field = make_field("port", "number");
        field.min = Some(1024.0);
        field.max = Some(65535.0);

        // 정확히 min
        let at_min = Value::Number(serde_json::Number::from(1024));
        assert!(validate_setting_value(&field, Some(&at_min)).is_ok());

        // 정확히 max
        let at_max = Value::Number(serde_json::Number::from(65535));
        assert!(validate_setting_value(&field, Some(&at_max)).is_ok());

        // min - 1
        let below_min = Value::Number(serde_json::Number::from(1023));
        assert!(validate_setting_value(&field, Some(&below_min)).is_err());

        // max + 1
        let above_max = Value::Number(serde_json::Number::from(65536));
        assert!(validate_setting_value(&field, Some(&above_max)).is_err());
    }

    /// 숫자 필드에 배열 전달 → TypeMismatch
    #[test]
    fn test_number_field_with_array_is_type_mismatch() {
        let field = make_field("port", "number");
        let val = Value::Array(vec![Value::Number(serde_json::Number::from(8080))]);
        let result = validate_setting_value(&field, Some(&val));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::TypeMismatch);
    }

    /// 숫자 필드에 파싱 불가능한 문자열 → TypeMismatch
    #[test]
    fn test_number_field_with_non_numeric_string() {
        let field = make_field("port", "number");
        let val = Value::String("not_a_number".into());
        let result = validate_setting_value(&field, Some(&val));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::TypeMismatch);
    }

    /// boolean 필드에 숫자 0/1 → TypeMismatch (strict)
    #[test]
    fn test_boolean_field_rejects_number() {
        let field = make_field("is_pvp", "boolean");
        let val = Value::Number(serde_json::Number::from(1));
        let result = validate_setting_value(&field, Some(&val));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::TypeMismatch);
    }

    /// select 필드에 빈 옵션 목록 → 어떤 값도 거부
    #[test]
    fn test_select_empty_options_rejects_all() {
        let mut field = make_field("mode", "select");
        field.options = Some(vec![]);
        let val = Value::String("any".into());
        let result = validate_setting_value(&field, Some(&val));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::InvalidOption);
    }

    /// select 필드의 대소문자 구분 확인
    #[test]
    fn test_select_is_case_sensitive() {
        let mut field = make_field("difficulty", "select");
        field.options = Some(vec!["Easy".into(), "Normal".into(), "Hard".into()]);

        let valid = Value::String("Normal".into());
        assert!(validate_setting_value(&field, Some(&valid)).is_ok());

        let wrong_case = Value::String("normal".into());
        assert!(validate_setting_value(&field, Some(&wrong_case)).is_err());
    }

    /// text 필드에 숫자/불리언 → Ok (문자열 변환 가능)
    #[test]
    fn test_text_field_accepts_number_and_bool() {
        let field = make_field("notes", "text");
        assert!(validate_setting_value(&field, Some(&Value::Number(serde_json::Number::from(42)))).is_ok());
        assert!(validate_setting_value(&field, Some(&Value::Bool(true))).is_ok());
    }

    /// text 필드에 배열 → TypeMismatch
    #[test]
    fn test_text_field_rejects_array() {
        let field = make_field("notes", "text");
        let val = Value::Array(vec![]);
        let result = validate_setting_value(&field, Some(&val));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_type, ValidationErrorType::TypeMismatch);
    }

    /// 알 수 없는 field_type → 통과 (forward compatibility)
    #[test]
    fn test_unknown_field_type_passes() {
        let field = make_field("custom", "slider"); // 미래의 새 타입
        let val = Value::String("42".into());
        assert!(validate_setting_value(&field, Some(&val)).is_ok());
    }

    #[test]
    fn patch_validation_only_checks_sent_fields() {
        let mut players = make_field("max_players", "number");
        players.required = Some(true);
        players.min = Some(1.0);
        players.max = Some(100.0);
        let mut motd = make_field("motd", "text");
        motd.required = Some(true);
        let fields = vec![players, motd];
        // 필수 필드 motd를 보내지 않아도 통과 (기존 값 유지)
        let patch = serde_json::json!({ "max_players": 10 });
        assert!(validate_settings_patch(&fields, patch.as_object().unwrap()).is_empty());
        // 보낸 값은 여전히 검사하고, 필수 값을 지우려는 것도 막는다
        let bad = serde_json::json!({ "max_players": 500, "motd": "" });
        assert_eq!(validate_settings_patch(&fields, bad.as_object().unwrap()).len(), 2);
    }

    /// 포트 충돌 — 같은 game port와 rest_port 간 크로스 충돌
    #[test]
    fn test_port_conflict_cross_type() {
        let mut instance_a = ServerInstance::new("server-a", "minecraft");
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(25575); // game port가 다른 서버의 rest port와 같음

        let mut instance_b = ServerInstance::new("server-b", "minecraft");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(25565);
        instance_b.rest_port = Some(25575); // rest_port가 A의 game port와 같음

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![instance_a.clone(), instance_b];

        let conflicts = check_port_conflicts(&instance_a, &all, &running, None);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].port, 25575);
        assert_eq!(conflicts[0].port_type, "port");
        assert_eq!(conflicts[0].conflicting_port_type, "rest_port");
    }

    /// 포트 충돌 — 3개 인스턴스와 다중 충돌
    #[test]
    fn test_port_conflict_multiple_instances() {
        let mut target = ServerInstance::new("target", "palworld");
        target.id = "t".to_string();
        target.port = Some(8211);
        target.rest_port = Some(8212);

        let mut other1 = ServerInstance::new("other-1", "palworld");
        other1.id = "o1".to_string();
        other1.port = Some(8211); // game port 충돌

        let mut other2 = ServerInstance::new("other-2", "palworld");
        other2.id = "o2".to_string();
        other2.port = Some(9999);
        other2.rest_port = Some(8212); // rest_port 충돌

        let running: std::collections::HashSet<String> = ["o1".into(), "o2".into()].into();
        let all = vec![target.clone(), other1, other2];

        let conflicts = check_port_conflicts(&target, &all, &running, None);
        assert_eq!(conflicts.len(), 2, "Should detect 2 conflicts: {:?}", conflicts);
    }

    /// ValidationError — Display trait 출력 포맷
    #[test]
    fn test_validation_error_display() {
        let err = ValidationError {
            field: "port".to_string(),
            message: "Required field 'port' is missing".to_string(),
            error_type: ValidationErrorType::Required,
        };
        let display = format!("{}", err);
        assert_eq!(display, "port: Required field 'port' is missing");
    }

    #[test]
    fn ports_held_by_another_program_are_found_over_tcp_and_udp() {
        let tcp = std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let udp = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let (tcp_port, udp_port) = (tcp.local_addr().unwrap().port(), udp.local_addr().unwrap().port());
        let ports = vec![("port".to_string(), tcp_port), ("query_port".to_string(), udp_port)];
        assert_eq!(ports_held_elsewhere(&ports), ports);
        drop((tcp, udp));
        assert!(ports_held_elsewhere(&ports).is_empty(), "released ports are free again");
    }

    /// 이 PC에서 Windows가 예약한(제외 범위) TCP 포트 하나. Hyper-V·WinNAT가 켜진 PC는 부팅마다 100개 묶음을 잡는다.
    /// 한 포트짜리 범위(5985 WinRM 등)는 서비스가 실제로 듣고 있을 수 있어 넓은 범위의 가운데를 고른다
    #[cfg(windows)]
    /// 예약 범위 안에서 지금 아무 프로그램도 쥐고 있지 않은 포트. 예약한 쪽(WinNAT 등)이 범위 안 포트를 실제로
    /// 듣고 있기도 해서(CI 러너) 사용 중 포트 목록에 있는 것은 건너뛴다
    fn an_excluded_tcp_port() -> Option<u16> {
        let out = std::process::Command::new("netsh").args(["int", "ipv4", "show", "excludedportrange", "protocol=tcp"]).output().ok()?;
        let held = held_ports::read();
        String::from_utf8_lossy(&out.stdout).lines().find_map(|line| {
            let mut nums = line.split_whitespace().map(str::parse::<u16>);
            match (nums.next(), nums.next()) {
                (Some(Ok(start)), Some(Ok(end))) if end - start >= 10 => (start + 1..end).find(|p| !held.contains(p)),
                _ => None,
            }
        })
    }

    // 실제 PC: 팰월드 REST 기본 포트 8212가 예약 범위(8135-8234)에 들어 있어 서버가 REST를 열지 못했고,
    // 사용 중 포트 목록에는 나오지 않아 시작 전 검사도 통과했다
    #[cfg(windows)]
    #[test]
    fn ports_in_a_windows_excluded_range_are_reported_as_reserved() {
        if an_excluded_tcp_port().is_none() {
            eprintln!("this PC has no excluded port range; nothing to check");
            return;
        }
        // 고른 뒤 확인하기 전에 다른 프로그램(병렬 시험이 연 UDP 소켓 등)이 그 번호를 쥘 수 있다: 비어 있는 것을 다시 고른다
        let ports = (0..5)
            .filter_map(|_| an_excluded_tcp_port())
            .map(|port| vec![("rest_port".to_string(), port)])
            .find(|ports| ports_held_elsewhere(ports).is_empty())
            .expect("the held-port table does not list reserved ports");
        assert_eq!(ports_reserved_by_windows(&ports), ports);
        let free = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        assert!(ports_reserved_by_windows(&[("port".to_string(), free)]).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn new_instance_ports_move_out_of_a_windows_excluded_range() {
        let Some(port) = an_excluded_tcp_port() else {
            eprintln!("this PC has no excluded port range; nothing to check");
            return;
        };
        let mut inst = ServerInstance::new("p", "palworld");
        inst.rest_port = Some(port);
        let taken: std::collections::HashSet<u16> = std::collections::HashSet::new();
        assert!(shift_ports_clear_of(&mut inst, &[], |p| taken.contains(&p) || port_reserved_by_windows(p)).is_some());
        let moved = inst.rest_port.unwrap();
        assert!(!port_reserved_by_windows(moved), "{port} moved to {moved}, which is still reserved");
    }

    #[test]
    fn start_check_ports_add_declared_module_ports_once() {
        let mut inst = ServerInstance::new("s", "zomboid");
        inst.port = Some(16261);
        inst.rest_port = Some(27015);
        inst.module_settings.insert("port".into(), serde_json::json!(16261));
        inst.module_settings.insert("steam_port1".into(), serde_json::json!(8766));
        inst.module_settings.insert("dup".into(), serde_json::json!(16261));
        let protocols: std::collections::HashMap<String, Vec<String>> = [("zomboid".to_string(), vec![])].into();
        let ports = start_check_ports(&inst, Some(&protocols), &["port", "steam_port1", "dup"]);
        assert_eq!(ports, vec![("port".to_string(), 16261), ("steam_port1".to_string(), 8766)], "rest port is skipped without rest support");
    }

    /// PortConflict — Display trait 출력 포맷
    #[test]
    fn test_port_conflict_display() {
        let conflict = PortConflict {
            port: 25565,
            port_type: "port".to_string(),
            conflicting_instance_name: "other-server".to_string(),
            conflicting_instance_id: "abc".to_string(),
            conflicting_port_type: "port".to_string(),
        };
        let display = format!("{}", conflict);
        assert!(display.contains("25565"));
        assert!(display.contains("other-server"));
    }

    /// 포트 충돌 — module_protocols=None이면 모든 포트 비교 (하위 호환)
    #[test]
    fn test_port_conflict_without_protocols_includes_all_ports() {
        let mut instance_a = ServerInstance::new("srv-a", "unknown_module");
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(8000);
        instance_a.rest_port = Some(8002);

        let mut instance_b = ServerInstance::new("srv-b", "unknown_module");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(9000);
        instance_b.rest_port = Some(8002); // rest_port 충돌

        let running: std::collections::HashSet<String> = ["bbb".into()].into();
        let all = vec![instance_a.clone(), instance_b];

        // module_protocols=None → 모든 포트 비교
        let conflicts = check_port_conflicts(&instance_a, &all, &running, None);
        assert_eq!(conflicts.len(), 1, "Without protocols, all ports should be compared");
        assert_eq!(conflicts[0].port, 8002);
    }
}
