use super::*;
use std::path::Path;
use std::sync::Arc;

#[test]
fn highlight_rules_carry_background_underline_priority() {
    let meta = parse_module_toml(
        "[module]\nname = \"m\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\nschema_version = 1\n\n\
         [[syntax_highlight.rules]]\nname = \"a\"\npattern = \"x\"\ncolor = \"warn\"\n\n\
         [[syntax_highlight.rules]]\nname = \"b\"\npattern = \"(?i)y\"\ncolor = \"#fff\"\n\
         background = \"dim\"\nunderline = true\npriority = 5\n",
    )
    .unwrap();
    let rules = &meta.syntax_highlight.unwrap().rules;
    assert_eq!((rules[0].underline, rules[0].priority, rules[0].background.as_deref()), (false, 0, None));
    assert_eq!((rules[1].underline, rules[1].priority, rules[1].background.as_deref()), (true, 5, Some("dim")));
}

/// 인스턴스 `protocol_mode`는 모듈이 받는 프로토콜만: `auto`·예전 `rcon`·받지 않는 값은 모듈 기본값으로 (F13)
#[test]
fn protocol_mode_follows_the_module() {
    let meta = |protocols: &str| {
        parse_module_toml(&format!(
            "[module]\nname = \"m\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\nschema_version = 1\n\n[protocols]\n{protocols}\n"
        ))
        .unwrap()
    };
    let mc = meta("supported = [\"stdin\", \"rcon\"]\ndefault = \"stdin\"\nrcon_port = 25575");
    assert_eq!(mc.protocol_mode_for("auto").as_deref(), Some("stdin"));
    assert_eq!(mc.protocol_mode_for("rcon").as_deref(), Some("stdin"));
    assert_eq!(mc.protocol_mode_for("rest").as_deref(), Some("stdin"));
    assert_eq!(mc.protocol_mode_for("stdin"), None);
    let pal = meta("supported = [\"rest\"]");
    assert_eq!(pal.protocol_mode_for("auto").as_deref(), Some("rest"));
    assert_eq!(pal.protocol_mode_for("rest"), None);
    let rcon_only = meta("supported = [\"rcon\"]\ndefault = \"rcon\"\nrcon_port = 27015");
    assert_eq!(rcon_only.protocol_mode_for("rcon"), None);
    // 프로토콜을 선언하지 않은 모듈은 그대로 둔다
    let plain = parse_module_toml("[module]\nname = \"m\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\nschema_version = 1\n").unwrap();
    assert_eq!(plain.protocol_mode_for("auto"), None);
}

/// 서버 목록·콘솔·감시 주기가 자주 부른다: 캐시한 목록을 복제하지 않고 함께 쓰고, 캐시를 비우면 다시 읽는다
#[test]
fn cached_modules_are_shared_until_the_cache_is_cleared() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("plain")).unwrap();
    std::fs::write(
        tmp.path().join("plain").join("module.toml"),
        "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("plain").join("lifecycle.py"), "").unwrap();
    let loader = ModuleLoader::new(tmp.path().to_str().unwrap());
    let first = loader.modules().unwrap();
    assert!(Arc::ptr_eq(&first, &loader.modules().unwrap()));
    assert_eq!(loader.get_module("plain").unwrap().metadata.name, "plain");
    loader.invalidate_cache();
    let reread = loader.modules().unwrap();
    assert!(!Arc::ptr_eq(&first, &reread));
    assert_eq!(reread.len(), 1);
}

#[test]
fn folders_and_files_starting_with_a_dot_are_not_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let toml = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";
    for dir in ["plain", ".old-plain"] {
        std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        std::fs::write(tmp.path().join(dir).join("module.toml"), toml).unwrap();
        std::fs::write(tmp.path().join(dir).join("lifecycle.py"), "").unwrap();
    }
    std::fs::write(tmp.path().join(".tmp-module-plain.zip"), "partial").unwrap();
    let loader = ModuleLoader::new(tmp.path().to_str().unwrap());
    assert_eq!(loader.discover_modules().unwrap().len(), 1);
    assert!(loader.load_errors().is_empty(), "{:?}", loader.load_errors());
}

#[test]
fn test_module_loader_creation() {
    let loader = ModuleLoader::new("./modules");
    assert_eq!(loader.modules_dir, "./modules");
}

#[test]
fn test_discover_modules_empty_dir() {
    let loader = ModuleLoader::new("./nonexistent_modules");
    let modules = loader.discover_modules().unwrap();
    assert!(modules.is_empty());
}

#[test]
fn test_parse_module_toml_minimal() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.name, "test-game");
    assert_eq!(meta.version, "1.0.0");
    assert_eq!(meta.entry, "lifecycle.py");
    assert!(meta.description.is_none());
    assert!(meta.settings.is_none());
    assert!(meta.commands.is_none());
}

#[test]
fn test_parse_module_toml_missing_name() {
    let toml = r#"
[module]
version = "1.0.0"
entry = "lifecycle.py"
"#;
    let result = parse_module_toml(toml);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("name"), "Error should mention missing field 'name': {}", err_msg);
}

#[test]
fn commands_hook_alone_keeps_the_commands_section() {
    let meta = parse_module_toml(
        "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[commands]\nhook = true\n",
    )
    .unwrap();
    let commands = meta.commands.expect("hook without fields still declares the section");
    assert!(commands.hook && commands.fields.is_empty());
}

#[test]
fn test_parse_module_toml_with_settings_and_commands() {
    let toml = r#"
[module]
name = "minecraft"
version = "2.0.0"
entry = "lifecycle.py"
log_pattern = '/(?P<level>INFO|WARN|ERROR)\]'

[protocols]
supported = ["rcon", "stdin"]
default = "rcon"
interaction_mode = "console"

[config]
process_name = "java.exe"
default_port = 25565
stop_command = "stop"

[[settings.fields]]
name = "ram"
type = "text"
label = "Memory"
default = "2G"
group = "saba-chan"

[[commands.fields]]
name = "players"
label = "Player List"
method = "rcon"
rcon_template = "list"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.name, "minecraft");
    assert_eq!(meta.log_pattern.as_deref(), Some("/(?P<level>INFO|WARN|ERROR)\\]"));
    assert_eq!(meta.interaction_mode.as_deref(), Some("console"));
    assert_eq!(meta.default_port, Some(25565));
    assert_eq!(meta.stop_command.as_deref(), Some("stop"));
    
    let settings = meta.settings.unwrap();
    assert_eq!(settings.fields.len(), 1);
    assert_eq!(settings.fields[0].name, "ram");
    
    let commands = meta.commands.unwrap();
    assert_eq!(commands.fields.len(), 1);
    assert_eq!(commands.fields[0].name, "players");
    assert_eq!(commands.fields[0].console_template.as_deref(), Some("list"), "old name rcon_template still loads");
    assert_eq!(commands.fields[0].method.as_deref(), Some("console"), "old method name rcon becomes console");
    let v1 = format!("{V1}\n[[commands.fields]]\nname = \"say\"\nmethod = \"console\"\nconsole_template = \"say {{message}}\"\n");
    let meta = parse_module_toml(&v1).unwrap();
    assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
    assert_eq!(meta.commands.unwrap().fields[0].console_template.as_deref(), Some("say {message}"));
}

#[test]
fn test_parse_module_toml_extra_config_fields_ignored() {
    let toml = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[config]
executable_path = "PalServer.exe"
server_executable = "PalServer.exe"
process_name = "PalServer"
default_port = 8211
ram = "4G"
custom_unknown_field = "should not cause error"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.name, "palworld");
    assert_eq!(meta.executable_path.as_deref(), Some("PalServer.exe"));
    assert_eq!(meta.server_executable.as_deref(), Some("PalServer.exe"));
    assert_eq!(meta.default_port, Some(8211));
}

#[test]
fn test_parse_module_toml_missing_version() {
    let toml = r#"
[module]
name = "test-game"
entry = "lifecycle.py"
"#;
    let result = parse_module_toml(toml);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("version"), "Error should mention missing 'version': {}", err_msg);
}

#[test]
fn test_parse_module_toml_missing_entry() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
"#;
    let result = parse_module_toml(toml);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("entry"), "Error should mention missing 'entry': {}", err_msg);
}

#[test]
fn test_parse_module_toml_empty_string() {
    let result = parse_module_toml("");
    assert!(result.is_err(), "Empty TOML should fail");
}

#[test]
fn test_parse_module_toml_aliases() {
    let toml = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[aliases]
module_aliases = ["팰월드", "팔월드", "pw"]

[aliases.commands]
start = { aliases = ["시작", "실행", "켜기"], description = "Start the server" }
stop = { aliases = ["정지", "끄기"], description = "Stop the server" }
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.module_aliases.len(), 3);
    assert!(meta.module_aliases.contains(&"팰월드".to_string()));
    assert!(meta.module_aliases.contains(&"pw".to_string()));

    assert_eq!(meta.command_aliases.len(), 2);
    let start = meta.command_aliases.get("start").unwrap();
    assert_eq!(start.aliases.len(), 3);
    assert!(start.aliases.contains(&"시작".to_string()));
    assert_eq!(start.description.as_deref(), Some("Start the server"));

    let stop = meta.command_aliases.get("stop").unwrap();
    assert_eq!(stop.aliases.len(), 2);
}

#[test]
fn test_parse_module_toml_aliases_empty_by_default() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert!(meta.module_aliases.is_empty());
    assert!(meta.command_aliases.is_empty());
}

#[test]
fn test_parse_module_toml_invalid_toml_syntax() {
    let result = parse_module_toml("this is not [valid toml =!");
    assert!(result.is_err(), "Invalid TOML syntax should fail");
}

#[test]
fn test_parse_module_toml_no_module_section() {
    let toml = r#"
[config]
process_name = "java"
"#;
    let result = parse_module_toml(toml);
    assert!(result.is_err(), "TOML without [module] section should fail");
}

#[test]
fn test_parse_module_toml_protocols_section() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[protocols]
supported = ["rcon", "rest"]
default = "rest"
interaction_mode = "commands"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.interaction_mode.as_deref(), Some("commands"));
    assert_eq!(meta.rcon_only_port(), None, "rest is there: rcon is never used");
}

#[test]
fn rcon_is_used_only_when_it_is_the_only_protocol_and_needs_a_port() {
    let with = |protocols: &str| parse_module_toml(&format!("[module]\nschema_version = 1\nname = \"g\"\nversion = \"1\"\nentry = \"lifecycle.py\"\n[protocols]\n{protocols}\n"));
    assert_eq!(with("supported = [\"rcon\"]\nrcon_port = 27015").unwrap().rcon_only_port(), Some(27015));
    assert_eq!(with("supported = [\"stdin\", \"rcon\"]\nrcon_port = 27015").unwrap().rcon_only_port(), None);
    let err = with("supported = [\"rcon\"]").unwrap_err().to_string();
    assert!(err.contains("rcon_port"), "{err}");
}

/// 관리형 시작을 끄면 콘솔 stdin이 없다: stdin과 rcon을 함께 받는 모듈은 그때 RCON이 명령 길이다 (사용자 정의 2026-11-08)
#[test]
fn stdin_modules_with_rcon_fall_back_to_rcon_without_a_console() {
    let with = |protocols: &str| parse_module_toml(&format!("[module]\nschema_version = 1\nname = \"g\"\nversion = \"1\"\nentry = \"lifecycle.py\"\n[protocols]\n{protocols}\n"));
    assert_eq!(with("supported = [\"stdin\", \"rcon\"]\nrcon_port = 25575").unwrap().rcon_port_without_console(), Some(25575));
    assert_eq!(with("supported = [\"rcon\"]\nrcon_port = 27015").unwrap().rcon_port_without_console(), Some(27015));
    assert_eq!(with("supported = [\"stdin\", \"rest\", \"rcon\"]\nrcon_port = 25575").unwrap().rcon_port_without_console(), None, "REST works without a console");
    assert_eq!(with("supported = [\"stdin\"]").unwrap().rcon_port_without_console(), None);
    let err = with("supported = [\"stdin\", \"rcon\"]").unwrap_err().to_string();
    assert!(err.contains("rcon_port"), "{err}");
}

#[test]
fn test_parse_module_toml_multiple_settings_groups() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[[settings.fields]]
name = "ram"
type = "text"
label = "Memory"
default = "2G"
group = "performance"

[[settings.fields]]
name = "port"
type = "number"
label = "Port"
default = "25565"
group = "network"
"#;
    let meta = parse_module_toml(toml).unwrap();
    let settings = meta.settings.unwrap();
    assert_eq!(settings.fields.len(), 2);
    assert_eq!(settings.fields[0].group.as_deref(), Some("performance"));
    assert_eq!(settings.fields[1].group.as_deref(), Some("network"));
}

#[test]
fn test_parse_module_toml_cmd_patterns() {
    let toml = r#"
[module]
name = "zomboid"
version = "1.0.0"
entry = "lifecycle.py"

[config]
process_name = "java.exe"
default_port = 16261

[detection]
process_patterns = ["ProjectZomboid64", "zombie.network.GameServer"]
cmd_patterns = ["zombie.network.GameServer", "ProjectZomboid", "pzserver"]
common_paths = ["C:\\PZServer"]
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.process_name.as_deref(), Some("java.exe"));
    assert_eq!(meta.process_patterns.len(), 2);
    assert_eq!(meta.cmd_patterns.len(), 3);
    assert!(meta.cmd_patterns.contains(&"zombie.network.GameServer".to_string()));
    assert!(meta.cmd_patterns.contains(&"ProjectZomboid".to_string()));
}

#[test]
fn test_parse_module_toml_cmd_patterns_empty_by_default() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert!(meta.cmd_patterns.is_empty());
}

// ── 컴포넌트 의존성(dependencies) 테스트 ──

#[test]
fn test_parse_module_toml_dependencies_field() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"ext-steamcmd" = ">=1.0.0"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.dependencies.len(), 2);
    assert_eq!(meta.dependencies.get("saba-core").unwrap(), ">=0.3.0");
    assert_eq!(meta.dependencies.get("ext-steamcmd").unwrap(), ">=1.0.0");
}

#[test]
fn test_parse_module_toml_dependencies_empty_by_default() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
    let meta = parse_module_toml(toml).unwrap();
    assert!(meta.dependencies.is_empty());
}

#[test]
fn test_module_check_dependencies_all_satisfied() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"ext-steamcmd" = ">=1.0.0"
"#;
    let meta = parse_module_toml(toml).unwrap();
    let mut installed = std::collections::HashMap::new();
    installed.insert("saba-core".to_string(), "0.5.0".to_string());
    installed.insert("ext-steamcmd".to_string(), "1.2.0".to_string());

    let issues = meta.check_dependencies(&installed);
    assert!(issues.is_empty(), "All dependencies should be satisfied");
}

#[test]
fn test_module_check_dependencies_version_too_low() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"#;
    let meta = parse_module_toml(toml).unwrap();
    let mut installed = std::collections::HashMap::new();
    installed.insert("saba-core".to_string(), "0.2.0".to_string());

    let issues = meta.check_dependencies(&installed);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].0, "saba-core");
    assert_eq!(issues[0].1, ">=0.3.0");
    assert_eq!(issues[0].2.as_deref(), Some("0.2.0"));
}

#[test]
fn test_module_check_dependencies_component_missing() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"ext-docker" = ">=1.0.0"
"#;
    let meta = parse_module_toml(toml).unwrap();
    let installed = std::collections::HashMap::new(); // empty

    let issues = meta.check_dependencies(&installed);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].0, "ext-docker");
    assert!(issues[0].2.is_none(), "Component not installed → None");
}

#[test]
fn test_module_check_dependencies_exact_version_boundary() {
    let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"#;
    let meta = parse_module_toml(toml).unwrap();
    let mut installed = std::collections::HashMap::new();
    installed.insert("saba-core".to_string(), "0.3.0".to_string()); // exactly matches

    let issues = meta.check_dependencies(&installed);
    assert!(issues.is_empty(), "Exact version match should satisfy >=");
}

/// 시험판 데몬(`0.1.11-beta.1`)은 정식보다 낮다. 새 번호대를 시험판부터 받으려면 `-0`을 붙인다
#[test]
fn dependency_with_zero_prerelease_accepts_test_releases_of_that_version() {
    let meta = parse_module_toml("[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[module.dependencies]\n\"saba-core\" = \">=0.1.11-0\"\n").unwrap();
    let unmet = |core: &str| !meta.check_dependencies(&[("saba-core".to_string(), core.to_string())].into()).is_empty();
    for accepted in ["0.1.11-beta.1", "0.1.11-rc.1", "0.1.11", "0.1.12"] {
        assert!(!unmet(accepted), "{accepted}");
    }
    assert!(unmet("0.1.10"));
}

#[test]
fn launch_log_files_are_read() {
    let toml = "[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[launch]\nlog_files = [\"Pal/Saved/Logs/*.log\"]\n";
    assert_eq!(parse_module_toml(toml).unwrap().log_files, vec!["Pal/Saved/Logs/*.log"]);
    let plain = "[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";
    assert!(parse_module_toml(plain).unwrap().log_files.is_empty());
    assert_eq!(parse_module_toml(plain).unwrap().console, ConsoleMode::Pipe);
    let pty = toml.replace("log_files", "console = \"pty\"\nlog_files");
    assert_eq!(parse_module_toml(&pty).unwrap().console, ConsoleMode::Pty);
}

/// [launch] line_start: 줄바꿈 없이 이어 쓴 출력을 나눌 줄 머리. 없으면 지금처럼 나누지 않는다.
/// 틀린 정규식과 빈 글자에 맞는 정규식(모든 글자 사이에서 나뉜다)은 모듈을 받지 않는다
#[test]
fn launch_line_start_is_read_and_checked() {
    let toml = |line_start: &str| format!("[module]\nschema_version = 1\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[launch]\nline_start = '{}'\n", line_start);
    assert_eq!(parse_module_toml(&toml(r"\[LOG\]")).unwrap().line_start.as_deref(), Some(r"\[LOG\]"));
    let plain = "[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";
    assert_eq!(parse_module_toml(plain).unwrap().line_start, None);
    assert!(parse_module_toml(&toml("(")).is_err(), "invalid regex");
    assert!(parse_module_toml(&toml("x*")).is_err(), "matches the empty string");
    // 스키마 이전 모듈은 경고만 남기고 쓰지 않는다
    let old = toml("(").replace("schema_version = 1\n", "");
    assert_eq!(parse_module_toml(&old).unwrap().line_start, None);
}

const V1: &str = "[module]\nschema_version = 1\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";

/// 설정의 게임 버전 범위(`versions`)를 읽고, 읽을 수 없는 범위는 로드 오류다
#[test]
fn setting_version_ranges_are_read_and_checked() {
    let good = format!("{V1}\n[[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\nversions = \"<1.21.9\"\n");
    let meta = parse_module_toml(&good).unwrap();
    assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
    assert_eq!(meta.settings.unwrap().fields[0].versions.as_deref(), Some("<1.21.9"));
    let bad = good.replace("<1.21.9", "before 1.21.9");
    assert!(parse_module_toml(&bad).unwrap_err().to_string().contains("versions"));
}

/// 0.1.11: 설정의 게임 파일 키(`file_key`)와 로그 진단 규칙(`[[diagnose]]`)은 module.toml 한 곳에 적고
/// 공용 도우미가 읽는다. 데몬은 겹침·빈 값·심각도만 검사한다
#[test]
fn file_keys_and_diagnose_rules_are_read_and_checked() {
    let good = format!("{V1}\n[[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\nfile_key = \"max-players\"\n\n[[diagnose]]\ncode = \"PORT_IN_USE\"\nseverity = \"critical\"\npatterns = [\"Address already in use\"]\n");
    let meta = parse_module_toml(&good).unwrap();
    assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
    assert_eq!(meta.settings.unwrap().fields[0].file_key.as_deref(), Some("max-players"));
    assert_eq!(meta.diagnose, vec![DiagnoseRule { code: "PORT_IN_USE".into(), severity: Some("critical".into()), patterns: vec!["Address already in use".into()] }]);
    let twice = good.replace("name = \"max_players\"", "name = \"a\"") + "\n[[settings.fields]]\nname = \"b\"\nfile_key = \"max-players\"\n";
    assert!(parse_module_toml(&twice).unwrap_err().to_string().contains("used by another field"));
    let injected = good.replace("file_key = \"max-players\"", "file_key = \"a=b\"");
    assert!(parse_module_toml(&injected).unwrap_err().to_string().contains("file_key"));
    let severity = good.replace("severity = \"critical\"", "severity = \"fatal\"");
    assert!(parse_module_toml(&severity).unwrap_err().to_string().contains("severity"));
    let empty = good.replace("patterns = [\"Address already in use\"]", "patterns = []");
    assert!(parse_module_toml(&empty).unwrap_err().to_string().contains("patterns"));
    let duplicate = format!("{good}\n[[diagnose]]\ncode = \"PORT_IN_USE\"\npatterns = [\"x\"]\n");
    assert!(parse_module_toml(&duplicate).unwrap_err().to_string().contains("duplicate code"));
}

/// 만들 때 고를 수 있는 값(`install_options`): 사바쨩이 설치할 수 있는 값만 새 서버 화면에 보인다.
/// 나머지 `options`는 가져온 서버를 판정할 때만 쓴다. `options` 안의 값이어야 하고 `install = true`인 select에만 쓴다
#[test]
fn install_options_are_a_checked_subset_of_options() {
    let field = |extra: &str| format!("{V1}\n[[settings.fields]]\nname = \"server_type\"\ntype = \"select\"\noptions = [\"vanilla\", \"spigot\"]\ndefault = \"vanilla\"\n{extra}");
    let meta = parse_module_toml(&field("install = true\ninstall_options = [\"vanilla\"]\n")).unwrap();
    assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
    assert_eq!(meta.settings.unwrap().fields[0].install_options.as_deref(), Some(&["vanilla".to_string()][..]));
    assert!(parse_module_toml(&field("install = true\n")).unwrap().settings.unwrap().fields[0].install_options.is_none());
    let outside = field("install = true\ninstall_options = [\"paper\"]\n");
    assert!(parse_module_toml(&outside).unwrap_err().to_string().contains("install_options"));
    let not_install = field("install_options = [\"vanilla\"]\n");
    assert!(parse_module_toml(&not_install).unwrap_err().to_string().contains("install = true"));
    let empty = field("install = true\ninstall_options = []\n");
    assert!(parse_module_toml(&empty).unwrap_err().to_string().contains("install_options"));
    let default_outside = field("install = true\ninstall_options = [\"spigot\"]\n");
    assert!(parse_module_toml(&default_outside).unwrap_err().to_string().contains("default"));
    let preset = field("install = true\ninstall_options = [\"vanilla\"]\n\n[[presets]]\nid = \"bukkit\"\n[presets.settings]\nserver_type = \"spigot\"\n");
    assert!(parse_module_toml(&preset).unwrap_err().to_string().contains("install_options"));
}

/// 0.1.11: 더 새 데몬의 키를 쓴 모듈도 예전 데몬이 읽는다. 모르는 키는 경고로 남기고 무시한다
/// (꼭 필요한 기능은 `saba-core` 버전 조건으로 막는다)
#[test]
fn schema_v1_ignores_unknown_keys_with_a_warning() {
    let meta = parse_module_toml(&format!("{V1}\n[config]\nram = \"4G\"\n")).unwrap();
    assert!(meta.warnings.iter().any(|w| w.contains("config.ram")), "{:?}", meta.warnings);
    let meta = parse_module_toml(&format!("{V1}\n[[commands.fields]]\nname = \"kick\"\nrcon_templat = \"kick\"\n")).unwrap();
    assert!(meta.warnings.iter().any(|w| w.contains("rcon_templat")), "{:?}", meta.warnings);
    assert_eq!(meta.commands.map(|c| c.fields.len()), Some(1));
    assert!(parse_module_toml(V1).unwrap().warnings.is_empty());
    // 알려진 키의 잘못된 값은 여전히 로드 실패다
    assert!(parse_module_toml(&format!("{V1}\n[config]\nstop_timeout = \"long\"\n")).is_err());
}

#[test]
fn modules_without_schema_version_still_load_with_warnings() {
    let toml = "\u{feff}[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nram = \"4G\"\n";
    let meta = parse_module_toml(toml).unwrap();
    assert_eq!(meta.schema_version, 0);
    assert!(meta.warnings.iter().any(|w| w.contains("config.ram")), "{:?}", meta.warnings);
    assert!(meta.warnings.iter().any(|w| w.contains("schema_version")));
}

/// 06 §2·§6: players_detail·stop_call_limit은 데몬 몫이다. 같은 이름의 설정은 로드하되 경고한다
#[test]
fn settings_named_like_daemon_call_keys_are_warned_about() {
    let meta = parse_module_toml(&format!("{V1}\n[[settings.fields]]\nname = \"players_detail\"\ntype = \"boolean\"\n")).unwrap();
    assert!(meta.warnings.iter().any(|w| w.contains("players_detail")), "{:?}", meta.warnings);
}

#[test]
fn newer_schema_version_is_refused() {
    let toml = V1.replace("schema_version = 1", "schema_version = 3");
    let err = parse_module_toml(&toml).unwrap_err();
    assert!(err.to_string().contains("schema_version 3"), "{err}");
    let failed = ModuleLoadError::new(Path::new("m"), &err);
    assert_eq!((failed.error_code.as_deref(), failed.required), (Some("module_schema_unsupported"), Some(3)));
}

/// MK2 (06 §7): 스키마 2는 스키마 1에 list·multiline 타입과 설정 키를 더한 것이다. 스키마 1에 있으면 로드 실패이고,
/// 실패한 모듈은 `failed[]`에 `module_schema_invalid`와 `problems`로 보인다
#[test]
fn schema_v2_adds_setting_types_and_keys_that_schema_v1_refuses() {
    let fields = "[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\nmax_items = 5\nrestart_required = true\n\n\
                  [[settings.fields]]\nname = \"motd\"\ntype = \"multiline\"\nmax_length = 200\n";
    let v2 = parse_module_toml(&format!("{}\n{fields}", V1.replace("schema_version = 1", "schema_version = 2"))).unwrap();
    assert_eq!(v2.schema_version, 2);
    let admins = &v2.settings.as_ref().unwrap().fields[0];
    assert_eq!((admins.field_type.as_str(), admins.max_items, admins.restart_required), ("list", Some(5), true));

    let err = parse_module_toml(&format!("{V1}\n{fields}")).unwrap_err();
    let failed = ModuleLoadError::new(Path::new("m"), &err);
    assert_eq!(failed.error_code.as_deref(), Some("module_schema_invalid"));
    for needle in ["'list'", "'multiline'", "max_items", "restart_required", "max_length"] {
        assert!(failed.problems.iter().any(|p| p.contains(needle)), "{needle}: {:?}", failed.problems);
    }
    assert!(failed.error.contains("schema v1"), "{}", failed.error);
}

/// 스키마를 밝힌 모듈은 알려진 키의 타입 오류도 `module_schema_invalid`와 `problems`로 보이고,
/// list 기본값은 항목 수·중복·형식까지 PATCH와 같은 검사를 받는다
#[test]
fn schema_type_errors_and_bad_list_defaults_are_schema_problems() {
    let v2 = V1.replace("schema_version = 1", "schema_version = 2");
    let err = parse_module_toml(&format!("{v2}\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\nmin_items = \"two\"\n")).unwrap_err();
    let failed = ModuleLoadError::new(Path::new("m"), &err);
    assert_eq!(failed.error_code.as_deref(), Some("module_schema_invalid"), "{}", failed.error);
    assert!(failed.problems.iter().any(|p| p.contains("min_items")), "{:?}", failed.problems);

    let list = |keys: &str, default: &str| parse_module_toml(&format!("{v2}\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n{keys}\ndefault = {default}\n"));
    assert!(list("max_items = 2\npattern = \"[a-z]+\"", "[\"a\", \"b\"]").is_ok());
    for (keys, default) in [("max_items = 1", "[\"a\", \"b\"]"), ("min_items = 2", "[\"a\"]"), ("", "[\"a\", \"a\"]"), ("pattern = \"[a-z]+\"", "[\"A\"]")] {
        let err = list(keys, default).unwrap_err();
        let failed = ModuleLoadError::new(Path::new("m"), &err);
        assert!(failed.problems.iter().any(|p| p.contains("'admins'") && p.contains("default")), "{keys} {default}: {:?}", failed.problems);
    }
}

/// 리뷰 554: 스키마 이전 모듈의 list·multiline은 0.1.8처럼 글자로 다루고, list 프리셋 값도 로드 때 검사한다
#[test]
fn pre_schema_list_types_stay_text_and_list_presets_are_checked() {
    let old = format!("{}\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"multiline\"\n", V1.replace("schema_version = 1\n", ""));
    let meta = parse_module_toml(&old).unwrap();
    let types: Vec<&str> = meta.settings.as_ref().unwrap().fields.iter().map(|f| f.field_type.as_str()).collect();
    assert_eq!(types, ["text", "text"]);

    let v2 = V1.replace("schema_version = 1", "schema_version = 2");
    let preset = |value: &str| {
        parse_module_toml(&format!(
            "{v2}\n[[settings.fields]]\nname = \"mode\"\ntype = \"list\"\noptions = [\"a\", \"b\"]\n\n[[presets]]\nid = \"p\"\nlabel = \"P\"\nsettings = {{ mode = {value} }}\n"
        ))
    };
    assert!(preset("[\"a\"]").is_ok());
    for bad in ["\"a\"", "[\"c\"]", "[\"a\", \"a\"]"] {
        let err = preset(bad).unwrap_err().to_string();
        assert!(err.contains("presets 'p'"), "{bad}: {err}");
    }
}

/// 포트포워딩 확인 (2026-10-26): 밖에서 붙는 포트는 `public`으로 프로토콜을 밝힌다. 포트 필드에만 쓴다
#[test]
fn public_declares_the_protocol_of_a_port_field() {
    let field = |keys: &str| parse_module_toml(&format!("{V1}\n[[settings.fields]]\nname = \"port\"\ntype = \"number\"\n{keys}\n"));
    let meta = field("port = true\npublic = \"udp\"").unwrap();
    assert_eq!(meta.settings.unwrap().fields[0].public.as_deref(), Some("udp"));
    for (keys, needle) in [("port = true\npublic = \"icmp\"", "must be one of"), ("public = \"tcp\"", "port = true")] {
        let err = field(keys).unwrap_err().to_string();
        assert!(err.contains("public") && err.contains(needle), "{keys}: {err}");
    }
}

/// D3 (0.1.14-beta.2): REST 접속값은 모듈이 선언한 설정에 있다. 선언이 없으면 `rest_host`·`rest_port` 설정을 본다
#[test]
fn rest_connection_values_come_from_the_declared_settings() {
    let fields = "[[settings.fields]]\nname = \"api_port\"\ntype = \"number\"\nport = true\ndefault = 8212\n\n\
                  [[settings.fields]]\nname = \"admin_pw\"\ntype = \"password\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n";
    let module = |rest: &str| parse_module_toml(&format!("{V1}\n[protocols]\nsupported = [\"rest\"]\n\n[protocols.rest]\n{rest}\n\n{fields}"));
    let meta = module("port_setting = \"api_port\"\npassword_setting = \"admin_pw\"\nusername = \"admin\"").unwrap();
    let mut inst = crate::instance::ServerInstance::new("s", "m");
    assert_eq!((meta.rest_host(&inst), meta.rest_port(&inst), meta.rest_password(&inst)), (Some("127.0.0.1".into()), Some(8212), None), "module defaults");
    inst.module_settings.extend([("api_port".to_string(), serde_json::json!("9000")), ("rest_host".to_string(), serde_json::json!("10.0.0.2")), ("admin_pw".to_string(), serde_json::json!("pw"))]);
    assert_eq!((meta.rest_host(&inst), meta.rest_port(&inst), meta.rest_password(&inst)), (Some("10.0.0.2".into()), Some(9000), Some("pw")));
    assert_eq!((meta.rest().unwrap().username.as_deref(), meta.rest_target_settings()), (Some("admin"), vec!["rest_host", "api_port"]));

    for (rest, needle) in [
        ("port_setting = \"nope\"", "port_setting 'nope'"),
        ("port_setting = \"motd\"", "port = true"),
        ("port_setting = \"api_port\"\npassword_setting = \"motd\"", "password"),
    ] {
        let err = module(rest).unwrap_err().to_string();
        assert!(err.contains("protocols.rest") && err.contains(needle), "{rest}: {err}");
    }

    // 선언이 없는 모듈(0.1.14-beta.1까지): 기본 설정 이름. REST를 받지 않는 모듈에는 접속값이 없다
    let old = parse_module_toml(&format!("{V1}\n[protocols]\nsupported = [\"rest\"]\n\n[[settings.fields]]\nname = \"rest_port\"\ntype = \"number\"\nport = true\ndefault = 8212\n")).unwrap();
    assert_eq!((old.rest_port(&inst), old.rest_password(&inst)), (Some(8212), None));
    let console = parse_module_toml(&format!("{V1}\n[protocols]\nsupported = [\"stdin\"]\n")).unwrap();
    inst.module_settings.insert("rest_port".into(), serde_json::json!(8212));
    assert_eq!((console.rest().is_some(), console.rest_port(&inst), console.rest_target_settings().len()), (false, None, 0));
}

/// D4·A-02·A-07·A-09 (0.1.14-beta.2): 런타임 경로 설정, 진단 로그 파일, 원격 접속 강제 값, 내려받는 크기
#[test]
fn runtime_settings_diagnose_files_remote_access_and_download_size_are_declared() {
    let toml = format!(
        "{V1}\n[launch]\ndiagnose_files = [\"logs/latest.log\"]\n\n[install]\nmethod = \"steamcmd\"\napp_id = 1\ndownload_size = 4096\n\n\
         [remote_access]\nforced = {{ \"enable-query\" = \"false\" }}\n\n[remote_access.rcon]\non = {{ \"enable-rcon\" = \"true\" }}\noff = {{ \"enable-rcon\" = \"false\" }}\nport = \"rcon.port\"\npassword = \"rcon.password\"\n\n\
         [[settings.fields]]\nname = \"jre\"\ntype = \"file\"\nruntime = \"java\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n"
    );
    let meta = parse_module_toml(&toml).unwrap();
    assert_eq!(meta.runtime_settings(), vec![("jre", "java")]);
    assert_eq!(meta.install.as_ref().unwrap().download_size, Some(4096));
    // 사용자가 정한 값만 런타임으로 인정한다 (기본값·가져온 값은 아니다)
    let mut inst = crate::instance::ServerInstance::new("s", "g");
    inst.module_settings.insert("jre".into(), serde_json::json!("/opt/jdk/bin/java"));
    assert!(meta.user_runtime_paths(&inst).is_empty());
    inst.user_settings.insert("jre".into());
    assert_eq!(meta.user_runtime_paths(&inst), ["/opt/jdk/bin/java"]);
    // 선언 전 모듈(0.1.14-beta.1까지)의 `java_path`는 선언 없이도 Java 런타임 경로로 읽는다
    let legacy = parse_module_toml(&format!("{V1}\n[[settings.fields]]\nname = \"java_path\"\ntype = \"file\"\n")).unwrap();
    assert_eq!(legacy.runtime_settings(), vec![("java_path", "java")]);

    let err = parse_module_toml(&toml.replace("runtime = \"java\"", "runtime = \"cobol\"")).unwrap_err().to_string();
    assert!(err.contains("jre") && err.contains("runtime"), "{err}");
    let err = parse_module_toml(&toml.replace("logs/latest.log", "../outside.log")).unwrap_err().to_string();
    assert!(err.contains("diagnose_files"), "{err}");
}

#[test]
fn schema_v1_checks_values() {
    let bad_risk = format!("{V1}\n[[commands.fields]]\nname = \"stop\"\nrisk = \"scary\"\n");
    assert!(parse_module_toml(&bad_risk).unwrap_err().to_string().contains("risk"));
    let ok_risk = bad_risk.replace("scary", "danger");
    let meta = parse_module_toml(&ok_risk).unwrap();
    assert_eq!(meta.commands.unwrap().fields[0].risk.as_deref(), Some("danger"));
    let escape = V1.replace("lifecycle.py", "../evil.py");
    assert!(parse_module_toml(&escape).unwrap_err().to_string().contains("module.entry"));
    for bad in ["supported = [\"stdin\", \"x</b>\"]", "supported = [\"stdin\"]\ndefault = \"telnet\""] {
        let err = parse_module_toml(&format!("{V1}\n[protocols]\n{bad}\n")).unwrap_err().to_string();
        assert!(err.contains("protocols"), "{err}");
    }
    assert!(parse_module_toml(&format!("{V1}\n[protocols]\nsupported = [\"rest\", \"stdin\"]\ndefault = \"rest\"\n")).is_ok());
}

/// 클라이언트가 그리지 못하는 설정·명령 입력 type은 화면에서 조용히 사라진다: 로드할 때 알린다 (MK2)
#[test]
fn schema_v1_checks_setting_and_command_input_types() {
    let setting = |ty: &str| format!("{V1}\n[[settings.fields]]\nname = \"motd\"\ntype = \"{ty}\"\n");
    let err = parse_module_toml(&setting("textarea")).unwrap_err().to_string();
    assert!(err.contains("settings field 'motd'") && err.contains("textarea"), "{err}");
    for ty in SETTING_TYPES {
        assert!(parse_module_toml(&setting(ty)).is_ok(), "{ty}");
    }
    let input = |ty: &str| format!("{V1}\n[[commands.fields]]\nname = \"kick\"\ninputs = [{{ name = \"player\", type = \"{ty}\" }}]\n");
    let err = parse_module_toml(&input("slider")).unwrap_err().to_string();
    assert!(err.contains("commands 'kick'") && err.contains("slider"), "{err}");
    assert!(parse_module_toml(&input("password")).is_ok());
    // 스키마 이전 모듈은 경고만 하고 올린다
    assert!(parse_module_toml(&setting("textarea").replace("schema_version = 1\n", "")).is_ok());
}

/// 06 §8.2: visible_when은 같은 모듈의 값 하나짜리 설정을 가리키고, 값은 그 설정 타입에 맞아야 하며, 순환하면 안 된다
#[test]
fn visible_when_is_checked_when_the_module_loads() {
    let v2 = V1.replace("schema_version = 1", "schema_version = 2");
    let module = |cond: &str| {
        parse_module_toml(&format!(
            "{v2}\n[[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\n\n[[settings.fields]]\nname = \"port\"\ntype = \"number\"\n\n\
             [[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n\n[[settings.fields]]\nname = \"pvp_damage\"\ntype = \"text\"\nvisible_when = {cond}\n"
        ))
    };
    let ok = module("{ field = \"pvp\", equals = true }").unwrap();
    let cond = ok.settings.unwrap().fields[3].visible_when.clone().unwrap();
    assert_eq!((cond.field.as_str(), cond.equals), ("pvp", Some(serde_json::json!(true))));
    assert!(module("{ field = \"port\", in = [25565, 25566] }").is_ok());
    for (cond, needle) in [
        ("{ field = \"nope\", equals = 1 }", "not a setting"),
        ("{ field = \"pvp_damage\", equals = \"x\" }", "itself"),
        ("{ field = \"admins\", equals = \"a\" }", "list setting"),
        ("{ field = \"pvp\", equals = \"yes\" }", "does not fit"),
        ("{ field = \"pvp\" }", "exactly one"),
    ] {
        let err = module(cond).unwrap_err().to_string();
        assert!(err.contains(needle), "{cond}: {err}");
    }
    let cycle = format!(
        "{v2}\n[[settings.fields]]\nname = \"a\"\nvisible_when = {{ field = \"b\", equals = \"x\" }}\n\n[[settings.fields]]\nname = \"b\"\nvisible_when = {{ field = \"a\", equals = \"x\" }}\n"
    );
    assert!(parse_module_toml(&cycle).unwrap_err().to_string().contains("cycle"));
}

#[test]
fn schema_v1_reads_names_detection_and_extension_sections() {
    let toml = format!(
        "{V1}display_name = \"G\"\ngame_name = \"Game\"\nauthor = \"me\"\n\n[detection]\ngraceful_exit_patterns = [\"Stopping\"]\ncrash_artifacts = [\"crash-reports/*.txt\"]\n\n[extensions.backup]\npaths = [\"world\"]\n"
    );
    let meta = parse_module_toml(&toml).unwrap();
    assert_eq!(meta.display_name.as_deref(), Some("G"));
    assert_eq!(meta.game_name.as_deref(), Some("Game"));
    assert_eq!(meta.graceful_exit_patterns, vec!["Stopping"]);
    assert_eq!(meta.crash_artifacts, vec!["crash-reports/*.txt"]);
    assert_eq!(meta.extensions["backup"]["paths"][0], "world");
}

/// R39: 준비 문구는 `[extensions.saba-core]`에 둔다 (0.1.8 로더가 거부하지 않는 자리). 틀린 값은 그 값만 빼고 경고다
/// 테라리아 점검: 갱신마다 새 줄을 쓰는 진행 표시는 모듈이 모양을 선언하면 러너가 단계마다 첫·마지막 줄로 접는다
#[test]
fn schema_v1_reads_progress_patterns_from_the_daemon_table() {
    let meta = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nprogress_patterns = ['^Saving \\d+%$', '(']\n")).unwrap();
    assert_eq!(meta.progress_patterns, vec!["^Saving \\d+%$".to_string()]);
    assert!(meta.warnings.iter().any(|w| w.contains("progress_patterns") && w.contains("'('")), "{:?}", meta.warnings);
    assert!(parse_module_toml(V1).unwrap().progress_patterns.is_empty());
}

/// D3·M1: 지원 OS는 `[extensions.saba-core] platforms`에 둔다 (0.1.10 로더가 거부하지 않는 자리). 없으면 막지 않는다
#[test]
fn schema_v1_reads_supported_platforms_from_the_daemon_table() {
    let meta = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nplatforms = ['windows', 'Linux', 'beos']\n")).unwrap();
    assert_eq!(meta.platforms, Some(vec!["windows".to_string(), "linux".to_string()]));
    assert!(meta.warnings.iter().any(|w| w.contains("platforms") && w.contains("beos")), "{:?}", meta.warnings);
    let os_ok = |os| meta.unsupported_on(os, "x86_64").is_none();
    assert!(os_ok("windows") && os_ok("linux") && !os_ok("macos"));
    let undeclared = parse_module_toml(V1).unwrap();
    assert_eq!(undeclared.platforms, None);
    assert_eq!(undeclared.unsupported_on("macos", "aarch64"), None, "a module that declares nothing is not blocked");
    let wrong = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nplatforms = 'linux'\n")).unwrap();
    assert_eq!(wrong.platforms, None);
    assert!(wrong.warnings.iter().any(|w| w.contains("platforms")), "{:?}", wrong.warnings);
}

/// 지원 CPU는 `platforms` 옆 `architectures`에 둔다 (`std::env::consts::ARCH` 이름). 없으면 막지 않는다
#[test]
fn schema_v1_reads_supported_architectures_from_the_daemon_table() {
    let meta = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\narchitectures = ['x86_64', 'X86_64', 'sparc']\n")).unwrap();
    assert_eq!(meta.architectures, Some(vec!["x86_64".to_string()]));
    assert!(meta.warnings.iter().any(|w| w.contains("architectures") && w.contains("sparc")), "{:?}", meta.warnings);
    assert_eq!(meta.unsupported_on("linux", "x86_64"), None);
    assert_eq!(meta.unsupported_on("linux", "aarch64"), Some("arch"));
    let undeclared = parse_module_toml(V1).unwrap();
    assert_eq!(undeclared.unsupported_on("linux", "aarch64"), None, "a module that declares nothing is not blocked");
    let aliases = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\narchitectures = ['amd64', 'arm64']\n")).unwrap();
    assert_eq!(aliases.architectures, Some(vec!["x86_64".to_string(), "aarch64".to_string()]));
    let unknown = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\narchitectures = ['sparc']\n")).unwrap();
    assert_eq!(unknown.unsupported_on("linux", "x86_64"), None, "a list with no known CPU blocks nowhere");
    let both = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nplatforms = ['windows']\narchitectures = ['x86_64']\n")).unwrap();
    assert_eq!(both.unsupported_on("linux", "aarch64"), Some("os"), "the OS is named first");
}

/// 받기 전 카탈로그 meta(`platforms`·`architectures`)도 설치된 모듈과 같은 규칙으로 판정한다 (이름 다듬기, 모르는 이름 무시)
#[test]
fn catalog_meta_is_judged_like_an_installed_module() {
    let meta = serde_json::json!({"platforms": ["Windows", "linux"], "architectures": ["amd64"]});
    assert_eq!(catalog_unsupported_on(&meta, "macos", "aarch64"), Some("os"));
    assert_eq!(catalog_unsupported_on(&meta, "linux", "aarch64"), Some("arch"));
    assert_eq!(catalog_unsupported_on(&meta, "windows", "x86_64"), None);
    assert_eq!(catalog_unsupported_on(&serde_json::json!({"architectures": ["x86_64", "aarch64"]}), "macos", "aarch64"), None);
    assert_eq!(catalog_unsupported_on(&serde_json::json!({"platforms": ["beos"]}), "macos", "aarch64"), None, "no known name blocks nowhere");
    assert_eq!(catalog_unsupported_on(&serde_json::json!({}), "macos", "aarch64"), None, "an old catalog without the keys blocks nowhere");
}

#[test]
fn schema_v1_reads_ready_patterns_from_the_daemon_table() {
    let toml = format!("{V1}\n[extensions.saba-core]\nready_patterns = ['Done \\(', '(']\nready_timeout = 120\n");
    let meta = parse_module_toml(&toml).unwrap();
    assert_eq!((meta.ready_patterns.clone(), meta.ready_timeout), (vec!["Done \\(".to_string()], Some(120)));
    assert!(meta.warnings.iter().any(|w| w.contains("'('")), "{:?}", meta.warnings);
    let zero = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nready_timeout = 0\n")).unwrap();
    assert_eq!(zero.ready_timeout, None);
    assert!(zero.warnings.iter().any(|w| w.contains("ready_timeout")));
    let long = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nready_timeout = 3600\n")).unwrap();
    assert_eq!(long.ready_timeout, Some(900), "a start notice is only remembered for 15 minutes");
    assert!(long.warnings.iter().any(|w| w.contains("at most 900")), "{:?}", long.warnings);
    assert_eq!(parse_module_toml(V1).unwrap().ready_patterns, Vec::<String>::new());
    let later = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nready_timeout = 30\nfrom_a_later_core = true\n")).unwrap();
    assert_eq!(later.ready_timeout, Some(30), "a key from a newer daemon does not stop the module from loading");
    assert!(later.warnings.iter().any(|w| w.contains("from_a_later_core")), "{:?}", later.warnings);
    assert!(!crate::extension::is_valid_extension_id(DAEMON_TABLE), "the daemon table name is reserved");
}

#[test]
fn contract_paths_data_roots_reject_only_invalid_entries() {
    let source = format!("{V1}\n[[settings.fields]]\nname = 'cache_dir'\ntype = 'file'\n\n[[settings.fields]]\nname = 'title'\ntype = 'text'\n\n[extensions.saba-core]\ndata_roots = ['{{cache_dir}}', '{{home}}/Worlds', '/tmp', '{{cache_dir}}/../escape', '{{title}}', '{{missing}}', '{{cache_dir}}/C:/escape', '{{home}}/{{cache_dir}}', 1]\n");
    let meta = parse_module_toml(&source).unwrap();
    assert_eq!(serde_json::to_value(&meta).unwrap()["data_roots"], serde_json::json!(["{cache_dir}", "{home}/Worlds"]));
    assert_eq!(meta.warnings.len(), 7, "{:?}", meta.warnings);
}

#[test]
fn the_minecraft_ready_line_matches_what_the_console_holds() {
    // minecraft/module.toml 표 원문 (모듈 main edc7004). 콘솔 줄은 게임 stdout 한 줄 그대로다(러너 머리 `[날짜] [STDOUT/INFO]`는 떼고 준다).
    // 바닐라·Paper 머리에 Forge·NeoForge의 로거 칸 ` [minecraft/로거]`까지 받는다
    let table = r#"
[extensions.saba-core]
ready_patterns = ['^\[\d{2}:\d{2}:\d{2}(?:\] \[[^\]]*/| )INFO\](?: \[[^\]]*\])?: Done \(\d+(\.\d+)?s\)! For help, type "help"']
player_join_patterns = [
'^\[\d{2}:\d{2}:\d{2}(?:\] \[[^\]]*/| )INFO\](?: \[[^\]]*\])?: (?P<name>[A-Za-z0-9_]{1,16}) joined the game\s*$',
'^\[\d{2}:\d{2}:\d{2}(?:\] \[[^\]]*/| )INFO\](?: \[[^\]]*\])?: (?P<name>[A-Za-z0-9_]{1,16}) \(formerly known as [A-Za-z0-9_]{1,16}\) joined the game\s*$',
]
player_leave_patterns = ['^\[\d{2}:\d{2}:\d{2}(?:\] \[[^\]]*/| )INFO\](?: \[[^\]]*\])?: (?P<name>[A-Za-z0-9_]{1,16}) left the game\s*$']
"#;
    // 같은 표를 `[patterns]` 조각으로 쓴 것: 맞는 줄이 같아야 한다
    let with_fragments = r#"
[patterns]
log_head = '^\[\d{2}:\d{2}:\d{2}(?:\] \[[^\]]*/| )INFO\](?: \[[^\]]*\])?: '
player_name = '[A-Za-z0-9_]{1,16}'

[extensions.saba-core]
ready_patterns = ['{log_head}Done \(\d+(\.\d+)?s\)! For help, type "help"']
player_join_patterns = [
'{log_head}(?P<name>{player_name}) joined the game\s*$',
'{log_head}(?P<name>{player_name}) \(formerly known as {player_name}\) joined the game\s*$',
]
player_leave_patterns = ['{log_head}(?P<name>{player_name}) left the game\s*$']
"#;
    for table in [table, with_fragments] {
        let meta = parse_module_toml(&format!("{V1}{table}")).unwrap();
        let ready = regex::Regex::new(&meta.ready_patterns[0]).unwrap();
        assert!(ready.is_match(r#"[20:07:01] [Server thread/INFO]: Done (0.267s)! For help, type "help""#));
        assert!(ready.is_match(r#"[20:07:01 INFO]: Done (12.5s)! For help, type "help""#), "Paper's header");
        assert!(ready.is_match(r#"[20:07:01] [Server thread/INFO] [minecraft/DedicatedServer]: Done (9.1s)! For help, type "help""#), "Forge's logger field");
        for chat in [
            r#"[20:07:05] [Server thread/INFO]: <Steve> Done (0.267s)! For help, type "help""#,
            r#"[20:07:05] [Server thread/INFO]: <Steve> ]: Done (0.267s)! For help, type "help""#,
            r#"[20:07:05] [Server thread/INFO]: [Not Secure] <Steve> ]: Done (0.267s)! For help, type "help""#,
            r#"[20:07:05] [Server thread/INFO] [minecraft/MinecraftServer]: <Steve> [x]: Done (0.267s)! For help, type "help""#,
        ] {
            assert!(!ready.is_match(chat), "a chat line is not the server: {chat}");
        }
        assert!(!ready.is_match("[20:06:58] [Server thread/INFO]: Preparing level \"world\""));
        assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
        assert_eq!((meta.player_join_patterns.len(), meta.player_leave_patterns.len()), (2, 1));
        let players = crate::supervisor::player_events::Patterns::new(&meta.player_join_patterns, &meta.player_leave_patterns).unwrap();
        use crate::supervisor::player_events::Change::{Join, Leave};
        assert_eq!(players.parse("[20:07:05] [Server thread/INFO]: Steve joined the game"), Some(("Steve".into(), Join)));
        assert_eq!(players.parse("[20:07:05 INFO]: Steve joined the game"), Some(("Steve".into(), Join)), "Paper's header");
        assert_eq!(players.parse("[20:07:05] [Server thread/INFO] [minecraft/MinecraftServer]: Steve joined the game"), Some(("Steve".into(), Join)), "Forge's logger field");
        assert_eq!(players.parse("[20:07:06] [Server thread/INFO]: Alex (formerly known as Al) joined the game"), Some(("Alex".into(), Join)));
        assert_eq!(players.parse("[20:07:09] [Server thread/INFO]: Steve left the game"), Some(("Steve".into(), Leave)));
        for chat in [
            "[20:07:07] [Server thread/INFO]: <Steve> ]: Notch joined the game",
            "[20:07:07] [Server thread/INFO]: [Not Secure] <Steve> ]: Notch joined the game",
            "[20:07:07] [Server thread/INFO]: <Steve> Notch left the game",
            "[20:07:07] [Server thread/INFO]: <Steve> [x]: Notch joined the game",
            "[20:07:07] [Server thread/INFO] [minecraft/MinecraftServer]: <Steve> Notch joined the game",
            "[20:07:07] [Server thread/INFO] [minecraft/MinecraftServer]: <Steve> [20:07:07] [Server thread/INFO]: Notch joined the game",
        ] {
            assert_eq!(players.parse(chat), None, "chat is not a player: {chat}");
        }
    }
}

/// `[patterns]` 조각은 데몬이 읽는 모든 정규식 키에서 `{이름}`으로 쓴다. 반복 횟수 `{2}`·`{1,16}`, 선언하지 않은 이름,
/// `\{`로 막은 자리, 정규식이 아닌 키(`[backup] paths`의 `{설정 이름}`)는 그대로다
#[test]
fn pattern_fragments_expand_in_every_regex_key() {
    let source = format!(
        r#"{V1}
[patterns]
head = '^\[\d{{2}}:\d{{2}}\] '
name = '[A-Za-z0-9_]{{1,16}}'
greeting = '{{head}}hello {{name}}'

[launch]
line_start = '{{head}}'

[detection]
graceful_exit_patterns = ['{{head}}bye']

[backup]
paths = ['{{name}}']
save_done_pattern = '{{head}}saved'

[[settings.fields]]
name = 'admin'
pattern = '^{{name}}$'

[[commands.fields]]
name = 'save'
method = 'console'
console_template = 'save'
reply_pattern = '{{head}}(?P<what>{{name}}) saved'

[commands]
failure_patterns = ['{{head}}denied']

[[syntax_highlight.rules]]
name = 'time'
pattern = '{{head}}'
color = '#888888'

[extensions.saba-core]
ready_patterns = ['{{greeting}}!', '{{head}}x{{2}}', '{{head}}\{{name\}} \{{head}}']
chat_patterns = ['{{head}}<(?P<name>{{name}})> (?P<message>.*)']
progress_patterns = ['{{head}}{{progress}}']
"#
    );
    let meta = parse_module_toml(&source).unwrap();
    // 선언하지 않은 이름은 쓴 그대로 정규식이 된다 (Rust `regex`에선 틀린 반복이라 그 항목만 버린다)
    assert_eq!(meta.warnings, vec!["extensions.saba-core.progress_patterns '{head}{progress}' is not a valid regex; ignored".to_string()]);
    let matches = |pattern: &str, line: &str| regex::Regex::new(pattern).unwrap().is_match(line);
    let ready = &meta.ready_patterns;
    assert!(matches(&ready[0], "[12:34] hello Steve!"), "a fragment may use another fragment");
    assert!(matches(&ready[1], "[12:34] xx") && !matches(&ready[1], "[12:34] x{2}"), "a quantifier is not a name");
    assert!(matches(&ready[2], "[12:34] {name} {head}"), "an escaped brace stays literal: {}", ready[2]);
    assert!(matches(&meta.chat_patterns[0], "[12:34] <Steve> hi"));
    assert!(matches(meta.line_start.as_deref().unwrap(), "[12:34] "));
    assert!(matches(&meta.graceful_exit_patterns[0], "[12:34] bye"));
    let backup = meta.backup.as_ref().unwrap();
    assert!(matches(backup.save_done_pattern.as_deref().unwrap(), "[12:34] saved"));
    assert_eq!(backup.paths, vec!["{name}".to_string()], "only regex keys use fragments");
    let admin = &meta.settings.as_ref().unwrap().fields[0];
    assert!(matches(admin.pattern.as_deref().unwrap(), "Steve") && !matches(admin.pattern.as_deref().unwrap(), "Steve!"));
    let commands = meta.commands.as_ref().unwrap();
    assert!(matches(commands.fields[0].reply_pattern.as_deref().unwrap(), "[12:34] world saved"));
    assert!(matches(&commands.failure_patterns[0], "[12:34] denied"));
    assert!(matches(&meta.syntax_highlight.as_ref().unwrap().rules[0].pattern, "[12:34] "));
}

/// 자기 자신을 (다른 조각을 거쳐서라도) 참조하는 조각과 이름이나 정규식이 틀린 조각은 스키마 오류다.
/// 스키마 이전 모듈은 경고만 남기고 그 조각의 `{이름}`을 글자 그대로 둔다
#[test]
fn pattern_fragments_that_refer_to_themselves_are_rejected() {
    let fragments = "[patterns]\nloop = 'a{loop}'\nping = '{pong}'\npong = '{ping}b'\nuser = '{ping}c'\n'1st' = 'x'\nbroken = '('\nfine = 'ok'\n\n[extensions.saba-core]\nready_patterns = ['{fine}']\n";
    let err = parse_module_toml(&format!("{V1}\n{fragments}")).unwrap_err();
    let failed = ModuleLoadError::new(Path::new("m"), &err);
    assert_eq!(failed.error_code.as_deref(), Some("module_schema_invalid"));
    for needle in ["patterns.loop refers to itself", "patterns.ping refers to itself", "patterns.1st", "patterns.broken"] {
        assert!(failed.problems.iter().any(|p| p.contains(needle)), "{needle}: {:?}", failed.problems);
    }
    assert!(!failed.problems.iter().any(|p| p.contains("patterns.fine")), "{:?}", failed.problems);

    let legacy = parse_module_toml(&format!("{V1}\n{fragments}").replacen("schema_version = 1\n", "", 1)).unwrap();
    assert!(legacy.warnings.iter().any(|w| w.contains("patterns.loop refers to itself")), "{:?}", legacy.warnings);
    assert_eq!(legacy.ready_patterns, vec!["(?:ok)".to_string()]);
}

/// 조각을 풀어 늘어나는 글자는 모듈마다 `PATTERN_EXPANSION_LIMIT`까지다. 넘기는 조각과 키는 다른 조각 오류처럼 다룬다
#[test]
fn pattern_fragments_stop_growing_at_the_limit() {
    // 조각마다 앞 조각을 두 번 써서 단계마다 두 배가 된다
    let mut doubling = "[patterns]\np0 = 'aaaaaaaaaaaaaaaa'\n".to_string();
    for i in 1..=20 {
        doubling.push_str(&format!("p{i} = '{{p{}}}{{p{}}}'\n", i - 1, i - 1));
    }
    let err = parse_module_toml(&format!("{V1}\n{doubling}")).unwrap_err();
    let failed = ModuleLoadError::new(Path::new("m"), &err);
    assert!(failed.problems.iter().any(|p| p.starts_with("patterns.p") && p.contains("expand to more than")), "{:?}", failed.problems);

    // 조각 하나는 작아도 여러 키에서 거듭 쓰면 같은 한도에 걸린다
    let wide = format!("[patterns]\nbig = '{}'\n\n[extensions.saba-core]\nready_patterns = [{}]\n", "a".repeat(4096), vec!["'{big}'"; 300].join(", "));
    let legacy = parse_module_toml(&format!("{V1}\n{wide}").replacen("schema_version = 1\n", "", 1)).unwrap();
    assert!(legacy.ready_patterns.len() < 300 && !legacy.ready_patterns.is_empty(), "{}", legacy.ready_patterns.len());
    assert!(legacy.warnings.iter().any(|w| w.contains("ready_patterns '{big}'") && w.contains("expand to more than")), "{:?}", legacy.warnings.first());
}

/// MK8 §8.3: 접속·퇴장 문구도 `[extensions.saba-core]`에 둔다. `(?P<name>…)`이 없거나 정규식이 아니면 그 항목만 버린다
#[test]
fn schema_v1_reads_player_patterns_from_the_daemon_table() {
    let table = "[extensions.saba-core]\nplayer_join_patterns = ['(?P<name>\\w+) joined', '(\\w+) joined', '(']\nplayer_leave_patterns = '(?P<name>\\w+) left'\n";
    let meta = parse_module_toml(&format!("{V1}\n{table}")).unwrap();
    assert_eq!(meta.player_join_patterns, vec!["(?P<name>\\w+) joined".to_string()]);
    assert_eq!(meta.player_leave_patterns, Vec::<String>::new());
    for needle in ["'(\\w+) joined'", "'('", "player_leave_patterns must be a list"] {
        assert!(meta.warnings.iter().any(|w| w.contains(needle)), "{needle}: {:?}", meta.warnings);
    }
}

/// 게임 채팅 전달: `chat_patterns`는 `name`과 `message` 그룹이 둘 다 있어야 한다
#[test]
fn schema_v1_reads_chat_patterns_that_name_the_player_and_the_message() {
    let table = "[extensions.saba-core]\nchat_patterns = ['^<(?P<name>\\w+)> (?P<message>.*)$', '^<(?P<name>\\w+)> .*$']\n";
    let meta = parse_module_toml(&format!("{V1}\n{table}")).unwrap();
    assert_eq!(meta.chat_patterns, vec!["^<(?P<name>\\w+)> (?P<message>.*)$".to_string()]);
    assert!(meta.warnings.iter().any(|w| w.contains("(?P<message>...)")), "{:?}", meta.warnings);
}

/// 디스코드 → 게임 채팅: `chat_send`는 lifecycle 함수 이름이다. 함수 이름이 아니면 버린다
#[test]
fn schema_v1_reads_the_chat_send_function() {
    let meta = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nchat_send = \"send_chat\"\n")).unwrap();
    assert_eq!(meta.chat_send_function.as_deref(), Some("send_chat"));
    let bad = parse_module_toml(&format!("{V1}\n[extensions.saba-core]\nchat_send = \"rm -rf\"\n")).unwrap();
    assert_eq!(bad.chat_send_function, None);
    assert!(bad.warnings.iter().any(|w| w.contains("chat_send")), "{:?}", bad.warnings);
}

#[test]
fn schema_v1_reads_backup_and_checks_command_refs() {
    let toml = format!(
        "{V1}\n[config]\nannounce_command = \"say\"\n\n[backup]\npaths = [\"{{level_name}}\"]\nsave_command = \"save-all\"\n\n[[commands.fields]]\nname = \"say\"\n\n[[commands.fields]]\nname = \"save-all\"\n"
    );
    let meta = parse_module_toml(&toml).unwrap();
    let backup = meta.backup.unwrap();
    assert_eq!(backup.paths, vec!["{level_name}"]);
    assert_eq!(backup.save_command.as_deref(), Some("save-all"));
    assert!(!backup.hook);
    assert_eq!(meta.announce_command.as_deref(), Some("say"));
    let typo = toml.replace("save_command = \"save-all\"", "save_command = \"save_all\"");
    assert!(parse_module_toml(&typo).unwrap_err().to_string().contains("backup.save_command"));
}

#[test]
fn schema_v1_reads_kick_command_and_checks_it_names_a_command() {
    let toml = format!("{V1}\n[config]\nkick_command = \"kick\"\n\n[[commands.fields]]\nname = \"kick\"\n");
    assert_eq!(parse_module_toml(&toml).unwrap().kick_command.as_deref(), Some("kick"));
    let typo = toml.replace("kick_command = \"kick\"", "kick_command = \"kik\"");
    assert!(parse_module_toml(&typo).unwrap_err().to_string().contains("config.kick_command"));
}

#[test]
fn schema_v1_reads_live_backup_keys() {
    let toml = format!(
        "{V1}\n[backup]\npaths = [\"world\"]\nexclude = [\"session.lock\"]\npause_command = \"save-off\"\nsave_command = \"save-all\"\nresume_command = \"save-on\"\n\n[[commands.fields]]\nname = \"save-all\"\n\n[[commands.fields]]\nname = \"save-off\"\n\n[[commands.fields]]\nname = \"save-on\"\n"
    );
    let backup = parse_module_toml(&toml).unwrap().backup.unwrap();
    assert_eq!(backup.exclude, vec!["session.lock"]);
    assert_eq!((backup.pause_command.as_deref(), backup.resume_command.as_deref()), (Some("save-off"), Some("save-on")));
    let typo = toml.replace("resume_command = \"save-on\"", "resume_command = \"save_on\"");
    assert!(parse_module_toml(&typo).unwrap_err().to_string().contains("backup.resume_command"));
    // 멈춤만 있고 되돌릴 명령이 없으면 자동 저장이 꺼진 채 남는다
    let unpaired = toml.replace("resume_command = \"save-on\"\n", "");
    assert!(parse_module_toml(&unpaired).unwrap_err().to_string().contains("backup.pause_command"));
    let done = toml.replace("exclude =", "save_done_pattern = 'Saved the game$'\nexclude =");
    assert_eq!(parse_module_toml(&done).unwrap().backup.unwrap().save_done_pattern.as_deref(), Some("Saved the game$"));
    let bad = toml.replace("exclude =", "save_done_pattern = 'Saved ('\nexclude =");
    assert!(parse_module_toml(&bad).unwrap_err().to_string().contains("backup.save_done_pattern"));
}

#[test]
fn schema_v1_reads_presets_and_rejects_bad_ones() {
    let toml = format!(
        "{V1}\n[[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\nmin = 1\nmax = 100\n\n[[settings.fields]]\nname = \"difficulty\"\ntype = \"select\"\noptions = [\"easy\", \"hard\"]\n\n[[settings.fields]]\nname = \"secret\"\ntype = \"password\"\n\n[[presets]]\nid = \"friends\"\nlabel = \"Friends\"\n[presets.settings]\nmax_players = 10\ndifficulty = \"easy\"\n"
    );
    let meta = parse_module_toml(&toml).unwrap();
    assert_eq!(meta.presets.len(), 1);
    assert_eq!(meta.presets[0].id, "friends");
    assert_eq!(meta.presets[0].settings["max_players"], 10);
    for (bad, why) in [
        ("max_players = 10", "max_players = 500"),
        ("difficulty = \"easy\"", "difficulty = \"peaceful\""),
        ("difficulty = \"easy\"", "secret = \"x\""),
        ("difficulty = \"easy\"", "nope = 1"),
        ("id = \"friends\"", "id = \"Friends!\""),
    ] {
        let broken = toml.replace(bad, why);
        assert!(parse_module_toml(&broken).unwrap_err().to_string().contains("presets"), "{why}");
    }
}

#[test]
fn schema_v1_reads_queries_and_rejects_bad_ones() {
    let toml = format!(
        "{V1}\n[[queries]]\nname = \"players\"\nlabel = \"Players\"\nfunction = \"list_players\"\n\n[[queries]]\nname = \"player-info\"\nfunction = \"player_info\"\ninputs = [{{ name = \"player\", type = \"text\", required = true }}, {{ name = \"limit\", type = \"number\", min = 1, max = 50 }}]\n"
    );
    let meta = parse_module_toml(&toml).unwrap();
    assert_eq!(meta.queries.len(), 2);
    assert_eq!((meta.queries[0].name.as_str(), meta.queries[0].function.as_str()), ("players", "list_players"));
    assert_eq!(meta.queries[1].inputs[1].max, Some(50.0));
    for (bad, why) in [
        ("name = \"player-info\"", "name = \"players\""),
        ("name = \"players\"", "name = \"Players!\""),
        ("function = \"list_players\"", "function = \"list-players\""),
        ("function = \"list_players\"", "function = \"stop\""),
        ("type = \"text\"", "type = \"password\""),
        ("name = \"limit\"", "name = \"player\""),
        ("min = 1, max = 50", "min = 9, max = 2"),
    ] {
        let broken = toml.replacen(bad, why, 1);
        assert!(parse_module_toml(&broken).unwrap_err().to_string().contains("queries"), "{why}");
    }
    // 조회에는 위험도가 없다: 모르는 키로 경고만 남긴다
    let meta = parse_module_toml(&format!("{V1}\n[[queries]]\nname = \"p\"\nfunction = \"f\"\nrisk = \"safe\"\n")).unwrap();
    assert!(meta.warnings.iter().any(|w| w.contains("risk")), "{:?}", meta.warnings);
    // 부른 사람만 볼 조회는 `private = true` (안 쓰면 다 같이 본다)
    assert!(!meta.queries[0].private);
    let meta = parse_module_toml(&format!("{V1}\n[[queries]]\nname = \"p\"\nfunction = \"f\"\nprivate = true\n")).unwrap();
    assert!(meta.queries[0].private && meta.warnings.is_empty(), "{:?}", meta.warnings);
    let legacy = toml.replacen("schema_version = 1\n", "", 1).replacen("function = \"list_players\"", "function = \"reset_server\"", 1);
    assert_ne!(legacy, toml);
    let meta = parse_module_toml(&legacy).unwrap();
    assert!(meta.queries.is_empty(), "a module without a schema keeps no queries when one is wrong");
}

#[test]
fn a_module_names_the_function_that_counts_players() {
    let toml = format!("{V1}\n[status]\nplayers = \"player_count\"\n");
    assert_eq!(parse_module_toml(&toml).unwrap().players_function.as_deref(), Some("player_count"));
    assert_eq!(parse_module_toml(V1).unwrap().players_function, None);
    for bad in ["stop", "player-count"] {
        let err = parse_module_toml(&toml.replace("player_count", bad)).unwrap_err().to_string();
        assert!(err.contains("status.players"), "{err}");
    }
    let legacy = toml.replacen("schema_version = 1\n", "", 1).replace("player_count", "reset_server");
    assert_eq!(parse_module_toml(&legacy).unwrap().players_function, None, "never call a function that changes the server");
}

#[test]
fn schema_v1_reads_agreements_and_rejects_bad_ones() {
    let toml = format!("{V1}\n[[agreements]]\nid = \"eula\"\nurl = \"https://example.com/eula\"\n");
    let meta = parse_module_toml(&toml).unwrap();
    assert_eq!(meta.agreements, vec![ModuleAgreement { id: "eula".into(), url: "https://example.com/eula".into() }]);
    for (bad, why) in [("id = \"eula\"", "id = \"EULA\""), ("https://", "http://")] {
        assert!(parse_module_toml(&toml.replacen(bad, why, 1)).is_err(), "{why}");
    }
    let titled = parse_module_toml(&toml.replacen("url = ", "title = \"x\"\nurl = ", 1)).unwrap();
    assert!(titled.warnings.iter().any(|w| w.contains("title")), "{:?}", titled.warnings);
    let twice = format!("{toml}\n[[agreements]]\nid = \"eula\"\nurl = \"https://example.com/2\"\n");
    assert!(parse_module_toml(&twice).unwrap_err().to_string().contains("duplicate"));
}

/// 플레이어 목록(`[[lists]]`): 넣기·빼기는 이미 있는 명령 이름, 켜고 끄는 칸은 boolean 설정 이름이어야 한다
#[test]
fn schema_v1_reads_lists_and_rejects_bad_ones() {
    let toml = format!(
        "{V1}\n[[settings.fields]]\nname = \"white_list\"\ntype = \"boolean\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n\n\
         [[commands.fields]]\nname = \"allow\"\nlabel = \"Allow\"\nmethod = \"console\"\nconsole_template = \"allow {{player}}\"\ninputs = [{{ name = \"player\", type = \"text\", required = true }}]\n\n\
         [[commands.fields]]\nname = \"save\"\nlabel = \"Save\"\nmethod = \"console\"\nconsole_template = \"save\"\n\n\
         [[lists]]\nid = \"whitelist\"\nadd = \"allow\"\nremove = \"allow\"\nsetting = \"white_list\"\n"
    );
    let meta = parse_module_toml(&toml).unwrap();
    assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
    assert_eq!(meta.lists, vec![ModuleList { id: "whitelist".into(), add: "allow".into(), remove: "allow".into(), setting: Some("white_list".into()), danger: false }]);
    for (good, bad, why) in [
        ("id = \"whitelist\"", "id = \"White List\"", "id"),
        ("add = \"allow\"", "add = \"missing\"", "add"),
        ("remove = \"allow\"", "remove = \"save\"", "remove"),
        ("setting = \"white_list\"", "setting = \"motd\"", "setting"),
    ] {
        let err = parse_module_toml(&toml.replacen(good, bad, 1)).unwrap_err().to_string();
        assert!(err.contains("lists") && err.contains(why), "{why}: {err}");
    }
    let twice = format!("{toml}\n[[lists]]\nid = \"whitelist\"\nadd = \"allow\"\nremove = \"allow\"\n");
    assert!(parse_module_toml(&twice).unwrap_err().to_string().contains("duplicate"));
    // 스키마 이전 모듈의 잘못된 목록은 켜지 않는다
    let legacy = toml.replacen("schema_version = 1\n", "", 1).replacen("add = \"allow\"", "add = \"missing\"", 1);
    assert!(parse_module_toml(&legacy).unwrap().lists.is_empty());
}

#[test]
fn json_schema_forbids_unknown_keys() {
    let schema = module_json_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"], serde_json::json!(["module"]));
    let defs = &schema["definitions"];
    assert_eq!(defs["ConfigSection"]["additionalProperties"], false);
    assert!(defs["ModuleSection"]["properties"]["schema_version"].is_object());
}

#[test]
fn loader_reports_broken_modules_and_missing_entry() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::write(good.join("module.toml"), V1.replace("\"g\"", "\"good\"")).unwrap();
    std::fs::write(good.join("lifecycle.py"), "").unwrap();
    std::fs::write(good.join("server.jar"), "").unwrap();
    let no_entry = dir.path().join("noentry");
    std::fs::create_dir_all(&no_entry).unwrap();
    std::fs::write(no_entry.join("module.toml"), V1).unwrap();
    let loader = ModuleLoader::new(dir.path().to_str().unwrap());
    let modules = loader.discover_modules().unwrap();
    assert_eq!(modules.len(), 1);
    assert!(modules[0].entry_path().ends_with("lifecycle.py"));
    let errors = loader.load_errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].error.contains("not found"), "{:?}", errors);

    let mut m = modules[0].clone();
    assert!(!m.matches_dir(&good), "no signatures, no match");
    m.metadata.dir_signatures = vec!["server.jar".into(), "lifecycle.py".into()];
    assert!(m.matches_dir(&good));
    m.metadata.dir_signatures.push("missing.txt".into());
    assert!(!m.matches_dir(&good));

    // 한 항목 안의 `|`는 그중 하나, `*`는 파일 이름 패턴 (Paper·Fabric처럼 jar 이름이 버전마다 다른 서버)
    std::fs::write(good.join("paper-1.21.4-15.jar"), "").unwrap();
    m.metadata.dir_signatures = vec!["missing.jar|paper-*.jar".into(), "lifecycle.py".into()];
    assert!(m.matches_dir(&good));
    m.metadata.dir_signatures = vec!["missing.jar|purpur-*.jar".into()];
    assert!(!m.matches_dir(&good));
    m.metadata.dir_signatures = vec!["../*".into()];
    assert!(!m.matches_dir(&good), "patterns stay inside the folder");
}

/// 하위 폴더로 감싼 ZIP(`probe/module.toml`)은 고른 module.toml의 폴더를 모듈 폴더로 쓴다
#[test]
fn a_module_zip_wrapped_in_a_folder_loads() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut writer = zip::ZipWriter::new(std::fs::File::create(dir.path().join("probe.zip")).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("probe/module.toml", options).unwrap();
    writer.write_all(V1.replace("\"g\"", "\"probe\"").as_bytes()).unwrap();
    writer.start_file("probe/lifecycle.py", options).unwrap();
    writer.write_all(b"").unwrap();
    writer.finish().unwrap();
    let loader = ModuleLoader::new(dir.path().to_str().unwrap());
    let modules = loader.discover_modules().unwrap();
    assert_eq!(modules.len(), 1, "{:?}", loader.load_errors());
    assert!(Path::new(&modules[0].entry_path()).is_file());
    assert_eq!(Path::new(&modules[0].source), dir.path().join("probe.zip"));
}

/// 내려받은 모듈 ZIP 안의 링크는 풀지 않고 이유를 남긴다 (폴더 밖 파일을 가리킬 수 있다)
#[test]
fn module_zip_with_a_link_is_refused() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut writer = zip::ZipWriter::new(std::fs::File::create(dir.path().join("linked.zip")).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("module.toml", options).unwrap();
    writer.write_all(V1.replace("\"g\"", "\"linked\"").as_bytes()).unwrap();
    writer.start_file("lifecycle.py", options).unwrap();
    writer.add_symlink("escape", "/etc/passwd", options).unwrap();
    writer.finish().unwrap();
    let loader = ModuleLoader::new(dir.path().to_str().unwrap());
    assert!(loader.discover_modules().unwrap().is_empty());
    let errors = loader.load_errors();
    assert!(errors[0].error.contains("contains a link"), "{:?}", errors);
    assert!(!dir.path().join(".extracted/linked").exists());
}

/// 모듈이 더 새 saba-core를 요구하면 로드하지 않고 이유를 남긴다 (M9)
#[test]
fn module_needing_newer_core_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    for (name, need) in [("fits", DAEMON_VERSION.to_string()), ("future", "99.0.0".to_string())] {
        let m = dir.path().join(name);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(
            m.join("module.toml"),
            format!("{}\n[module.dependencies]\n\"saba-core\" = \">={}\"\n", V1.replace("\"g\"", &format!("\"{name}\"")), need),
        )
        .unwrap();
        std::fs::write(m.join("lifecycle.py"), "").unwrap();
    }
    let loader = ModuleLoader::new(dir.path().to_str().unwrap());
    let modules = loader.discover_modules().unwrap();
    assert_eq!(modules.iter().map(|m| m.metadata.name.as_str()).collect::<Vec<_>>(), ["fits"]);
    let errors = loader.load_errors();
    assert!(errors[0].error.contains("requires saba-core >=99.0.0"), "{:?}", errors);
}

#[test]
fn test_module_check_dependencies_cross_type() {
    let toml = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"gui" = ">=0.2.0"
"ext-steamcmd" = ">=1.0.0"
"cli" = ">=0.1.0"
"#;
    let meta = parse_module_toml(toml).unwrap();
    let mut installed = std::collections::HashMap::new();
    installed.insert("saba-core".to_string(), "0.5.0".to_string());
    installed.insert("gui".to_string(), "0.3.0".to_string());
    installed.insert("ext-steamcmd".to_string(), "1.0.0".to_string());
    installed.insert("cli".to_string(), "0.1.0".to_string());

    let issues = meta.check_dependencies(&installed);
    assert!(issues.is_empty(), "All cross-type dependencies should be satisfied");

    // Now test with one missing
    let mut partial = std::collections::HashMap::new();
    partial.insert("saba-core".to_string(), "0.5.0".to_string());
    partial.insert("gui".to_string(), "0.1.0".to_string()); // too low
    partial.insert("ext-steamcmd".to_string(), "1.0.0".to_string());
    // cli missing entirely

    let issues = meta.check_dependencies(&partial);
    assert_eq!(issues.len(), 2, "gui too low + cli missing");
}
