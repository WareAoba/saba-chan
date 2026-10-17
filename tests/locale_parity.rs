//! 데몬이 쓰는 번역 파일(common.json, bot.json)은 모든 언어가 en과 같은 키와 같은 자리 표시자를 가진다.
//! gui.json, cli.json은 각 화면을 만드는 쪽이 채운다.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

const FILES: &[&str] = &["common.json", "bot.json"];

fn flatten(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                flatten(&key, v, out);
            }
        }
        other => {
            out.insert(prefix.to_string(), other.as_str().map(str::to_string).unwrap_or_else(|| other.to_string()));
        }
    }
}

fn load(lang: &str, file: &str) -> BTreeMap<String, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales").join(lang).join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
    let mut out = BTreeMap::new();
    flatten("", &serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {}", path.display(), e)), &mut out);
    out
}

fn placeholders(s: &str) -> Vec<&str> {
    let mut found: Vec<&str> = s.match_indices("{{").filter_map(|(i, _)| s[i..].find("}}").map(|j| &s[i..i + j + 2])).collect();
    found.sort();
    found
}

#[test]
fn daemon_locales_match_english() {
    let langs: Vec<String> = std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .filter(|l| l != "en")
        .collect();
    assert!(langs.len() >= 9, "expected the 10 shipped languages, found {:?}", langs);
    let mut problems = Vec::new();
    for file in FILES {
        let en = load("en", file);
        for lang in &langs {
            let other = load(lang, file);
            for key in en.keys().filter(|k| !other.contains_key(*k)) {
                problems.push(format!("{}/{}: missing {}", lang, file, key));
            }
            for key in other.keys().filter(|k| !en.contains_key(*k)) {
                problems.push(format!("{}/{}: not in en {}", lang, file, key));
            }
            for (key, value) in &en {
                if let Some(v) = other.get(key) {
                    if placeholders(value) != placeholders(v) {
                        problems.push(format!("{}/{}: placeholders differ in {}", lang, file, key));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
