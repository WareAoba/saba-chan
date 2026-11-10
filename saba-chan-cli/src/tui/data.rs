//! 화면에 보일 데몬 상태. 모두 데몬 API로 읽고, 여기서는 아무것도 바꾸지 않는다
//!
//! 상태(머리줄·목록)는 몇 초마다, 보고 있는 콘솔(서버 콘솔·데몬 로그)은 더 자주 끝부분만 읽는다.
//! 콘솔 줄은 데몬이 붙여 준 구문 강조 토큰(`spans`)과 함께 그대로 둔다 (칠하기는 `highlight`).

use crate::ctx::Ctx;
use serde_json::Value;

/// 콘솔·로그 끝에서 읽어 오는 줄 수
pub const TAIL: usize = 300;

/// 한 번 읽은 상태. `None`은 이번에 읽지 않았다는 뜻 (화면에 남은 값을 그대로 쓴다)
#[derive(Default)]
pub struct Fetched {
    pub health: Option<Value>,
    pub servers: Vec<Value>,
    pub metrics: Option<Value>,
    pub agent: Option<Value>,
    pub updates: Option<Value>,
    pub modules: Option<Vec<Value>>,
    pub extensions: Option<Vec<Value>>,
    pub launchable: Option<Value>,
    /// 디스코드 쪽: 이 PC와 페어링된 디스코드 서버들 (`bot nodes`)
    pub guilds: Option<Vec<Value>>,
}

/// 보고 있는 콘솔 한 번 읽은 것
pub enum View {
    /// (서버 id, 콘솔 끝 줄들)
    Console(String, Vec<Value>),
    DaemonLog(Vec<Value>),
}

/// 무엇을 더 읽을지 (지금 화면)
#[derive(Default, Clone)]
pub struct Want {
    pub modules: bool,
    pub extensions: bool,
    /// 띄울 수 있는 인터페이스를 아직 모른다
    pub launchable: bool,
    pub guilds: bool,
}

/// 어느 콘솔을 볼지
#[derive(Clone, PartialEq)]
pub enum ViewWant {
    Console(String),
    DaemonLog,
}

/// 화면이 쓰는 데몬 상태
#[derive(Default)]
pub struct Snapshot {
    /// `GET /health`. None이면 데몬 꺼짐
    pub health: Option<Value>,
    pub servers: Vec<Value>,
    pub metrics: Option<Value>,
    /// `GET /api/relay/agent`
    pub agent: Option<Value>,
    /// `GET /api/updates/status`
    pub updates: Option<Value>,
    pub modules: Vec<Value>,
    pub extensions: Vec<Value>,
    /// `GET /api/clients/launch` (GUI로 전환 버튼을 보일지)
    pub launchable: Option<Value>,
    /// 데몬 로그 끝 줄들 (`{timestamp, level, target, message, spans}`)
    pub daemon_log: Vec<Value>,
    /// (서버 id, 콘솔 끝 줄들 `{content, source, level, spans}`)
    pub console: Option<(String, Vec<Value>)>,
    /// 페어링된 디스코드 서버 `{guildId, guildName, …}`
    pub guilds: Vec<Value>,
}

impl Snapshot {
    /// 읽은 상태를 둔다. 설치된 번역 판(`/api/updates/status`의 `locales_version`)이 바뀌었으면 true: cli.json을 다시 읽는다
    pub fn apply(&mut self, f: Fetched) -> bool {
        let locales_changed = match (self.updates.as_ref(), f.updates.as_ref().and_then(|u| u.get("locales_version"))) {
            (Some(prev), Some(next)) => prev.get("locales_version").is_some_and(|p| p != next),
            _ => false,
        };
        self.health = f.health;
        self.servers = f.servers;
        if self.health.is_none() {
            self.metrics = None;
            self.agent = None;
            self.launchable = None;
        }
        if f.metrics.is_some() {
            self.metrics = f.metrics;
        }
        if f.agent.is_some() {
            self.agent = f.agent;
        }
        if f.updates.is_some() {
            self.updates = f.updates;
        }
        if let Some(m) = f.modules {
            self.modules = m;
        }
        if let Some(e) = f.extensions {
            self.extensions = e;
        }
        if f.launchable.is_some() {
            self.launchable = f.launchable;
        }
        if let Some(g) = f.guilds {
            self.guilds = g;
        }
        locales_changed
    }

    pub fn apply_view(&mut self, v: View) {
        match v {
            View::Console(id, lines) => self.console = Some((id, lines)),
            View::DaemonLog(lines) => self.daemon_log = lines,
        }
    }

    pub fn running(&self) -> usize {
        self.servers.iter().filter(|v| s(v, "status") == "running").count()
    }
}

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// 유닉스 초(밀리초도) → 이 PC 시계로 `fmt`
pub fn clock_fmt(secs: u64, fmt: &str) -> String {
    let secs = if secs > 10_000_000_000 { secs / 1000 } else { secs };
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format(fmt).to_string())
        .unwrap_or_default()
}

/// 유닉스 초 → 이 PC 시계의 HH:MM
pub fn clock(secs: u64) -> String {
    clock_fmt(secs, "%H:%M")
}

/// 한 번 읽기: 데몬이 꺼져 있으면 나머지는 묻지 않는다
pub async fn fetch(ctx: &Ctx, want: &Want) -> Fetched {
    let Ok(health) = ctx.api.health().await else { return Fetched::default() };
    let api = &ctx.api;
    let (servers, metrics, agent, updates) = tokio::join!(api.servers(true), api.system_metrics(), api.agent(), api.update_status());
    let list = |v: Option<Value>, key: &str| v.and_then(|v| v.get(key).and_then(Value::as_array).cloned());
    let modules = match want.modules {
        true => list(api.modules().await.ok(), "modules"),
        false => None,
    };
    let extensions = match want.extensions {
        true => list(api.extensions().await.ok(), "extensions"),
        false => None,
    };
    let launchable = match want.launchable {
        true => api.launchable().await.ok(),
        false => None,
    };
    // 페어링 전이면 빈 목록 (디스코드 쪽이 "페어링하세요"를 보인다)
    let guilds = match want.guilds {
        true => Some(crate::cmd::bot::paired_nodes(ctx).await.map(|r| r.get("nodes").or(Some(&r)).and_then(Value::as_array).cloned().unwrap_or_default()).unwrap_or_default()),
        false => None,
    };
    Fetched {
        health: Some(health),
        servers: servers.ok().and_then(|v| v.get("servers").and_then(Value::as_array).cloned()).unwrap_or_default(),
        metrics: metrics.ok(),
        agent: agent.ok(),
        updates: updates.ok(),
        modules,
        extensions,
        launchable,
        guilds,
    }
}

/// 보고 있는 콘솔의 끝부분 (토큰과 함께). 못 읽으면 None
pub async fn fetch_view(ctx: &Ctx, want: &ViewWant) -> Option<View> {
    let lines = |r: Value| r.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
    match want {
        ViewWant::Console(id) => match ctx.api.console_tokens(id, None, Some(TAIL), true).await {
            Ok(r) => Some(View::Console(id.clone(), lines(r))),
            // 한 번도 켠 적 없는 서버는 404: 빈 콘솔
            Err(e) if matches!(e.downcast_ref::<crate::error::CliError>(), Some(crate::error::CliError::Api { status: 404, .. })) => {
                Some(View::Console(id.clone(), Vec::new()))
            }
            Err(_) => None,
        },
        ViewWant::DaemonLog => ctx.api.daemon_console_tokens(None, Some(TAIL), true).await.ok().map(|r| View::DaemonLog(lines(r))),
    }
}

/// 상세 탭에서 데몬 API로 읽는 것
#[derive(Debug, Clone, PartialEq)]
pub enum PaneWant {
    /// 서버 id
    ServerSettings(String),
    Players(String),
    Backups(String),
    Log(String),
    /// 콘솔 명령 제안에 쓰는 그 서버의 게임 명령
    Catalog(String),
    /// 디스코드 서버 id
    Perm(String),
    Discord(String),
    AppConfig,
}

/// 상세 탭에서 읽은 것
pub enum PaneData {
    /// `server settings --json`과 같은 문서
    Settings(Value),
    Config(Value),
    /// 권한 폼과 디스코드 서버 id
    Perm(Form, String),
    /// 디스코드 서버 설정과 그 id
    Discord(Value, String),
    Rows(Vec<Value>),
    Text(Vec<String>),
    Catalog(String, crate::catalog::Catalog),
    Failed(String),
}

use super::form::{Field, Form, Kind};

fn failed(e: &anyhow::Error) -> PaneData {
    PaneData::Failed(crate::error::classify(e).headline())
}

/// 상세 탭 내용 읽기
pub async fn fetch_pane(ctx: &Ctx, want: &PaneWant) -> PaneData {
    let rows = |r: &Value, key: &str| r.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    match want {
        PaneWant::ServerSettings(id) => match crate::cmd::server::load_settings(ctx, Some(id)).await {
            Ok(d) => PaneData::Settings(d.doc()),
            Err(e) => failed(&e),
        },
        PaneWant::Players(id) => match ctx.api.players(id).await {
            Ok(r) => PaneData::Rows(rows(&r, "players")),
            Err(e) if matches!(e.downcast_ref::<crate::error::CliError>(), Some(crate::error::CliError::Api { status: 409, .. })) => {
                PaneData::Failed(t!("tui.pane.not_running"))
            }
            Err(e) => failed(&e),
        },
        PaneWant::Backups(id) => match ctx.api.backups(id).await {
            Ok(r) => PaneData::Rows(rows(&r, "backups")),
            Err(e) => failed(&e),
        },
        PaneWant::Log(id) => match ctx.api.logs(id, None, Some(TAIL), None).await {
            Ok(r) => PaneData::Text(rows(&r, "lines").iter().map(|l| l.as_str().map(str::to_string).unwrap_or_else(|| l.to_string())).collect()),
            Err(e) if matches!(e.downcast_ref::<crate::error::CliError>(), Some(crate::error::CliError::Api { status: 404, .. })) => PaneData::Text(Vec::new()),
            Err(e) => failed(&e),
        },
        PaneWant::Catalog(id) => match crate::resolve::server(ctx, Some(id)).await {
            Ok(sv) => PaneData::Catalog(id.clone(), crate::cmd::server::catalog_for_server(ctx, &sv).await.unwrap_or_default()),
            Err(e) => failed(&e),
        },
        PaneWant::Perm(guild) => {
            let (perm, board, members) =
                tokio::join!(ctx.api.permissions(guild), ctx.api.permission_commands(guild, crate::i18n::lang()), crate::cmd::bot::members(ctx, guild));
            match (perm, board) {
                (Ok(perm), Ok(board)) => PaneData::Perm(perm_form(&perm, &board, &members.unwrap_or(Value::Null)), guild.clone()),
                (Err(e), _) | (_, Err(e)) => failed(&e),
            }
        }
        PaneWant::Discord(guild) => match ctx.api.discord_settings(guild).await {
            Ok(r) => PaneData::Discord(r, guild.clone()),
            Err(e) => failed(&e),
        },
        PaneWant::AppConfig => match ctx.api.app_config().await {
            Ok(r) => PaneData::Config(r),
            Err(e) => failed(&e),
        },
    }
}

fn text_field(key: &str, label: String, group: &str, kind: Kind, value: String, default: &str) -> Field {
    Field {
        key: key.to_string(),
        label,
        group: group.to_string(),
        kind,
        original: value.clone(),
        value,
        default: default.to_string(),
        description: String::new(),
        restart: false,
    }
}

/// 앱 설정 폼: 데몬과 CLI가 쓰는 앱 설정만 (창 모양·소리 같은 GUI 설정은 GUI에서). 키는 `config set`이 받는 이름
pub fn config_form(cfg: &Value) -> Form {
    let b = |k: &str, default: bool| cfg.get(k).and_then(Value::as_bool).unwrap_or(default).to_string();
    let n = |k: &str, default: i64| cfg.get(k).and_then(Value::as_i64).unwrap_or(default).to_string();
    let num = |min: f64, max: f64| Kind::Number { min: Some(min), max: Some(max) };
    let field = |key: &str, group: &str, kind: Kind, value: String, default: &str| {
        let mut f = text_field(key, t!(&format!("tui.cfg.{key}")), group, kind, value, default);
        f.description = crate::i18n::try_t(&format!("tui.cfg_about.{key}")).unwrap_or_default();
        f
    };
    let languages = crate::i18n::LANGUAGES.iter().map(|(code, name)| (code.to_string(), name.to_string())).collect();
    let stamps = ["off", "24h", "12h"].iter().map(|k| (k.to_string(), t!(&format!("tui.cfg.stamp_{k}")))).collect();
    let language = s(cfg, "language");
    let mut port = field("ipcPort", "connection", num(1024.0, 65535.0), n("ipcPort", 57474), "57474");
    port.restart = true;
    Form::new(vec![
        field("language", "connection", Kind::Choice(languages), if language.is_empty() { crate::i18n::lang().to_string() } else { language.to_string() }, ""),
        port,
        field("newServerAutoRestart", "servers", Kind::Bool, b("newServerAutoRestart", true), "true"),
        field("newServerBackupKeep", "servers", num(0.0, 100.0), n("newServerBackupKeep", 10), "10"),
        field("autoGeneratePasswords", "servers", Kind::Bool, b("autoGeneratePasswords", true), "true"),
        field("portConflictCheck", "servers", Kind::Bool, b("portConflictCheck", true), "true"),
        field("consoleTimestamps", "console", Kind::Choice(stamps), cfg.get("consoleTimestamps").and_then(Value::as_str).unwrap_or("off").to_string(), "off"),
        field("consoleTimestampDate", "console", Kind::Bool, b("consoleTimestampDate", false), "false"),
        field("auditKeepDays", "storage", num(0.0, 3650.0), n("auditKeepDays", 0), "0"),
    ])
}

/// 노래·읽어주기 폼: 디스코드 서버 설정에서. 칸 키는 `bot music|tts`의 옵션 이름 (`enabled`는 --on/--off)
pub fn discord_form(sub: &str, r: &Value) -> Form {
    let b = |p: &str| match r.pointer(p).and_then(Value::as_bool) {
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let n = |p: &str| r.pointer(p).map(|v| if v.is_null() { String::new() } else { v.to_string() }).unwrap_or_default();
    let num = |min: f64, max: f64| Kind::Number { min: Some(min), max: Some(max) };
    let fields = if sub == "music" {
        vec![
            text_field("enabled", t!("bot.music"), "basic", Kind::Bool, b("/music/enabled"), "false"),
            text_field("ui", t!("bot.music_ui"), "basic", Kind::Bool, b("/music/ui"), "true"),
            text_field("queue-lines", t!("bot.music_queue_lines"), "basic", num(1.0, 20.0), n("/music/queueLines"), "5"),
            text_field("refresh", t!("bot.music_refresh"), "basic", num(2000.0, 10000.0), n("/music/refreshInterval"), "5000"),
            text_field("normalize", t!("bot.music_normalize"), "basic", Kind::Bool, b("/music/normalize"), "false"),
        ]
    } else {
        let speed = r.pointer("/tts/speed").and_then(Value::as_i64).map(|v| format!("{}", v as f64 / 100.0)).unwrap_or_default();
        let names = r.pointer("/tts/names").and_then(Value::as_str).unwrap_or("").to_string();
        let choices = ["always", "multi", "never"].iter().map(|k| (k.to_string(), t!(&format!("bot.tts_names_{k}")))).collect();
        vec![
            text_field("volume", t!("bot.tts_volume"), "basic", num(10.0, 150.0), n("/tts/volume"), "100"),
            text_field("speed", t!("bot.tts_speed"), "basic", num(0.8, 1.5), speed, "1"),
            text_field("pitch", t!("bot.tts_pitch"), "basic", num(-6.0, 6.0), n("/tts/pitch"), "0"),
            text_field("names", t!("bot.tts_names"), "basic", Kind::Choice(choices), names, "multi"),
            text_field("voice-chat-only", t!("bot.tts_voice_chat_only"), "basic", Kind::Bool, b("/tts/voiceChatOnly"), "false"),
            text_field("keep-read", t!("bot.tts_keep_read"), "basic", Kind::Bool, b("/tts/keepRead"), "false"),
        ]
    };
    Form::new(fields)
}

/// 권한 레벨을 `perm` 명령이 받는 낱말로 (기본 단계는 영어 이름, 더한 단계는 그 이름)
fn level_arg(id: i64, ladder: &[Value]) -> String {
    match id {
        3 => "owner".into(),
        2 => "admin".into(),
        1 => "user".into(),
        0 => "guest".into(),
        -1 => "banned".into(),
        other => ladder.iter().find(|l| l.get("id").and_then(Value::as_i64) == Some(other)).map(|l| s(l, "name").to_string()).unwrap_or_else(|| other.to_string()),
    }
}

fn level_label(id: i64, ladder: &[Value]) -> String {
    match ladder.iter().find(|l| l.get("id").and_then(Value::as_i64) == Some(id)).map(|l| s(l, "name")).filter(|n| !n.is_empty()) {
        Some(name) => name.to_string(),
        None => t!(&format!(
            "perm.level.{}",
            match id {
                3 => "owner",
                2 => "admin",
                1 => "user",
                0 => "chatbot",
                _ => "banned",
            }
        )),
    }
}

/// 권한 폼: 명령마다 쓸 수 있는 가장 낮은 레벨(서버별로 묶음), 그 아래 멤버 레벨
pub fn perm_form(perm: &Value, board: &Value, members: &Value) -> Form {
    let mut ladder: Vec<Value> = perm.get("ladder").and_then(Value::as_array).cloned().unwrap_or_default();
    if ladder.len() < 3 {
        ladder = [3, 2, 1, 0, -1].iter().map(|id| serde_json::json!({ "id": id })).collect();
    }
    let ids: Vec<i64> = ladder.iter().filter_map(|l| l.get("id").and_then(Value::as_i64)).collect();
    let choices = |skip_owner: bool| -> Vec<(String, String)> {
        ids.iter().filter(|id| !(skip_owner && **id == 3)).map(|id| (level_arg(*id, &ladder), level_label(*id, &ladder))).collect()
    };
    let servers: std::collections::HashMap<&str, &str> =
        board["instances"].as_array().into_iter().flatten().filter_map(|i| Some((i["id"].as_str()?, i["name"].as_str()?))).collect();
    let mut fields = Vec::new();
    for item in board["items"].as_array().into_iter().flatten() {
        let key = s(item, "key");
        let Some(level) = item.get("level").and_then(Value::as_i64) else { continue };
        if key.is_empty() {
            continue;
        }
        let title = if s(item, "title").is_empty() { s(item, "name") } else { s(item, "title") };
        let group = item["instance"].as_str().map(|id| servers.get(id).copied().unwrap_or(id).to_string()).unwrap_or_else(|| "common".into());
        fields.push(text_field(key, title.to_string(), &group, Kind::Choice(choices(false)), level_arg(level, &ladder), ""));
    }
    let names: std::collections::HashMap<String, String> =
        crate::resolve::member_items(members).iter().map(|m| (s(m, "id").to_string(), crate::resolve::member_label(m))).collect();
    for (id, level) in perm.get("members").and_then(Value::as_object).into_iter().flatten() {
        let level = level.as_i64().unwrap_or(1);
        let label = names.get(id).cloned().unwrap_or_else(|| id.clone());
        fields.push(text_field(&format!("member:{id}"), label, "members", Kind::Choice(choices(true)), level_arg(level, &ladder), ""));
    }
    Form::new(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 데몬이 꺼지면 데몬에 딸린 값(사용량·디스코드·전환 가능 여부)을 지운다. 이번에 읽지 않은 목록은 남긴다
    #[test]
    fn values_tied_to_the_daemon_are_cleared_when_it_goes_away() {
        let mut snap = Snapshot::default();
        snap.apply(Fetched {
            health: Some(json!({})),
            launchable: Some(json!({ "gui": true })),
            modules: Some(vec![json!({ "name": "palworld" })]),
            ..Fetched::default()
        });
        snap.apply(Fetched { health: Some(json!({})), ..Fetched::default() });
        assert!(snap.launchable.is_some() && snap.modules.len() == 1);
        snap.apply(Fetched::default());
        assert!(snap.health.is_none() && snap.launchable.is_none() && snap.metrics.is_none());
    }

    /// 권한 폼: 명령은 서버 이름으로 묶고 값은 `perm`이 받는 레벨 낱말, 멤버는 이름으로 (소유자는 고를 수 없다)
    #[test]
    fn the_permission_form_groups_commands_by_server_and_names_members() {
        crate::i18n::init("ko");
        let perm = json!({ "members": { "42": 0 } });
        let board = json!({
            "instances": [{ "id": "i1", "name": "마크야생" }],
            "items": [
                { "key": "start", "name": "start", "title": "시작", "instance": null, "level": 1 },
                { "key": "i1/kick", "name": "kick", "title": "추방", "instance": "i1", "level": 2 },
                { "key": "", "title": "빈 키", "level": 1 },
            ],
        });
        let members = json!({ "members": [{ "id": "42", "displayName": "철수", "username": "cs" }] });
        let f = perm_form(&perm, &board, &members);
        let row = |k: &str| f.fields.iter().find(|x| x.key == k).map(|x| (x.group.as_str(), x.label.as_str(), x.value.as_str()));
        assert_eq!(row("start"), Some(("common", "시작", "user")));
        assert_eq!(row("i1/kick"), Some(("마크야생", "추방", "admin")));
        assert_eq!(row("member:42"), Some(("members", "철수 (@cs)", "guest")));
        assert_eq!(f.fields.len(), 3);
        let Kind::Choice(levels) = &f.fields[2].kind else { panic!("members pick a level") };
        assert!(!levels.iter().any(|(k, _)| k == "owner"));
    }

    /// 읽어주기 속도는 100배로 저장된다: 폼에는 배속으로 (`bot tts --speed 1.2`와 같은 단위)
    #[test]
    fn the_tts_form_shows_speed_as_a_rate() {
        crate::i18n::init("ko");
        let f = discord_form("tts", &json!({ "tts": { "speed": 120, "volume": 80, "names": "never", "keepRead": true } }));
        let v = |k: &str| f.fields.iter().find(|x| x.key == k).map(|x| x.value.clone()).unwrap();
        assert_eq!((v("speed"), v("volume"), v("names"), v("keep-read"), v("pitch")), ("1.2".into(), "80".into(), "never".into(), "true".into(), String::new()));
        let m = discord_form("music", &json!({ "music": { "enabled": true, "queueLines": 5 } }));
        assert_eq!(m.fields.iter().find(|x| x.key == "enabled").unwrap().value, "true");
    }

    /// 앱 설정 폼은 데몬·CLI가 쓰는 칸만, 묶음별로 (GUI 창 설정은 보이지 않는다)
    #[test]
    fn the_config_form_shows_only_what_the_daemon_and_cli_use() {
        crate::i18n::init("ko");
        let f = config_form(&json!({ "language": "ja", "windowOpacity": 100, "sounds": { "enabled": true }, "newServerBackupKeep": 5 }));
        let keys: Vec<(&str, &str)> = f.fields.iter().map(|x| (x.key.as_str(), x.group.as_str())).collect();
        assert!(keys.contains(&("language", "connection")) && keys.contains(&("newServerBackupKeep", "servers")) && keys.contains(&("auditKeepDays", "storage")));
        assert!(!keys.iter().any(|(k, _)| *k == "windowOpacity" || k.starts_with("sounds")));
        assert_eq!(f.fields.iter().find(|x| x.key == "newServerBackupKeep").unwrap().value, "5");
        assert_eq!(f.fields.iter().find(|x| x.key == "language").unwrap().value, "ja");
        assert!(f.fields.iter().all(|x| !x.label.starts_with("tui.")), "every field has a label");
    }

    /// 번역 판(`locales_version`)이 바뀌면 cli.json을 다시 읽으라고 알린다. 처음 읽은 판·판을 모르는 데몬·이번에 못 읽음은 아니다
    #[test]
    fn reports_a_changed_locales_version() {
        let status = |v: Value| Fetched { health: Some(json!({})), updates: Some(json!({ "ok": true, "locales_version": v })), ..Fetched::default() };
        let mut snap = Snapshot::default();
        assert!(!snap.apply(status(json!("0.1.13"))));
        assert!(!snap.apply(status(json!("0.1.13"))));
        assert!(!snap.apply(Fetched::default()));
        assert!(snap.apply(status(json!("0.1.14"))));
        assert!(snap.apply(status(Value::Null)));
        assert!(!snap.apply(Fetched { health: Some(json!({})), updates: Some(json!({ "ok": true })), ..Fetched::default() }));
    }
}
