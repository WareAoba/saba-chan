//! 모듈이 돌아가는 OS·CPU: module.toml `[extensions.saba-core]`의 `platforms`·`architectures`.
//! 데몬(설치된 모듈·카탈로그)과 설치기(받기 전 목록)가 같은 판정을 쓴다

/// 이름 목록이 드는 module.toml 표 (`[extensions.saba-core]`)
const TABLE: &str = "saba-core";

/// `platforms`에 적을 수 있는 OS (`std::env::consts::OS`와 같은 이름)
pub const PLATFORMS: &[&str] = &["windows", "linux", "macos"];
/// `architectures`에 적을 수 있는 CPU (`std::env::consts::ARCH`와 같은 이름)
pub const ARCHITECTURES: &[&str] = &["x86_64", "aarch64"];

/// `architectures`에 흔히 쓰는 다른 이름 → `std::env::consts::ARCH` 이름
const ARCH_ALIASES: &[(&str, &str)] = &[("amd64", "x86_64"), ("x64", "x86_64"), ("arm64", "aarch64")];

/// `platforms`를 읽는다. 틀린 이름은 빼고 `warnings`에 남긴다
pub fn read_platforms(table: &serde_json::Value, warnings: &mut Vec<String>) -> Option<Vec<String>> {
    read_name_list(table, "platforms", PLATFORMS, &[], "OS", warnings)
}

/// `architectures`를 읽는다. 흔한 다른 이름(amd64, arm64)은 바꿔 읽는다
pub fn read_architectures(table: &serde_json::Value, warnings: &mut Vec<String>) -> Option<Vec<String>> {
    read_name_list(table, "architectures", ARCHITECTURES, ARCH_ALIASES, "CPU", warnings)
}

/// `[extensions.saba-core]`의 이름 목록 하나(`platforms`·`architectures`)를 읽는다. 대소문자는 가리지 않고 `aliases`는 바꿔 읽으며,
/// `allowed`에 없는 이름은 빼고 경고한다. 남은 이름이 없으면 선언하지 않은 것으로 본다 (틀린 이름 하나로 어디서도 못 돌게 되지 않게)
fn read_name_list(
    table: &serde_json::Value,
    key: &str,
    allowed: &[&str],
    aliases: &[(&str, &str)],
    what: &str,
    warnings: &mut Vec<String>,
) -> Option<Vec<String>> {
    match table.get(key) {
        None => None,
        Some(serde_json::Value::Array(items)) => {
            let mut names = Vec::new();
            for item in items {
                let name = item.as_str().map(str::to_ascii_lowercase).map(|p| aliases.iter().find(|(from, _)| *from == p).map_or(p, |(_, to)| to.to_string()));
                match name.filter(|p| allowed.contains(&p.as_str())) {
                    Some(p) if !names.contains(&p) => names.push(p),
                    Some(_) => {}
                    None => warnings.push(format!("extensions.{}.{} has an unknown {} {} (use {}); ignored", TABLE, key, what, item, allowed.join(", "))),
                }
            }
            (!names.is_empty()).then_some(names)
        }
        Some(_) => {
            warnings.push(format!("extensions.{}.{} must be a list of {} names; ignored", TABLE, key, what));
            None
        }
    }
}

/// 선언한 OS·CPU 목록으로 본 돌지 않는 이유: OS가 먼저다. 선언하지 않은 목록은 막지 않는다
pub fn unsupported_reason(platforms: Option<&[String]>, architectures: Option<&[String]>, os: &str, arch: &str) -> Option<&'static str> {
    if !platforms.is_none_or(|p| p.iter().any(|x| x == os)) {
        Some("os")
    } else if !architectures.is_none_or(|a| a.iter().any(|x| x == arch)) {
        Some("arch")
    } else {
        None
    }
}

/// 받기 전 카탈로그 모듈 meta의 `platforms`·`architectures`(module.toml `[extensions.saba-core]` 값 그대로)로 본 돌지 않는 이유.
/// 설치된 모듈과 같이 이름을 다듬고 모르는 이름은 버린다 (경고는 남기지 않는다: 고칠 사람은 모듈 저장소다)
pub fn catalog_unsupported_on(meta: &serde_json::Value, os: &str, arch: &str) -> Option<&'static str> {
    let mut ignored = Vec::new();
    let platforms = read_platforms(meta, &mut ignored);
    let architectures = read_architectures(meta, &mut ignored);
    unsupported_reason(platforms.as_deref(), architectures.as_deref(), os, arch)
}

