use super::*;
use super::queries::with_setting_issues;
use serde_json::json;

/// 켜고 끈 쪽: 디스코드에서 시켰으면 그 디스코드 서버 이름도 싣는다 (id는 싣지 않는다)
#[test]
fn origin_fields_carry_the_discord_server_name_but_no_ids() {
    let origin = crate::audit::Origin {
        user: Some("111".into()),
        guild: Some("222".into()),
        user_name: Some("주인님".into()),
        guild_name: Some("사바쨩 서버".into()),
        ..crate::audit::Origin::new("discord")
    };
    assert_eq!(
        Value::Object(origin_fields(&origin)),
        json!({"source": "discord", "userName": "주인님", "guildName": "사바쨩 서버"})
    );
    assert_eq!(Value::Object(origin_fields(&crate::audit::Origin::new("gui"))), json!({"source": "gui"}));
}

/// D3·A-09: 0.1.14-beta.1까지 instance.json에 있던 REST 접속값은 모듈이 선언한 설정으로 옮겨 저장한다.
/// 모듈을 아직 모르는 인스턴스의 값은 파일에 그대로 둔다 (모듈이 생긴 뒤 옮긴다)
#[tokio::test]
async fn rest_values_of_old_instance_files_move_to_the_declared_settings_on_load() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("modules").join("pal");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::write(
        dir.join("module.toml"),
        "[module]\nschema_version = 1\nname = \"pal\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"rest\"]\n\n\
             [protocols.rest]\nhost_setting = \"api_host\"\nport_setting = \"api_port\"\npassword_setting = \"admin_pw\"\nusername = \"admin\"\n\n\
             [install]\nmethod = \"steamcmd\"\napp_id = 2394010\n\n\
             [[settings.fields]]\nname = \"api_port\"\ntype = \"number\"\nport = true\nlabel = \"api\"\ndefault = 8212\n\n\
             [[settings.fields]]\nname = \"admin_pw\"\ntype = \"password\"\nlabel = \"pw\"\n",
    )
    .unwrap();
    let instances = tmp.path().join("instances");
    for (id, module) in [("old-pal", "pal"), ("old-gone", "gone")] {
        std::fs::create_dir_all(instances.join(id)).unwrap();
        let install_copies = json!({ "install_method_steamcmd": true, "steamcmd_app_id": 111, "steamcmd_anonymous": true, "steamcmd_beta": "unstable" });
        let file = json!({
            "id": id, "name": id, "module_name": module, "port": 8211, "extension_data": install_copies,
            "rest_host": "127.0.0.1", "rest_port": 8242, "rest_username": "admin", "rest_password": "pw",
        });
        std::fs::write(instances.join(id).join("instance.json"), file.to_string()).unwrap();
        std::fs::write(instances.join(id).join("settings.json"), json!({ "api_port": 8212 }).to_string()).unwrap();
    }
    let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), instances.to_str().unwrap());
    sup.initialize().await.unwrap();

    let saved = |id: &str, file: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(instances.join(id).join(file)).unwrap()).unwrap() };
    let moved = sup.instance_store.get("old-pal").unwrap();
    assert_eq!(moved.module_settings, saved("old-pal", "settings.json").as_object().unwrap().clone().into_iter().collect());
    assert_eq!(saved("old-pal", "settings.json"), json!({ "api_host": "127.0.0.1", "api_port": 8242, "admin_pw": "pw" }), "the port the daemon used wins over a stale setting");
    assert!(saved("old-pal", "instance.json").get("rest_port").is_none());
    // A-09: 인스턴스에 복사해 두던 `[install]` 값은 지우고, 설치 방식 표시와 사용자가 고른 브랜치만 남긴다
    assert_eq!(saved("old-pal", "instance.json")["extension_data"], json!({ "install_method_steamcmd": true, "steamcmd_beta": "unstable" }));
    let module = sup.module_loader.get_module("pal").unwrap();
    assert_eq!((module.metadata.rest_host(moved), module.metadata.rest_port(moved), module.metadata.rest_password(moved)), (Some("127.0.0.1".into()), Some(8242), Some("pw")));

    assert_eq!(saved("old-gone", "instance.json")["rest_port"], 8242, "kept until its module is known");
    assert_eq!(saved("old-gone", "settings.json"), json!({ "api_port": 8212 }));
}

/// A-06: 모듈에 `status`가 없으면 데몬이 추적하는 PID가 살아 있는지로 답한다 (PID만 보던 모듈 status 세 개를 대신한다)
#[tokio::test]
async fn a_module_without_status_is_answered_from_the_tracked_pid() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
    std::fs::write(
        dir.join("lifecycle.py"),
        "import json, sys\nsys.stdin.read()\nok = sys.argv[1] == 'functions'\n\
             print(json.dumps({'success': True, 'functions': ['stop']} if ok else {'success': False, 'message': 'Unknown function: ' + sys.argv[1]}))\n",
    )
    .unwrap();
    crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
    let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    sup.initialize().await.unwrap();
    let inst = crate::instance::ServerInstance::new("srv", "game");
    let id = inst.id.clone();
    sup.instance_store.add(inst).unwrap();
    let sup = tokio::sync::RwLock::new(sup);

    let stopped = Supervisor::get_server_status(&sup, &id, "game").await.unwrap();
    assert_eq!((stopped["status"].as_str(), stopped["pid"].as_u64()), (Some("stopped"), None), "{stopped}");
    sup.read().await.tracker.track(&id, std::process::id()).unwrap();
    let running = Supervisor::get_server_status(&sup, &id, "game").await.unwrap();
    assert_eq!((running["status"].as_str(), running["pid"].as_u64()), (Some("running"), Some(std::process::id() as u64)), "{running}");
    assert!(running["start_time"].is_number(), "{running}");
    // 추적표에 남았지만 이미 끝난 프로세스는 꺼진 것이다
    sup.read().await.tracker.track(&id, 4_000_001).unwrap();
    let gone = Supervisor::get_server_status(&sup, &id, "game").await.unwrap();
    assert_eq!(gone["status"], "stopped", "{gone}");
}

#[tokio::test]
async fn test_supervisor_initialization() {
    let supervisor = Supervisor::new("./modules");
    
    // Supervisor가 정상적으로 생성되었는지 확인
    assert_eq!(supervisor.instance_store.list().len(), 0);
}

#[tokio::test]
async fn test_module_discovery() {
    let supervisor = Supervisor::new("./modules");
    
    // 모듈 발견 (modules 디렉토리가 없어도 에러 없이 빈 리스트 반환)
    let modules = supervisor.list_modules();
    assert!(modules.is_ok());
}

#[tokio::test]
async fn test_refresh_modules() {
    let supervisor = Supervisor::new("./modules");
    
    // 첫 번째 발견
    let _ = supervisor.list_modules();
    
    // 캐시 무효화 및 재발견
    let refreshed = supervisor.refresh_modules();
    assert!(refreshed.is_ok());
}

#[tokio::test]
async fn test_execute_command_error_handling() {
    let supervisor = tokio::sync::RwLock::new(Supervisor::new("./modules"));
    
    // 존재하지 않는 인스턴스에 명령어 실행 시도
    let result = Supervisor::execute_command(
            &supervisor,
            "nonexistent-instance",
            "test-module",
            "test-command",
            json!({})
        )
        .await;
    
    // 에러가 발생해야 함
    assert!(result.is_err());
    
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("Instance not found"));
}

/// 인스턴스 함수 호출은 추적 중인 서버 PID를 싣는다: 격리 실행의 루프백 중계가 그 서버의 포트로만 잇는다
#[tokio::test]
async fn an_instance_call_carries_the_tracked_server_pid() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
    let mut sup = Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    let inst = crate::instance::ServerInstance::new("srv", "game");
    let id = inst.id.clone();
    sup.instance_store.add(inst).unwrap();
    assert_eq!(sup.instance_call(&id, "status").unwrap().server(), None);
    sup.tracker.track(&id, 4_000_001).unwrap();
    let call = sup.instance_call(&id, "status").unwrap();
    assert_eq!(call.server(), Some(4_000_001));
    assert_eq!(call.config()["running"], true);
}

/// 데몬이 모듈에 넘기는 값은 범용 키뿐이다: 실행 파일은 `server_executable`, 폴더는 `working_dir` (MK7).
/// 설치된 버전 확인도 시작·점검과 같은 값을 받는다
#[tokio::test]
async fn module_calls_carry_only_generic_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True, 'config': json.load(sys.stdin)}))\n").unwrap();
    std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
    crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
    let mut sup = Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.module_loader.discover_modules().unwrap();
    let server_dir = tmp.path().join("server");
    let mut inst = crate::instance::ServerInstance::new("srv", "game");
    inst.working_dir = Some(server_dir.to_string_lossy().into_owned());
    inst.executable_path = Some(server_dir.join("paper-1.21.jar").to_string_lossy().into_owned());
    let id = inst.id.clone();
    sup.instance_store.add(inst).unwrap();
    let validate = sup.instance_call(&id, "validate").unwrap();
    // 켜기 전 점검도 시작과 같이 인스턴스 폴더를 안다 (그 안에 받은 포터블 런타임을 본다)
    assert_eq!(validate.config()["instance_dir"], json!(sup.instance_store.instance_dir(&id).to_string_lossy()));
    let sup = tokio::sync::RwLock::new(sup);
    let installed = Supervisor::get_installed_version(&sup, "game", &id).await.unwrap();
    for config in [validate.config(), &installed["config"]] {
        assert_eq!(config["server_executable"], json!(server_dir.join("paper-1.21.jar").to_string_lossy()));
        assert_eq!(config["working_dir"], json!(server_dir.to_string_lossy()));
        for key in ["server_jar", "jar_name", "install_dir"] {
            assert!(config.get(key).is_none(), "{key} in {config}");
        }
    }
}

/// D-S11: 모듈 함수(설치 버전 확인 등)도 시작과 같은 실행 파일을 받는다. 옛 인스턴스의 module_settings 경로,
/// 작업 폴더와 모듈의 server_executable로 정해지는 경로를 시작만 보고 이쪽은 보지 않았다
#[test]
fn module_calls_get_the_executable_the_start_uses() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nserver_executable = \"server.jar\"\n").unwrap();
    let mut sup = Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.module_loader.discover_modules().unwrap();
    let server_dir = tmp.path().join("server");
    let mut legacy = crate::instance::ServerInstance::new("old", "game");
    legacy.working_dir = Some(server_dir.to_string_lossy().into_owned());
    legacy.module_settings.insert("executable_path".into(), json!(server_dir.join("custom.jar").to_string_lossy()));
    let mut plain = crate::instance::ServerInstance::new("new", "game");
    plain.working_dir = Some(server_dir.to_string_lossy().into_owned());
    for (inst, jar) in [(legacy, "custom.jar"), (plain, "server.jar")] {
        let id = inst.id.clone();
        let expected = sup.resolve_executable(&inst, "game");
        sup.instance_store.add(inst).unwrap();
        assert_eq!(expected.as_deref(), Some(server_dir.join(jar).to_string_lossy().as_ref()));
        let call = sup.instance_call(&id, "get_installed_version").unwrap();
        assert_eq!(call.config()["server_executable"], json!(expected), "{jar}");
    }
}

#[test]
fn a_module_failure_says_why_instead_of_unknown_error() {
    assert_eq!(module_failure(&json!({"success": false, "message": "jar missing"}), "mc", "get_launch_command").to_string(), "jar missing");
    assert_eq!(module_failure(&json!({"success": false, "error": "bad port"}), "mc", "get_launch_command").to_string(), "bad port");
    let legacy = module_failure(&json!({"command": "sh", "args": []}), "e2e-mock", "get_launch_command").to_string();
    assert!(legacy.contains("e2e-mock") && legacy.contains("command") && legacy.contains("args"), "{}", legacy);
    assert!(module_failure(&Value::Null, "m", "f").to_string().contains("nothing"));
}

#[test]
fn a_module_failure_keeps_its_fields_through_supervisor_errors() {
    let value = json!({ "success": false, "error_code": "module_path_outside", "message": "outside root", "path": "bad", "module": "game", "function": "get_launch_command" });
    let error = anyhow::anyhow!(module_failure(&value, "game", "get_launch_command"));
    assert_eq!(crate::plugin::error_response(&error), value);
}

#[test]
fn module_failure_reads_as_the_module_message() {
    let value = json!({ "success": false, "error": "cannot start", "message": "jar missing", "module": "game", "function": "get_launch_command" });
    let error = module_failure(&value, "game", "get_launch_command");
    assert_eq!(error.to_string(), "jar missing");
    assert_eq!(crate::plugin::error_response(&error), value);
}

/// 서버 점검은 저장된 설정이 선언한 타입·범위·선택지에 맞는지도 본다 (숨긴 칸·빈 값은 빼고). 사용자 지정 2026-11-08
#[test]
fn the_server_check_reports_settings_that_do_not_match_their_declared_type() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::write(
        dir.join("module.toml"),
        "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"players\"\ntype = \"number\"\nmin = 1\nmax = 10\n\n\
             [[settings.fields]]\nname = \"mode\"\ntype = \"select\"\noptions = [\"a\", \"b\"]\n\n\
             [[settings.fields]]\nname = \"hard\"\ntype = \"boolean\"\n\n\
             [[settings.fields]]\nname = \"note\"\ntype = \"number\"\nvisible_when = { field = \"hard\", equals = true }\n\n\
             [[settings.fields]]\nname = \"admin_pass\"\ntype = \"password\"\npattern = \"[0-9]+\"\n\n\
             [[settings.fields]]\nname = \"api_token\"\ntype = \"number\"\n",
    )
    .unwrap();
    let mut sup = Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    let mut inst = crate::instance::ServerInstance::new("g", "game");
    inst.id = "g".into();
    for (k, v) in [
        ("players", json!(50)),
        ("mode", json!("minecraft\\:normal")),
        ("hard", json!(false)),
        ("note", json!("x")),
        ("admin_pass", json!("hunter2-secret")),
        ("api_token", json!("tok-secret")),
    ] {
        inst.module_settings.insert(k.into(), v);
    }
    sup.instance_store.add(inst).unwrap();
    let issues = sup.setting_issues("g");
    let fields: Vec<&str> = issues.iter().filter_map(|i| i["field"].as_str()).collect();
    assert_eq!(fields, ["players", "mode", "admin_pass", "api_token"], "note is hidden, hard is fine: {issues:?}");
    // 리뷰 P2 (2026-11-08): 비밀 칸(password 타입·이름에 pass·token 등)은 값과 원문 메시지를 싣지 않는다 (원격 웹도 점검을 부른다)
    let text = serde_json::to_string(&issues).unwrap();
    assert!(!text.contains("hunter2") && !text.contains("tok-secret"), "{text}");
    assert!(issues[2].get("value").is_none() && issues[3].get("value").is_none());
    assert_eq!((issues[0]["code"].as_str(), issues[0]["error_type"].as_str()), (Some("setting_invalid"), Some("out_of_range")));
    assert_eq!(issues[1]["error_type"], "invalid_option");
    // 모듈 결과에 붙인다 (success는 그대로)
    let merged = with_setting_issues(json!({ "success": true, "issues": [{ "code": "X" }] }), issues.clone());
    assert_eq!((merged["success"].clone(), merged["issues"].as_array().unwrap().len()), (json!(true), 5));
    assert_eq!(with_setting_issues(json!({ "success": true }), issues)["issues"].as_array().unwrap().len(), 4);
}

/// 06 §2: 1분 접속자 수 갱신에 사용자 설정 `players_detail`이 따라가지 않는다
#[test]
fn a_module_setting_never_stands_in_for_a_daemon_call_key() {
    let mut inst = crate::instance::ServerInstance::new("mc", "minecraft");
    inst.module_settings.insert("players_detail".into(), json!(true));
    inst.module_settings.insert("stop_call_limit".into(), json!(9999));
    let mut cfg = serde_json::Map::new();
    Supervisor::insert_instance_fields(&mut cfg, &inst, std::path::Path::new("/data/instances/mc"));
    assert!(cfg.get("players_detail").is_none() && cfg.get("stop_call_limit").is_none(), "{cfg:?}");
}

// PC [A]: 미확인 좀보이드의 데이터 폴더(<인스턴스>/zomboid)가 서버 폴더 옆이라 격리 실행이 쓰지 못했다.
// 인스턴스 폴더 안을 가리키는 모듈 설정 폴더만 쓰기 대상으로 싣고, 데몬 파일(instance.json 등)과 인스턴스
// 폴더 자체는 싣지 않는다. 모듈 설정에서 온 폴더 키(리뷰 312: import_settings로 아무 경로나)는 config에 넣지 않는다
#[test]
fn modules_get_write_folders_only_inside_their_instance() {
    let root = std::path::Path::new("/data/instances/pz");
    let at = |rel: &str| root.join(rel).to_string_lossy().into_owned();
    let mut inst = crate::instance::ServerInstance::new("pz", "zomboid");
    for (key, value) in [
        ("cache_dir", at("zomboid")),
        ("save_dir", at("zomboid/Saves")),
        ("meta", at("instance.json")),
        ("runner", at("run/x")),
        // Windows는 이름의 대소문자, 끝의 점·공백, 짧은 이름(8.3), 스트림 표기를 같은 항목으로 연다
        ("runner_upper", at("RUN/x")),
        ("meta_dot", at("Instance.json.")),
        ("logs_space", at("logs /x")),
        ("short", at("SETTIN~1.JSO")),
        ("stream", at("run:x")),
        ("long_s", at("ſettings.json.tmp")),
        ("atomic_temp", at(".settings.json.42-0.TMP")),
        ("korean", at("세이브")),
        ("up", at("../other")),
        ("whole", root.to_string_lossy().into_owned()),
        ("outside", "/home/user/Documents".to_string()),
        ("install_dir", "/home/user".to_string()),
        ("working_dir", "/home/user".to_string()),
        ("instance_dir", "/home/user".to_string()),
    ] {
        inst.module_settings.insert(key.into(), json!(value));
    }
    let mut cfg = serde_json::Map::new();
    Supervisor::insert_instance_fields(&mut cfg, &inst, root);
    assert_eq!(cfg["instance_data_dirs"], json!([at("zomboid"), at("세이브")]));
    for key in ["install_dir", "working_dir"] {
        assert!(cfg.get(key).is_none(), "{key} from module settings");
    }
    assert_eq!(cfg["instance_dir"], json!(root.to_string_lossy()), "the daemon's instance folder, not the module setting");
    assert_eq!(cfg["cache_dir"], json!(at("zomboid")), "the setting itself still reaches the module");
}

/// 시작 경로 정책 (C8): 인스턴스 설정 > 모듈이 콘솔 없음을 밝혔는지 > managed 기본
#[test]
fn managed_start_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    for (name, mode) in [("console-game", "\ninteraction_mode = \"console\""), ("api-game", "\ninteraction_mode = \"commands\""), ("plain-game", "")] {
        let dir = modules_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"rcon\"]{mode}\n"),
        )
        .unwrap();
    }
    let supervisor = Supervisor::new_with_instances_dir(
        modules_dir.to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    let inst = |module: &str, setting: Option<Value>| {
        let mut i = crate::instance::ServerInstance::new("s", module);
        if let Some(v) = setting {
            i.module_settings.insert("managed_start".into(), v);
        }
        i
    };

    assert!(supervisor.uses_managed_start(&inst("console-game", None)));
    assert!(supervisor.uses_managed_start(&inst("plain-game", None)), "managed is the default");
    assert!(supervisor.uses_managed_start(&inst("missing-module", None)));
    assert!(supervisor.uses_managed_start(&inst("api-game", None)), "games without a console are managed too (C8b)");
    assert!(!supervisor.uses_managed_start(&inst("console-game", Some(json!(false)))));
    assert!(!supervisor.uses_managed_start(&inst("console-game", Some(json!("false")))));
    assert!(!supervisor.uses_managed_start(&inst("api-game", Some(json!(false)))));
}

/// 크래시 직후(모니터가 치우기 전) 들어온 정지는 크래시로 기록하되 다시 켜지 않는다 (D-S6)
/// 모듈·익스텐션 자동 교체가 문을 쥔 동안 서버 시작은 기다린다: 교체 직전의 사용 확인과 교체 사이에 서버가 켜지지 않는다
#[tokio::test]
async fn starts_wait_while_addons_are_being_swapped() {
    let tmp = tempfile::tempdir().unwrap();
    let mut supervisor = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    supervisor.initialize().await.unwrap();
    let gate = supervisor.addon_gate().try_hold_starts().expect("no one else starts on this supervisor");
    let sup = std::sync::Arc::new(tokio::sync::RwLock::new(supervisor));
    let managed = tokio::spawn({
        let sup = sup.clone();
        async move { Supervisor::start_managed_server(&sup, "none", "game", json!({})).await }
    });
    let plain = tokio::spawn({
        let sup = sup.clone();
        async move { Supervisor::start_server(&sup, "none", "game", json!({})).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!managed.is_finished() && !plain.is_finished(), "starts must wait for the swap");
    drop(gate);
    assert!(managed.await.unwrap().is_err(), "then it runs (no such instance here)");
    let _ = plain.await.unwrap();
}

/// 시작·설치가 문을 읽기로 쥔 동안 자동 교체는 기다리지 않고 미룬다: 기다리는 교체 뒤에 다른 시작·설치가 줄 서지 않는다
#[tokio::test]
async fn an_addon_swap_gives_up_instead_of_queueing_ahead_of_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let supervisor = || Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    let gate = supervisor().addon_gate();
    let install = gate.hold_addons().await;
    assert!(gate.try_hold_starts().is_none(), "a swap does not run under an install");
    assert!(supervisor().addon_gate().try_hold_starts().is_some(), "another supervisor's install does not hold this one's swaps");
    let another = tokio::time::timeout(Duration::from_millis(500), gate.hold_addons()).await;
    assert!(another.is_ok(), "nothing waits behind a swap that gave up");
    drop((install, another));
}

/// 필수 익스텐션은 지금 모듈의 `[install] requires_extensions`에 인스턴스가 더한 것이다 (시작 검사와 자동 교체 보류가 같은 답).
/// 만들 때의 모듈 목록만 들고 있는 인스턴스도 모듈이 새로 요구하는 것을 받는다
#[tokio::test]
async fn an_instance_needs_the_module_requirements_and_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    let dir = modules_dir.join("game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::write(
        dir.join("module.toml"),
        "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nrequires_extensions = [\"steamcmd\"]\n",
    )
    .unwrap();
    let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    supervisor.initialize().await.unwrap();
    let mut inst = crate::instance::ServerInstance::new("srv", "game");
    assert_eq!(supervisor.effective_required_extensions(&inst), vec!["steamcmd".to_string()]);
    inst.required_extensions = vec!["docker".into(), "steamcmd".into()];
    assert_eq!(supervisor.effective_required_extensions(&inst), vec!["steamcmd".to_string(), "docker".to_string()]);
}

/// 시작·정지마다 누가 시켰는지 `instance.started`·`instance.stopped`에 싣는다 (출처는 요청의 `audit::Origin`).
/// 예기치 않은 것은 사바쨩이 띄우지 않은 프로세스의 시작(`unexpected`)과 크래시·강제 종료(`instance.exit`)뿐이다
#[cfg(unix)]
#[tokio::test]
async fn starts_and_stops_say_who_asked() {
    use crate::audit::{with_origin, Origin};
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let games = [
        ("long", "", "sleep 30"),
        ("quits", "[detection]\ngraceful_exit_patterns = [\"Stopping the server\"]\n", "sleep 0.3; echo Stopping the server; exit 0"),
        ("boom", "", "sleep 0.3; kill -SEGV $$"),
    ];
    for (name, extra, sh) in games {
        let dir = modules.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n{extra}")).unwrap();
        std::fs::write(dir.join("lifecycle.py"), format!(
            "import json, sys\n\
                 if sys.argv[1] == 'get_launch_command':\n    \
                 print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n"
        )).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
    }
    let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    sup.initialize().await.unwrap();
    for (id, module) in [("w-long", "long"), ("w-quits", "quits"), ("w-boom", "boom"), ("w-ext", "long")] {
        let mut inst = crate::instance::ServerInstance::new(id, module);
        inst.id = id.into();
        if id == "w-ext" {
            inst.process_name = Some("fakegame".into());
        }
        sup.instance_store.add(inst).unwrap();
    }
    let ext_dir = sup.instance_store.instance_dir("w-ext").to_string_lossy().to_string();
    let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
    let mut sub = crate::events::bus().subscribe(None);
    // 받은 이벤트를 모아 두고, 찾은 것만 꺼낸다
    let mut got: Vec<(String, Value)> = Vec::new();
    let mut seen = move |kind: &str, id: &str| -> Vec<Value> {
        got.extend(std::iter::from_fn(|| sub.rx.try_recv().ok()).map(|e| (e.kind.clone(), e.data.clone())));
        let (hit, rest) = std::mem::take(&mut got).into_iter().partition(|(k, d)| k == kind && d["instanceId"] == id);
        got = rest;
        hit.into_iter().map(|(_, d)| d).collect()
    };

    // 사바쨩을 거친 시작·정지: 출처를 싣고 예기치 않은 것이 아니다
    let chatbot = Origin { via: Some("chatbot".into()), user_name: Some("Dana".into()), ..Origin::new("discord") };
    let paths = [
        (Origin::new("gui"), "gui"),
        (Origin::new("cli"), "cli"),
        (Origin { user: Some("42".into()), user_name: Some("Dana".into()), ..Origin::new("discord") }, "discord"),
        (chatbot, "chatbot"),
        (Origin::new("web"), "web"),
        (Origin { user: Some("host-a".into()), user_name: Some("집 PC".into()), ..Origin::new("remote") }, "remote"),
        (Origin::new("schedule"), "schedule"),
        (Origin::new("restart"), "restart"),
        (Origin::new("daemon"), "daemon"),
    ];
    for (origin, source) in paths {
        let user = origin.user_name.clone();
        let started = with_origin(origin.clone(), Supervisor::start_managed_server(&sup, "w-long", "long", json!({}))).await.unwrap();
        assert_eq!(started["success"], true, "{started}");
        let stopped = with_origin(origin, Supervisor::stop_server(&sup, "w-long", "long", true)).await.unwrap();
        assert_eq!(stopped["success"], true, "{stopped}");
        let started = seen("instance.started", "w-long");
        assert_eq!(started.len(), 1, "{source}: {started:?}");
        assert_eq!((started[0]["source"].as_str(), started[0]["unexpected"].as_bool()), (Some(source), Some(false)), "{source}");
        assert_eq!(started[0]["userName"].as_str(), user.as_deref(), "{source}");
        // 원격으로 시킨 PC만 알린다 (디스코드 사용자 id는 싣지 않는다)
        assert_eq!(started[0]["remoteHost"].as_str(), (source == "remote").then_some("host-a"), "{source}");
        let stopped = seen("instance.stopped", "w-long");
        assert_eq!(stopped.len(), 1, "{source}: {stopped:?}");
        assert_eq!((stopped[0]["source"].as_str(), stopped[0]["kind"].as_str()), (Some(source), Some("user_stop")), "{source}");
        assert_eq!(stopped[0]["userName"].as_str(), user.as_deref(), "{source}");
        assert_eq!(stopped[0]["remoteHost"].as_str(), (source == "remote").then_some("host-a"), "{source}");
    }

    let wait_exit = |id: &'static str| {
        let sup = sup.clone();
        async move {
            for _ in 0..100 {
                if !sup.read().await.managed_store.running_instance_ids().await.iter().any(|i| i == id) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!("{id} did not exit");
        }
    };
    // 사바쨩 밖의 프로세스: 데몬이 켜질 때 이미 돌던 것은 알리지 않는다. 그 뒤에 나타난 것만 예기치 않은 시작이다
    let ext = crate::process_monitor::RunningProcess { pid: 4_000_001, name: "fakegame".into(), executable_path: None, cmd: Vec::new(), cwd: Some(ext_dir) };
    sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
    assert!(seen("instance.started", "w-ext").is_empty(), "a server already running when the daemon started is not unexpected");
    // 사바쨩이 시키지 않았는데 사라졌다: 원인 모를 종료
    sup.write().await.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
    let gone = seen("instance.exit", "w-ext");
    assert_eq!(gone.iter().map(|e| e["kind"].clone()).collect::<Vec<_>>(), vec![json!("unknown_exit")]);
    sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
    let started = seen("instance.started", "w-ext");
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!((started[0]["source"].as_str(), started[0]["unexpected"].as_bool()), (Some("external"), Some(true)));
    // 사바쨩이 띄우는 중(아직 추적 전)에 모니터가 먼저 본 프로세스는 밖에서 켠 것이 아니다
    sup.write().await.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
    seen("instance.exit", "w-ext");
    sup.read().await.runtime.with("w-ext", |r| r.starting = true);
    sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
    sup.read().await.runtime.with("w-ext", |r| r.starting = false);
    assert!(seen("instance.started", "w-ext").is_empty(), "a server saba-chan is starting is not unexpected");

    // 끝까지 판정한 정지 뒤의 모니터 순회가 같은 실행을 원인 모를 종료로 다시 보지 않는다
    sup.write().await.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
    assert!(seen("instance.exit", "w-long").iter().all(|e| e["kind"] == "user_stop"), "a judged stop is not judged again");

    // 게임 안에서 끈 것: 정상 정지 (출처 game)
    with_origin(Origin::new("gui"), Supervisor::start_managed_server(&sup, "w-quits", "quits", json!({}))).await.unwrap();
    wait_exit("w-quits").await;
    sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
    let stopped = seen("instance.stopped", "w-quits");
    assert_eq!(stopped.len(), 1, "{stopped:?}");
    assert_eq!((stopped[0]["source"].as_str(), stopped[0]["kind"].as_str()), (Some("game"), Some("in_game_stop")));

    // 크래시: instance.exit 만 (정상 정지 알림은 없다)
    with_origin(Origin::new("gui"), Supervisor::start_managed_server(&sup, "w-boom", "boom", json!({}))).await.unwrap();
    wait_exit("w-boom").await;
    sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
    assert!(seen("instance.stopped", "w-boom").is_empty());
    let exits = seen("instance.exit", "w-boom");
    assert_eq!(exits.iter().map(|e| e["kind"].clone()).collect::<Vec<_>>(), vec![json!("crash")]);
}

/// managed 모드에서는 모듈이 stdin을 지원할 때만 명령을 stdin으로 보낸다 (M3)
#[cfg(unix)]
#[tokio::test]
async fn commands_route_to_stdin_only_for_managed_stdin_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    for (name, protocols) in [("console-game", r#"["rcon", "stdin"]"#), ("rest-game", r#"["rest"]"#)] {
        let dir = modules_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            format!(
                "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = {protocols}\n"
            ),
        )
        .unwrap();
    }
    let instances_dir = tmp.path().join("instances");
    std::fs::create_dir_all(&instances_dir).unwrap();
    let mut supervisor = Supervisor::new_with_instances_dir(
        modules_dir.to_str().unwrap(),
        instances_dir.to_str().unwrap(),
    );

    for (id, module) in [("console", "console-game"), ("rest", "rest-game"), ("idle", "console-game")] {
        let mut inst = crate::instance::ServerInstance::new(id, module);
        inst.id = id.to_string();
        supervisor.instance_store.add(inst).unwrap();
    }
    for id in ["console", "rest"] {
        let proc = managed_process::ManagedProcess::spawn(
            "cat",
            &[],
            tmp.path().to_str().unwrap(),
            Vec::new(),
            None,
            None,
        )
        .await
        .unwrap();
        supervisor.managed_store.insert(id, proc).await;
    }

    assert!(supervisor.routes_commands_to_stdin("console").await);
    assert!(!supervisor.routes_commands_to_stdin("rest").await, "module without stdin keeps RCON/REST");
    assert!(!supervisor.routes_commands_to_stdin("idle").await, "no managed process keeps RCON");

    for id in ["console", "rest"] {
        if let Some(proc) = supervisor.managed_store.get(id).await {
            let _ = process::force_kill_pid(proc.pid);
        }
    }
}

/// `?spans=1`: 디스크 로그 줄에도 모듈 규칙의 토큰 구간이 붙고, 없는 요청에는 안 붙는다
#[tokio::test]
async fn console_spans_use_module_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    let dir = modules_dir.join("hl-game");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    std::fs::write(
        dir.join("module.toml"),
        "[module]\nname = \"hl-game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[syntax_highlight.rules]]\nname = \"join\"\npattern = '(?P<hl>\\w+) joined'\ncolor = \"player\"\n",
    )
    .unwrap();
    let supervisor = Supervisor::new_with_instances_dir(
        modules_dir.to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    let mut inst = crate::instance::ServerInstance::new("srv", "hl-game");
    inst.id = "srv".into();
    let mut supervisor = supervisor;
    supervisor.instance_store.add(inst).unwrap();
    let log = managed_process::console_log_path(&supervisor.instance_store.instance_dir("srv"));
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, "[2026-10-17 10:00:00] [STDOUT/INFO] Steve joined the game\n").unwrap();

    let with = supervisor.get_console_output_with_spans("srv", None, Some(10)).await.unwrap();
    assert_eq!(with["source"], "disk");
    assert_eq!(with["lines"][0]["spans"], json!([[0, 5, "player"]]));
    let without = supervisor.get_console_output("srv", None, Some(10)).await.unwrap();
    assert!(without["lines"][0].get("spans").is_none());
}

/// REST 전용 모듈의 서버는 러너가 stdin을 쥐어도(의사 콘솔) 콘솔 입력을 받지 않는다:
/// 콘솔 응답의 `stdin_available`은 `/stdin`이 받는지와 같다 (다르면 클라이언트가 입력칸을 열고 409를 받는다)
#[cfg(unix)]
#[tokio::test]
async fn console_input_follows_the_module_protocols() {
    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    for (name, supported) in [("rest-game", "[\"rest\"]"), ("stdin-game", "[\"stdin\"]")] {
        let dir = modules_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = {supported}\n"),
        )
        .unwrap();
    }
    let mut supervisor = Supervisor::new_with_instances_dir(
        modules_dir.to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    for (id, module) in [("rest", "rest-game"), ("stdin", "stdin-game")] {
        let mut inst = crate::instance::ServerInstance::new(id, module);
        inst.id = id.into();
        supervisor.instance_store.add(inst).unwrap();
        let proc = managed_process::ManagedProcess::spawn("/bin/sh", &["-c".into(), "sleep 30".into()], ".", Vec::new(), None, None)
            .await
            .unwrap();
        supervisor.managed_store.insert(id, proc).await;
    }
    let rest = supervisor.get_console_output("rest", None, Some(10)).await.unwrap();
    let stdin = supervisor.get_console_output("stdin", None, Some(10)).await.unwrap();
    for id in ["rest", "stdin"] {
        let _ = supervisor.managed_store.get(id).await.unwrap().force_kill().await;
    }
    assert_eq!(rest["stdin_available"], false, "{rest}");
    assert_eq!(stdin["stdin_available"], true, "{stdin}");
}

/// 데몬을 다시 켜도 알림 번호는 앞 실행보다 크다: 클라이언트가 들고 있는 번호 뒤의 새 알림을 놓치지 않는다 (L50)
#[test]
fn notice_ids_keep_growing_across_a_daemon_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let event = || PortConflictStopEvent {
        id: 0,
        stopped_name: "b".into(),
        stopped_id: "b-id".into(),
        existing_name: "a".into(),
        existing_id: "a-id".into(),
        port: 1,
        timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
    };
    let daemon = || Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    let first = daemon();
    first.record_port_conflict_stops([event(), event()]);
    let seen = first.port_conflict_stops_after(0).last().unwrap().id;
    let restarted = daemon();
    restarted.record_port_conflict_stops([event()]);
    assert_eq!(restarted.port_conflict_stops_after(seen).len(), 1, "a client that saw id {seen} gets the new notice");
}

/// 시작 중(준비 문구를 기다림)이고 채팅 연동이 켜진 서버를 지우면, 상태를 버리기 전에 준비 알림은 시작 실패로,
/// 연동은 프로세스가 끝난 것으로 닫아 알린다 (지우기 전과 같은 알림)
#[cfg(unix)]
#[tokio::test]
async fn removing_a_starting_instance_closes_its_ready_wait_and_chat_links() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
    let inst = crate::instance::ServerInstance::new("half-up", "game");
    let id = inst.id.clone();
    sup.instance_store.add(inst).unwrap();
    let proc = Arc::new(ManagedProcess::spawn("/bin/sh", &["-c".into(), "sleep 30".into()], ".", Vec::new(), None, None).await.unwrap());
    let mut sub = crate::events::bus().subscribe(None);
    let started = Ok(json!({ "success": true, "managed": true }));
    ready::after_start(&sup.runtime, &id, "half-up", &started, Some(proc.clone()), &["^never$".to_string()], std::time::Duration::from_secs(600));
    chat_link::toggle(&id, "111111111111111111", "222222222222222222", None, |_| Ok::<(), ()>(())).unwrap();

    sup.remove_instance(&id).unwrap();
    let (mut ready_seen, mut link_seen) = (None, None);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while ready_seen.is_none() || link_seen.is_none() {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Ok(e) = tokio::time::timeout(left, sub.rx.recv()).await else { break };
        let e = e.unwrap();
        if e.data["instanceId"] != id.as_str() {
            continue;
        }
        match e.kind.as_str() {
            "instance.ready" => ready_seen = Some((e.data["ok"].clone(), e.data["code"].clone())),
            "instance.chat_link" => link_seen = Some((e.data["active"].clone(), e.data["reason"].clone())),
            _ => {}
        }
    }
    let _ = proc.force_kill().await;
    assert_eq!(ready_seen, Some((json!(false), json!("start_failed"))));
    assert_eq!(link_seen, Some((json!(false), json!("stopped"))));
    assert!(chat_link::list(&id).is_empty());
}
