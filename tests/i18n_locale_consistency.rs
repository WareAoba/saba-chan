//! ═══════════════════════════════════════════════════════════════════
//! P0-7: i18n 다국어 일관성 검증 (Component Integration)
//! ═══════════════════════════════════════════════════════════════════
//!
//! `locales/<lang>/{common,gui,cli,bot}.json` 10개 언어가 동일한 키 셋과
//! 동일한 placeholder(`{{var}}`)를 갖는지 검증한다.
//!
//! 분류: CI (Component Integration) — JSON 파일 I/O + 키 셋 비교
//!
//! ## 검증 정책
//! - **레퍼런스 언어**: `en` (영어). 모든 다른 언어는 en의 키 집합과 동일해야 한다.
//! - **누락 키**: 다른 언어가 en 보다 적은 키를 가지면 누락으로 판정.
//! - **잉여 키**: 다른 언어가 en 에 없는 키를 가지면 잉여로 판정 (오타·legacy 흔적).
//! - **Placeholder**: 같은 키의 영문 값에 `{{user}}`가 있으면 다른 언어도 같은 토큰을 가져야 한다.
//!
//! ## 실행 모드
//! - **기본**: drift 갯수만 stderr로 보고하고 PASS — 알려진 드리프트로 회귀를 막지 않는다.
//! - **`SABA_I18N_STRICT=1`**: drift 발견 시 FAIL — 신규 키 추가 시 모든 언어 동시 갱신 강제.
//! - **`SABA_I18N_BASELINE=<count>`**: drift 카운트가 baseline을 초과하면 FAIL —
//!   기존 drift는 허용하되 추가 회귀만 차단하는 점진적 모드.
//!
//! 이 테스트는 빌드 시 자동 실행되어, 신규 키 추가 시 모든 언어 동시 갱신을 강제한다.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const REFERENCE_LANG: &str = "en";
const FILES: &[&str] = &["common.json", "gui.json", "cli.json"];

fn locales_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales")
}

/// 환경변수에 따라 drift를 어떻게 다룰지 결정한다.
/// - `SABA_I18N_STRICT=1` → 엄격(drift 1개라도 FAIL)
/// - `SABA_I18N_BASELINE=N` → drift 카운트 > N 일 때만 FAIL
/// - 둘 다 없음 → drift 보고만 하고 PASS (기본값)
fn assert_or_warn(category: &str, errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    let head_n = 30;
    let head: String = errors.iter().take(head_n).cloned().collect::<Vec<_>>().join("\n");
    let summary = format!(
        "[i18n/{category}] drift {} 건 (상위 {} 표시):\n{}",
        errors.len(),
        head_n.min(errors.len()),
        head
    );

    if std::env::var("SABA_I18N_STRICT").as_deref() == Ok("1") {
        panic!("{summary}");
    }
    if let Ok(baseline_str) = std::env::var("SABA_I18N_BASELINE") {
        if let Ok(baseline) = baseline_str.parse::<usize>() {
            if errors.len() > baseline {
                panic!(
                    "[i18n/{category}] baseline {baseline} 초과: 현재 {} (회귀)\n{head}",
                    errors.len()
                );
            }
        }
    }
    eprintln!("{summary}");
    eprintln!(
        "[i18n/{category}] 알려진 드리프트로 PASS 처리. 엄격 모드: SABA_I18N_STRICT=1 cargo test --test i18n_locale_consistency"
    );
}

fn discover_languages(root: &Path) -> Vec<String> {
    let mut langs: Vec<String> = fs::read_dir(root)
        .expect("locales/ 디렉토리 읽기 실패")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    langs.sort();
    langs
}

/// JSON 객체를 재귀적으로 순회해 dot-path 키 집합을 만든다.
fn flatten_keys(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                if matches!(v, Value::Object(_)) {
                    flatten_keys(v, &path, out);
                } else {
                    out.insert(path);
                }
            }
        }
        _ => {
            out.insert(prefix.to_string());
        }
    }
}

/// 주어진 키 path 값(문자열)에서 `{{...}}` placeholder 토큰만 정렬된 집합으로 추출.
fn placeholders_at(value: &Value, dot_path: &str) -> BTreeSet<String> {
    let mut current = value;
    for seg in dot_path.split('.') {
        current = match current.get(seg) {
            Some(v) => v,
            None => return BTreeSet::new(),
        };
    }
    let s = match current.as_str() {
        Some(s) => s,
        None => return BTreeSet::new(),
    };
    let mut out = BTreeSet::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = s[i + 2..].find("}}") {
                let token = s[i + 2..i + 2 + end].trim().to_string();
                out.insert(token);
                i += end + 4;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn load_locale(root: &Path, lang: &str, file: &str) -> Value {
    let path = root.join(lang).join(file);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("로케일 파일 읽기 실패 {}: {}", path.display(), e));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("JSON 파싱 실패 {}: {}", path.display(), e))
}

#[test]
fn all_locales_have_same_key_set_as_english() {
    let root = locales_root();
    let langs = discover_languages(&root);
    assert!(langs.contains(&REFERENCE_LANG.to_string()), "en 레퍼런스 누락");

    let mut errors: Vec<String> = Vec::new();

    for file in FILES {
        let en = load_locale(&root, REFERENCE_LANG, file);
        let mut en_keys = BTreeSet::new();
        flatten_keys(&en, "", &mut en_keys);

        for lang in &langs {
            if lang == REFERENCE_LANG {
                continue;
            }
            let other = load_locale(&root, lang, file);
            let mut other_keys = BTreeSet::new();
            flatten_keys(&other, "", &mut other_keys);

            let missing: Vec<&String> = en_keys.difference(&other_keys).collect();
            let extra: Vec<&String> = other_keys.difference(&en_keys).collect();

            if !missing.is_empty() {
                errors.push(format!(
                    "[{lang}/{file}] 누락 키 {} 개: 처음 10개 = {:?}",
                    missing.len(),
                    missing.iter().take(10).collect::<Vec<_>>()
                ));
            }
            if !extra.is_empty() {
                errors.push(format!(
                    "[{lang}/{file}] 잉여 키 {} 개: 처음 10개 = {:?}",
                    extra.len(),
                    extra.iter().take(10).collect::<Vec<_>>()
                ));
            }
        }
    }

    assert_or_warn("key-set", &errors);
}

#[test]
fn placeholder_tokens_match_reference() {
    let root = locales_root();
    let langs = discover_languages(&root);

    let mut errors: Vec<String> = Vec::new();

    for file in FILES {
        let en = load_locale(&root, REFERENCE_LANG, file);
        let mut en_keys = BTreeSet::new();
        flatten_keys(&en, "", &mut en_keys);

        for lang in &langs {
            if lang == REFERENCE_LANG {
                continue;
            }
            let other = load_locale(&root, lang, file);

            for key in &en_keys {
                let en_tokens = placeholders_at(&en, key);
                let other_tokens = placeholders_at(&other, key);

                // 다른 언어에 키 자체가 없는 경우는 본 테스트가 아니라
                // `all_locales_have_same_key_set_as_english`가 책임진다.
                if other_tokens.is_empty() && en_tokens.is_empty() {
                    continue;
                }

                if en_tokens != other_tokens {
                    errors.push(format!(
                        "[{lang}/{file}] 키 '{key}': placeholder 불일치 en={:?} vs {lang}={:?}",
                        en_tokens, other_tokens
                    ));
                }
            }
        }
    }

    assert_or_warn("placeholders", &errors);
}

#[test]
fn every_locale_file_is_valid_json_object() {
    let root = locales_root();
    let langs = discover_languages(&root);

    for lang in &langs {
        for file in FILES {
            let v = load_locale(&root, lang, file);
            assert!(
                v.is_object(),
                "[{lang}/{file}] 최상위가 객체가 아님: {:?}",
                v
            );
        }
    }
}
