//! 원격 문지기: 사바쨩 웹(릴레이 경유)이 부를 수 있는 데몬 API
//!
//! 데몬 라우트마다 허용·거부를 이 표 한 곳에 적는다. 표에 없는 요청은 거부한다.
//! 범위는 서버·인스턴스 관리와 멤버 권한이고, 공식 목록의 모듈·플러그인 설치와 데몬 콘솔 읽기를 더한다.
//! 토큰·비밀값·데몬 설정·경로·업데이트·샌드박스·릴레이 연결·챗봇은 PC에서만 한다.
//! 새 라우트를 만들면 여기 분류를 적는다 (`every_route_is_classified` 테스트가 빠진 것을 잡는다).

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Allow,
    Deny,
}

pub struct Route {
    pub method: &'static str,
    pub pattern: &'static str,
    pub access: Access,
}

const fn allow(method: &'static str, pattern: &'static str) -> Route {
    Route {
        method,
        pattern,
        access: Access::Allow,
    }
}

const fn deny(method: &'static str, pattern: &'static str) -> Route {
    Route {
        method,
        pattern,
        access: Access::Deny,
    }
}

pub static ROUTES: &[Route] = &[
    allow("GET", "/health"),
    allow("GET", "/api/version"),
    // 진단·감사 정보에는 PC 경로가 담긴다
    deny("GET", "/api/diagnostics"),
    deny("GET", "/api/onboarding"),
    deny("GET", "/api/audit"),
    // 이벤트는 원격 연결이 따로 밀어 준다 (SSE는 요청·응답 한 번으로 끝나지 않는다)
    deny("GET", "/api/events"),
    // ── 서버 ──
    allow("GET", "/api/servers"),
    allow("GET", "/api/server/{name}/status"),
    allow("POST", "/api/server/{name}/start"),
    allow("POST", "/api/server/{name}/stop"),
    allow("POST", "/api/instance/{id}/start"),
    allow("POST", "/api/instance/{id}/stop"),
    allow("POST", "/api/instance/{id}/restart"),
    allow("POST", "/api/instance/{id}/managed/start"),
    allow("GET", "/api/instance/{id}/events"),
    allow("GET", "/api/instance/{id}/console"),
    allow("POST", "/api/instance/{id}/stdin"),
    allow("GET", "/api/instance/{id}/logs"),
    allow("GET", "/api/instance/{id}/logs/export"),
    allow("GET", "/api/instance/{id}/players"),
    allow("POST", "/api/instance/{id}/command"),
    allow("GET", "/api/instance/{id}/commands"),
    allow("POST", "/api/instance/{id}/rcon"),
    allow("POST", "/api/instance/{id}/rest"),
    allow("POST", "/api/instance/{id}/query/{name}"),
    allow("POST", "/api/instance/{id}/validate"),
    allow("POST", "/api/instance/{id}/diagnose"),
    // 포트포워딩 확인: 릴레이가 요청한 PC의 공인 주소로만 확인한다
    allow("GET", "/api/instance/{id}/port-check"),
    allow("POST", "/api/instance/{id}/port-check"),
    // ── 인스턴스 ──
    allow("GET", "/api/instances"),
    allow("POST", "/api/instances"),
    allow("PUT", "/api/instances/reorder"),
    allow("GET", "/api/instance/{id}"),
    allow("PATCH", "/api/instance/{id}"),
    allow("DELETE", "/api/instance/{id}"),
    allow("POST", "/api/instance/{id}/rename"),
    // 저장하지 않은 설정 초안으로 실행 줄을 만든다: 모듈이 초안의 경로로 프로그램을 부를 수 있다
    deny("POST", "/api/instance/{id}/launch-preview"),
    allow("GET", "/api/provision-progress/{name}"),
    allow("DELETE", "/api/provision-progress/{name}"),
    allow("GET", "/api/instance/{id}/check-update"),
    allow("POST", "/api/instance/{id}/apply-update"),
    allow("GET", "/api/instance/{id}/installed-version"),
    // 게임 설정 원문 (관리자 비밀번호가 들어 있다). 웹은 인스턴스 설정(비밀값을 뺀다)으로 고친다
    deny("GET", "/api/instance/{id}/properties"),
    allow("PUT", "/api/instance/{id}/properties"),
    allow("POST", "/api/instance/{id}/properties/reset"),
    deny("GET", "/api/instance/{id}/settings/live"),
    allow("POST", "/api/instance/{id}/server/reset"),
    allow("POST", "/api/instance/{id}/agreements"),
    // ── 백업·일정 ──
    allow("GET", "/api/instance/{id}/backups"),
    allow("POST", "/api/instance/{id}/backups"),
    allow("DELETE", "/api/instance/{id}/backups/{name}"),
    allow("POST", "/api/instance/{id}/backups/{name}/restore"),
    allow("GET", "/api/instance/{id}/schedule"),
    allow("PUT", "/api/instance/{id}/schedule"),
    // ── 모듈: 공식 목록에서만 받는다. 폴더에서 들이기·삭제는 PC에서 ──
    allow("GET", "/api/modules"),
    allow("POST", "/api/modules/refresh"),
    allow("GET", "/api/modules/schema"),
    deny("GET", "/api/maker"),
    deny("POST", "/api/modules/detect"),
    allow("GET", "/api/modules/manifest"),
    allow("POST", "/api/modules/manifest/{id}/install"),
    deny("DELETE", "/api/modules/{id}"),
    allow("GET", "/api/module/{name}"),
    allow("GET", "/api/module/{name}/audit"),
    allow("GET", "/api/module/{name}/locales"),
    allow("GET", "/api/module/{name}/versions"),
    allow("GET", "/api/module/{name}/version/{version}"),
    // 설치 폴더와 실행 파일 경로를 받는다
    deny("POST", "/api/module/{name}/install"),
    // ── 플러그인: 공식 목록에서 받고 켜고 끈다. 주소 지정 설치는 `blocked_fields`가 막는다 ──
    allow("GET", "/api/extensions"),
    allow("GET", "/api/extensions/init-status"),
    deny("POST", "/api/extensions/rescan"),
    allow("GET", "/api/extensions/manifest"),
    allow("GET", "/api/extensions/updates"),
    allow("POST", "/api/extensions/{id}/enable"),
    allow("POST", "/api/extensions/{id}/disable"),
    deny("POST", "/api/extensions/{id}/mount"),
    deny("POST", "/api/extensions/{id}/unmount"),
    allow("POST", "/api/extensions/{id}/install"),
    deny("DELETE", "/api/extensions/{id}"),
    // 설정에는 비밀값이 들 수 있다
    deny("GET", "/api/extensions/{id}/config"),
    deny("PUT", "/api/extensions/{id}/config"),
    deny("GET", "/api/extensions/{id}/gui"),
    deny("GET", "/api/extensions/{id}/gui/styles"),
    deny("GET", "/api/extensions/{id}/icon"),
    allow("GET", "/api/extensions/{id}/audit"),
    allow("GET", "/api/extensions/{id}/i18n/{locale}"),
    // ── 데몬·앱 설정, 토큰, 경로 ──
    deny("GET", "/api/config/bot"),
    deny("PUT", "/api/config/bot"),
    deny("PATCH", "/api/config/bot"),
    deny("GET", "/api/config/gui"),
    deny("PUT", "/api/config/gui"),
    deny("PATCH", "/api/config/gui"),
    deny("GET", "/api/config/client/{kind}"),
    deny("PUT", "/api/config/client/{kind}"),
    deny("GET", "/api/config/{kind}/defaults"),
    deny("POST", "/api/config/{kind}/reset"),
    deny("POST", "/api/migration/export"),
    deny("POST", "/api/migration/import"),
    deny("GET", "/api/config/node-token"),
    deny("PUT", "/api/config/node-token"),
    deny("DELETE", "/api/config/node-token"),
    // 원격 데몬 포트와 짝 맺은 PC (D1)
    deny("GET", "/api/remote/settings"),
    deny("PUT", "/api/remote/settings"),
    deny("GET", "/api/remote/clients"),
    deny("DELETE", "/api/remote/clients/{host_id}"),
    deny("GET", "/api/remote/peers"),
    deny("GET", "/api/remote/daemons"),
    deny("GET", "/api/remote/servers"),
    deny("GET", "/api/system/browse"),
    deny("POST", "/api/remote/daemons"),
    deny("DELETE", "/api/remote/daemons/{host_id}"),
    deny("GET", "/api/remote/daemons/{host_id}/proxy/{*rest}"),
    deny("POST", "/api/remote/daemons/{host_id}/proxy/{*rest}"),
    deny("PUT", "/api/remote/daemons/{host_id}/proxy/{*rest}"),
    deny("PATCH", "/api/remote/daemons/{host_id}/proxy/{*rest}"),
    deny("DELETE", "/api/remote/daemons/{host_id}/proxy/{*rest}"),
    deny("GET", "/api/system/components"),
    allow("GET", "/api/system/metrics"),
    deny("GET", "/api/system/paths"),
    deny("POST", "/api/system/paths/move"),
    // 이 PC에 로그인할 때 무엇을 띄울지는 PC에서만 정한다
    deny("GET", "/api/system/autostart"),
    deny("PUT", "/api/system/autostart"),
    // ── 클라이언트·데몬 제어: 콘솔 읽기만 ──
    deny("POST", "/api/client/register"),
    deny("GET", "/api/clients"),
    deny("GET", "/api/clients/launch"),
    deny("POST", "/api/clients/launch"),
    deny("POST", "/api/client/{id}/heartbeat"),
    deny("DELETE", "/api/client/{id}/unregister"),
    deny("POST", "/api/daemon/shutdown"),
    allow("GET", "/api/daemon/console"),
    // ── 멤버 권한과 길드별 서버 노출 ──
    allow("GET", "/api/permissions/{guild_id}"),
    allow("PATCH", "/api/permissions/{guild_id}"),
    allow("PUT", "/api/permissions/{guild_id}/members/{discord_id}"),
    allow("DELETE", "/api/permissions/{guild_id}/members/{discord_id}"),
    allow("GET", "/api/instance/{id}/words"),
    allow("PUT", "/api/instance/{id}/words"),
    allow("PUT", "/api/guilds/{guild}/allowed-instances"),
    allow("GET", "/api/relay/host/{host_id}/nodes"),
    allow("GET", "/api/relay/node/{guild_id}/members"),
    // ── 디스코드 설정·릴레이 연결: 웹이 타고 있는 연결 자체다 ──
    deny("GET", "/api/discord/{guild_id}/settings"),
    deny("PATCH", "/api/discord/{guild_id}/settings"),
    deny("GET", "/api/relay/host/{host_id}/status"),
    deny("GET", "/api/relay/node/{guild_id}/legacy-members"),
    deny("GET", "/api/relay/node/{guild_id}/channels"),
    deny("PUT", "/api/relay/node/{guild_id}/channels"),
    deny("GET", "/api/relay/node/{guild_id}/chatbot/models"),
    deny("GET", "/api/relay/bot-invite"),
    deny("POST", "/api/relay/pair/initiate"),
    deny("GET", "/api/relay/pair/{code}/status"),
    deny("GET", "/api/relay/link-requests"),
    deny("POST", "/api/relay/link-requests/{request_id}"),
    deny("GET", "/api/relay/privacy/erase"),
    deny("POST", "/api/relay/privacy/erase"),
    deny("GET", "/api/relay/agent"),
    deny("POST", "/api/relay/agent/start"),
    deny("POST", "/api/relay/agent/stop"),
    deny("POST", "/api/chat/command"),
    // ── 챗봇: API 키가 오간다 ──
    deny("GET", "/api/chatbot"),
    deny("PUT", "/api/chatbot"),
    deny("DELETE", "/api/chatbot"),
    deny("DELETE", "/api/chatbot/memory"),
    deny("GET", "/api/chatbot/guilds/{guild}"),
    deny("PATCH", "/api/chatbot/guilds/{guild}"),
    // ── 샌드박스 예외 ──
    deny("GET", "/api/sandbox/unisolated"),
    deny("POST", "/api/sandbox/unisolated/{component}"),
    deny("DELETE", "/api/sandbox/unisolated/{component}"),
    deny("GET", "/api/sandbox/extension-gui"),
    deny("POST", "/api/sandbox/extension-gui/{component}"),
    deny("DELETE", "/api/sandbox/extension-gui/{component}"),
    // ── 액션 카탈로그: 목록만. 실행은 안에서 라우터를 다시 불러 이 표를 비켜 간다 ──
    allow("GET", "/api/actions"),
    deny("POST", "/api/actions/plan"),
    deny("GET", "/api/actions/runs/{run_id}"),
    deny("POST", "/api/actions/{id}/plan"),
    deny("POST", "/api/actions/{id}/run"),
    // ── 업데이트 ──
    deny("GET", "/api/updates/status"),
    deny("POST", "/api/updates/check"),
    deny("POST", "/api/updates/download"),
    deny("GET", "/api/updates/download/progress"),
    deny("POST", "/api/updates/apply"),
    deny("POST", "/api/updates/launch-updater"),
    deny("POST", "/api/updates/install"),
    deny("GET", "/api/updates/last-result"),
    deny("DELETE", "/api/updates/last-result"),
    deny("GET", "/api/updates/integrity"),
    deny("GET", "/api/updates/config"),
    deny("PUT", "/api/updates/config"),
    deny("GET", "/api/runtime/preflight"),
];

/// 원격 요청을 받지 않는 이유
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// 표에서 거부했거나 표에 없다
    NotAllowed,
    /// 경로가 이상하다 (`..`, 나눔 문자, 빈 조각)
    BadPath,
}

/// `path`(쿼리 포함 가능)를 받을지. 받으면 그 라우트의 패턴을 돌려준다
pub fn decide(method: &str, path: &str) -> Result<&'static str, Refusal> {
    let path = path.split_once('?').map_or(path, |(p, _)| p);
    let segments: Vec<&str> = path.strip_prefix('/').ok_or(Refusal::BadPath)?.split('/').collect();
    if segments.iter().any(|s| !safe_segment(s)) {
        return Err(Refusal::BadPath);
    }
    // 라우터처럼 리터럴 조각이 많이 맞는 패턴을 고른 뒤 그 패턴의 메서드를 본다
    // (매개변수 패턴의 허용으로 리터럴 라우트의 다른 처리기에 닿지 않게)
    let pattern = ROUTES
        .iter()
        .filter_map(|r| match_score(r.pattern, &segments).map(|score| (score, r.pattern)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, p)| p)
        .ok_or(Refusal::NotAllowed)?;
    match ROUTES.iter().find(|r| r.pattern == pattern && r.method == method) {
        Some(r) if r.access == Access::Allow => Ok(r.pattern),
        _ => Err(Refusal::NotAllowed),
    }
}

/// 패턴이 조각들과 맞으면 리터럴이 맞은 자리 (앞쪽 리터럴이 클수록 앞선다)
fn match_score(pattern: &str, segments: &[&str]) -> Option<Vec<bool>> {
    let parts: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    if parts.len() != segments.len() {
        return None;
    }
    parts
        .iter()
        .zip(segments)
        .map(|(p, s)| {
            if p.starts_with('{') {
                Some(false)
            } else {
                (p == s).then_some(true)
            }
        })
        .collect()
}

/// 비어 있지 않고, 풀어도 나눔 문자·`.`·`..`·NUL이 아닌 경로 조각
fn safe_segment(segment: &str) -> bool {
    match percent_decode(segment) {
        Some(d) => !d.is_empty() && d != "." && d != ".." && !d.contains(['/', '\\', '\0']),
        None => false,
    }
}

pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 응답에 인스턴스 설정을 담는 라우트 (비밀값을 빼고 돌려준다, `exec::hide_secrets`)
pub fn shows_instances(method: &str, pattern: &str) -> bool {
    method == "GET" && matches!(pattern, "/api/instances" | "/api/instance/{id}" | "/api/servers")
}

/// 허용된 라우트라도 원격에서 바꾸면 안 되는 본문 키. `file_fields`는 그 인스턴스 모듈의 PC 전용 설정 키(비밀값·경로)다
pub fn blocked_fields(method: &str, pattern: &str, body: Option<&Value>, file_fields: &[String]) -> Vec<String> {
    let Some(body) = body.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut blocked: Vec<String> = match (method, pattern) {
        ("PATCH", "/api/instance/{id}") => body
            .keys()
            .filter(|k| SETTINGS_PC_ONLY.contains(&k.as_str()) || file_fields.contains(k))
            .cloned()
            .collect(),
        ("POST", "/api/instances") => {
            let mut keys: Vec<String> = body
                .keys()
                .filter(|k| CREATE_PC_ONLY.contains(&k.as_str()))
                .cloned()
                .collect();
            if let Some(settings) = body.get("module_settings").and_then(Value::as_object) {
                keys.extend(
                    settings
                        .keys()
                        .filter(|k| file_fields.contains(k) || SETTINGS_PC_ONLY.contains(&k.as_str()))
                        .map(|k| format!("module_settings.{k}")),
                );
            }
            keys
        }
        // 게임 설정 파일 쓰기도 인스턴스 설정과 같은 칸을 막는다 (같은 값이 파일을 거쳐 인스턴스 설정으로 돌아온다)
        ("PUT", "/api/instance/{id}/properties") => body
            .get("settings")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|settings| settings.keys())
            .filter(|k| SETTINGS_PC_ONLY.contains(&k.as_str()) || file_fields.contains(k))
            .map(|k| format!("settings.{k}"))
            .collect(),
        // 대상은 인스턴스 설정의 REST 주소만. 다른 호스트로 보내면 게임 관리자 비밀번호가 실려 나간다
        ("POST", "/api/instance/{id}/rest") => body
            .keys()
            .filter(|k| ["rest_host", "rest_port", "use_https"].contains(&k.as_str()))
            .cloned()
            .collect(),
        ("POST", "/api/extensions/{id}/install") => body
            .keys()
            .filter(|k| ["download_url", "sha256"].contains(&k.as_str()))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    blocked.sort();
    blocked
}

/// 인스턴스 설정에서 PC에서만 바꾸는 키: 실행 파일·실행 인자·폴더·프로세스 찾기·REST 대상과 비밀번호·익스텐션 데이터.
/// 이것들이 열리면 "서버 시작"이 PC에서 아무 프로그램이나 실행하는 버튼이 된다
const SETTINGS_PC_ONLY: &[&str] = &[
    "executable_path",
    "working_dir",
    "extra_args",
    "process_name",
    "auto_detect",
    "rest_host",
    "rest_port",
    "rest_password",
    "extension_data",
];

/// 인스턴스를 만들 때 PC에서만 정하는 키 (기존 서버 폴더 가져오기 포함)
const CREATE_PC_ONLY: &[&str] = &[
    "executable_path",
    "working_dir",
    "migration_source",
    "rest_password",
    "extension_data",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 라우터 소스의 `.route("경로", get(..).post(..))`에서 (메서드, 경로)를 모두 뽑는다
    fn router_routes() -> Vec<(String, String)> {
        let route = regex::Regex::new(r#"\.route\(\s*"([^"]+)"\s*,"#).unwrap();
        let method = regex::Regex::new(r"(?:^|[\s.:(])(get|post|put|patch|delete)\(").unwrap();
        let mut out = Vec::new();
        for source in [include_str!("../ipc/mod.rs"), include_str!("../ipc/updates.rs")] {
            let starts: Vec<_> = route
                .captures_iter(source)
                .map(|c| (c.get(0).unwrap().end(), c[1].to_string()))
                .collect();
            for (i, (start, path)) in starts.iter().enumerate() {
                // 다음 `.route(`까지가 이 라우트의 처리기다. 첫 처리기 묶음(괄호 짝)만 본다
                let end = starts.get(i + 1).map_or(source.len(), |(s, _)| *s);
                let mut depth = 1;
                let mut stop = end;
                for (offset, ch) in source[*start..end].char_indices() {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                stop = start + offset;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                for m in method.captures_iter(&source[*start..stop]) {
                    out.push((m[1].to_ascii_uppercase(), path.clone()));
                }
            }
        }
        out
    }

    #[test]
    fn every_route_is_classified() {
        let routes = router_routes();
        assert!(routes.len() > 100, "parsed only {} routes", routes.len());
        let missing: Vec<_> = routes
            .iter()
            .filter(|(m, p)| !ROUTES.iter().any(|r| r.method == m && r.pattern == p))
            .collect();
        assert!(
            missing.is_empty(),
            "classify these routes in remote/gate.rs ROUTES: {missing:?}"
        );
        let stale: Vec<_> = ROUTES
            .iter()
            .filter(|r| !routes.iter().any(|(m, p)| r.method == m && r.pattern == p))
            .map(|r| (r.method, r.pattern))
            .collect();
        assert!(
            stale.is_empty(),
            "ROUTES lists routes the router no longer has: {stale:?}"
        );
    }

    #[test]
    fn allows_server_management() {
        assert_eq!(decide("POST", "/api/instance/abc/start"), Ok("/api/instance/{id}/start"));
        assert_eq!(
            decide("GET", "/api/instance/abc/logs?source=latest&lines=200"),
            Ok("/api/instance/{id}/logs")
        );
        assert_eq!(
            decide("DELETE", "/api/instance/abc/backups/2026-10-24_1200"),
            Ok("/api/instance/{id}/backups/{name}")
        );
    }

    #[test]
    fn refuses_pc_only_routes() {
        for (method, path) in [
            ("GET", "/api/config/node-token"),
            ("POST", "/api/daemon/shutdown"),
            ("POST", "/api/updates/install"),
            ("POST", "/api/actions/update.apply/run"),
            ("POST", "/api/modules/detect"),
            ("POST", "/api/module/minecraft/install"),
            // 저장하지 않은 설정 초안으로 실행 줄을 만든다 (모듈이 그 경로의 프로그램을 부를 수 있다)
            ("POST", "/api/instance/abc/launch-preview"),
            // 게임 설정 원문에는 관리자 비밀번호가 들어 있다
            ("GET", "/api/instance/abc/settings/live"),
            ("GET", "/api/instance/abc/properties"),
        ] {
            assert_eq!(decide(method, path), Err(Refusal::NotAllowed), "{method} {path}");
        }
    }

    #[test]
    fn refuses_unknown_routes_and_methods() {
        assert_eq!(decide("GET", "/api/brand-new"), Err(Refusal::NotAllowed));
        assert_eq!(decide("PUT", "/api/instance/abc/start"), Err(Refusal::NotAllowed));
        assert_eq!(
            decide("get", "/api/servers"),
            Err(Refusal::NotAllowed),
            "method is exact"
        );
    }

    #[test]
    fn literal_segment_wins_over_parameter() {
        // 라우터처럼 리터럴 라우트가 매개변수 라우트보다 먼저다: DELETE는 `/api/modules/{id}`가 아니라 GET만 있는 리터럴에 닿는다
        assert_eq!(decide("GET", "/api/modules/manifest"), Ok("/api/modules/manifest"));
        assert_eq!(
            decide("DELETE", "/api/modules/manifest"),
            Err(Refusal::NotAllowed),
            "DELETE /api/modules/{{id}} is denied"
        );
    }

    #[test]
    fn refuses_traversal_and_encoded_separators() {
        for path in [
            "/api/instance/../config/node-token",
            "/api/instance/a%2F..%2Fb/start",
            "/api/instance/a%5Cb/start",
            "/api/instance//start",
            "/api/instance/%2e%2e/start",
            "api/servers",
        ] {
            assert_eq!(decide("POST", path), Err(Refusal::BadPath), "{path}");
        }
    }

    #[test]
    fn accepts_percent_encoded_names() {
        assert_eq!(
            decide("GET", "/api/server/%EB%A7%88%EC%9D%B8/status"),
            Ok("/api/server/{name}/status")
        );
    }

    #[test]
    fn instance_settings_cannot_change_launch_or_paths() {
        let body = json!({ "port": 25565, "executable_path": "C:/x.exe", "working_dir": "C:/", "extra_args": ["-x"], "java_path": "C:/j.exe", "motd": "hi" });
        let blocked = blocked_fields("PATCH", "/api/instance/{id}", Some(&body), &["java_path".to_string()]);
        assert_eq!(
            blocked,
            vec!["executable_path", "extra_args", "java_path", "working_dir"]
        );
        assert!(blocked_fields("PATCH", "/api/instance/{id}", Some(&json!({ "port": 1 })), &[]).is_empty());
        // 게임 설정 파일 쓰기로 같은 칸을 비켜 가지 못한다
        let body = json!({ "settings": { "rest_password": "x", "java_path": "/bin/sh", "motd": "hi" } });
        assert_eq!(
            blocked_fields("PUT", "/api/instance/{id}/properties", Some(&body), &["java_path".to_string()]),
            vec!["settings.java_path", "settings.rest_password"]
        );
        assert!(blocked_fields("PUT", "/api/instance/{id}/properties", Some(&json!({ "settings": { "motd": "hi" } })), &[]).is_empty());
    }

    #[test]
    fn instance_creation_cannot_pick_paths() {
        let body = json!({ "name": "s", "module_name": "minecraft", "working_dir": "C:/", "migration_source": "C:/old", "module_settings": { "java_path": "C:/j.exe", "motd": "hi" } });
        let blocked = blocked_fields("POST", "/api/instances", Some(&body), &["java_path".to_string()]);
        assert_eq!(
            blocked,
            vec!["migration_source", "module_settings.java_path", "working_dir"]
        );
    }

    /// REST 대상(주소·포트)은 PC에서만 정한다: 인스턴스 설정·만들기·게임 설정 파일 어느 쪽으로도
    #[test]
    fn the_rest_target_is_set_only_on_the_pc() {
        let patch = json!({ "rest_port": 1, "port": 2 });
        assert_eq!(blocked_fields("PATCH", "/api/instance/{id}", Some(&patch), &[]), vec!["rest_port"]);
        let file = json!({ "settings": { "rest_port": 1 } });
        assert_eq!(blocked_fields("PUT", "/api/instance/{id}/properties", Some(&file), &[]), vec!["settings.rest_port"]);
        let create = json!({ "name": "s", "module_name": "palworld", "module_settings": { "rest_host": "h", "rest_port": 1, "motd": "hi" } });
        assert_eq!(
            blocked_fields("POST", "/api/instances", Some(&create), &[]),
            vec!["module_settings.rest_host", "module_settings.rest_port"]
        );
    }

    #[test]
    fn rest_calls_cannot_pick_another_host() {
        let body = json!({ "endpoint": "/x", "rest_host": "attacker.example", "rest_port": 80, "use_https": true });
        assert_eq!(
            blocked_fields("POST", "/api/instance/{id}/rest", Some(&body), &[]),
            vec!["rest_host", "rest_port", "use_https"]
        );
        assert!(blocked_fields("POST", "/api/instance/{id}/rest", Some(&json!({ "endpoint": "/x" })), &[]).is_empty());
    }

    #[test]
    fn extension_install_cannot_name_a_download() {
        let body = json!({ "download_url": "https://evil.example/x.zip", "sha256": "00" });
        assert_eq!(
            blocked_fields("POST", "/api/extensions/{id}/install", Some(&body), &[]),
            vec!["download_url", "sha256"]
        );
        assert!(blocked_fields("POST", "/api/extensions/{id}/install", None, &[]).is_empty());
    }
}
