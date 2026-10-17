//! Cloud agent 인증 — `.node_token` 로드 + HMAC-SHA256 요청 서명
//!
//! 릴레이 서버 [`server-chan/relay-server/src/middleware/auth.ts`] 의
//! `authenticateNode` 가 검증하는 헤더 4종을 생성한다:
//!   - `Authorization: Bearer <token>`
//!   - `x-request-timestamp: <unix seconds>`
//!   - `x-request-nonce: <uuid>`
//!   - `x-request-signature: hex(HMAC-SHA256(method\nurl_path\nts\nnonce\nbody, secret))`
//!
//! 토큰 형식: `sbn_<nodeId>.<secret>` (페어링 시 릴레이가 발급).

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Node token file path 상수 — `<data_dir>/.node_token`.
pub const NODE_TOKEN_FILE: &str = ".node_token";

/// 노드 토큰 파싱 결과.
#[derive(Debug, Clone)]
pub struct ParsedToken {
    pub raw: String,
    pub node_id: String,
    pub secret: String,
}

/// `sbn_<nodeId>.<secret>` 포맷을 검증·분해한다.
///
/// # Errors
/// `sbn_` prefix 누락 / `.` 구분자 누락 / 빈 nodeId·secret 모두 None.
pub fn parse_token(raw: &str) -> Option<ParsedToken> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix("sbn_")?;
    let dot = rest.find('.')?;
    let (node_id, secret) = rest.split_at(dot);
    let secret = &secret[1..]; // skip the '.'
    if node_id.is_empty() || secret.is_empty() {
        return None;
    }
    Some(ParsedToken {
        raw: trimmed.to_string(),
        node_id: node_id.to_string(),
        secret: secret.to_string(),
    })
}

/// 지정한 디렉토리에서 `.node_token` 파일을 읽고 파싱한다.
///
/// 토큰이 없거나(클라우드 미페어링) 형식이 잘못된 경우 `None` 반환.
pub fn load_node_token_from(data_dir: &std::path::Path) -> Option<ParsedToken> {
    let token_path: PathBuf = data_dir.join(NODE_TOKEN_FILE);
    let raw = std::fs::read_to_string(&token_path).ok()?;
    parse_token(&raw)
}

/// 현재 unix epoch 초 (테스트에서 mock 가능하도록 분리).
pub fn current_unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// HMAC-SHA256 서명 헤더 4쌍을 생성한다.
///
/// `body`는 직렬화된 JSON 문자열(없으면 빈 문자열)을 그대로 넣는다.
/// URL_path는 **쿼리스트링 제외** 경로만 사용 (서버 검증과 일치).
pub fn signed_headers(
    token: &ParsedToken,
    method: &str,
    url_path: &str,
    body: &str,
    ts: u64,
    nonce: &str,
) -> Vec<(String, String)> {
    let payload = format!(
        "{}\n{}\n{}\n{}\n{}",
        method.to_uppercase(),
        url_path,
        ts,
        nonce,
        body
    );

    let mut mac = HmacSha256::new_from_slice(token.secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());

    vec![
        ("Authorization".into(), format!("Bearer {}", token.raw)),
        ("X-Saba-Client".into(), "1".into()),
        ("x-request-timestamp".into(), ts.to_string()),
        ("x-request-nonce".into(), nonce.to_string()),
        ("x-request-signature".into(), sig),
    ]
}

/// `signed_headers`를 `reqwest::RequestBuilder`에 적용한다.
pub fn apply_signed(
    builder: reqwest::RequestBuilder,
    token: &ParsedToken,
    method: &str,
    url_path: &str,
    body: &str,
) -> reqwest::RequestBuilder {
    let ts = current_unix_ts();
    let nonce = uuid::Uuid::new_v4().to_string();
    let headers = signed_headers(token, method, url_path, body, ts, &nonce);
    let mut b = builder;
    for (k, v) in headers {
        b = b.header(&k, &v);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_valid() {
        let t = parse_token("sbn_abc123.secretXYZ").unwrap();
        assert_eq!(t.node_id, "abc123");
        assert_eq!(t.secret, "secretXYZ");
        assert_eq!(t.raw, "sbn_abc123.secretXYZ");
    }

    #[test]
    fn parse_token_strips_whitespace() {
        let t = parse_token("  sbn_n.s\n").unwrap();
        assert_eq!(t.node_id, "n");
        assert_eq!(t.secret, "s");
    }

    #[test]
    fn parse_token_rejects_missing_prefix() {
        assert!(parse_token("abc.def").is_none());
    }

    #[test]
    fn parse_token_rejects_missing_dot() {
        assert!(parse_token("sbn_abc123").is_none());
    }

    #[test]
    fn parse_token_rejects_empty_parts() {
        assert!(parse_token("sbn_.secret").is_none());
        assert!(parse_token("sbn_node.").is_none());
    }

    #[test]
    fn signed_headers_deterministic() {
        let token = ParsedToken {
            raw: "sbn_n.s".to_string(),
            node_id: "n".to_string(),
            secret: "s".to_string(),
        };
        let h1 = signed_headers(&token, "GET", "/poll", "", 1700000000, "nonce-1");
        let h2 = signed_headers(&token, "GET", "/poll", "", 1700000000, "nonce-1");
        assert_eq!(h1, h2, "동일 입력은 동일 서명을 산출해야 한다");
    }

    #[test]
    fn signed_headers_changes_with_body() {
        let token = ParsedToken {
            raw: "sbn_n.s".to_string(),
            node_id: "n".to_string(),
            secret: "s".to_string(),
        };
        let h1 = signed_headers(&token, "POST", "/heartbeat", "{}", 1, "x");
        let h2 = signed_headers(&token, "POST", "/heartbeat", "{\"a\":1}", 1, "x");
        let sig1 = h1.iter().find(|(k, _)| k == "x-request-signature").unwrap().1.clone();
        let sig2 = h2.iter().find(|(k, _)| k == "x-request-signature").unwrap().1.clone();
        assert_ne!(sig1, sig2, "body 차이는 서명을 바꿔야 한다");
    }

    #[test]
    fn signed_headers_payload_format_matches_server() {
        // 릴레이 서버의 `auth.ts` 와 정확히 동일한 페이로드 포맷:
        //   METHOD\nURL_PATH\nTS\nNONCE\nBODY
        let token = ParsedToken {
            raw: "sbn_n.s".to_string(),
            node_id: "n".to_string(),
            secret: "s".to_string(),
        };
        let headers = signed_headers(&token, "post", "/heartbeat", "body-data", 42, "uuid-1");

        // 수동 계산
        let payload = "POST\n/heartbeat\n42\nuuid-1\nbody-data";
        let mut mac = HmacSha256::new_from_slice(b"s").unwrap();
        mac.update(payload.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());

        let actual = headers.iter().find(|(k, _)| k == "x-request-signature").unwrap().1.clone();
        assert_eq!(actual, expected);
    }

    #[test]
    fn load_token_from_dir_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(NODE_TOKEN_FILE), "sbn_aaa.bbb").unwrap();
        let t = load_node_token_from(dir.path()).unwrap();
        assert_eq!(t.node_id, "aaa");
        assert_eq!(t.secret, "bbb");
    }

    #[test]
    fn load_token_from_dir_returns_none_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_node_token_from(dir.path()).is_none());
    }
}
