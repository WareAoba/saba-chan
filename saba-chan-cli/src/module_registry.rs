//! 모듈 레지스트리 — module.toml에서 별명·명령어를 읽어 매핑 테이블 생성
//!
//! `palworld start`, `팰월드 시작`, `mc save` 같은 입력을 해석할 수 있습니다.
//! 모듈 별명, 명령어 별명 모두 지원합니다.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ═══════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════

/// 명령어 입력 필드 정의 (module.toml → [[commands.fields]] → inputs)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CommandInput {
    pub name: String,
    pub input_type: String, // "text", "number", "select", "password", "file"
    pub required: bool,
}

/// 모듈 명령어 정의 (module.toml → [[commands.fields]])
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ModuleCommand {
    pub name: String,
    pub description: String,
    pub method: String, // "rest", "rcon", "stdin", "dual"
    pub inputs: Vec<CommandInput>,
}

/// 모듈 정보 (module.toml 한 개 분량)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ModuleInfo {
    pub name: String,
    pub game_name: String,
    pub display_name: String,
    pub interaction_mode: Option<String>,  // "console" or "commands"
    pub commands: Vec<ModuleCommand>,
}

/// 모듈 레지스트리 — 전역 별명 테이블
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
// Implementation
// ═══════════════════════════════════════════════════════

impl ModuleRegistry {
    /// 주어진 modules 디렉토리에서 모든 module.toml을 로드
    pub fn load(modules_dir: &str) -> Self {
        let mut modules = Vec::new();
        let mut alias_to_module = HashMap::new();
        let mut cmd_aliases: HashMap<String, HashMap<String, String>> = HashMap::new();

        let dir = Path::new(modules_dir);
        if !dir.exists() {
            return Self {
                modules,
                alias_to_module,
                cmd_aliases,
            };
        }

        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => {
                return Self {
                    modules,
                    alias_to_module,
                    cmd_aliases,
                }
            }
        };

        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let toml_path = entry.path().join("module.toml");
            if !toml_path.exists() {
                continue;
            }

            let content = match fs::read_to_string(&toml_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let table: toml::Value = match content.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // ── [module] 섹션 ──
            let name = table
                .get("module")
                .and_then(|m| m.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }

            let game_name = table
                .get("module")
                .and_then(|m| m.get("game_name"))
                .and_then(|v| v.as_str())
                .unwrap_or(&name)
                .to_string();

            let display_name = table
                .get("module")
                .and_then(|m| m.get("display_name"))
                .and_then(|v| v.as_str())
                .unwrap_or(&name)
                .to_string();

            // ── 모듈 별명 등록 ──
            // name, game_name, display_name 자동 등록
            alias_to_module.insert(name.to_lowercase(), name.clone());
            alias_to_module.insert(game_name.to_lowercase(), name.clone());
            alias_to_module.insert(display_name.to_lowercase(), name.clone());

            // ── [protocols] 섹션 → interaction_mode ──
            let interaction_mode = table
                .get("protocols")
                .and_then(|p| p.get("interaction_mode"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // [aliases].module_aliases 배열
            if let Some(aliases) = table
                .get("aliases")
                .and_then(|a| a.get("module_aliases"))
                .and_then(|v| v.as_array())
            {
                for a in aliases {
                    if let Some(s) = a.as_str() {
                        alias_to_module.insert(s.to_lowercase(), name.clone());
                    }
                }
            }

            // ── 명령어 정의 파싱 ──
            let mut commands = Vec::new();
            if let Some(cmd_fields) = table
                .get("commands")
                .and_then(|c| c.get("fields"))
                .and_then(|v| v.as_array())
            {
                for field in cmd_fields {
                    let cmd_name = field
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if cmd_name.is_empty() {
                        continue;
                    }

                    let description = field
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let method = field
                        .get("method")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let mut inputs = Vec::new();
                    if let Some(inp_arr) = field.get("inputs").and_then(|v| v.as_array()) {
                        for inp in inp_arr {
                            inputs.push(CommandInput {
                                name: inp
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                input_type: inp
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("text")
                                    .to_string(),
                                required: inp
                                    .get("required")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false),
                            });
                        }
                    }

                    commands.push(ModuleCommand {
                        name: cmd_name,
                        description,
                        method,
                        inputs,
                    });
                }
            }

            // ── 명령어 별명 파싱 ──
            let mut this_cmd_aliases = HashMap::new();
            if let Some(cmds_table) = table
                .get("aliases")
                .and_then(|a| a.get("commands"))
                .and_then(|v| v.as_table())
            {
                for (cmd_name, cmd_val) in cmds_table {
                    // 명령어 이름 자체를 자기 자신으로 매핑
                    this_cmd_aliases.insert(cmd_name.to_lowercase(), cmd_name.clone());

                    if let Some(aliases) = cmd_val.get("aliases").and_then(|v| v.as_array()) {
                        for al in aliases {
                            if let Some(s) = al.as_str() {
                                this_cmd_aliases.insert(s.to_lowercase(), cmd_name.clone());
                            }
                        }
                    }
                }
            }

            cmd_aliases.insert(name.clone(), this_cmd_aliases);

            modules.push(ModuleInfo {
                name,
                game_name,
                display_name,
                interaction_mode,
                commands,
            });
        }

        Self {
            modules,
            alias_to_module,
            cmd_aliases,
        }
    }

    /// 입력 문자열로 모듈 이름을 리졸브 (별명 지원, owned String 반환)
    pub fn resolve_module_name(&self, input: &str) -> Option<String> {
        let key = input.to_lowercase();
        self.alias_to_module.get(&key).cloned()
    }

    /// 모듈 정보 가져오기 (canonical name으로)
    pub fn get_module(&self, name: &str) -> Option<&ModuleInfo> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// 모듈 내에서 명령어 이름 리졸브 (별명 지원)
    pub fn resolve_command(&self, module_name: &str, input: &str) -> Option<String> {
        let key = input.to_lowercase();
        self.cmd_aliases.get(module_name)?.get(&key).cloned()
    }

    /// 모듈 명령어 정의 가져오기
    pub fn get_command_def(&self, module_name: &str, cmd_name: &str) -> Option<&ModuleCommand> {
        self.modules
            .iter()
            .find(|m| m.name == module_name)?
            .commands
            .iter()
            .find(|c| c.name == cmd_name)
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
        assert_eq!(reg.modules[0].game_name, "Test Game");
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
    fn test_lifecycle_commands() {
        assert!(LIFECYCLE_COMMANDS.contains(&"start"));
        assert!(LIFECYCLE_COMMANDS.contains(&"stop"));
        assert!(LIFECYCLE_COMMANDS.contains(&"restart"));
        assert!(LIFECYCLE_COMMANDS.contains(&"status"));
    }
}
