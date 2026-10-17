//! 모듈/명령어 별명 해석
//!
//! 모듈 별명: 모듈 이름 → module.toml [aliases].module_aliases → 사용자 설정 순으로
//! 등록하고, 같은 별명을 두 모듈이 가지면 먼저 등록한 쪽을 쓰되 충돌로 기록합니다.
//! 명령어 별명은 모듈 단위로 해석합니다 (다른 모듈의 별명과 섞이지 않음).

use super::api::ModuleView;
use serde_json::Value;
use std::collections::HashMap;

/// 모든 모듈에 공통인 내장 명령어
pub const BUILTIN_COMMANDS: &[&str] = &["start", "stop", "status"];

#[derive(Debug, Default)]
pub struct ModuleAliases {
    /// (별명, 모듈 이름) — 등록 순서 유지
    entries: Vec<(String, String)>,
    /// (별명, 별명을 주장한 모듈들)
    conflicts: Vec<(String, Vec<String>)>,
}

impl ModuleAliases {
    pub fn build(modules: &[ModuleView], custom: &HashMap<String, String>) -> Self {
        let mut me = Self::default();
        for m in modules {
            me.register(&m.name, &m.name);
        }
        for m in modules {
            for alias in &m.module_aliases {
                me.register(alias, &m.name);
            }
        }
        // 사용자 별명: "a, b, c" 콤마 구분. 결정적인 순서를 위해 모듈 이름순.
        let mut custom_sorted: Vec<_> = custom.iter().collect();
        custom_sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (module, aliases) in custom_sorted {
            for alias in split_aliases(aliases) {
                me.register(alias, module);
            }
        }
        me
    }

    fn register(&mut self, alias: &str, module: &str) {
        if let Some((_, owner)) = self.entries.iter().find(|(a, _)| a.eq_ignore_ascii_case_unicode(alias)) {
            if owner != module {
                let owner = owner.clone();
                match self.conflicts.iter_mut().find(|(a, _)| a.eq_ignore_ascii_case_unicode(alias)) {
                    Some((_, mods)) => {
                        if !mods.iter().any(|m| m == module) {
                            mods.push(module.to_string());
                        }
                    }
                    None => self.conflicts.push((alias.to_string(), vec![owner, module.to_string()])),
                }
            }
            return;
        }
        self.entries.push((alias.to_string(), module.to_string()));
    }

    /// 별명 → 모듈 이름. 모르면 입력 그대로.
    pub fn resolve(&self, input: &str) -> String {
        self.entries
            .iter()
            .find(|(a, _)| a.eq_ignore_ascii_case_unicode(input))
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| input.to_string())
    }

    pub fn is_known(&self, input: &str) -> bool {
        self.entries.iter().any(|(a, _)| a.eq_ignore_ascii_case_unicode(input))
    }

    /// 충돌한 별명이면 관련 모듈 목록
    pub fn conflict(&self, input: &str) -> Option<&[String]> {
        self.conflicts
            .iter()
            .find(|(a, _)| a.eq_ignore_ascii_case_unicode(input))
            .map(|(_, mods)| mods.as_slice())
    }

    /// 모듈 이름 자신을 뺀 별명 목록 (도움말 표시용)
    pub fn aliases_of(&self, module: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(a, m)| m == module && a != module)
            .map(|(a, _)| a.as_str())
            .collect()
    }
}

/// 명령어 별명 → 명령어 이름 (모듈 범위). 우선순위: 사용자 설정 → module.toml → 내장/정의 명령 이름.
pub fn resolve_command(module: Option<&ModuleView>, custom: &HashMap<String, Value>, input: &str) -> String {
    let mut table: Vec<(String, String)> = Vec::new();
    if let Some(m) = module {
        if let Some(Value::Object(cmds)) = custom.get(&m.name) {
            for (cmd, aliases) in cmds {
                if let Some(s) = aliases.as_str() {
                    for alias in split_aliases(s) {
                        table.push((alias.to_string(), cmd.clone()));
                    }
                }
            }
        }
        for (cmd, aliases) in &m.command_aliases {
            for alias in aliases {
                table.push((alias.clone(), cmd.clone()));
            }
        }
        for c in &m.commands {
            table.push((c.name.clone(), c.name.clone()));
        }
    }
    for b in BUILTIN_COMMANDS {
        table.push((b.to_string(), b.to_string()));
    }
    table
        .into_iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case_unicode(input))
        .map(|(_, cmd)| cmd)
        .unwrap_or_else(|| input.to_string())
}

fn split_aliases(s: &str) -> impl Iterator<Item = &str> {
    s.split(',').map(str::trim).filter(|a| !a.is_empty())
}

/// "a, b, c" 목록에 `input`이 있는지 (대소문자 무시)
pub fn alias_listed(list: &str, input: &str) -> bool {
    split_aliases(list).any(|a| a.to_string().eq_ignore_ascii_case_unicode(input))
}

/// 대소문자 무시 비교 (ASCII 밖 문자도 소문자화). 한글 별명은 그대로 비교된다.
trait CaseInsensitive {
    fn eq_ignore_ascii_case_unicode(&self, other: &str) -> bool;
}

impl CaseInsensitive for String {
    fn eq_ignore_ascii_case_unicode(&self, other: &str) -> bool {
        self.eq_ignore_ascii_case(other) || self.to_lowercase() == other.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn module(name: &str, aliases: &[&str]) -> ModuleView {
        ModuleView {
            name: name.into(),
            module_aliases: aliases.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_and_records_conflicts() {
        let mods = vec![module("palworld", &["팰월드", "pw"]), module("minecraft", &["mc", "pw"])];
        let custom = HashMap::from([("minecraft".to_string(), "마크, MC2".to_string())]);
        let a = ModuleAliases::build(&mods, &custom);
        assert_eq!(a.resolve("PW"), "palworld");
        assert_eq!(a.resolve("mc2"), "minecraft");
        assert_eq!(a.resolve("마크"), "minecraft");
        assert_eq!(a.resolve("unknown"), "unknown");
        assert_eq!(a.conflict("pw").unwrap(), &["palworld".to_string(), "minecraft".to_string()]);
        assert!(a.conflict("mc").is_none());
        assert_eq!(a.aliases_of("palworld"), vec!["팰월드", "pw"]);
    }

    #[test]
    fn command_aliases_are_module_scoped() {
        let mut pal = module("palworld", &[]);
        pal.command_aliases = vec![("start".into(), vec!["시작".into()])];
        let custom = HashMap::from([("palworld".to_string(), json!({"players": "접속자, who"}))]);
        assert_eq!(resolve_command(Some(&pal), &custom, "시작"), "start");
        assert_eq!(resolve_command(Some(&pal), &custom, "WHO"), "players");
        assert_eq!(resolve_command(None, &custom, "who"), "who");
        assert_eq!(resolve_command(None, &custom, "Stop"), "stop");
    }
}
