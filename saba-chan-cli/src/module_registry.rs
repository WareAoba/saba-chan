//! 모듈 레지스트리 — module.toml에서 별명·명령어를 읽어 매핑 테이블 생성
//!
//! `palworld start`, `팰월드 시작`, `mc save` 같은 입력을 해석할 수 있습니다.
//! 모듈 별명, 명령어 별명 모두 지원합니다.
//!
//! 로드 순서: 데몬 `GET /api/modules` → 각 모듈의 `path/module.toml`을 파싱
//! (표시 이름·EULA 정보 포함). TOML을 읽을 수 없으면 API 응답 필드로 대체한다.
//! 데몬에 연결할 수 없으면 modules 디렉토리를 직접 스캔한다.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

// ═══════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════

/// 명령어 입력 필드 정의 (`[[commands.fields]]` → `inputs`)
#[derive(Debug, Clone)]
pub struct CommandInput {
    pub name: String,
    pub input_type: String, // "text", "number", "select", "password", "file"
    pub required: bool,
    pub default: Option<Value>,
}

/// 모듈 명령어 정의 (`[[commands.fields]]`)
#[derive(Debug, Clone)]
pub struct ModuleCommand {
    pub name: String,
    pub description: String,
    pub method: String, // "rest", "rcon", "stdin", "dual"
    pub inputs: Vec<CommandInput>,
}

/// 모듈 정보 (module.toml 한 개 분량)
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub display_name: String,
    pub interaction_mode: Option<String>, // "console" or "commands"
    pub commands: Vec<ModuleCommand>,
    pub requires_eula: bool,
}

impl ModuleInfo {
    /// 모듈의 명령어 중 RCON 메서드를 사용하는 것이 있는지 확인
    pub fn has_rcon_commands(&self) -> bool {
        self.commands.iter().any(|c| c.method == "rcon")
    }
}

/// 모듈 하나를 파싱한 결과 (레지스트리에 등록하기 전 단계)
struct ParsedModule {
    info: ModuleInfo,
    /// 모듈 별명 (name, game_name, display_name 포함)
    aliases: Vec<String>,
    /// canonical 명령어 이름 → 별명 목록
    cmd_aliases: HashMap<String, Vec<String>>,
}

/// 모듈 레지스트리 — 전역 별명 테이블
#[derive(Default)]
pub struct ModuleRegistry {
    pub modules: Vec<ModuleInfo>,
    /// lowercase alias → canonical module name
    alias_to_module: HashMap<String, String>,
    /// module_name → (lowercase alias → canonical command name)
    cmd_aliases: HashMap<String, HashMap<String, String>>,
}

// 라이프사이클 명령어 (모든 모듈 공통)
pub const LIFECYCLE_COMMANDS: &[&str] = &["start", "stop", "restart", "status"];

// ═══════════════════════════════════════════════════════
// Parsing helpers
// ═══════════════════════════════════════════════════════

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str()).filter(|s| !s.is_empty())
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

/// `commands.fields` 배열 → 명령어 정의 (TOML/API 공통 레이아웃)
fn parse_commands(commands: Option<&Value>) -> Vec<ModuleCommand> {
    let Some(fields) = commands.and_then(|c| c.get("fields")).and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(|field| {
            let name = str_at(field, "name")?.to_string();
            let inputs = field
                .get("inputs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|inp| CommandInput {
                            name: str_at(inp, "name").unwrap_or("").to_string(),
                            input_type: str_at(inp, "type").unwrap_or("text").to_string(),
                            required: inp.get("required").and_then(|v| v.as_bool()).unwrap_or(false),
                            default: inp.get("default").cloned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(ModuleCommand {
                name,
                description: str_at(field, "description").unwrap_or("").to_string(),
                method: str_at(field, "method").unwrap_or("").to_string(),
                inputs,
            })
        })
        .collect()
}

/// `{cmd: {aliases: [...]}}` 객체 → 명령어 별명 맵
fn parse_cmd_aliases(obj: Option<&Value>) -> HashMap<String, Vec<String>> {
    obj.and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(cmd, val)| (cmd.clone(), string_array(val.get("aliases"))))
                .collect()
        })
        .unwrap_or_default()
}

/// module.toml(JSON으로 변환한 값) → 모듈 정보
fn parse_module_toml_value(table: &Value) -> Option<ParsedModule> {
    let module = table.get("module")?;
    let name = str_at(module, "name")?.to_string();
    let game_name = str_at(module, "game_name").unwrap_or(&name).to_string();
    let display_name = str_at(module, "display_name").unwrap_or(&name).to_string();
    let aliases_section = table.get("aliases");

    let mut aliases = vec![name.clone(), game_name, display_name.clone()];
    aliases.extend(string_array(aliases_section.and_then(|a| a.get("module_aliases"))));

    Some(ParsedModule {
        info: ModuleInfo {
            interaction_mode: table
                .get("protocols")
                .and_then(|p| str_at(p, "interaction_mode"))
                .map(|s| s.to_string()),
            requires_eula: table
                .get("config")
                .and_then(|c| c.get("eula"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            commands: parse_commands(table.get("commands")),
            name,
            display_name,
        },
        aliases,
        cmd_aliases: parse_cmd_aliases(aliases_section.and_then(|a| a.get("commands"))),
    })
}

/// `<dir>/module.toml` 파일을 읽어 파싱한다.
fn parse_module_toml(module_dir: &Path) -> Option<ParsedModule> {
    let content = fs::read_to_string(module_dir.join("module.toml")).ok()?;
    let table: toml::Value = content.parse().ok()?;
    parse_module_toml_value(&serde_json::to_value(table).ok()?)
}

/// `GET /api/modules`의 항목 하나 → 모듈 정보 (TOML을 읽을 수 없을 때의 대체 경로)
fn parse_api_entry(m: &Value) -> Option<ParsedModule> {
    let name = str_at(m, "name")?.to_string();
    let mut aliases = vec![name.clone()];
    aliases.extend(string_array(m.get("module_aliases")));
    Some(ParsedModule {
        info: ModuleInfo {
            display_name: name.clone(),
            interaction_mode: str_at(m, "interaction_mode").map(|s| s.to_string()),
            commands: parse_commands(m.get("commands")),
            requires_eula: false,
            name,
        },
        aliases,
        cmd_aliases: parse_cmd_aliases(m.get("command_aliases")),
    })
}

// ═══════════════════════════════════════════════════════
// Implementation
// ═══════════════════════════════════════════════════════

impl ModuleRegistry {
    /// 레지스트리 구성. 데몬 API가 가용하면 우선 사용하고, 실패 시 파일시스템 스캔.
    /// 단위 테스트에서는 데몬을 호출하지 않는다.
    pub fn load(modules_dir: &str) -> Self {
        #[cfg(not(test))]
        if let Ok(data) = crate::ipc::blocking_request(reqwest::Method::GET, "/api/modules", None) {
            if let Some(list) = data.get("modules").and_then(|v| v.as_array()) {
                return Self::from_daemon_list(list);
            }
        }
        Self::load_from_filesystem(modules_dir)
    }

    /// 데몬 모듈 목록으로 구성. 이름이 없는 항목은 건너뛴다.
    fn from_daemon_list(list: &[Value]) -> Self {
        let mut reg = Self::default();
        for m in list {
            let parsed = str_at(m, "path")
                .and_then(|p| parse_module_toml(Path::new(p)))
                .or_else(|| parse_api_entry(m));
            if let Some(p) = parsed {
                reg.add(p);
            }
        }
        reg
    }

    /// modules 디렉토리의 하위 폴더에서 module.toml을 직접 파싱 (폴백)
    fn load_from_filesystem(modules_dir: &str) -> Self {
        let mut reg = Self::default();
        let Ok(entries) = fs::read_dir(modules_dir) else {
            return reg;
        };
        for entry in entries.flatten() {
            if let Some(p) = parse_module_toml(&entry.path()) {
                reg.add(p);
            }
        }
        reg
    }

    fn add(&mut self, parsed: ParsedModule) {
        let name = parsed.info.name.clone();
        for alias in parsed.aliases {
            self.alias_to_module.insert(alias.to_lowercase(), name.clone());
        }
        // 라이프사이클 명령과 정의된 명령은 별명이 없어도 자기 이름으로 리졸브된다.
        let mut cmd_map: HashMap<String, String> = LIFECYCLE_COMMANDS
            .iter()
            .map(|c| c.to_string())
            .chain(parsed.info.commands.iter().map(|c| c.name.clone()))
            .map(|c| (c.to_lowercase(), c))
            .collect();
        for (cmd, aliases) in parsed.cmd_aliases {
            cmd_map.insert(cmd.to_lowercase(), cmd.clone());
            for alias in aliases {
                cmd_map.insert(alias.to_lowercase(), cmd.clone());
            }
        }
        self.cmd_aliases.insert(name, cmd_map);
        self.modules.push(parsed.info);
    }

    /// 입력 문자열로 모듈 이름을 리졸브 (별명 지원)
    pub fn resolve_module_name(&self, input: &str) -> Option<String> {
        self.alias_to_module.get(&input.to_lowercase()).cloned()
    }

    /// 모듈 정보 가져오기 (canonical name으로)
    pub fn get_module(&self, name: &str) -> Option<&ModuleInfo> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// 모듈 내에서 명령어 이름 리졸브 (별명 지원)
    pub fn resolve_command(&self, module_name: &str, input: &str) -> Option<String> {
        self.cmd_aliases.get(module_name)?.get(&input.to_lowercase()).cloned()
    }

    /// 모듈 명령어 정의 가져오기
    pub fn get_command_def(&self, module_name: &str, cmd_name: &str) -> Option<&ModuleCommand> {
        self.get_module(module_name)?.commands.iter().find(|c| c.name == cmd_name)
    }

    /// 등록된 모든 모듈 canonical name
    pub fn module_names(&self) -> Vec<&str> {
        self.modules.iter().map(|m| m.name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 헬퍼: 미니 module.toml을 임시 디렉토리에 생성
    fn create_test_registry() -> (tempfile::TempDir, ModuleRegistry) {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("test_game");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(
            mod_dir.join("module.toml"),
            r#"
[module]
name = "test_game"
game_name = "Test Game"
display_name = "테스트 게임"

[protocols]
interaction_mode = "console"

[aliases]
module_aliases = ["tg", "테겜"]

[aliases.commands.say]
aliases = ["말하기", "speak"]

[aliases.commands.save]
aliases = ["저장", "backup"]

[[commands.fields]]
name = "say"
description = "Send a message"
method = "rcon"

[[commands.fields]]
name = "save"
description = "Save the world"
method = "rcon"
"#,
        )
        .unwrap();

        let registry = ModuleRegistry::load(tmp.path().to_str().unwrap());
        (tmp, registry)
    }

    #[test]
    fn test_load_modules() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(reg.modules.len(), 1);
        assert_eq!(reg.modules[0].name, "test_game");
        assert_eq!(reg.modules[0].display_name, "테스트 게임");
    }

    #[test]
    fn test_module_names() {
        let (_tmp, reg) = create_test_registry();
        let names = reg.module_names();
        assert!(names.contains(&"test_game"));
    }

    #[test]
    fn test_resolve_module_name_canonical() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(
            reg.resolve_module_name("test_game"),
            Some("test_game".into())
        );
    }

    #[test]
    fn test_resolve_module_name_alias() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(reg.resolve_module_name("tg"), Some("test_game".into()));
        assert_eq!(reg.resolve_module_name("테겜"), Some("test_game".into()));
        // game_name도 별명으로 동작
        assert_eq!(
            reg.resolve_module_name("test game"),
            Some("test_game".into())
        );
        // display_name도 동작
        assert_eq!(
            reg.resolve_module_name("테스트 게임"),
            Some("test_game".into())
        );
    }

    #[test]
    fn test_resolve_module_name_case_insensitive() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(
            reg.resolve_module_name("TEST_GAME"),
            Some("test_game".into())
        );
        assert_eq!(reg.resolve_module_name("TG"), Some("test_game".into()));
    }

    #[test]
    fn test_resolve_module_name_unknown() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(reg.resolve_module_name("unknown_mod"), None);
    }

    #[test]
    fn test_resolve_command_canonical() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(
            reg.resolve_command("test_game", "say"),
            Some("say".into())
        );
        assert_eq!(
            reg.resolve_command("test_game", "save"),
            Some("save".into())
        );
    }

    #[test]
    fn test_resolve_command_alias() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(
            reg.resolve_command("test_game", "말하기"),
            Some("say".into())
        );
        assert_eq!(
            reg.resolve_command("test_game", "speak"),
            Some("say".into())
        );
        assert_eq!(
            reg.resolve_command("test_game", "저장"),
            Some("save".into())
        );
        assert_eq!(
            reg.resolve_command("test_game", "backup"),
            Some("save".into())
        );
    }

    #[test]
    fn test_resolve_command_unknown() {
        let (_tmp, reg) = create_test_registry();
        assert_eq!(reg.resolve_command("test_game", "nosuchcmd"), None);
        assert_eq!(reg.resolve_command("no_module", "say"), None);
    }

    #[test]
    fn test_get_module() {
        let (_tmp, reg) = create_test_registry();
        let m = reg.get_module("test_game").unwrap();
        assert_eq!(m.name, "test_game");
        assert_eq!(m.commands.len(), 2);
        assert!(reg.get_module("nonexistent").is_none());
    }

    #[test]
    fn test_get_command_def() {
        let (_tmp, reg) = create_test_registry();
        let cmd = reg.get_command_def("test_game", "say").unwrap();
        assert_eq!(cmd.method, "rcon");
        assert_eq!(cmd.description, "Send a message");
        assert!(reg.get_command_def("test_game", "nosuch").is_none());
    }

    #[test]
    fn test_load_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = ModuleRegistry::load(tmp.path().to_str().unwrap());
        assert!(reg.modules.is_empty());
        assert!(reg.module_names().is_empty());
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let reg = ModuleRegistry::load("/nonexistent/path/that/does/not/exist");
        assert!(reg.modules.is_empty());
    }

    #[test]
    fn test_has_rcon_commands() {
        let (_tmp, reg) = create_test_registry();
        // test_game의 say, save 명령어는 모두 method = "rcon"
        let m = reg.get_module("test_game").unwrap();
        assert!(m.has_rcon_commands());
    }

    #[test]
    fn test_has_rcon_commands_false_for_rest_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("rest_game");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(
            mod_dir.join("module.toml"),
            r#"
[module]
name = "rest_game"
game_name = "REST Game"
display_name = "REST 게임"

[[commands.fields]]
name = "backup"
description = "Backup the world"
method = "rest"
"#,
        )
        .unwrap();
        let reg = ModuleRegistry::load(tmp.path().to_str().unwrap());
        let m = reg.get_module("rest_game").unwrap();
        assert!(!m.has_rcon_commands());
    }

    #[test]
    fn commands_without_aliases_resolve_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("plain");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(
            mod_dir.join("module.toml"),
            "[module]\nname = \"plain\"\n\n[[commands.fields]]\nname = \"Kick\"\nmethod = \"rcon\"\n",
        )
        .unwrap();
        let reg = ModuleRegistry::load(tmp.path().to_str().unwrap());
        assert_eq!(reg.resolve_command("plain", "kick"), Some("Kick".into()));
        assert_eq!(reg.resolve_command("plain", "start"), Some("start".into()));
    }

    /// 데몬 목록: `path`의 module.toml에서 표시 이름·EULA를 읽고,
    /// `name`이 없는 항목 하나 때문에 전체 로드가 중단되지 않아야 한다.
    #[test]
    fn from_daemon_list_reads_toml_and_skips_nameless_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("mc");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(
            mod_dir.join("module.toml"),
            "[module]\nname = \"mc\"\ndisplay_name = \"Minecraft\"\ngame_name = \"Minecraft Java\"\n\n[config]\neula = true\n",
        )
        .unwrap();
        let list = vec![
            serde_json::json!({ "version": "1" }),
            serde_json::json!({ "name": "mc", "path": mod_dir.to_string_lossy() }),
            serde_json::json!({
                "name": "pal", "path": "/nonexistent", "interaction_mode": "commands",
                "module_aliases": ["팰월드"],
                "command_aliases": { "save": { "aliases": ["저장"] } }
            }),
        ];
        let reg = ModuleRegistry::from_daemon_list(&list);
        assert_eq!(reg.modules.len(), 2);
        let mc = reg.get_module("mc").unwrap();
        assert_eq!(mc.display_name, "Minecraft");
        assert!(mc.requires_eula);
        assert_eq!(reg.resolve_module_name("minecraft java"), Some("mc".into()));
        // TOML을 못 읽는 항목은 API 필드로 대체
        assert_eq!(reg.resolve_module_name("팰월드"), Some("pal".into()));
        assert_eq!(reg.resolve_command("pal", "저장"), Some("save".into()));
        assert_eq!(reg.get_module("pal").unwrap().interaction_mode.as_deref(), Some("commands"));
    }

    #[test]
    fn test_lifecycle_commands() {
        assert!(LIFECYCLE_COMMANDS.contains(&"start"));
        assert!(LIFECYCLE_COMMANDS.contains(&"stop"));
        assert!(LIFECYCLE_COMMANDS.contains(&"restart"));
        assert!(LIFECYCLE_COMMANDS.contains(&"status"));
    }
}
