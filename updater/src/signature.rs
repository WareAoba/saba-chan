//! Ed25519 기반 manifest 서명 검증
//!
//! - 릴레이는 `{ manifest, signature, keyId }` 구조의 JSON을 서빙.
//! - 서명 대상은 `manifest` 객체의 **정규화 JSON 직렬화** 바이트.
//! - 정규화는 `serde_json::to_vec`의 결정론적 직렬화를 사용하지 않고,
//!   대신 `canonical_json_bytes`로 **키를 사전순 정렬**한 뒤 직렬화한다.
//! - 공개키는 `PublicKey` 상수 테이블에 `keyId → 32바이트` 매핑으로 번들.
//!
//! ## 보안 원칙
//! - 개인키는 **릴리즈 빌더**(GH Actions secret)에만 존재.
//! - 릴레이 DB에는 이미 서명된 문자열만 저장 → DB 침해 ≠ 위조.
//! - `keyId` 라우팅으로 키 로테이션 지원 (구·신 키 동시 허용).
//! - 다운그레이드 방지는 상위 레이어(`scheduler`)에서 처리.

use ed25519_dalek::{Signature, Verifier, VerifyingKey, SIGNATURE_LENGTH};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 서명된 manifest JSON의 래퍼 (릴레이가 서빙하는 원본 구조).
///
/// `manifest`는 내용에 접근하기 전에 [`verify_signed_manifest`]로 검증해야 한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedManifest {
    /// Manifest 본문 (서명 대상). `serde_json::Value`로 보관하여
    /// 정규화 시점까지 원본 구조를 변형하지 않는다.
    pub manifest: serde_json::Value,
    /// base64로 인코딩된 Ed25519 서명 (64바이트)
    pub signature: String,
    /// 키 식별자 (예: "saba-updater-v1")
    #[serde(rename = "keyId")]
    pub key_id: String,
}

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("알 수 없는 key_id: {0}")]
    UnknownKeyId(String),
    #[error("서명 디코드 실패: {0}")]
    InvalidSignatureEncoding(String),
    #[error("서명 길이 오류: {0} (기대 {}바이트)", SIGNATURE_LENGTH)]
    InvalidSignatureLength(usize),
    #[error("정규화 JSON 직렬화 실패: {0}")]
    CanonicalSerialize(String),
    #[error("서명 검증 실패")]
    VerifyFailed,
}

// ─────────────────────────────────────────────────────────────
//  공개키 번들
// ─────────────────────────────────────────────────────────────

/// (key_id, 32바이트 Ed25519 공개키) 테이블.
///
/// 프로덕션 릴레이 검증에 사용되는 saba-updater v1 공개키.
/// 비밀키는 관리자 머신의 `%APPDATA%/saba-chan/.keys/updater-v1.sk.hex`에만 존재하며,
/// `examples/gen_ed25519_keypair.rs`로 생성되었다. 키 로테이션 시 `v2`, `v3` 순으로
/// 추가하고 `bundled()`에 병존시킨다. 구 키는 해당 릴리스가 EOL된 후 제거.
///
/// 테스트는 [`PublicKeyRegistry::from_entries`] 로 주입한다.
const SABA_UPDATER_V1: [u8; 32] = [
    0x6f, 0x12, 0xb0, 0xab, 0x55, 0xea, 0xdb, 0x23,
    0x3f, 0x18, 0x98, 0x06, 0xb8, 0x26, 0x07, 0x59,
    0xe7, 0x97, 0x52, 0x34, 0x37, 0x31, 0xa9, 0x30,
    0x1a, 0xea, 0x98, 0x1c, 0xe6, 0x53, 0x11, 0x15,
];

pub struct PublicKeyRegistry {
    entries: Vec<(String, VerifyingKey)>,
}

impl PublicKeyRegistry {
    /// 번들된 기본 공개키 레지스트리.
    ///
    /// `SABA_UPDATER_V1` 이 유효한 공개키가 아니면 (바이트 배열이 curve point로 디코딩 불가)
    /// 해당 키는 제외되고 빈 레지스트리가 반환된다. 이 경우 모든 서명 검증은 실패하여
    /// 릴레이 경로가 안전하게 비활성화된다.
    pub fn bundled() -> Self {
        let mut entries: Vec<(String, VerifyingKey)> = Vec::new();
        if let Ok(vk) = VerifyingKey::from_bytes(&SABA_UPDATER_V1) {
            entries.push(("saba-updater-v1".to_string(), vk));
        }
        Self { entries }
    }

    /// 테스트/특수용: 임의의 (key_id, verifying_key) 목록으로 레지스트리 생성.
    pub fn from_entries(entries: Vec<(String, VerifyingKey)>) -> Self {
        Self { entries }
    }

    pub fn find(&self, key_id: &str) -> Option<&VerifyingKey> {
        self.entries
            .iter()
            .find(|(id, _)| id == key_id)
            .map(|(_, vk)| vk)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────
//  정규화 JSON (JCS 간소화: 키 사전순 정렬 + 공백 없음)
// ─────────────────────────────────────────────────────────────

/// `serde_json::Value`를 **맵 키가 사전순 정렬된** 공백 없는 JSON 바이트로 직렬화한다.
///
/// RFC 8785(JCS)의 엄밀한 숫자 정규화까지는 하지 않지만, 릴리즈 빌더와
/// 업데이터가 동일한 `serde_json`을 사용하므로 왕복이 일치한다. 실제 JCS 구현은
/// 추후 필요 시 교체한다 (단, 기존 서명과의 호환성은 유지 필요).
pub fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, SignatureError> {
    let mut out = Vec::with_capacity(256);
    write_canonical(value, &mut out)
        .map_err(|e| SignatureError::CanonicalSerialize(e.to_string()))?;
    Ok(out)
}

fn write_canonical(v: &serde_json::Value, out: &mut Vec<u8>) -> std::io::Result<()> {
    use serde_json::Value;
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => {
            // serde_json::Number는 기본 표현을 그대로 사용 — 빌더/검증기가 동일 구현이면 안전.
            out.extend_from_slice(n.to_string().as_bytes());
        }
        Value::String(s) => {
            let encoded = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
            out.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(arr) => {
            out.push(b'[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            // 키 사전순 정렬 (바이트 기준).
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let key_str = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string());
                out.extend_from_slice(key_str.as_bytes());
                out.push(b':');
                write_canonical(&map[k.as_str()], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
//  서명 검증
// ─────────────────────────────────────────────────────────────

/// 릴레이 응답(JSON 문자열 또는 구조체)에서 manifest 내용을 꺼내 서명을 검증한다.
///
/// 성공 시 검증된 `manifest` 객체를 반환. 실패 시 어떤 단계에서 실패했는지
/// [`SignatureError`]로 상세 보고한다.
pub fn verify_signed_manifest(
    signed: &SignedManifest,
    keys: &PublicKeyRegistry,
) -> Result<serde_json::Value, SignatureError> {
    let vk = keys
        .find(&signed.key_id)
        .ok_or_else(|| SignatureError::UnknownKeyId(signed.key_id.clone()))?;

    // base64 → 64바이트
    use base64::Engine as _;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signed.signature.as_bytes())
        .map_err(|e| SignatureError::InvalidSignatureEncoding(e.to_string()))?;
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return Err(SignatureError::InvalidSignatureLength(sig_bytes.len()));
    }
    let mut arr = [0u8; SIGNATURE_LENGTH];
    arr.copy_from_slice(&sig_bytes);
    let signature = Signature::from_bytes(&arr);

    let payload = canonical_json_bytes(&signed.manifest)?;

    vk.verify(&payload, &signature)
        .map_err(|_| SignatureError::VerifyFailed)?;

    Ok(signed.manifest.clone())
}

// ═══════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    fn make_keypair() -> (SigningKey, VerifyingKey) {
        // 결정론적 키 (테스트용)
        let seed = [7u8; 32];
        let sk = SigningKey::from_bytes(&seed);
        let vk = sk.verifying_key();
        (sk, vk)
    }

    fn sign_payload(sk: &SigningKey, manifest: &serde_json::Value) -> String {
        let bytes = canonical_json_bytes(manifest).unwrap();
        let sig = sk.sign(&bytes);
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    }

    #[test]
    fn canonical_json_sorts_keys_alphabetically() {
        let a = json!({ "b": 1, "a": 2, "c": { "z": 9, "y": 8 } });
        let b = json!({ "a": 2, "c": { "y": 8, "z": 9 }, "b": 1 });
        let ca = canonical_json_bytes(&a).unwrap();
        let cb = canonical_json_bytes(&b).unwrap();
        assert_eq!(ca, cb);
        assert_eq!(
            std::str::from_utf8(&ca).unwrap(),
            r#"{"a":2,"b":1,"c":{"y":8,"z":9}}"#
        );
    }

    #[test]
    fn canonical_json_handles_arrays_and_strings() {
        let v = json!({
            "notes": { "ko": "안녕", "en": "hi" },
            "list": [3, 1, 2],
        });
        let out = canonical_json_bytes(&v).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.starts_with(r#"{"list":[3,1,2],"notes":"#));
        assert!(s.contains(r#""en":"hi""#));
        assert!(s.contains(r#""ko":"안녕""#));
    }

    #[test]
    fn verify_round_trip() {
        let (sk, vk) = make_keypair();
        let manifest = json!({
            "version": "0.7.2",
            "channel": "stable",
            "components": { "saba-core": { "sha256": "a".repeat(64) } }
        });
        let sig = sign_payload(&sk, &manifest);

        let registry = PublicKeyRegistry::from_entries(vec![("saba-updater-v1".to_string(), vk)]);
        let signed = SignedManifest {
            manifest: manifest.clone(),
            signature: sig,
            key_id: "saba-updater-v1".to_string(),
        };
        let verified = verify_signed_manifest(&signed, &registry).unwrap();
        assert_eq!(verified, manifest);
    }

    #[test]
    fn verify_fails_on_tampered_manifest() {
        let (sk, vk) = make_keypair();
        let manifest = json!({ "version": "0.7.2" });
        let sig = sign_payload(&sk, &manifest);

        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);
        let tampered = json!({ "version": "0.7.3" });  // 변조
        let signed = SignedManifest {
            manifest: tampered,
            signature: sig,
            key_id: "v1".to_string(),
        };
        let err = verify_signed_manifest(&signed, &registry).unwrap_err();
        assert!(matches!(err, SignatureError::VerifyFailed));
    }

    #[test]
    fn verify_fails_on_unknown_key_id() {
        let (sk, vk) = make_keypair();
        let manifest = json!({ "v": 1 });
        let sig = sign_payload(&sk, &manifest);

        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);
        let signed = SignedManifest {
            manifest,
            signature: sig,
            key_id: "v999".to_string(),
        };
        assert!(matches!(
            verify_signed_manifest(&signed, &registry).unwrap_err(),
            SignatureError::UnknownKeyId(_)
        ));
    }

    #[test]
    fn verify_fails_on_malformed_base64() {
        let (_, vk) = make_keypair();
        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);
        let signed = SignedManifest {
            manifest: json!({}),
            signature: "!!!not-base64!!!".to_string(),
            key_id: "v1".to_string(),
        };
        assert!(matches!(
            verify_signed_manifest(&signed, &registry).unwrap_err(),
            SignatureError::InvalidSignatureEncoding(_)
        ));
    }

    #[test]
    fn verify_fails_on_short_signature() {
        let (_, vk) = make_keypair();
        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);
        use base64::Engine as _;
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let signed = SignedManifest {
            manifest: json!({}),
            signature: short,
            key_id: "v1".to_string(),
        };
        assert!(matches!(
            verify_signed_manifest(&signed, &registry).unwrap_err(),
            SignatureError::InvalidSignatureLength(16)
        ));
    }

    #[test]
    fn key_rotation_allows_multiple_keys() {
        let (sk_v1, vk_v1) = make_keypair();
        let seed_v2 = [9u8; 32];
        let sk_v2 = SigningKey::from_bytes(&seed_v2);
        let vk_v2 = sk_v2.verifying_key();

        let registry = PublicKeyRegistry::from_entries(vec![
            ("v1".to_string(), vk_v1),
            ("v2".to_string(), vk_v2),
        ]);

        let manifest = json!({ "version": "1.0.0" });
        // v1으로 서명
        let sig_v1 = sign_payload(&sk_v1, &manifest);
        let signed_v1 = SignedManifest {
            manifest: manifest.clone(),
            signature: sig_v1,
            key_id: "v1".to_string(),
        };
        assert!(verify_signed_manifest(&signed_v1, &registry).is_ok());

        // v2로 서명
        let sig_v2 = sign_payload(&sk_v2, &manifest);
        let signed_v2 = SignedManifest {
            manifest,
            signature: sig_v2,
            key_id: "v2".to_string(),
        };
        assert!(verify_signed_manifest(&signed_v2, &registry).is_ok());
    }

    /// `PublicKeyRegistry::bundled()` 가 실제 공개키를 로드하는지 검증.
    ///
    /// 이 테스트가 실패하면 `SABA_UPDATER_V1` 상수가 유효한 Ed25519 공개키가 아니라는 뜻이다.
    /// 릴레이 배포 전 반드시 통과해야 한다.
    #[test]
    fn bundled_registry_loads_production_key() {
        let registry = PublicKeyRegistry::bundled();
        assert!(
            !registry.is_empty(),
            "bundled() 가 빈 레지스트리를 반환함 — SABA_UPDATER_V1 공개키가 유효한 curve point인지 확인 필요"
        );
        assert!(
            registry.find("saba-updater-v1").is_some(),
            "key_id 'saba-updater-v1' 이 번들된 레지스트리에서 발견되지 않음"
        );
    }

    #[test]
    fn injected_public_key_verifies_test_fixture() {
        let sk = SigningKey::from_bytes(&[0x5a; 32]);
        let manifest = json!({ "version": "0.1.0-test", "kind": "test-key-check" });
        let signed = SignedManifest {
            signature: sign_payload(&sk, &manifest),
            manifest,
            key_id: "test-updater-v1".to_string(),
        };
        let registry = PublicKeyRegistry::from_entries(vec![(
            "test-updater-v1".to_string(),
            sk.verifying_key(),
        )]);
        assert!(verify_signed_manifest(&signed, &registry).is_ok());
        assert!(verify_signed_manifest(&signed, &PublicKeyRegistry::bundled()).is_err());
    }
}