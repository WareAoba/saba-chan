//! 설정값 타입 검증 및 포트 충돌 검사 모듈
//!
//! - `validate_setting_value`: module.toml의 SettingField 스키마를 기반으로
//!   개별 설정값의 타입·범위·필수여부를 검증합니다.
//! - `validate_all_settings`: 모듈의 모든 설정 필드를 한 번에 검증합니다.
//! - `check_port_conflicts`: 실행 중인 인스턴스와 포트 충돌을 검사합니다.

use crate::instance::ServerInstance;
use crate::supervisor::module_loader::SettingField;
use serde_json::Value;

/// 개별 설정 필드 검증 에러
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
    #[allow(dead_code)] // 공개 API — 에러 종류 구분용
    pub error_type: ValidationErrorType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationErrorType {
    Required,
    TypeMismatch,
    OutOfRange,
    InvalidOption,
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
    /// 포트 종류 ("port", "rcon_port", "rest_port")
    pub port_type: String,
    /// 충돌하는 상대 인스턴스 이름
    pub conflicting_instance_name: String,
    /// 충돌하는 상대 인스턴스 ID
    #[allow(dead_code)] // 공개 API — 충돌 진단 정보
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
        _ => Ok(()), // 알 수 없는 타입은 통과
    }
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

/// 모듈의 전체 설정 필드를 한꺼번에 검증합니다.
///
/// # Arguments
/// * `fields` - module.toml의 settings.fields 목록
/// * `settings` - 사용자가 입력한 전체 설정값 (JSON 오브젝트)
///
/// # Returns
/// 검증 오류 목록 (비어있으면 모두 유효)
pub fn validate_all_settings(
    fields: &[SettingField],
    settings: &serde_json::Map<String, Value>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for field in fields {
        let value = settings.get(&field.name);
        if let Err(e) = validate_setting_value(field, value) {
            errors.push(e);
        }
    }

    errors
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

/// 인스턴스에서 모듈이 실제 사용하는 포트만 수집합니다.
///
/// `module_protocols`가 제공되면 해당 모듈의 `protocols_supported`를 참조하여
/// - game port: 항상 포함
/// - rcon_port: 모듈이 "rcon"을 지원할 때만 포함
/// - rest_port: 모듈이 "rest"를 지원할 때만 포함
fn collect_active_ports<'a>(
    instance: &'a ServerInstance,
    module_protocols: Option<&std::collections::HashMap<String, Vec<String>>>,
) -> Vec<(u16, &'a str)> {
    let protocols = module_protocols.and_then(|mp| mp.get(&instance.module_name));
    // 프로토콜 정보가 없으면 (하위 호환) 모든 포트를 포함
    let supports_rcon = protocols.is_none_or(|p| p.iter().any(|s| s == "rcon"));
    let supports_rest = protocols.is_none_or(|p| p.iter().any(|s| s == "rest"));

    let mut ports = Vec::new();
    if let Some(port) = instance.port {
        ports.push((port, "port"));
    }
    if supports_rcon {
        if let Some(rcon_port) = instance.rcon_port {
            ports.push((rcon_port, "rcon_port"));
        }
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
    fn test_port_conflict_detection() {
        let instance_a = ServerInstance::new("server-a", "minecraft");
        let mut instance_a = instance_a;
        instance_a.id = "aaa".to_string();
        instance_a.port = Some(25565);
        instance_a.rcon_port = Some(25575);

        let mut instance_b = ServerInstance::new("server-b", "minecraft");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(25565); // 같은 포트!
        instance_b.rcon_port = Some(25576);

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
        instance_a.rcon_port = Some(27015);
        instance_a.rest_port = Some(8212);  // 잘못 설정된 기본값

        let mut instance_b = ServerInstance::new("zomboid-2", "zomboid");
        instance_b.id = "bbb".to_string();
        instance_b.port = Some(16262);
        instance_b.rcon_port = Some(27016);
        instance_b.rest_port = Some(8212);  // 같은 rest_port (그러나 미사용)

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![instance_a.clone(), instance_b];

        // 프로토콜 맵: zomboid는 rcon+stdin만 지원 (rest 없음)
        let mut module_protocols = std::collections::HashMap::new();
        module_protocols.insert("zomboid".to_string(), vec!["rcon".to_string(), "stdin".to_string()]);

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
        zomboid.rcon_port = Some(27015);
        zomboid.rest_port = Some(8212);  // 잘못 설정된 기본값

        let mut palworld = ServerInstance::new("palworld-1", "palworld");
        palworld.id = "bbb".to_string();
        palworld.port = Some(8211);
        palworld.rest_port = Some(8212);  // REST 실제 사용

        let running: std::collections::HashSet<String> = ["bbb".to_string()].into();
        let all = vec![zomboid.clone(), palworld];

        let mut module_protocols = std::collections::HashMap::new();
        module_protocols.insert("zomboid".to_string(), vec!["rcon".to_string(), "stdin".to_string()]);
        module_protocols.insert("palworld".to_string(), vec!["rest".to_string()]);

        // zomboid가 대상: rest_port를 사용하지 않으므로 충돌 없어야 함
        let conflicts = check_port_conflicts(&zomboid, &all, &running, Some(&module_protocols));
        assert!(conflicts.is_empty(), "대상 모듈이 REST 미지원이면 rest_port 충돌 없어야 합니다: {:?}", conflicts);
    }
}
