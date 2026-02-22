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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    /// 충돌하는 상대 인스턴스 이름
    pub conflicting_instance_name: String,
    #[allow(dead_code)]
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
/// # Arguments
/// * `target` - 검사할 인스턴스
/// * `all_instances` - 모든 인스턴스 목록
/// * `running_ids` - 현재 실행 중인 인스턴스 ID 집합
///
/// # Returns
/// 충돌 목록 (비어있으면 충돌 없음)
pub fn check_port_conflicts(
    target: &ServerInstance,
    all_instances: &[ServerInstance],
    running_ids: &std::collections::HashSet<String>,
) -> Vec<PortConflict> {
    let mut conflicts = Vec::new();

    // 대상 인스턴스의 모든 포트 수집
    let target_ports: Vec<(u16, &str)> = [
        (target.port, "port"),
        (target.rcon_port, "rcon_port"),
        (target.rest_port, "rest_port"),
    ]
    .iter()
    .filter_map(|(p, name)| p.map(|port| (port, *name)))
    .collect();

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

        // 상대 인스턴스의 모든 포트
        let other_ports: Vec<(u16, &str)> = [
            (other.port, "port"),
            (other.rcon_port, "rcon_port"),
            (other.rest_port, "rest_port"),
        ]
        .iter()
        .filter_map(|(p, name)| p.map(|port| (port, *name)))
        .collect();

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

        let conflicts = check_port_conflicts(&instance_a, &all, &running);
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

        let conflicts = check_port_conflicts(&instance_a, &all, &running);
        assert!(conflicts.is_empty());
    }
}
