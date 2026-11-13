/// 익스텐션 hook에 넘기는 인스턴스는 REST 비밀번호와 비밀스러운 설정 값을 가리고, 나머지는 그대로 둔다 (D-E2)
#[test]
fn hooks_get_the_instance_without_its_secrets() {
    let mut inst = crate::instance::ServerInstance::new("srv", "game");
    inst.module_settings.insert("server_password".into(), serde_json::json!("hunter2-join"));
    inst.module_settings.insert("max_players".into(), serde_json::json!(16));
    let value = crate::extension::hook_context(&inst, std::path::Path::new("/data/srv"), None)["instance"].clone();
    let text = value.to_string();
    assert!(!text.contains("hunter2"), "{text}");
    assert_eq!(value["module_settings"]["max_players"], 16);
    assert_eq!(value["name"], "srv");
    let settings = crate::extension::redact_for_hooks(None, &serde_json::json!({ "rest_password": "hunter2-new", "port": 7777 }));
    assert!(!settings.to_string().contains("hunter2"), "{settings}");
    assert_eq!(settings["port"], 7777);
    // 비밀값과 같은 글자의 이름·경로는 비밀이 아니다: 값 대신 키 이름으로만 가린다
    let mut same = crate::instance::ServerInstance::new("palworld", "game");
    same.working_dir = Some("C:/srv/palworld/data".into());
    same.module_settings.insert("server_password".into(), serde_json::json!("palworld"));
    let value = crate::extension::hook_context(&same, std::path::Path::new("/data/srv"), None)["instance"].clone();
    assert_eq!(value["name"], "palworld");
    assert_eq!(value["working_dir"], "C:/srv/palworld/data");
    assert_ne!(value["module_settings"]["server_password"], "palworld");
}

#[test]
fn extension_archives_respect_file_count_and_expanded_size_limits() {
    use std::io::Write;
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut bytes);
        for name in ["one", "two"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"data").unwrap();
        }
        zip.finish().unwrap();
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.into_inner())).unwrap();
    let limits = |entries, expanded| crate::archive::Limits { what: "Extension package", entries, expanded };
    assert!(crate::archive::check(&mut archive, &limits(1, 100)).is_err());
    assert!(crate::archive::check(&mut archive, &limits(10, 7)).is_err());
    assert!(crate::archive::check(&mut archive, &limits(10, 8)).is_ok());
}

#[test]
fn contract_paths_i18n_never_reads_outside_the_extension_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let ext = tmp.path().join("ext");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(tmp.path().join("ko.json"), r#"{"private":"outside"}"#).unwrap();
    let mut manager = super::ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let manifest = serde_json::from_value(serde_json::json!({ "id":"ext", "name":"Ext", "version":"1.0.0", "i18n_dir":".." })).unwrap();
    manager.discovered.insert("ext".into(), super::DiscoveredExtension { manifest, dir: ext, warnings: vec![] });
    assert!(manager.load_i18n("ext", "ko").is_none());
    let ext = manager.discovered.get_mut("ext").unwrap();
    std::fs::write(ext.dir.join("ko.json"), r#"{"label":"inside"}"#).unwrap();
    ext.manifest.i18n_dir = Some(".".into());
    assert_eq!(manager.load_i18n("ext", "ko").unwrap()["label"], "inside");
}

/// FIRED_HOOKS의 이름마다 데몬 코드(이 파일 밖) 어딘가에서 그 이름으로 hook을 부른다
#[test]
fn every_listed_hook_is_fired_somewhere() {
    fn sources(dir: &std::path::Path, out: &mut String) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("extension/manifest.rs") && !p.ends_with("extension/tests.rs") {
                out.push_str(&std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    let mut code = String::new();
    sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut code);
    let unfired: Vec<&&str> = FIRED_HOOKS.iter().filter(|h| !code.contains(&format!("\"{}\"", h))).collect();
    assert!(unfired.is_empty(), "listed in FIRED_HOOKS but never fired: {:?}", unfired);
}

use super::*;
use serde_json::json;

#[test]
fn extension_id_must_be_a_single_folder_name() {
    for ok in ["steamcmd", "music-bot", "a_b2"] {
        assert!(is_valid_extension_id(ok), "{}", ok);
    }
    for bad in ["", "../instances", "..", "a/b", "a\\b", "C:x", "/abs", "a.b"] {
        assert!(!is_valid_extension_id(bad), "{}", bad);
    }
    let err = parse_manifest(r#"{"id":"../instances","name":"x","version":"1.0.0"}"#).unwrap_err();
    assert!(err.to_string().contains("invalid extension id"), "{}", err);
}

#[test]
fn test_evaluate_condition_bool_true() {
    let mut ext_data = HashMap::new();
    ext_data.insert(
        "docker_enabled".to_string(),
        Value::Bool(true),
    );
    assert!(ExtensionManager::evaluate_condition(
        "instance.ext_data.docker_enabled",
        &ext_data
    ));
}

#[test]
fn test_evaluate_condition_bool_false() {
    let mut ext_data = HashMap::new();
    ext_data.insert(
        "docker_enabled".to_string(),
        Value::Bool(false),
    );
    assert!(!ExtensionManager::evaluate_condition(
        "instance.ext_data.docker_enabled",
        &ext_data
    ));
}

#[test]
fn test_evaluate_condition_missing_key() {
    let ext_data = HashMap::new();
    assert!(!ExtensionManager::evaluate_condition(
        "instance.ext_data.docker_enabled",
        &ext_data
    ));
}

#[test]
fn test_evaluate_condition_unknown_pattern() {
    let ext_data = HashMap::new();
    assert!(!ExtensionManager::evaluate_condition(
        "some.other.pattern",
        &ext_data
    ));
}

#[test]
fn test_manifest_deserialization() {
    let json = json!({
        "id": "docker",
        "name": "Docker Isolation",
        "version": "1.0.0",
        "description": "Docker container isolation",
        "python_modules": {
            "compose_manager": "compose_manager.py"
        },
        "hooks": {
            "server.pre_start": {
                "module": "compose_manager",
                "function": "start",
                "condition": "instance.ext_data.docker_enabled"
            }
        },
        "instance_fields": {
            "docker_enabled": { "type": "boolean", "default": false }
        }
    });

    let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
    assert_eq!(manifest.id, "docker");
    assert_eq!(manifest.hooks.len(), 1);
    assert!(manifest.hooks.contains_key("server.pre_start"));
    assert_eq!(manifest.instance_fields.len(), 1);
}

#[test]
fn test_extension_manager_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let found = mgr.discover().unwrap();
    assert!(found.is_empty());
}

#[test]
fn test_extension_manager_discover() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("test_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let found = mgr.discover().unwrap();
    assert_eq!(found, vec!["test_ext"]);
    assert!(mgr.discovered.contains_key("test_ext"));
}

#[test]
fn test_enable_disable() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("test_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

    assert!(!mgr.is_enabled("test_ext"));
    mgr.enable("test_ext").unwrap();
    assert!(mgr.is_enabled("test_ext"));
    mgr.disable("test_ext", &no_instances).unwrap();
    assert!(!mgr.is_enabled("test_ext"));
}

/// id 그대로인 폴더(예전 설치)와 '_' 폴더가 함께 있으면 발견·폴더 찾기·신뢰 판정이 모두 '_' 폴더를 쓴다
#[test]
fn both_extension_folders_resolve_to_the_underscore_one() {
    let tmp = tempfile::tempdir().unwrap();
    for (folder, version) in [("ue4-ini", "1.0.0"), ("ue4_ini", "2.0.0")] {
        std::fs::create_dir_all(tmp.path().join(folder)).unwrap();
        std::fs::write(tmp.path().join(folder).join("manifest.json"), format!(r#"{{"id":"ue4-ini","name":"U","version":"{version}"}}"#)).unwrap();
    }
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    let underscore = tmp.path().join("ue4_ini");
    assert_eq!(mgr.discovered["ue4-ini"].dir, underscore);
    assert_eq!(mgr.discovered["ue4-ini"].manifest.version, "2.0.0");
    assert_eq!(extension_dir(tmp.path(), "ue4-ini"), Some(underscore.clone()));
    let fresh = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    assert_eq!(fresh.ext_dir_on_disk("ue4-ini"), underscore, "before discovery too");
    std::fs::remove_dir_all(&underscore).unwrap();
    assert_eq!(extension_dir(tmp.path(), "ue4-ini"), Some(tmp.path().join("ue4-ini")), "an old install by id");
    assert_eq!(extension_dir(tmp.path(), "missing"), None);
}

/// 필수 익스텐션은 켜지고 디스크에 있어야 쓸 수 있다. 켜졌지만 디스크에 없는 것은 따로 알린다
#[test]
fn missing_required_extensions_tell_disabled_from_not_installed() {
    let tmp = tempfile::tempdir().unwrap();
    for id in ["ready", "off"] {
        std::fs::create_dir_all(tmp.path().join(id)).unwrap();
        std::fs::write(tmp.path().join(id).join("manifest.json"), format!(r#"{{"id":"{id}","name":"T","version":"0.1.0"}}"#)).unwrap();
    }
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enabled.insert("ready".to_string());
    mgr.enabled.insert("gone".to_string());

    let required: Vec<String> = ["gone", "ready", "off", "unknown"].iter().map(|s| s.to_string()).collect();
    let found = mgr.missing_required(&required);
    assert_eq!(found.missing, vec!["gone", "off", "unknown"]);
    assert_eq!(found.not_installed, vec!["gone"]);
    assert_eq!(mgr.missing_required(&["ready".to_string()]), MissingExtensions::default());
}

/// 설정 파일을 쓸 수 없으면 켜기·끄기·설정 저장이 실패로 답하고 메모리 상태도 그대로 둔다
#[test]
fn unsaved_state_is_reported_and_not_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("test_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#).unwrap();
    // 설정 폴더 자리에 파일이 있어 config.json을 만들 수 없다
    let blocker = tmp.path().join("not-a-dir");
    std::fs::write(&blocker, "").unwrap();
    let mut mgr = ExtensionManager::with_config_file(tmp.path().to_str().unwrap(), ConfigFile::at(&blocker));
    mgr.discover().unwrap();
    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

    assert!(mgr.enable("test_ext").is_err());
    assert!(!mgr.is_enabled("test_ext"));

    mgr.enabled.insert("test_ext".to_string());
    assert!(mgr.disable("test_ext", &no_instances).is_err());
    assert!(mgr.is_enabled("test_ext"));

    let values = HashMap::from([("a".to_string(), json!(1))]);
    assert!(mgr.set_extension_config("test_ext", values).is_err());
    assert!(mgr.get_extension_config("test_ext").is_empty());

    assert!(mgr.unmount("test_ext", &no_instances).is_err());
    assert!(mgr.is_enabled("test_ext") && mgr.discovered.contains_key("test_ext"));

    // 켜진 채로는 끄기(저장)부터 실패해 폴더를 지우지 않는다. 꺼진 것은 폴더를 지우고 끝낸다
    assert!(mgr.remove("test_ext", &no_instances).is_err());
    assert!(ext_dir.exists());
    mgr.enabled.remove("test_ext");
    assert!(mgr.remove("test_ext", &no_instances).is_ok());
    assert!(!ext_dir.exists());
}

#[test]
fn test_enable_unknown_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    assert!(mgr.enable("nonexistent").is_err());
}

#[test]
fn data_slots_take_old_names_and_the_daemon_decides_what_a_server_shows() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "box", json!({ "id": "box", "name": "box", "version": "0.1.0", "cli": { "slots": {
        "InstanceList.badge": { "text": "📦", "condition": "instance.ext_data.boxed" },
        "ServerDetail.status": [{ "label": "Box", "value_from": "box_state" }, { "label": "Missing", "value_from": "nope" }, { "label": "Key", "value_from": "api_token" }],
        "CreateInstance.options": [{ "field": "boxed", "type": "boolean", "label": "Box" }]
    } } }));
    write_ext(tmp.path(), "off", json!({ "id": "off", "name": "off", "version": "0.1.0", "cli": { "slots": {
        "ServerCard.badge": { "text": "x" }
    } } }));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enabled.insert("box".to_string());
    // 만들 때 물을 칸은 켜진 익스텐션의 것만 (옛 이름 포함)
    assert_eq!(mgr.create_options(), vec![json!({ "field": "boxed", "type": "boolean", "label": "Box" })]);

    let cli = mgr.discovered("box").unwrap().manifest.cli.clone().unwrap();
    let slots = normalized_slots(&cli);
    assert_eq!(slots[SLOT_OPTIONS], json!([{ "field": "boxed", "type": "boolean", "label": "Box" }]));
    assert_eq!(slots[SLOT_BADGE].as_array().unwrap().len(), 1, "old name lands under the new one as an array");

    let mut data = HashMap::new();
    let status = HashMap::from([("box".to_string(), json!({ "box_state": "up", "api_token": "T0KEN" }))]);
    let shown = mgr.server_slots(&data, &status);
    assert!(shown.get(SLOT_BADGE).is_none(), "condition false, disabled extension skipped");
    assert_eq!(shown[SLOT_STATUS], json!([{ "extension": "box", "label": "Box", "value": "up" }]));

    data.insert("boxed".to_string(), json!(true));
    data.insert("box_state".to_string(), json!(3));
    let shown = mgr.server_slots(&data, &HashMap::new());
    assert_eq!(shown[SLOT_BADGE], json!([{ "extension": "box", "text": "📦" }]));
    assert_eq!(shown[SLOT_STATUS], json!([{ "extension": "box", "label": "Box", "value": "3" }]), "falls back to extension_data");
}

// ── 동적 마운트/언마운트 테스트 ──

fn write_ext(root: &std::path::Path, dir: &str, manifest: Value) {
    let d = root.join(dir);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), manifest.to_string()).unwrap();
}

#[test]
fn manifest_schema_v1_is_strict_and_legacy_warns() {
    let ok = json!({"schema_version": 1, "id": "a", "name": "A", "version": "0.1.0",
        "python_modules": {"m": "m.py"}, "hooks": {"daemon.startup": {"module": "m", "function": "f"}}});
    let (_, warnings) = parse_manifest(&ok.to_string()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");

    let mut typo = ok.clone();
    typo["instance_feilds"] = json!({});
    assert!(parse_manifest(&typo.to_string()).unwrap_err().to_string().contains("instance_feilds"));

    let mut never = ok.clone();
    never["hooks"]["server.pre_create"] = json!({"module": "m", "function": "g", "async": true});
    let err = parse_manifest(&never.to_string()).unwrap_err().to_string();
    assert!(err.contains("server.pre_create") && err.contains("async"), "{err}");

    let mut legacy = never.clone();
    legacy.as_object_mut().unwrap().remove("schema_version");
    let (m, warnings) = parse_manifest(&legacy.to_string()).unwrap();
    assert_eq!(m.schema_version, 0);
    assert!(warnings.iter().any(|w| w.contains("never fired")), "{warnings:?}");

    let mut runtime = ok.clone();
    runtime["runtime"] = json!({"persistent": true});
    let (m, warnings) = parse_manifest(&runtime.to_string()).unwrap();
    assert!(m.runtime.persistent && warnings.is_empty(), "{warnings:?}");
    runtime["runtime"]["persistant"] = json!(true);
    assert!(parse_manifest(&runtime.to_string()).unwrap_err().to_string().contains("persistant"));

    let mut perms = ok.clone();
    perms["permissions"] = json!({"network": true, "spawn_process": true});
    let (m, _) = parse_manifest(&perms.to_string()).unwrap();
    assert!(m.permissions.network && m.permissions.spawn_process && !m.permissions.native_code);
    perms["permissions"]["netwrok"] = json!(true);
    assert!(parse_manifest(&perms.to_string()).unwrap_err().to_string().contains("netwrok"));

    // 파이썬 모듈 경로는 익스텐션 폴더 안이어야 한다 (예전 형식도 경고가 아니라 오류)
    for bad in ["../other/m.py", "sub/../../m.py", "/abs/m.py", ""] {
        let mut outside = ok.clone();
        outside["python_modules"]["m"] = json!(bad);
        assert!(parse_manifest(&outside.to_string()).is_err(), "{bad}");
        outside.as_object_mut().unwrap().remove("schema_version");
        assert!(parse_manifest(&outside.to_string()).is_err(), "legacy {bad}");
    }
    let mut nested = ok.clone();
    nested["python_modules"]["m"] = json!("src/py/m.py");
    assert!(parse_manifest(&nested.to_string()).is_ok());

    let mut newer = ok.clone();
    newer["schema_version"] = json!(2);
    assert!(parse_manifest(&newer.to_string()).unwrap_err().to_string().contains("schema_version 2"));
}

#[test]
fn event_hooks_take_any_event_kind_but_no_condition() {
    let with = |hook: &str, binding: Value| {
        json!({"schema_version": 1, "id": "a", "name": "A", "version": "0.1.0",
            "python_modules": {"m": "m.py"}, "hooks": {hook: binding}})
        .to_string()
    };
    let plain = json!({"module": "m", "function": "f"});
    for ok in ["event:backup.done", "event:instance.exit", "event:some_future.kind"] {
        assert!(parse_manifest(&with(ok, plain.clone())).unwrap().1.is_empty(), "{ok}");
    }
    for bad in ["event:", "event:Backup.Done", "event: backup.done"] {
        assert!(parse_manifest(&with(bad, plain.clone())).unwrap_err().to_string().contains("never fired"), "{bad}");
    }
    let gated = json!({"module": "m", "function": "f", "condition": "instance.ext_data.x"});
    assert!(parse_manifest(&with("event:backup.done", gated)).unwrap_err().to_string().contains("condition"));
}

#[test]
fn scan_skips_folders_whose_name_is_not_the_id_and_reports_them() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "good", json!({"id": "good", "name": "G", "version": "0.1.0"}));
    write_ext(tmp.path(), "ue4_ini", json!({"id": "ue4-ini", "name": "U", "version": "0.1.0"}));
    write_ext(tmp.path(), "wrong_dir", json!({"id": "other", "name": "W", "version": "0.1.0"}));
    write_ext(tmp.path(), "broken", json!({"name": "no id"}));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let mut found = mgr.discover().unwrap();
    found.sort();
    assert_eq!(found, ["good", "ue4-ini"]);
    let errors = mgr.load_errors();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().any(|e| e.error.contains("does not match")));
}

#[test]
fn min_app_version_and_core_dependency_are_checked_on_enable() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "future", json!({"id": "future", "name": "F", "version": "0.1.0", "min_app_version": "999.0.0"}));
    write_ext(tmp.path(), "core", json!({"id": "core", "name": "C", "version": "0.1.0",
        "min_app_version": "0.0.1", "dependencies": {"saba-core": ">=0.0.1"}}));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    let err = mgr.enable("future").unwrap_err();
    assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "app_version_unsatisfied");
    let item = mgr.list().into_iter().find(|i| i.id == "future").unwrap();
    assert!(item.incompatible.unwrap().contains("999.0.0"));
    mgr.enable("core").expect("saba-core dependency is satisfied by this daemon");
}

#[test]
fn in_use_ignores_empty_values_and_remove_checks_disabled_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "box", json!({"id": "box", "name": "B", "version": "0.1.0",
        "instance_fields": {"box_enabled": {"type": "boolean"}}}));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    let unused: HashMap<String, Value> =
        [("box_enabled".to_string(), json!(false))].into_iter().collect();
    let zero: HashMap<String, Value> = [("box_enabled".to_string(), json!(0))].into_iter().collect();
    let used: HashMap<String, Value> = [("box_enabled".to_string(), json!(true))].into_iter().collect();
    assert!(mgr.instances_using_ext("box", &[("a", &unused), ("b", &zero)]).is_empty());
    assert!(mgr.extensions_in_use(&[("a", &unused)]).is_empty());
    assert_eq!(mgr.extensions_in_use(&[("a", &unused), ("b", &used)]), ["box".to_string()].into_iter().collect());
    let err = mgr.remove("box", &[("stopped", &used)]).unwrap_err();
    assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "in_use");
    assert!(tmp.path().join("box").exists());
}

fn zip_package(manifest: Value, extra: &str) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("manifest.json", opts).unwrap();
        w.write_all(manifest.to_string().as_bytes()).unwrap();
        w.start_file("data.txt", opts).unwrap();
        w.write_all(extra.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn install_checks_package_then_swaps_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let v = |ver: &str| json!({"id": "box", "name": "B", "version": ver,
        "instance_fields": {"box_enabled": {"type": "boolean"}}, "dependencies": {"helper": ">=0.1.0"}});
    write_ext(tmp.path(), "box", v("0.1.0"));
    std::fs::write(tmp.path().join("box/data.txt"), "old").unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let pkg = zip_package(v("0.2.0"), "new");
    let err = ExtensionManager::stage_package(tmp.path(), "box", &pkg, Some("00")).unwrap_err();
    assert!(err.to_string().contains("sha256"), "{err}");
    let err = ExtensionManager::stage_package(tmp.path(), "other", &pkg, None).unwrap_err();
    assert!(format!("{err:#}").contains("does not match"), "{err:#}");
    assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "old");

    use sha2::{Digest, Sha256};
    let sha = hex::encode(Sha256::digest(&pkg));
    let staged = ExtensionManager::stage_package(tmp.path(), "box", &pkg, Some(&sha)).unwrap();
    let running: HashMap<String, Value> = [("box_enabled".to_string(), json!(true))].into_iter().collect();
    let err = mgr.install_staged("box", &staged, &[("srv", &running)], false).unwrap_err();
    assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "in_use");
    assert!(!staged.exists(), "refused package is cleaned up");

    let staged = ExtensionManager::stage_package(tmp.path(), "box", &pkg, None).unwrap();
    mgr.install_staged("box", &staged, &[], false).unwrap();
    assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "new");
    assert_eq!(mgr.discovered["box"].manifest.version, "0.2.0");
    assert!(!tmp.path().join(".old-box").exists());

    let remote: Vec<RemoteExtensionInfo> = serde_json::from_value(json!([
        {"id": "helper", "name": "H", "version": "0.1.0", "download_url": "x"}
    ])).unwrap();
    assert_eq!(mgr.missing_dependencies("box", &remote), ["helper"]);
}

/// `a-b`와 `a_b`는 같은 폴더(`a_b`)를 쓴다: 다른 id가 이미 쓰는 폴더는 설치로 바꾸지 않는다 (일반 설치·폴더에 둔 zip 둘 다)
#[test]
fn installing_never_replaces_the_folder_of_another_extension() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "a_b", json!({"id": "a_b", "name": "A", "version": "0.1.0"}));
    std::fs::write(tmp.path().join("a_b/data.txt"), "mine").unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let pkg = zip_package(json!({"id": "a-b", "name": "B", "version": "0.1.0"}), "other");
    let staged = ExtensionManager::stage_package(tmp.path(), "a-b", &pkg, None).unwrap();
    let err = mgr.install_staged("a-b", &staged, &[], false).unwrap_err();
    assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "folder_in_use", "{err:#}");
    assert!(!staged.exists(), "refused package is cleaned up");
    assert_eq!(std::fs::read_to_string(tmp.path().join("a_b/data.txt")).unwrap(), "mine");

    std::fs::write(tmp.path().join("a-b.zip"), &pkg).unwrap();
    mgr.discover().unwrap();
    assert_eq!(std::fs::read_to_string(tmp.path().join("a_b/data.txt")).unwrap(), "mine");
    assert_eq!(mgr.discovered["a_b"].manifest.name, "A");
    assert!(!mgr.discovered.contains_key("a-b"));
}

/// 더 새 앱을 요구하는 패키지는 켤 때와 같은 app_version_unsatisfied로 거절한다 (클라이언트가 "업데이트가 필요해요"로 보인다)
#[test]
fn package_needing_newer_app_is_refused_with_version_code() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = zip_package(json!({"id": "future", "name": "F", "version": "0.1.0", "min_app_version": "999.0.0"}), "x");
    let err = ExtensionManager::stage_package(tmp.path(), "future", &pkg, None).unwrap_err();
    let ext = err.downcast_ref::<ExtensionError>().expect("structured error");
    assert_eq!((ext.error_code.as_str(), ext.related.clone()), ("app_version_unsatisfied", vec!["999.0.0".to_string()]));
    // F8: 사바쨩이 모자라 거절했으면 요구 버전을 따로 싣는다
    assert_eq!(ext.required_core().as_deref(), Some("999.0.0"));
    let core = ExtensionError::component_version_unsatisfied("x", "saba-core", ">=1.2.0", Some("1.0.0"));
    assert_eq!(core.required_core().as_deref(), Some("1.2.0"));
    let other = ExtensionError::component_version_unsatisfied("x", "ext-steamcmd", ">=1.2.0", None);
    assert_eq!(other.required_core(), None);
}

#[test]
fn overlapping_installs_of_one_id_keep_their_own_staged_files() {
    let tmp = tempfile::tempdir().unwrap();
    let m = json!({"id": "box", "name": "B", "version": "0.1.0"});
    let first = ExtensionManager::stage_package(tmp.path(), "box", &zip_package(m.clone(), "first"), None).unwrap();
    let second = ExtensionManager::stage_package(tmp.path(), "box", &zip_package(m, "second"), None).unwrap();
    assert_ne!(first, second);
    assert_eq!(std::fs::read_to_string(first.join("data.txt")).unwrap(), "first");
    assert_eq!(std::fs::read_to_string(second.join("data.txt")).unwrap(), "second");

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.install_staged("box", &first, &[], false).unwrap();
    assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "first");
}

#[test]
fn dropped_zip_replaces_existing_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let m = json!({"id": "box", "name": "B", "version": "0.1.0"});
    write_ext(tmp.path(), "box", m.clone());
    std::fs::write(tmp.path().join("box/data.txt"), "old").unwrap();
    std::fs::write(tmp.path().join("extension-box.zip"), zip_package(m, "new")).unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "new");
    assert!(!tmp.path().join("extension-box.zip").exists());
    assert!(mgr.discovered.contains_key("box"));
}

#[test]
fn hooks_run_in_priority_then_id_order() {
    let tmp = tempfile::tempdir().unwrap();
    for (id, priority) in [("beta", None), ("zeta", Some(10)), ("alpha", None), ("off", Some(0))] {
        let dir = tmp.path().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut manifest = json!({
            "id": id, "name": id, "version": "0.1.0",
            "python_modules": { "m": "m.py" },
            "hooks": { "daemon.startup": { "module": "m", "function": "f" } },
        });
        if let Some(p) = priority {
            manifest["priority"] = json!(p);
        }
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    for id in ["alpha", "beta", "zeta"] {
        mgr.enabled.insert(id.to_string());
    }
    for _ in 0..5 {
        let order: Vec<&str> = mgr.hooks_for("daemon.startup").iter().map(|(e, _)| e.manifest.id.as_str()).collect();
        assert_eq!(order, ["zeta", "alpha", "beta"], "disabled 'off' is skipped, priority 10 before default 100");
    }
}

#[test]
fn a_null_setting_removes_the_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let set = |pairs: &[(&str, Value)]| pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<HashMap<_, _>>();
    mgr.set_extension_config("box", set(&[("a", json!(1)), ("b", json!("x"))])).unwrap();
    mgr.set_extension_config("box", set(&[("a", Value::Null)])).unwrap();
    assert_eq!(mgr.get_extension_config("box"), set(&[("b", json!("x"))]));
}

#[test]
fn hook_context_carries_only_the_called_extensions_settings() {
    let tmp = tempfile::tempdir().unwrap();
    for id in ["alpha", "beta"] {
        let dir = tmp.path().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({
            "id": id, "name": id, "version": "0.1.0",
            "python_modules": { "m": "m.py" },
            "hooks": { "daemon.startup": { "module": "m", "function": "f" } },
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    for id in ["alpha", "beta"] {
        mgr.enabled.insert(id.to_string());
        let own: HashMap<String, Value> = [("token".to_string(), json!(format!("{id}-secret")))].into_iter().collect();
        mgr.extension_config.insert(id.to_string(), own);
    }
    let plan = mgr.plan_hook("daemon.startup", json!({}), None);
    assert_eq!(plan.calls.len(), 2);
    for call in &plan.calls {
        let id = call.ext_id.as_str();
        assert_eq!(call.context["extension_config"], json!({ id: { "token": format!("{id}-secret") } }), "{id}");
    }
}

#[test]
fn test_mount_unmount() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("my_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"my_ext","name":"My Extension","version":"0.1.0"}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    // 아직 discover 안 했으므로 비어 있음
    assert!(mgr.list().is_empty());

    // 핫 마운트
    mgr.mount("my_ext").unwrap();
    assert_eq!(mgr.list().len(), 1);

    // 핫 언마운트
    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
    mgr.unmount("my_ext", &no_instances).unwrap();
    assert!(mgr.list().is_empty());
}

#[test]
fn test_mount_bad_id_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    // 디렉토리 이름은 "wrong_dir"이지만 manifest id는 "correct_id"
    let ext_dir = tmp.path().join("wrong_dir");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"correct_id","name":"Test","version":"0.1.0"}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let result = mgr.mount("wrong_dir");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not match"));
}

#[test]
fn test_rescan_finds_new_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    assert!(mgr.list().is_empty());

    // 디스크에 새 익스텐션 추가
    let ext_dir = tmp.path().join("late_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"late_ext","name":"Late","version":"0.2.0"}"#,
    )
    .unwrap();

    let newly_found = mgr.rescan().unwrap();
    assert_eq!(newly_found, vec!["late_ext"]);
    assert_eq!(mgr.list().len(), 1);
}

// ── 의존성 검증 테스트 ──

#[test]
fn test_enable_with_missing_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("child_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let result = mgr.enable("child_ext");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("parent_ext"));
}

#[test]
fn test_enable_with_satisfied_dependency() {
    let tmp = tempfile::tempdir().unwrap();

    // parent_ext
    let parent_dir = tmp.path().join("parent_ext");
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(
        parent_dir.join("manifest.json"),
        r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
    )
    .unwrap();

    // child_ext depends on parent_ext
    let child_dir = tmp.path().join("child_ext");
    std::fs::create_dir_all(&child_dir).unwrap();
    std::fs::write(
        child_dir.join("manifest.json"),
        r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    // parent 먼저 활성화
    mgr.enable("parent_ext").unwrap();
    // 이제 child 활성화 가능
    mgr.enable("child_ext").unwrap();
    assert!(mgr.is_enabled("child_ext"));
}

#[test]
fn test_disable_blocked_by_dependent() {
    let tmp = tempfile::tempdir().unwrap();

    let parent_dir = tmp.path().join("parent_ext");
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(
        parent_dir.join("manifest.json"),
        r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
    )
    .unwrap();

    let child_dir = tmp.path().join("child_ext");
    std::fs::create_dir_all(&child_dir).unwrap();
    std::fs::write(
        child_dir.join("manifest.json"),
        r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enable("parent_ext").unwrap();
    mgr.enable("child_ext").unwrap();

    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

    // parent를 비활성화하려 하면 child가 의존하므로 실패
    let result = mgr.disable("parent_ext", &no_instances);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("child_ext"));
}

#[test]
fn test_disable_blocked_by_instance_usage() {
    let tmp = tempfile::tempdir().unwrap();

    let ext_dir = tmp.path().join("docker");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"docker","name":"Docker","version":"1.0.0","instance_fields":{"docker_enabled":{"type":"boolean","default":false}}}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enable("docker").unwrap();

    // 인스턴스가 docker_enabled=true로 사용 중
    let mut ext_data = HashMap::new();
    ext_data.insert("docker_enabled".to_string(), Value::Bool(true));
    let instances: Vec<(&str, &HashMap<String, Value>)> =
        vec![("my_server", &ext_data)];

    let result = mgr.disable("docker", &instances);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("my_server"));
}

#[test]
fn test_unmount_blocked_by_dependent() {
    let tmp = tempfile::tempdir().unwrap();

    let parent_dir = tmp.path().join("parent_ext");
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(
        parent_dir.join("manifest.json"),
        r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
    )
    .unwrap();

    let child_dir = tmp.path().join("child_ext");
    std::fs::create_dir_all(&child_dir).unwrap();
    std::fs::write(
        child_dir.join("manifest.json"),
        r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
    )
    .unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enable("parent_ext").unwrap();
    mgr.enable("child_ext").unwrap();

    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
    let result = mgr.unmount("parent_ext", &no_instances);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("child_ext"));
}

// ═══════════════════════════════════════════════════════════════
//  추가 심층 테스트
// ═══════════════════════════════════════════════════════════════

/// 조건 평가 — 숫자 0 → false, 비제로 → true
#[test]
fn test_evaluate_condition_number_values() {
    let mut ext_data = HashMap::new();
    ext_data.insert("cpu_limit".to_string(), Value::Number(serde_json::Number::from(0)));
    assert!(!ExtensionManager::evaluate_condition(
        "instance.ext_data.cpu_limit", &ext_data
    ));

    ext_data.insert("cpu_limit".to_string(), Value::Number(serde_json::Number::from(4)));
    assert!(ExtensionManager::evaluate_condition(
        "instance.ext_data.cpu_limit", &ext_data
    ));
}

/// 조건 평가 — 빈 문자열 → false, 비빈 문자열 → true
#[test]
fn test_evaluate_condition_string_values() {
    let mut ext_data = HashMap::new();
    ext_data.insert("image".to_string(), Value::String("".to_string()));
    assert!(!ExtensionManager::evaluate_condition(
        "instance.ext_data.image", &ext_data
    ));

    ext_data.insert("image".to_string(), Value::String("cm2network/steamcmd".to_string()));
    assert!(ExtensionManager::evaluate_condition(
        "instance.ext_data.image", &ext_data
    ));
}

/// 매니페스트 — 풀 필드 역직렬화 (GUI, CLI, hooks, dependencies, i18n)
#[test]
fn test_manifest_full_fields_deserialization() {
    let json = json!({
        "id": "docker",
        "name": "Docker Isolation",
        "version": "2.0.0",
        "description": "Container isolation for game servers",
        "author": "saba-chan",
        "dependencies": ["steamcmd"],
        "python_modules": {
            "compose_manager": "compose_manager.py",
            "health_check": "health.py"
        },
        "hooks": {
            "server.pre_start": {
                "module": "compose_manager",
                "function": "pre_start",
                "condition": "instance.ext_data.docker_enabled"
            },
            "server.stop": {
                "module": "compose_manager",
                "function": "post_stop"
            }
        },
        "instance_fields": {
            "docker_enabled": { "type": "boolean", "default": false },
            "docker_image": { "type": "string" }
        },
        "gui": {
            "bundle": "docker-panel.js",
            "styles": "docker-panel.css",
            "slots": { "InstanceList.badge": "DockerBadge" }
        },
        "cli": {
            "slots": { "InstanceList.badge": {"text": "🐳"} }
        },
        "i18n_dir": "locales",
        "module_config_section": "docker"
    });

    let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
    assert_eq!(manifest.id, "docker");
    assert_eq!(manifest.version, "2.0.0");
    assert_eq!(manifest.author, "saba-chan");
    assert_eq!(manifest.dependencies.len(), 1);
    assert_eq!(manifest.dependencies.get("steamcmd").unwrap(), "*");
    assert_eq!(manifest.hooks.len(), 2);
    assert!(manifest.hooks.contains_key("server.pre_start"));
    assert!(manifest.hooks.contains_key("server.stop"));
    assert_eq!(manifest.python_modules.len(), 2);
    assert_eq!(manifest.instance_fields.len(), 2);
    assert!(manifest.gui.is_some());
    assert!(manifest.cli.is_some());
    assert_eq!(manifest.i18n_dir.as_deref(), Some("locales"));
    assert_eq!(manifest.module_config_section.as_deref(), Some("docker"));
}

/// 매니페스트 — 최소 필드만으로도 역직렬화 가능
#[test]
fn test_manifest_minimal_deserialization() {
    let json = json!({"id": "x", "name": "X", "version": "0.0.1"});
    let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
    assert_eq!(manifest.id, "x");
    assert!(manifest.hooks.is_empty());
    assert!(manifest.dependencies.is_empty());
    assert!(manifest.gui.is_none());
}

/// 잘못된 JSON으로 매니페스트 역직렬화 실패
#[test]
fn test_manifest_invalid_json() {
    let json_no_id = json!({"name": "NoID", "version": "0.1.0"});
    assert!(serde_json::from_value::<ExtensionManifest>(json_no_id).is_err());
}

/// 다이아몬드 의존성 — A→B, A→C, B→D, C→D
#[test]
fn test_diamond_dependency_enable_order() {
    let tmp = tempfile::tempdir().unwrap();

    let create_ext = |id: &str, deps: &[&str]| {
        let dir = tmp.path().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({
            "id": id,
            "name": id,
            "version": "0.1.0",
            "dependencies": deps
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    };

    create_ext("ext_d", &[]);
    create_ext("ext_b", &["ext_d"]);
    create_ext("ext_c", &["ext_d"]);
    create_ext("ext_a", &["ext_b", "ext_c"]);

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    // 순서대로 활성화해야 함
    assert!(mgr.enable("ext_a").is_err(), "A는 B, C 미활성 시 실패");
    mgr.enable("ext_d").unwrap();
    assert!(mgr.enable("ext_b").is_ok());
    assert!(mgr.enable("ext_a").is_err(), "A는 C 미활성 시 여전히 실패");
    assert!(mgr.enable("ext_c").is_ok());
    assert!(mgr.enable("ext_a").is_ok(), "A의 모든 의존성 충족");
}

/// 삭제 — 비활성화 후 디렉토리 삭제
#[test]
fn test_remove_extension_cleans_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("removable");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("manifest.json"),
        r#"{"id":"removable","name":"Remove Me","version":"0.1.0"}"#,
    ).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    assert_eq!(mgr.list().len(), 1);

    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
    mgr.remove("removable", &no_instances).unwrap();
    assert!(mgr.list().is_empty());
    assert!(!ext_dir.exists(), "Extension directory should be deleted");
}

/// 삭제 — 의존하는 익스텐션이 있으면 실패
#[test]
fn test_remove_blocked_by_active_dependent() {
    let tmp = tempfile::tempdir().unwrap();
    let parent_dir = tmp.path().join("parent");
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(parent_dir.join("manifest.json"),
        r#"{"id":"parent","name":"Parent","version":"0.1.0"}"#).unwrap();

    let child_dir = tmp.path().join("child");
    std::fs::create_dir_all(&child_dir).unwrap();
    std::fs::write(child_dir.join("manifest.json"),
        r#"{"id":"child","name":"Child","version":"0.1.0","dependencies":["parent"]}"#).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enable("parent").unwrap();
    mgr.enable("child").unwrap();

    let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
    let result = mgr.remove("parent", &no_instances);
    assert!(result.is_err(), "Cannot remove parent while child depends on it");
}

/// list() 결과 검증 — enabled 상태, hooks, instance_fields 정확히 반영
#[test]
fn test_list_reflects_extension_state() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("ext_a");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "ext_a",
        "name": "Extension A",
        "version": "1.2.3",
        "description": "Test extension",
        "author": "Tester",
        "hooks": { "server.pre_start": { "module": "m", "function": "f" } },
        "instance_fields": { "my_flag": { "type": "boolean", "default": false } }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let list = mgr.list();
    assert_eq!(list.len(), 1);
    let item = &list[0];
    assert_eq!(item.id, "ext_a");
    assert_eq!(item.version, "1.2.3");
    assert!(!item.enabled, "Initially disabled");
    assert_eq!(item.hooks, vec!["server.pre_start"]);
    assert!(item.instance_fields.contains_key("my_flag"));

    mgr.enable("ext_a").unwrap();
    let list = mgr.list();
    assert!(list[0].enabled, "Should be enabled after enable()");
}

/// hooks_for — 비활성 익스텐션의 hook은 반환되지 않아야 함
#[test]
fn test_hooks_for_only_returns_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("hook_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "hook_ext", "name": "Hook Ext", "version": "0.1.0",
        "hooks": { "server.pre_start": { "module": "m", "function": "f" } },
        "python_modules": { "m": "m.py" }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    // 비활성 → hooks_for 비어있음
    assert!(mgr.hooks_for("server.pre_start").is_empty());

    // 활성화 → hooks_for에 포함
    mgr.enable("hook_ext").unwrap();
    let hooks = mgr.hooks_for("server.pre_start");
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].0.manifest.id, "hook_ext");
    assert_eq!(hooks[0].1.function, "f");

    // 존재하지 않는 hook 이름
    assert!(mgr.hooks_for("nonexistent.hook").is_empty());
}

/// check_updates_against — 로컬 < 원격이면 업데이트 정보 반환
#[test]
fn test_check_updates_against() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("test_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"),
        r#"{"id":"test_ext","name":"Test","version":"1.0.0"}"#).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let remote = vec![RemoteExtensionInfo {
        id: "test_ext".to_string(),
        name: "Test".to_string(),
        version: "2.0.0".to_string(),
        description: String::new(),
        author: String::new(),
        download_url: "https://example.com/test_ext.zip".to_string(),
        sha256: None,
        min_app_version: None,
        incompatible: None,
        asset: None,
        install_dir: None,
        dependencies: HashMap::new(),
        has_gui: false,
        has_i18n: false,
        tags: vec![],
        homepage: None,
    }];

    let updates = mgr.check_updates_against(&remote);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, "test_ext");
    assert_eq!(updates[0].installed_version, "1.0.0");
    assert_eq!(updates[0].latest_version, "2.0.0");
}

/// check_updates_against — 이미 최신이면 빈 목록
#[test]
fn test_check_updates_already_latest() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("test_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"),
        r#"{"id":"test_ext","name":"Test","version":"2.0.0"}"#).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let remote = vec![RemoteExtensionInfo {
        id: "test_ext".to_string(),
        name: "Test".to_string(),
        version: "2.0.0".to_string(),
        description: String::new(),
        author: String::new(),
        download_url: "https://example.com/test_ext.zip".to_string(),
        sha256: None,
        min_app_version: None,
        incompatible: None,
        asset: None,
        install_dir: None,
        dependencies: HashMap::new(),
        has_gui: false,
        has_i18n: false,
        tags: vec![],
        homepage: None,
    }];

    let updates = mgr.check_updates_against(&remote);
    assert!(updates.is_empty(), "Same version should not be an update");
}

/// 영속화 — enable → new_isolated 재생성 → enabled 상태 유지
#[test]
fn test_state_persistence_across_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("persistent_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"),
        r#"{"id":"persistent_ext","name":"Persistent","version":"0.1.0"}"#).unwrap();

    // 1차: enable
    {
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("persistent_ext").unwrap();
        assert!(mgr.is_enabled("persistent_ext"));
    }

    // 2차: 재생성 → 상태 복원
    {
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        assert!(mgr.is_enabled("persistent_ext"), "Enabled state must persist across reload");
    }
}

/// zip 자동 추출 테스트
#[test]
fn test_discover_extracts_zip_extension() {
    let tmp = tempfile::tempdir().unwrap();

    // manifest.json이 들어있는 zip 파일 생성
    let zip_path = tmp.path().join("zip_ext.zip");
    let manifest_content = r#"{"id":"zip_ext","name":"Zip Extension","version":"0.1.0"}"#;

    let file = std::fs::File::create(&zip_path).unwrap();
    let mut zip_writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("manifest.json", options).unwrap();
    std::io::Write::write_all(&mut zip_writer, manifest_content.as_bytes()).unwrap();
    zip_writer.finish().unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    let found = mgr.discover().unwrap();
    assert!(
        found.contains(&"zip_ext".to_string()),
        "Zip extension should be auto-extracted and discovered: {:?}", found
    );

    // zip 파일이 삭제되었어야 함
    assert!(!zip_path.exists(), "Zip file should be removed after extraction");
}

// ── 컴포넌트 버전 의존성(dependencies) 테스트 ──

#[test]
fn test_enable_with_component_version_satisfied() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("my_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "my_ext",
        "name": "My Extension",
        "version": "1.0.0",
        "dependencies": { "saba-core": ">=0.3.0" }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let mut versions = HashMap::new();
    versions.insert("saba-core".to_string(), "0.5.0".to_string());

    let result = mgr.enable_with_versions("my_ext", &versions);
    assert!(result.is_ok(), "Should enable when component version is satisfied");
}

#[test]
fn test_enable_with_component_version_too_low() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("my_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "my_ext",
        "name": "My Extension",
        "version": "1.0.0",
        "dependencies": { "saba-core": ">=0.3.0" }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let mut versions = HashMap::new();
    versions.insert("saba-core".to_string(), "0.2.0".to_string());

    let result = mgr.enable_with_versions("my_ext", &versions);
    assert!(result.is_err(), "Should fail when component version is too low");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("saba-core"), "Error should mention the component");
    assert!(err_msg.contains("0.2.0"), "Error should mention installed version");
}

#[test]
fn test_enable_with_component_not_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("my_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "my_ext",
        "name": "My Extension",
        "version": "1.0.0",
        "dependencies": { "gui": ">=0.2.0" }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    // No versions provided → gui not installed
    let result = mgr.enable_with_versions("my_ext", &HashMap::new());
    assert!(result.is_err(), "Should fail when required component is not installed");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("gui"));
    assert!(err_msg.contains("not installed"));
}

#[test]
fn test_enable_without_versions_skips_requires_check() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("my_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "my_ext",
        "name": "My Extension",
        "version": "1.0.0",
        "dependencies": { "saba-core": ">=99.0.0" }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    // enable() without versions → no installed_versions → requires check fails
    let result = mgr.enable("my_ext");
    assert!(result.is_err(), "enable() without version info should fail if requires is set");
}

#[test]
fn test_enable_cross_type_requires() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("advanced_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "advanced_ext",
        "name": "Advanced",
        "version": "1.0.0",
        "dependencies": {
            "saba-core": ">=0.3.0",
            "gui": ">=0.2.0",
            "cli": ">=0.1.0"
        }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let mut versions = HashMap::new();
    versions.insert("saba-core".to_string(), "0.5.0".to_string());
    versions.insert("gui".to_string(), "0.3.0".to_string());
    versions.insert("cli".to_string(), "0.1.0".to_string());

    let result = mgr.enable_with_versions("advanced_ext", &versions);
    assert!(result.is_ok(), "All cross-type component deps satisfied");
}

#[test]
fn test_enable_requires_plus_extension_dependency() {
    let tmp = tempfile::tempdir().unwrap();

    // parent extension (no requires)
    let parent_dir = tmp.path().join("parent_ext");
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(parent_dir.join("manifest.json"), json!({
        "id": "parent_ext", "name": "Parent", "version": "0.1.0"
    }).to_string()).unwrap();

    // child extension — depends on parent_ext + requires saba-core >=0.3.0
    let child_dir = tmp.path().join("child_ext");
    std::fs::create_dir_all(&child_dir).unwrap();
    std::fs::write(child_dir.join("manifest.json"), json!({
        "id": "child_ext",
        "name": "Child",
        "version": "1.0.0",
        "dependencies": {
            "parent_ext": "*",
            "saba-core": ">=0.3.0"
        }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let mut versions = HashMap::new();
    versions.insert("saba-core".to_string(), "0.5.0".to_string());

    // parent not enabled → child fails on ext dependency
    let result = mgr.enable_with_versions("child_ext", &versions);
    assert!(result.is_err(), "Should fail: parent not enabled");

    // enable parent, then child should succeed
    mgr.enable_with_versions("parent_ext", &versions).unwrap();
    let result = mgr.enable_with_versions("child_ext", &versions);
    assert!(result.is_ok(), "Both ext dep and component dep satisfied");
}

#[test]
fn test_manifest_dependencies_field_deserialization() {
    // 맵 형식
    let manifest: ExtensionManifest = serde_json::from_value(json!({
        "id": "test_ext",
        "name": "Test",
        "version": "1.0.0",
        "dependencies": {
            "saba-core": ">=0.3.0",
            "gui": ">=0.2.0",
            "docker": ">=1.0.0"
        }
    })).unwrap();

    assert_eq!(manifest.dependencies.len(), 3);
    assert_eq!(manifest.dependencies.get("saba-core").unwrap(), ">=0.3.0");
    assert_eq!(manifest.dependencies.get("gui").unwrap(), ">=0.2.0");
    assert_eq!(manifest.dependencies.get("docker").unwrap(), ">=1.0.0");

    // 배열 형식 (하위 호환)
    let manifest2: ExtensionManifest = serde_json::from_value(json!({
        "id": "test_ext",
        "name": "Test",
        "version": "1.0.0",
        "dependencies": ["steamcmd", "ue4-ini"]
    })).unwrap();

    assert_eq!(manifest2.dependencies.len(), 2);
    assert_eq!(manifest2.dependencies.get("steamcmd").unwrap(), "*");
    assert_eq!(manifest2.dependencies.get("ue4-ini").unwrap(), "*");
}

#[test]
fn test_manifest_dependencies_empty_by_default() {
    let manifest: ExtensionManifest = serde_json::from_value(json!({
        "id": "test_ext",
        "name": "Test",
        "version": "1.0.0"
    })).unwrap();

    assert!(manifest.dependencies.is_empty(), "dependencies should default to empty");
}

#[test]
fn test_list_includes_dependencies() {
    let tmp = tempfile::tempdir().unwrap();
    let ext_dir = tmp.path().join("ext_req");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("manifest.json"), json!({
        "id": "ext_req",
        "name": "Ext with Dependencies",
        "version": "1.0.0",
        "dependencies": { "saba-core": ">=0.5.0" }
    }).to_string()).unwrap();

    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let list = mgr.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].dependencies.get("saba-core").unwrap(), ">=0.5.0");
}

#[test]
fn test_component_version_unsatisfied_error() {
    let err = ExtensionError::component_version_unsatisfied(
        "my_ext", "saba-core", ">=0.5.0", Some("0.3.0")
    );
    assert_eq!(err.error_code, "component_version_unsatisfied");
    assert!(err.message.contains("saba-core"));
    assert!(err.message.contains(">=0.5.0"));
    assert!(err.message.contains("0.3.0"));
    assert_eq!(err.related, vec!["saba-core", ">=0.5.0"]);
}

#[test]
fn test_component_version_unsatisfied_not_installed() {
    let err = ExtensionError::component_version_unsatisfied(
        "my_ext", "gui", ">=0.2.0", None
    );
    assert!(err.message.contains("not installed"));
}

// ═══════════════════════════════════════════════════════════════
//  원격 manifest.json 파싱 호환성 테스트
// ═══════════════════════════════════════════════════════════════



/// 실제 manifest.json (로컬 설치용) 파싱 호환성 테스트
/// 릴리즈 manifest의 `ext-*` 컴포넌트가 익스텐션 목록이 된다 (U9e)
#[test]
fn release_manifest_ext_components_become_the_catalog() {
    let manifest: saba_chan_updater_lib::relay::ReleaseManifest = serde_json::from_value(serde_json::json!({
        "version": "0.2.0", "channel": "stable", "platform": "windows-x64",
        "components": {
            "saba-core": {"version": "0.2.0", "asset": "c.zip", "sha256": "a", "url": "https://r2/c.zip"},
            "module-palworld": {"version": "0.1.0", "asset": "module-palworld.zip", "sha256": "b", "url": "https://r2/m.zip"},
            "ext-ue4-ini": {
                "version": "0.1.2", "asset": "ext-ue4-ini.zip", "sha256": "c", "url": "https://r2/e.zip",
                "install_dir": "extensions/ue4_ini",
                "meta": {"name": "UE4 INI", "dependencies": ["steamcmd"], "min_app_version": "0.1.0"}
            }
        }
    })).unwrap();
    let list = ExtensionManager::from_release(&manifest);
    assert_eq!(list.len(), 1);
    let ext = &list[0];
    assert_eq!((ext.id.as_str(), ext.name.as_str(), ext.version.as_str()), ("ue4-ini", "UE4 INI", "0.1.2"));
    assert_eq!(ext.download_url, "https://r2/e.zip");
    assert_eq!(ext.sha256.as_deref(), Some("c"));
    assert_eq!(ext.install_dir.as_deref(), Some("extensions/ue4_ini"));
    assert!(ext.dependencies.contains_key("steamcmd"));
}

#[test]
fn test_parse_docker_manifest() {
    let json = r#"{
          "id": "docker",
          "name": "Docker Isolation",
          "version": "0.1.0",
          "description": "Docker 컨테이너를 사용한 게임 서버 격리 실행",
          "author": "saba-chan",
          "min_app_version": "0.1.0",
          "dependencies": { "steamcmd": ">=0.1.0" },
          "python_modules": {
            "docker_engine": "docker_engine.py",
            "compose_manager": "compose_manager.py"
          },
          "hooks": {
            "daemon.startup": {
              "module": "docker_engine",
              "function": "ensure",
              "condition": null
            },
            "server.pre_start": {
              "module": "compose_manager",
              "function": "start",
              "condition": "instance.ext_data.docker_enabled"
            },
            "server.post_create": {
              "module": "compose_manager",
              "function": "provision",
              "condition": "instance.ext_data.docker_enabled",
              "async": true
            }
          },
          "gui": {
            "bundle": "gui/dist/docker.umd.js",
            "slots": {
              "ServerCard.badge": "DockerBadge",
              "ServerSettings.tab": "DockerTab"
            }
          },
          "cli": {
            "slots": {
              "InstanceList.badge": { "text": "🐳", "condition": "instance.ext_data.docker_enabled" }
            }
          },
          "module_config_section": "docker",
          "instance_fields": {
            "docker_enabled": { "type": "boolean", "default": false },
            "docker_cpu_limit": { "type": "number", "optional": true },
            "docker_memory_limit": { "type": "string", "optional": true }
          },
          "i18n_dir": "i18n/"
        }"#;

    let manifest: ExtensionManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.id, "docker");
    assert_eq!(manifest.dependencies.get("steamcmd").unwrap(), ">=0.1.0");
    assert_eq!(manifest.python_modules.len(), 2);
    assert_eq!(manifest.hooks.len(), 3);

    // condition: null → None
    let startup = &manifest.hooks["daemon.startup"];
    assert!(startup.condition.is_none());

    // condition: "..." → Some(...)
    let pre_start = &manifest.hooks["server.pre_start"];
    assert_eq!(pre_start.condition.as_deref(), Some("instance.ext_data.docker_enabled"));

    // async: true → is_async: Some(true)
    assert!(manifest.hooks.contains_key("server.post_create"));

    // gui
    let gui = manifest.gui.unwrap();
    assert_eq!(gui.bundle.as_deref(), Some("gui/dist/docker.umd.js"));
    assert_eq!(gui.slots.len(), 2);

    // cli
    let cli = manifest.cli.unwrap();
    assert_eq!(cli.slots.len(), 1);

    // module_config_section
    assert_eq!(manifest.module_config_section.as_deref(), Some("docker"));

    // instance_fields
    assert_eq!(manifest.instance_fields.len(), 3);
    assert_eq!(manifest.instance_fields["docker_enabled"].field_type, "boolean");

    // i18n_dir
    assert_eq!(manifest.i18n_dir.as_deref(), Some("i18n/"));
}

/// gui.slots: slot id → 표시 정보 객체 (탭 이름·아이콘). 옛 문자열 모양도 받는다. GUI가 slot 표와 대조한다
#[test]
fn gui_slots_take_display_objects() {
    let json = r#"{
          "id": "pet", "name": "Pet", "version": "0.1.0",
          "gui": { "bundle": "gui/dist/pet.umd.js", "slots": {
            "App.page": { "label": "page.title", "icon": "globe" },
            "ServerCard.badge": "Badge"
          } }
        }"#;
    let manifest: ExtensionManifest = serde_json::from_str(json).unwrap();
    let gui = manifest.gui.unwrap();
    assert_eq!(gui.slots["App.page"]["label"], "page.title");
    assert_eq!(gui.slots["ServerCard.badge"], "Badge");
}

/// music manifest — gui.builtin, dependencies: [], i18n_dir: null
#[test]
fn test_parse_music_manifest() {
    let json = r#"{
          "id": "music",
          "name": "Music Bot",
          "version": "0.1.0",
          "dependencies": [],
          "python_modules": { "music_deps": "music_deps.py" },
          "hooks": {
            "daemon.startup": { "module": "music_deps", "function": "check_dependencies", "condition": null }
          },
          "gui": { "builtin": true, "slots": {} },
          "instance_fields": {},
          "i18n_dir": null
        }"#;

    let manifest: ExtensionManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.id, "music");
    assert!(manifest.dependencies.is_empty()); // [] → empty HashMap
    let gui = manifest.gui.unwrap();
    assert_eq!(gui.builtin, Some(true));
    assert!(gui.slots.is_empty());
    assert!(manifest.i18n_dir.is_none()); // null → None
}

// ── 모듈 설치 때 필요한 익스텐션 켜기 ──

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn module_install_enables_required_extensions_and_their_dependencies_first() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "base", json!({"id": "base", "name": "B", "version": "0.1.0"}));
    write_ext(tmp.path(), "steamcmd", json!({"id": "steamcmd", "name": "S", "version": "0.1.0", "dependencies": ["base"]}));
    write_ext(tmp.path(), "ue4_ini", json!({"id": "ue4-ini", "name": "U", "version": "0.1.0"}));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();
    mgr.enable("ue4-ini").unwrap();

    let (enabled, failed) = mgr.enable_for_module(&ids(&["steamcmd", "ue4-ini"]), &[], &[]);

    assert!(failed.is_empty(), "{:?}", failed.iter().map(|(id, e)| format!("{id}: {e}")).collect::<Vec<_>>());
    assert_eq!(enabled, ids(&["base", "steamcmd"]), "already-on ue4-ini is not reported");
    assert!(mgr.is_enabled("base") && mgr.is_enabled("steamcmd") && mgr.is_enabled("ue4-ini"));
    let reloaded = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    assert!(reloaded.is_enabled("steamcmd"), "saved to the config file");
}

#[test]
fn module_update_keeps_an_extension_the_user_turned_off_but_enables_new_needs() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "steamcmd", json!({"id": "steamcmd", "name": "S", "version": "0.1.0"}));
    write_ext(tmp.path(), "ue4_ini", json!({"id": "ue4-ini", "name": "U", "version": "0.1.0"}));
    write_ext(tmp.path(), "rcon", json!({"id": "rcon", "name": "R", "version": "0.1.0"}));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    // 예전 판도 steamcmd·ue4-ini를 요구했다. ue4-ini는 지웠다가 이번에 다시 받았고, rcon은 새로 필요해졌다
    let (enabled, failed) =
        mgr.enable_for_module(&ids(&["steamcmd", "ue4-ini", "rcon"]), &ids(&["steamcmd", "ue4-ini"]), &ids(&["ue4-ini"]));

    assert!(failed.is_empty());
    assert_eq!(enabled, ids(&["ue4-ini", "rcon"]));
    assert!(!mgr.is_enabled("steamcmd"), "the user had turned it off");
}

#[test]
fn module_install_reports_extensions_it_cannot_enable() {
    let tmp = tempfile::tempdir().unwrap();
    write_ext(tmp.path(), "future", json!({"id": "future", "name": "F", "version": "0.1.0", "min_app_version": "999.0.0"}));
    let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
    mgr.discover().unwrap();

    let (enabled, failed) = mgr.enable_for_module(&ids(&["future", "missing"]), &[], &[]);

    assert!(enabled.is_empty());
    let codes: Vec<(String, String)> = failed
        .iter()
        .map(|(id, e)| (id.clone(), e.downcast_ref::<ExtensionError>().unwrap().error_code.clone()))
        .collect();
    assert_eq!(codes, vec![("future".into(), "app_version_unsatisfied".into()), ("missing".into(), "not_found".into())]);
}
