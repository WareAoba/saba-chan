//! 채팅 처리기가 데몬에게 바라는 기능 (슬롯)
//!
//! 처리기는 이 트레이트만 알고, 실제 구현은 IPCServer(기존 핸들러 재사용)가
//! 맡습니다. 테스트에서는 가짜 구현을 꽂아 처리기만 따로 검증합니다.

use crate::ipc::internal::InternalResponse;
use crate::ipc::{CommandRequest, IPCServer};
use crate::module_loader::CommandField;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

/// 채팅 처리에 필요한 인스턴스 정보
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub module: String,
    pub status: String,
    /// 이번 실행을 시작한 때 (유닉스 초). 상태 답의 켜진 시간
    #[serde(default)]
    pub start_time: Option<u64>,
    /// 시작했지만 아직 준비 문구가 찍히지 않았다 (`supervisor::ready`). 목록 응답에는 없고 데몬 구현이 채운다
    #[serde(skip)]
    pub starting: bool,
    /// 사용 모드 (`crate::controllers::Access`)
    #[serde(default)]
    pub access: crate::controllers::Access,
    /// 전용 멤버 명단 (`crate::controllers`)
    #[serde(default)]
    pub controllers: Vec<crate::controllers::Controller>,
}

impl ServerView {
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
}

/// 채팅 처리에 필요한 모듈 정보
#[derive(Debug, Clone, Default)]
pub struct ModuleView {
    pub name: String,
    pub module_aliases: Vec<String>,
    /// 게임명: module.toml `display_name`. `in_language`가 그 언어 모듈 locales의 `module.display_name`을 더한다. 모듈 별명처럼 서버를 부른다
    pub game_names: Vec<String>,
    /// module.toml `display_name` 그대로 (heartbeat 모듈 `display_name`, 상황판 채널의 게임 이름)
    pub display_name: Option<String>,
    /// 명령어 이름 → 별명 목록 (module.toml [aliases.commands]. `in_language`가 그 언어 locales `aliases.commands.<이름>`을 더한다)
    pub command_aliases: Vec<(String, Vec<String>)>,
    pub commands: Vec<CommandField>,
    /// [errors] 오류 코드 → 문구
    pub errors: HashMap<String, String>,
    /// [[presets]] 추천 설정 묶음 (heartbeat로 relay에 id·이름만 나간다)
    pub presets: Vec<crate::module_loader::ModulePreset>,
    /// `[commands] failure_patterns`: 콘솔 답이 맞으면 실패
    pub failure_patterns: Vec<String>,
    /// 모듈 폴더 (`locales/<lang>.json`을 읽는다). 없으면 module.toml 문구만
    pub locale_dir: Option<std::path::PathBuf>,
    /// `[status] players`를 선언했다 (채팅 `players`가 접속자 조회)
    pub players: bool,
    /// 설정 스키마 (명령 결과 칸 이름을 설정 이름으로, `commands::output::labels`)
    pub settings: Vec<crate::module_loader::SettingField>,
}

impl ModuleView {
    pub fn command(&self, name: &str) -> Option<&CommandField> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// 명령의 module.toml 별칭 (없으면 빈 목록)
    pub fn aliases_of(&self, command: &str) -> &[String] {
        self.command_aliases.iter().find(|(c, _)| c == command).map(|(_, a)| a.as_slice()).unwrap_or(&[])
    }

    /// 모듈 locales의 문구 (`pointer`는 `/commands/say/done` 꼴). 그 언어에 없으면 영어
    pub fn text(&self, lang: &str, pointer: &str) -> Option<String> {
        let dir = self.locale_dir.as_ref()?;
        let read = |lang: &str| -> Option<String> { module_locale(dir, lang)?.pointer(pointer)?.as_str().map(str::to_string) };
        read(lang).or_else(|| read("en"))
    }

    /// 한 언어로 부르는 모습: 게임명·모듈 별명·명령 별명에 그 언어 모듈 locales의 낱말(`locale_aliases`)을 더한다.
    /// module.toml의 낱말은 언어가 없는 낱말이라 늘 남는다. `None`이면 모든 언어를 더한다 (낱말 검사가 어느 언어의 별명과도 겹치지 않게)
    pub fn in_language(mut self, lang: Option<&str>) -> Self {
        let Some(dir) = self.locale_dir.clone() else { return self };
        let langs = match lang {
            Some(l) => vec![l],
            None => crate::chat::i18n::languages(),
        };
        for l in langs {
            let Some(found) = locale_aliases(&dir, l) else { continue };
            push_words(&mut self.game_names, found.game_name.into_iter());
            push_words(&mut self.module_aliases, found.module.into_iter());
            for (cmd, words) in found.commands {
                match self.command_aliases.iter_mut().find(|(c, _)| *c == cmd) {
                    Some((_, list)) => push_words(list, words.into_iter()),
                    None => self.command_aliases.push((cmd, words)),
                }
            }
        }
        self.command_aliases.sort_by(|a, b| a.0.cmp(&b.0));
        self
    }

    /// 모듈 locales 한 언어 전체 (없으면 Null)
    pub fn locale(&self, lang: &str) -> serde_json::Value {
        self.locale_dir
            .as_ref()
            .and_then(|dir| module_locale(dir, lang))
            .map(|v| v.as_ref().clone())
            .unwrap_or(serde_json::Value::Null)
    }
}

/// 모듈 폴더의 `locales/<lang>.json`. 채팅 명령·하트비트마다 부르므로 파일 크기·수정 시각이 그대로면 다시 읽지 않는다
pub fn module_locale(module_dir: &std::path::Path, lang: &str) -> Option<std::sync::Arc<serde_json::Value>> {
    type Stamp = (Option<std::time::SystemTime>, u64);
    type Cache = HashMap<std::path::PathBuf, (Stamp, std::sync::Arc<serde_json::Value>)>;
    static CACHE: std::sync::LazyLock<std::sync::Mutex<Cache>> = std::sync::LazyLock::new(Default::default);
    let path = module_dir.join("locales").join(format!("{}.json", lang));
    let meta = std::fs::metadata(&path).ok()?;
    let stamp: Stamp = (meta.modified().ok(), meta.len());
    if let Some((seen, value)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(&path) {
        if *seen == stamp {
            return Some(value.clone());
        }
    }
    let value = std::sync::Arc::new(serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).ok()?).ok()?);
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).insert(path, (stamp, value.clone()));
    Some(value)
}

/// 모듈 locales 한 언어의 게임명 `module.display_name` (예전 모양 `display_name`도 읽는다)
pub fn locale_display_name(locale: &serde_json::Value) -> Option<&str> {
    let name = locale.pointer("/module/display_name").or_else(|| locale.get("display_name"))?.as_str()?.trim();
    (!name.is_empty()).then_some(name)
}

/// 모듈 locales 한 언어가 더하는 낱말
#[derive(Debug, Default, PartialEq)]
pub struct LocaleAliases {
    /// `module.display_name`
    pub game_name: Option<String>,
    /// `aliases.module`
    pub module: Vec<String>,
    /// 명령 이름 → `aliases.commands.<이름>` (이름 순, 낱말이 있는 명령만)
    pub commands: Vec<(String, Vec<String>)>,
}

/// 모듈 폴더의 `locales/<lang>.json`이 그 언어에서 더하는 게임명(`module.display_name`)과 별명
/// (`aliases: {module: [낱말], commands: {<명령>: [낱말]}}`). 파일이 없으면 None
pub fn locale_aliases(module_dir: &std::path::Path, lang: &str) -> Option<LocaleAliases> {
    let locale = module_locale(module_dir, lang)?;
    let list = |v: Option<&serde_json::Value>| -> Vec<String> {
        let mut out = Vec::new();
        push_words(&mut out, v.and_then(|v| v.as_array()).into_iter().flatten().filter_map(|w| w.as_str()).map(str::to_string));
        out
    };
    let mut commands: Vec<(String, Vec<String>)> = locale
        .pointer("/aliases/commands")
        .and_then(|c| c.as_object())
        .into_iter()
        .flatten()
        .map(|(name, words)| (name.clone(), list(Some(words))))
        .filter(|(_, words)| !words.is_empty())
        .collect();
    commands.sort_by(|a, b| a.0.cmp(&b.0));
    Some(LocaleAliases { game_name: locale_display_name(&locale).map(str::to_string), module: list(locale.pointer("/aliases/module")), commands })
}

/// 낱말을 목록 끝에 더한다. 앞뒤 공백은 떼고, 빈 낱말·쉼표가 든 낱말·이미 있는 낱말(대소문자·공백만 다른 것)은 뺀다
pub fn push_words(out: &mut Vec<String>, words: impl Iterator<Item = String>) {
    for word in words {
        let word = word.trim();
        if !word.is_empty() && !word.contains(crate::chat::words::is_comma) && !out.iter().any(|n| crate::chat::words::same(n, word)) {
            out.push(word.to_string());
        }
    }
}

/// 모듈의 게임명: module.toml `display_name` (언어별 게임명은 `ModuleView::in_language`가 더한다).
/// 띄어 쓴 이름도 부를 수 있다 (낱말 단위로 맞춘다). 쉼표가 든 이름은 뺀다
pub fn game_names(display_name: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    push_words(&mut out, display_name.map(str::to_string).into_iter());
    out
}

/// 처리기가 쓰는 데몬 기능. 응답은 HTTP 핸들러와 같은 (상태, JSON) 형태.
#[allow(async_fn_in_trait)]
pub trait DaemonApi {
    async fn servers(&self) -> Result<Vec<ServerView>, String>;
    async fn modules(&self) -> Vec<ModuleView>;
    /// 서버 시작 — 경로(managed 여부)는 데몬이 고른다
    async fn start(&self, server: &ServerView) -> InternalResponse;
    /// 정지 (정지 라우트와 같다: 저장하지 못하는 서버도 바로 정지한다)
    async fn stop(&self, server: &ServerView) -> InternalResponse;
    /// 게임 명령 (`POST /api/instance/:id/command`와 같은 경로). 인자는 순서대로.
    async fn command(&self, instance_id: &str, command: &str, args: Vec<String>) -> InternalResponse;
    /// 접속자 (`GET /api/instance/:id/players`)
    async fn players(&self, server: &ServerView) -> InternalResponse;
}

impl DaemonApi for IPCServer {
    async fn servers(&self) -> Result<Vec<ServerView>, String> {
        let resp = InternalResponse::from(crate::service::servers::list(self, 0).await);
        if !resp.is_success() {
            return Err(resp.str_field("error").unwrap_or("failed to list servers").to_string());
        }
        let servers = resp.body.get("servers").cloned().unwrap_or(json!([]));
        let mut servers: Vec<ServerView> = serde_json::from_value(servers).map_err(|e| e.to_string())?;
        let runtime = self.supervisor.read().await.runtime.clone();
        for s in &mut servers {
            s.starting = s.is_running() && crate::supervisor::ready::is_waiting(&runtime, &s.id);
        }
        Ok(servers)
    }

    async fn modules(&self) -> Vec<ModuleView> {
        // 목록만 잠금 안에서 얻는다: 아래 locales 읽기 동안 서버 시작·정지를 막지 않게
        let listed = self.supervisor.read().await.list_modules();
        let modules = match listed {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("[Chat] 모듈 목록 조회 실패: {}", e);
                return Vec::new();
            }
        };
        modules
            .into_iter()
            .map(|m| {
                let locale_dir = Some(std::path::PathBuf::from(&m.path));
                let meta = m.metadata;
                let mut command_aliases: Vec<(String, Vec<String>)> = meta
                    .command_aliases
                    .into_iter()
                    .map(|(cmd, info)| (cmd, info.aliases))
                    .collect();
                command_aliases.sort_by(|a, b| a.0.cmp(&b.0));
                ModuleView {
                    game_names: game_names(meta.display_name.as_deref()),
                    display_name: meta.display_name,
                    name: meta.name,
                    module_aliases: meta.module_aliases,
                    command_aliases,
                    failure_patterns: meta.commands.as_ref().map(|c| c.failure_patterns.clone()).unwrap_or_default(),
                    commands: meta.commands.map(|c| c.fields).unwrap_or_default(),
                    errors: meta.errors,
                    presets: meta.presets,
                    locale_dir,
                    players: meta.players_function.is_some(),
                    settings: meta.settings.map(|s| s.fields).unwrap_or_default(),
                }
            })
            .collect()
    }

    async fn start(&self, server: &ServerView) -> InternalResponse {
        crate::service::lifecycle::start(self, &server.id).await.into()
    }

    async fn stop(&self, server: &ServerView) -> InternalResponse {
        crate::service::lifecycle::user_stop(self, &server.id, false).await.into()
    }

    async fn command(&self, instance_id: &str, command: &str, args: Vec<String>) -> InternalResponse {
        let req = CommandRequest { command: command.to_string(), args: json!(args) };
        crate::service::command::execute(self, instance_id, req).await.into()
    }

    async fn players(&self, server: &ServerView) -> InternalResponse {
        crate::service::servers::players(self, &server.id).await.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_adds_only_its_own_game_name_and_aliases() {
        let tmp = tempfile::tempdir().unwrap();
        let locales = tmp.path().join("locales");
        std::fs::create_dir_all(&locales).unwrap();
        std::fs::write(
            locales.join("ko.json"),
            r#"{"module": {"display_name": "마인크래프트"}, "aliases": {"module": ["마크", " 마크 ", "mc"], "commands": {"start": ["시작"], "kick": ["추방"]}}, "commands": {"kick": {"label": "추방하기"}}}"#,
        )
        .unwrap();
        std::fs::write(locales.join("ja.json"), r#"{"module": {"display_name": "マインクラフト"}, "aliases": {"module": ["マイクラ"]}}"#).unwrap();
        // 띄어 쓴 이름은 넣고 쉼표가 든 이름은 뺀다. 대소문자만 다르면 하나로
        std::fs::write(locales.join("de.json"), r#"{"module": {"display_name": "Mine Craft"}}"#).unwrap();
        std::fs::write(locales.join("fr.json"), r#"{"module": {"display_name": "Mine, Craft"}}"#).unwrap();
        std::fs::write(locales.join("es.json"), r#"{"display_name": "minecraft"}"#).unwrap();
        let mc = ModuleView {
            name: "minecraft".into(),
            game_names: game_names(Some("Minecraft")),
            module_aliases: vec!["mc".into()],
            command_aliases: vec![("start".into(), vec!["go".into()])],
            locale_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };

        let ko = mc.clone().in_language(Some("ko"));
        assert_eq!(ko.game_names, vec!["Minecraft", "마인크래프트"]);
        assert_eq!(ko.module_aliases, vec!["mc", "마크"], "module.toml words stay, the language adds its own");
        assert_eq!(ko.command_aliases, vec![("kick".to_string(), vec!["추방".to_string()]), ("start".to_string(), vec!["go".to_string(), "시작".to_string()])]);

        let ja = mc.clone().in_language(Some("ja"));
        assert_eq!(ja.game_names, vec!["Minecraft", "マインクラフト"]);
        assert_eq!(ja.module_aliases, vec!["mc", "マイクラ"], "no Korean words in Japanese");
        assert_eq!(ja.command_aliases, vec![("start".to_string(), vec!["go".to_string()])]);

        let pt = mc.clone().in_language(Some("pt-BR"));
        assert_eq!((pt.game_names, pt.module_aliases), (vec!["Minecraft".to_string()], vec!["mc".to_string()]), "no locale file → module.toml words only");

        let all = mc.in_language(None);
        assert_eq!(all.game_names, vec!["Minecraft", "마인크래프트", "マインクラフト", "Mine Craft"]);
        assert_eq!(all.module_aliases, vec!["mc", "마크", "マイクラ"]);
        assert_eq!(game_names(Some("Terraria")), vec!["Terraria"]);
    }

    /// 채팅 명령·하트비트마다 부르므로 그대로인 locales 파일은 다시 읽지 않고, 바뀌면 새 내용을 읽는다
    #[test]
    fn module_locale_is_read_again_only_after_the_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let locales = tmp.path().join("locales");
        std::fs::create_dir_all(&locales).unwrap();
        std::fs::write(locales.join("ko.json"), r#"{"display_name": "테라리아"}"#).unwrap();
        let first = module_locale(tmp.path(), "ko").unwrap();
        let again = module_locale(tmp.path(), "ko").unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &again), "unchanged file → the same parsed value");
        std::fs::write(locales.join("ko.json"), r#"{"display_name": "테라리아 서버"}"#).unwrap();
        assert_eq!(module_locale(tmp.path(), "ko").unwrap()["display_name"], "테라리아 서버");
        assert!(module_locale(tmp.path(), "fr").is_none());
    }
}

