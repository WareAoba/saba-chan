//! 설정 부분 저장의 내용 지문(revision). 합치기(JSON Merge Patch)는 `saba_chan_updater_lib::config_file::merge_patch`.
//! `PATCH` 라우트가 바뀐 칸만 받고, 읽을 때 받은 revision이 지금과 다르면 거절한다

use serde_json::Value;

/// 내용 지문. 같은 내용이면 같은 값이다 (serde_json 객체는 키 순서로 쓰인다).
/// 내용에 비밀번호가 들 수 있고 지문은 원격에도 보이므로, 데몬이 켤 때마다 새로 정하는 키로 서명한다
/// (지문만으로 내용을 맞혀 볼 수 없게. 데몬이 다시 뜨면 지문이 바뀌어 한 번 다시 읽게 된다)
pub fn revision(value: &Value) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    let key = KEY.get_or_init(|| {
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        key
    });
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("any key length");
    mac.update(value.to_string().as_bytes());
    mac.finalize().into_bytes()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// 요청 본문 최상위의 `revision`을 떼어 낸다 (설정 키가 아니라 저장 조건이다)
pub fn take_revision(body: &mut Value) -> Option<String> {
    body.as_object_mut()?.remove("revision").and_then(|v| v.as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn revision_follows_the_content_not_the_key_order() {
        let a: Value = serde_json::from_str(r#"{"x":1,"y":{"b":2,"a":1}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"y":{"a":1,"b":2},"x":1}"#).unwrap();
        assert_eq!(revision(&a), revision(&b));
        assert_ne!(revision(&a), revision(&json!({"x":2,"y":{"a":1,"b":2}})));
    }

    /// 지문은 내용만으로 다시 셀 수 없다: 원격에서 본 지문으로 비밀번호를 맞혀 볼 수 없게
    #[test]
    fn the_revision_cannot_be_recomputed_from_the_content_alone() {
        use sha2::{Digest, Sha256};
        let value = json!({"rest_password": "hunter2"});
        let plain: String = Sha256::digest(value.to_string().as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect();
        assert_ne!(revision(&value), plain);
        assert_eq!(revision(&value).len(), 16);
    }

    #[test]
    fn the_revision_is_taken_out_of_the_body() {
        let mut body = json!({"revision": "abc", "theme": "dark"});
        assert_eq!(take_revision(&mut body).as_deref(), Some("abc"));
        assert_eq!(body, json!({"theme": "dark"}));
        assert_eq!(take_revision(&mut json!({"theme": "dark"})), None);
    }
}
