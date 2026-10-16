//! 릴리즈 manifest 서명 (Ed25519)
//!
//! 릴리즈 빌더가 manifest를 서명하고, 업데이터는 번들된 공개키로 검증한 뒤에만
//! manifest를 믿는다. 릴레이는 서명된 문자열을 보관·전달만 한다.
//!
//! ## 정규화
//! 서명 대상은 manifest JSON의 정규형이다: 객체 키를 사전순으로 정렬하고 공백 없이
//! 직렬화한 UTF-8 바이트(`canonical_json`). 서명자와 검증자가 같은 함수를 쓴다.
//!
//! ## 키 회전
//! `keyId`로 공개키를 고른다. 새 키를 추가할 때는 `TRUSTED_KEYS`에 넣은 클라이언트를
//! 먼저 배포하고, 일정 기간 두 키를 모두 허용한 뒤 옛 키를 뺀다.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 업데이터가 믿는 서명 공개키 (keyId, base64 원시 32바이트 공개키)
pub const TRUSTED_KEYS: &[(&str, &str)] = &[(
    "saba-updater-v1",
    "bxKwq1Xq2yM/GJgGuCYHWeeXUjQ3MakwGuqYHOZTERU=",
)];

/// 릴레이가 돌려주는 서명된 manifest 봉투
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedManifest {
    pub manifest: Value,
    /// base64(ed25519_sign(canonical_json(manifest)))
    pub signature: String,
    pub key_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("unknown signing key: {0}")]
    UnknownKey(String),
    #[error("signature is not valid base64")]
    BadEncoding,
    #[error("manifest signature does not match")]
    Invalid,
    #[error("invalid signing key: {0}")]
    BadKey(String),
}

/// 객체 키를 정렬하고 공백 없이 직렬화한다.
///
/// `serde_json`의 `Map`은 `preserve_order` 기능이 꺼져 있으면 BTreeMap이라
/// 직렬화 결과가 이미 키 정렬 상태다. 기능이 켜지는 경우에 대비해 재귀적으로
/// 다시 정렬한다.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                Value::Object(entries.into_iter().map(|(k, v)| (k.clone(), sort(v))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_vec(&sort(value)).expect("serializing a JSON value cannot fail")
}

/// 번들된 공개키로 서명된 manifest를 검증한다.
pub fn verify(signed: &SignedManifest) -> Result<(), SignatureError> {
    #[cfg(feature = "test-isolation")]
    if signed.key_id == TEST_KEY_ID {
        if let Ok(public) = std::env::var(TEST_PUBKEY_ENV) {
            return verify_with(signed, &[(TEST_KEY_ID, public.as_str())]);
        }
    }
    verify_with(signed, TRUSTED_KEYS)
}

/// 테스트 빌드(`test-isolation`)에서만 받는 키. E2E가 자기 키쌍으로 mock 릴레이를 서명한다.
#[cfg(feature = "test-isolation")]
pub const TEST_KEY_ID: &str = "saba-updater-test";
#[cfg(feature = "test-isolation")]
pub const TEST_PUBKEY_ENV: &str = "SABA_TEST_UPDATER_PUBKEY";

/// 주어진 공개키 목록으로 검증한다(테스트·키 회전용).
pub fn verify_with(signed: &SignedManifest, keys: &[(&str, &str)]) -> Result<(), SignatureError> {
    let (_, public_b64) = keys
        .iter()
        .find(|(id, _)| *id == signed.key_id)
        .ok_or_else(|| SignatureError::UnknownKey(signed.key_id.clone()))?;
    let public = BASE64.decode(public_b64).map_err(|_| SignatureError::BadEncoding)?;
    let sig = BASE64.decode(signed.signature.trim()).map_err(|_| SignatureError::BadEncoding)?;
    UnparsedPublicKey::new(&ED25519, public)
        .verify(&canonical_json(&signed.manifest), &sig)
        .map_err(|_| SignatureError::Invalid)
}

/// manifest에 서명한다. 릴리즈 빌더(`examples/release_signing.rs`)와 테스트가 쓴다.
///
/// `private_key`는 64자리 hex Ed25519 seed(기존 `updater-v1.sk.hex` 형식) 또는
/// `generate_key_pair()`가 만든 base64 PKCS#8 개인키다.
pub fn sign(manifest: Value, key_id: &str, private_key: &str) -> Result<SignedManifest, SignatureError> {
    let pair = load_key_pair(private_key)?;
    let sig = pair.sign(&canonical_json(&manifest));
    Ok(SignedManifest {
        manifest,
        signature: BASE64.encode(sig.as_ref()),
        key_id: key_id.to_string(),
    })
}

fn load_key_pair(private_key: &str) -> Result<Ed25519KeyPair, SignatureError> {
    let key = private_key.trim();
    if key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()) {
        let seed: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&key[i * 2..i * 2 + 2], 16).expect("checked hex"))
            .collect();
        return Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|e| SignatureError::BadKey(e.to_string()));
    }
    let pkcs8 = BASE64
        .decode(key)
        .map_err(|_| SignatureError::BadKey("neither 64-hex seed nor base64 PKCS#8".into()))?;
    Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|e| SignatureError::BadKey(e.to_string()))
}

/// 개인키(hex seed 또는 PKCS#8)에서 base64 공개키를 구한다. `TRUSTED_KEYS` 대조용.
pub fn public_key_of(private_key: &str) -> Result<String, SignatureError> {
    Ok(BASE64.encode(load_key_pair(private_key)?.public_key().as_ref()))
}

/// 새 키쌍을 만든다. 반환값은 (base64 PKCS#8 개인키, base64 공개키).
pub fn generate_key_pair() -> Result<(String, String), SignatureError> {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|e| SignatureError::BadKey(e.to_string()))?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|e| SignatureError::BadKey(e.to_string()))?;
    Ok((BASE64.encode(pkcs8.as_ref()), BASE64.encode(pair.public_key().as_ref())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_keys() -> (String, String) {
        generate_key_pair().unwrap()
    }

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let v = json!({ "b": 1, "a": { "d": [ { "z": 1, "y": 2 } ], "c": "x" } });
        assert_eq!(
            String::from_utf8(canonical_json(&v)).unwrap(),
            r#"{"a":{"c":"x","d":[{"y":2,"z":1}]},"b":1}"#
        );
    }

    #[test]
    fn signed_manifest_verifies() {
        let (private, public) = test_keys();
        let signed = sign(json!({ "version": "0.2.0" }), "k1", &private).unwrap();
        assert_eq!(verify_with(&signed, &[("k1", &public)]), Ok(()));
    }

    #[test]
    fn tampered_manifest_is_rejected() {
        let (private, public) = test_keys();
        let mut signed = sign(json!({ "version": "0.2.0" }), "k1", &private).unwrap();
        signed.manifest["version"] = json!("9.9.9");
        assert_eq!(verify_with(&signed, &[("k1", &public)]), Err(SignatureError::Invalid));
    }

    #[test]
    fn unknown_key_is_rejected() {
        let (private, public) = test_keys();
        let signed = sign(json!({}), "other", &private).unwrap();
        assert!(matches!(
            verify_with(&signed, &[("k1", &public)]),
            Err(SignatureError::UnknownKey(_))
        ));
    }

    #[test]
    fn key_order_in_transport_does_not_matter() {
        let (private, public) = test_keys();
        let signed = sign(json!({ "a": 1, "b": 2 }), "k1", &private).unwrap();
        // 릴레이가 JSON을 다시 직렬화하며 키 순서를 바꿔도 검증된다
        let reparsed: SignedManifest =
            serde_json::from_str(&serde_json::to_string(&signed).unwrap()).unwrap();
        assert_eq!(verify_with(&reparsed, &[("k1", &public)]), Ok(()));
    }

    #[test]
    fn hex_seed_keys_sign_and_verify() {
        let seed = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        // RFC 8032 테스트 벡터 1의 공개키
        let public = public_key_of(seed).unwrap();
        assert_eq!(
            BASE64.decode(&public).unwrap(),
            [
                0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
                0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a
            ]
        );
        let signed = sign(json!({ "version": "0.1.1" }), "k1", seed).unwrap();
        assert_eq!(verify_with(&signed, &[("k1", &public)]), Ok(()));
    }

    #[test]
    fn bundled_key_is_well_formed() {
        for (_, public) in TRUSTED_KEYS {
            assert_eq!(BASE64.decode(public).unwrap().len(), 32);
        }
    }
}
