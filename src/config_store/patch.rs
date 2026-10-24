//! 설정 부분 저장: JSON Merge Patch(RFC 7386)와 내용 지문(revision).
//! `PATCH` 라우트가 바뀐 칸만 받고, 읽을 때 받은 revision이 지금과 다르면 거절한다 (0.1.10 결정 1)

use serde_json::Value;

/// `target`에 `patch`를 얹는다: 객체는 키별로 내려가며 합치고, `null`은 그 키를 지우고, 그 밖의 값은 통째로 바꾼다
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(changes) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    let map = target.as_object_mut().expect("object");
    for (key, change) in changes {
        if change.is_null() {
            map.remove(key);
        } else {
            merge_patch(map.entry(key.clone()).or_insert(Value::Null), change);
        }
    }
}

/// 내용 지문. 같은 내용이면 같은 값이다 (serde_json 객체는 키 순서로 쓰인다)
pub fn revision(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.to_string().as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// 요청 본문 최상위의 `revision`을 떼어 낸다 (설정 키가 아니라 저장 조건이다)
pub fn take_revision(body: &mut Value) -> Option<String> {
    body.as_object_mut()?.remove("revision").and_then(|v| v.as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    /// RFC 7386 부록 A의 예
    #[test]
    fn follows_the_rfc_examples() {
        let cases = [
            (json!({"a":"b"}), json!({"a":"c"}), json!({"a":"c"})),
            (json!({"a":"b"}), json!({"b":"c"}), json!({"a":"b","b":"c"})),
            (json!({"a":"b"}), json!({"a":null}), json!({})),
            (json!({"a":"b","b":"c"}), json!({"a":null}), json!({"b":"c"})),
            (json!({"a":["b"]}), json!({"a":"c"}), json!({"a":"c"})),
            (json!({"a":"c"}), json!({"a":["b"]}), json!({"a":["b"]})),
            (json!({"a":{"b":"c"}}), json!({"a":{"b":"d","c":null}}), json!({"a":{"b":"d"}})),
            (json!({"a":[{"b":"c"}]}), json!({"a":[1]}), json!({"a":[1]})),
            (json!(["a","b"]), json!(["c","d"]), json!(["c","d"])),
            (json!({"a":"b"}), json!(["c"]), json!(["c"])),
            (json!({"a":"foo"}), json!(null), json!(null)),
            (json!({"a":"foo"}), json!("bar"), json!("bar")),
            (json!({"e":null}), json!({"a":1}), json!({"e":null,"a":1})),
            (json!([1,2]), json!({"a":"b","c":null}), json!({"a":"b"})),
            (json!({}), json!({"a":{"bb":{"ccc":null}}}), json!({"a":{"bb":{}}})),
        ];
        for (mut target, patch, expected) in cases {
            merge_patch(&mut target, &patch);
            assert_eq!(target, expected, "patch {patch}");
        }
    }

    #[test]
    fn revision_follows_the_content_not_the_key_order() {
        let a: Value = serde_json::from_str(r#"{"x":1,"y":{"b":2,"a":1}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"y":{"a":1,"b":2},"x":1}"#).unwrap();
        assert_eq!(revision(&a), revision(&b));
        assert_ne!(revision(&a), revision(&json!({"x":2,"y":{"a":1,"b":2}})));
    }

    #[test]
    fn the_revision_is_taken_out_of_the_body() {
        let mut body = json!({"revision": "abc", "theme": "dark"});
        assert_eq!(take_revision(&mut body).as_deref(), Some("abc"));
        assert_eq!(body, json!({"theme": "dark"}));
        assert_eq!(take_revision(&mut json!({"theme": "dark"})), None);
    }

    fn json_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i32>().prop_map(|n| json!(n)),
            "[a-c]{0,2}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
                prop::collection::btree_map("[a-d]", inner, 0..4).prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        /// 같은 patch를 두 번 얹어도 한 번 얹은 것과 같다 (다시 보낸 저장이 값을 바꾸지 않는다)
        #[test]
        fn applying_a_patch_twice_changes_nothing_more(target in json_value(), patch in json_value()) {
            let mut once = target.clone();
            merge_patch(&mut once, &patch);
            let mut twice = once.clone();
            merge_patch(&mut twice, &patch);
            prop_assert_eq!(once, twice);
        }

        /// patch에 없는 키는 그대로 남는다 (바뀐 칸만 보낸 저장이 다른 칸을 덮지 않는다)
        #[test]
        fn keys_outside_the_patch_are_left_alone(target in prop::collection::btree_map("[a-d]", json_value(), 0..4), patch in prop::collection::btree_map("[a-d]", json_value(), 0..4)) {
            let mut merged = Value::Object(target.clone().into_iter().collect());
            merge_patch(&mut merged, &Value::Object(patch.clone().into_iter().collect()));
            for (key, value) in &target {
                if !patch.contains_key(key) {
                    prop_assert_eq!(merged.get(key), Some(value));
                }
            }
        }
    }
}
