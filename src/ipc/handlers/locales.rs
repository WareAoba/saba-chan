//! 설치 폴더 번역을 내준다: `GET /api/locales/{lang}/{ns}` (05 §번역 런타임)
//!
//! GUI는 이것을 먼저 읽고, 없거나 키가 빠지면 빌드에 묶인 번역을 쓴다. 업데이터가 `locales/`를 통째로
//! 바꾸고 `locales/version`에 판 번호를 쓰면, 그 판 번호가 ETag이고 `/api/updates/status`의 `locales_version`이다.

use axum::extract::Path;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// 내줄 수 있는 묶음 (설치 폴더 `locales/<lang>/<ns>.json`). 이 목록 밖의 이름은 읽지 않는다
pub const NAMESPACES: &[&str] = &["common", "gui", "daemon", "web", "cli"];

/// 언어 코드 모양만 받는다: `^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$`
fn valid_language(lang: &str) -> bool {
    let mut parts = lang.split('-');
    let base = parts.next().unwrap_or("");
    (2..=3).contains(&base.len())
        && base.bytes().all(|b| b.is_ascii_lowercase())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// `If-None-Match`가 이 판을 가리키는가 (약한 표시·여러 값·`*`도 받는다)
fn matches_etag(if_none_match: &str, version: &str) -> bool {
    if_none_match
        .split(',')
        .map(str::trim)
        .any(|tag| tag == "*" || tag.trim_start_matches("W/").trim_matches('"') == version)
}

/// 한 묶음의 응답. `dir`은 설치 폴더 `locales/` (없으면 404)
pub(crate) fn respond(dir: Option<&std::path::Path>, lang: &str, ns: &str, if_none_match: Option<&str>) -> Response {
    if !valid_language(lang) || !NAMESPACES.contains(&ns) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(dir) = dir else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = dir.join(lang).join(format!("{ns}.json"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // 깨진 파일은 없는 것처럼: GUI가 내장본으로 떨어진다
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&text) {
        tracing::warn!("[locales] {} 파싱 실패: {}", path.display(), e);
        return StatusCode::NOT_FOUND.into_response();
    }
    let etag = crate::chat::i18n::version_in(dir)
        .and_then(|v| HeaderValue::from_str(&format!("\"{v}\"")).ok().map(|h| (v, h)));
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Some((version, value)) = etag {
        headers.insert(header::ETAG, value);
        if if_none_match.is_some_and(|tag| matches_etag(tag, &version)) {
            return (StatusCode::NOT_MODIFIED, headers).into_response();
        }
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    (StatusCode::OK, headers, text).into_response()
}

/// GET /api/locales/{lang}/{ns}
pub async fn get_locale(Path((lang, ns)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let if_none_match = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok());
    let dir = crate::chat::i18n::installed_dir();
    respond(dir.as_deref(), &lang, &ns, if_none_match)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(version: Option<&str>) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("ko")).unwrap();
        std::fs::write(tmp.path().join("ko/gui.json"), r#"{"a":"가"}"#).unwrap();
        if let Some(v) = version {
            std::fs::write(tmp.path().join("version"), format!("{v}\n")).unwrap();
        }
        tmp
    }

    async fn body(resp: Response) -> String {
        String::from_utf8(
            axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn serves_installed_bundle_with_version_as_etag() {
        let tmp = installed(Some("0.1.14"));
        let resp = respond(Some(tmp.path()), "ko", "gui", None);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::ETAG], "\"0.1.14\"");
        assert_eq!(body(resp).await, r#"{"a":"가"}"#);
    }

    #[test]
    fn same_version_is_not_modified() {
        let tmp = installed(Some("0.1.14"));
        assert_eq!(
            respond(Some(tmp.path()), "ko", "gui", Some("\"0.1.14\"")).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            respond(Some(tmp.path()), "ko", "gui", Some("W/\"0.1.14\"")).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            respond(Some(tmp.path()), "ko", "gui", Some("\"0.1.13\"")).status(),
            StatusCode::OK
        );
    }

    #[test]
    fn without_version_file_serves_without_etag() {
        let tmp = installed(None);
        let resp = respond(Some(tmp.path()), "ko", "gui", Some("\"\""));
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().get(header::ETAG).is_none());
    }

    #[test]
    fn missing_file_or_folder_or_broken_json_is_not_found() {
        let tmp = installed(Some("1"));
        assert_eq!(
            respond(Some(tmp.path()), "ja", "gui", None).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(respond(None, "ko", "gui", None).status(), StatusCode::NOT_FOUND);
        std::fs::write(tmp.path().join("ko/common.json"), "{").unwrap();
        assert_eq!(
            respond(Some(tmp.path()), "ko", "common", None).status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn only_language_codes_and_listed_namespaces_are_read() {
        let tmp = installed(Some("1"));
        std::fs::write(tmp.path().join("secret.json"), "{}").unwrap();
        for (lang, ns) in [
            ("..", "gui"),
            ("ko/..", "gui"),
            ("KO", "gui"),
            ("k", "gui"),
            ("ko-", "gui"),
            ("ko", "../secret"),
            ("ko", "secret"),
            ("ko", "gui.json"),
            ("ko", ""),
        ] {
            assert_eq!(
                respond(Some(tmp.path()), lang, ns, None).status(),
                StatusCode::BAD_REQUEST,
                "{lang}/{ns}"
            );
        }
        for lang in ["ko", "pt-BR", "zh-Hant-TW", "fil"] {
            assert!(valid_language(lang), "{lang}");
        }
    }
}
