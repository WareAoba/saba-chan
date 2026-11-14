//! 하트비트 메타데이터에 싣는 항목: 서버, 모듈·명령, 디스코드 권한·허용 인스턴스

use super::*;

/// 길드당 싣는 명령별 덮어쓰기 수. 넘으면 키 순으로 자르고 `commandLevelsTruncated`를 둔다 (릴레이는 데몬 판정에 맡긴다)
pub(super) const COMMAND_LEVELS_PER_GUILD: usize = 200;

/// 길드별 `{defaultLevel, members, chatbotCap, levels, commandLevels, required}` (릴레이 바깥 검사용). `nodeSettings`에 있는 길드는
/// 값이 없어도 늘 싣는다 (부여값을 다 지운 길드도 릴레이가 데몬 행을 지우게). 릴레이에서만 부여한 행은 릴레이가 남긴다.
/// `levels`는 그 길드 프리셋으로 푼 risk별 요구 레벨이다. 최종 판정은 데몬이 다시 한다
pub(super) fn permissions_by_guild(node_settings: &Value, targets: &crate::actions::RemoteTargets) -> Value {
    let map: serde_json::Map<String, Value> =
        node_settings.as_object().into_iter().flatten().map(|(guild, node)| (guild.clone(), permission_entry(node, targets))).collect();
    Value::Object(map)
}

/// 한 길드(`nodeSettings[guild]`)의 권한 스냅샷 (heartbeat `permissions[guild]`, `permission` action 응답이 같은 모양)
pub(super) fn permission_entry(node: &Value, targets: &crate::actions::RemoteTargets) -> Value {
    let p = &crate::chat::permissions::NodePermissions::from_node(Some(node));
    let mut keys: Vec<&String> = p.command_levels.keys().collect();
    keys.sort();
    let command_levels: serde_json::Map<String, Value> =
        keys.iter().take(COMMAND_LEVELS_PER_GUILD).map(|k| ((*k).clone(), json!(p.command_levels[*k]))).collect();
    let levels = json!({
        "safe": p.preset_need("safe"),
        "caution": p.preset_need("caution"),
        "danger": p.preset_need("danger"),
        // 관리(Kind::Admin) 액션은 risk 대신 이 레벨이다 (`actions::authorize`)
        "admin": p.admin_need(),
    });
    let mut entry = json!({
        "defaultLevel": p.default_level,
        "members": p.members,
        "chatbotCap": p.chatbot_cap,
        "levels": levels,
        "commandLevels": command_levels,
        // 단계 순서 (높은 것부터). 릴레이는 숫자가 아니라 이 순서로 견준다
        "ladder": p.ladder,
    });
    if keys.len() > COMMAND_LEVELS_PER_GUILD {
        // 덮어쓰기가 이만큼 많으면 서버별 요구 레벨도 그만큼 커지므로 싣지 않는다 (릴레이는 모르는 것으로 보고 데몬 판정에 맡긴다)
        entry["commandLevelsTruncated"] = json!(true);
    } else {
        // 액션·명령마다 지금 요구 레벨 (SINCE 0.1.14-beta.2). 릴레이는 이 값만 읽고 규칙을 따로 갖지 않는다
        let allowed: Option<Vec<&str>> = node.get("allowedInstances").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect());
        entry["required"] = crate::actions::required_snapshot(p, allowed.as_deref(), targets);
    }
    entry
}

/// 모듈 이름 → 디스코드·챗봇에 내보내는 명령 (이름, risk). heartbeat `modules[].commands.fields`와 같은 목록이다
pub(super) fn remote_commands(views: &[crate::chat::api::ModuleView]) -> Vec<(String, Vec<(String, String)>)> {
    views
        .iter()
        .map(|m| {
            let list = m.commands.iter().filter(|c| c.remote).map(|c| (c.name.clone(), crate::commands::risk_of(c.risk.as_deref()).to_string())).collect();
            (m.name.clone(), list)
        })
        .collect()
}

/// heartbeat `servers` 항목 → (인스턴스 id, 모듈 이름)
pub(super) fn remote_instances(servers: &[Value]) -> Vec<(String, String)> {
    let text = |s: &Value, key: &str| s.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    servers.iter().map(|s| (text(s, "id"), text(s, "module"))).collect()
}

/// 요구 레벨을 셀 대상 (`permission` action 응답용. heartbeat는 이미 읽은 목록을 쓴다)
pub(super) async fn remote_targets(server: &crate::ipc::IPCServer) -> crate::actions::RemoteTargets {
    use crate::chat::api::DaemonApi;
    let loaded = server.supervisor.read().await.list_modules().unwrap_or_default();
    let (actions, _) = crate::actions::heartbeat_actions(&crate::chat::i18n::languages(), &loaded);
    let instances = server.supervisor.read().await.instance_store.list().iter().map(|i| (i.id.clone(), i.module_name.clone())).collect();
    crate::actions::RemoteTargets { actions, commands: remote_commands(&server.modules().await), instances }
}

/// `bot.node_settings[guild].allowedInstances` → `{ guild: [instance id, ...] }`. 하나도 없으면 None
pub(super) fn allowed_instances_by_guild(node_settings: &Value) -> Option<Value> {
    ids_by_guild(node_settings, "allowedInstances")
}

/// `bot.node_settings[guild].<key>`의 인스턴스 ID 목록 → `{ guild: [instance id, ...] }`. 목록이 있는 길드가 없으면 None
pub(super) fn ids_by_guild(node_settings: &Value, key: &str) -> Option<Value> {
    let map: serde_json::Map<String, Value> = node_settings
        .as_object()?
        .iter()
        .filter_map(|(guild, cfg)| {
            let ids: Vec<Value> = cfg
                .get(key)?
                .as_array()?
                .iter()
                .filter(|v| v.is_string())
                .cloned()
                .collect();
            Some((guild.clone(), Value::Array(ids)))
        })
        .collect();
    (!map.is_empty()).then_some(Value::Object(map))
}

/// 하트비트의 모듈 하나: 이름·별명·원격 명령·프리셋과 언어별 이름·별명.
/// 모듈 locales 파일을 읽으므로 async 작업자 밖에서 부른다
pub(super) fn relay_module_entry(m: &crate::chat::api::ModuleView) -> Value {
    // 번역한 명령 이름: 모듈 locales를 언어마다 한 번 읽는다. 번역을 끈 서버는 릴레이가 빼고 단다 (서버별 `translateCommands`)
    let locales: std::collections::HashMap<&str, std::sync::Arc<Value>> = match &m.locale_dir {
        Some(dir) => crate::chat::i18n::languages()
            .into_iter()
            .filter_map(|l| Some((l, crate::chat::api::module_locale(dir, l)?)))
            .collect(),
        None => Default::default(),
    };
    // 명령마다·언어마다 부르므로 locales를 복제하지 않고 함께 쓴다
    let null = std::sync::Arc::new(Value::Null);
    let texts = |l: &str| locales.get(l).unwrap_or(&null).clone();
    let names = &texts;
    let texts: Option<&LocaleTexts<'_>> = Some(&texts);
    let mut entry = json!({
        "name": m.name,
        // 게임명도 별명처럼 서버를 부른다: 릴레이가 위험 명령 확인에서 같은 서버를 찾도록 함께 보낸다.
        // 여기는 언어 없는 낱말(module.toml)이고, 언어마다 더하는 낱말은 `module_aliases_by_language`다
        "module_aliases": m.game_names.iter().chain(&m.module_aliases).collect::<Vec<_>>(),
        // 디스코드·챗봇에 내보내지 않는 명령(`remote = false`)은 싣지 않는다
        "commands": { "fields": m.commands.iter().filter(|c| c.remote).map(|c| relay_command_entry(c, m.aliases_of(&c.name), texts)).collect::<Vec<_>>() },
        "presets": m.presets.iter().map(|p| json!({ "id": p.id, "label": p.label })).collect::<Vec<_>>(),
    });
    relay_module_names(&mut entry, m.display_name.as_deref(), names);
    if let Some(dir) = &m.locale_dir {
        relay_language_aliases(&mut entry, dir);
    }
    entry
}

/// 언어 → 모듈 locales 한 언어 전체 (없으면 Null). 명령마다·언어마다 부르므로 복제하지 않고 함께 쓴다
type LocaleTexts<'a> = dyn Fn(&str) -> std::sync::Arc<Value> + 'a;

/// 모듈의 게임 이름 — `display_name`(module.toml)과 `display_names`(언어 → 모듈 locales `display_name`). 없으면 키를 싣지 않는다.
/// 릴레이가 상황판 채널에 켜진 게임을 보이는 데 쓴다 (`game_names`와 달리 띄어 쓴 이름도 그대로 싣는다)
pub(super) fn relay_module_names(entry: &mut Value, display_name: Option<&str>, texts: &LocaleTexts<'_>) {
    if let Some(name) = display_name.map(str::trim).filter(|n| !n.is_empty()) {
        entry["display_name"] = json!(name);
    }
    let names: serde_json::Map<String, Value> = crate::chat::i18n::languages()
        .into_iter()
        .filter_map(|l| {
            let name = crate::chat::api::locale_display_name(&texts(l))?.to_string();
            Some((l.to_string(), json!(name)))
        })
        .collect();
    if !names.is_empty() {
        entry["display_names"] = Value::Object(names);
    }
}

/// 언어마다 더하는 별명 (모듈 locales, `chat::api::locale_aliases`): 모듈 `module_aliases_by_language`(그 언어 게임명과 `aliases.module`),
/// 명령 `commands.fields[].aliases_by_language`(`aliases.commands.<이름>`). 릴레이는 길드 언어의 것만 `module_aliases`·`aliases`에 더한다
pub(super) fn relay_language_aliases(entry: &mut Value, module_dir: &std::path::Path) {
    let mut module_by = serde_json::Map::new();
    let mut commands_by: std::collections::HashMap<String, serde_json::Map<String, Value>> = Default::default();
    for l in crate::chat::i18n::languages() {
        let Some(found) = crate::chat::api::locale_aliases(module_dir, l) else { continue };
        let mut words: Vec<String> = Vec::new();
        crate::chat::api::push_words(&mut words, found.game_name.into_iter().chain(found.module));
        if !words.is_empty() {
            module_by.insert(l.to_string(), json!(words));
        }
        for (cmd, words) in found.commands {
            commands_by.entry(cmd).or_default().insert(l.to_string(), json!(words));
        }
    }
    if !module_by.is_empty() {
        entry["module_aliases_by_language"] = Value::Object(module_by);
    }
    for field in entry["commands"]["fields"].as_array_mut().into_iter().flatten() {
        if let Some(by) = field["name"].as_str().and_then(|n| commands_by.remove(n)) {
            field["aliases_by_language"] = Value::Object(by);
        }
    }
}

/// 하트비트의 모듈 명령 한 줄: relay가 슬래시·챗봇 도구를 만드는 데 쓰는 필드만 (템플릿·엔드포인트는 PC 안에서만 쓴다).
/// `aliases`는 module.toml `[aliases.commands]`의 별칭 — 별칭으로 친 명령도 relay가 위험도 확인을 거치게 한다
/// `texts`: 언어 → 모듈 locales (번역한 명령 이름이 켜져 있을 때). 있으면 `commands.<이름>.label`·`description`과
/// 칸의 `inputs.<칸>.label`을 언어별 `titles`·`descriptions`로 싣는다 (릴레이 슬래시 명령의 언어별 이름·설명)
pub(super) fn relay_command_entry(c: &crate::module_loader::CommandField, aliases: &[String], texts: Option<&LocaleTexts<'_>>) -> Value {
    let by_language = |pointer: &str| -> Option<serde_json::Map<String, Value>> {
        let texts = texts?;
        let map: serde_json::Map<String, Value> = crate::chat::i18n::languages()
            .into_iter()
            .filter_map(|l| Some((l.to_string(), json!(texts(l).pointer(pointer)?.as_str()?))))
            .collect();
        (!map.is_empty()).then_some(map)
    };
    let inputs: Vec<Value> = c
        .inputs
        .iter()
        .map(|i| {
            let mut input = json!({ "name": i.name, "label": i.label, "type": i.input_type, "required": i.required, "min": i.min, "max": i.max, "options": i.options });
            if let Some(titles) = by_language(&format!("/commands/{}/inputs/{}/label", c.name, i.name)) {
                input["titles"] = Value::Object(titles);
            }
            input
        })
        .collect();
    let mut entry = json!({ "name": c.name, "label": c.label, "description": c.description, "risk": c.risk, "inputs": inputs });
    if !aliases.is_empty() {
        entry["aliases"] = json!(aliases);
    }
    for (key, text) in [("titles", "label"), ("descriptions", "description")] {
        if let Some(map) = by_language(&format!("/commands/{}/{}", c.name, text)) {
            entry[key] = Value::Object(map);
        }
    }
    entry
}

/// 익스텐션 상태 줄 길이 상한 (글자)
const STATUS_LINE_MAX: usize = 100;

/// 하트비트의 서버 한 줄: relay가 읽는 필드만 싣는다. 설정값·경로·비밀번호는 PC 밖으로 나가지 않는다.
/// 실행 중이면 `usage{cpu, memMb}`, `players{online, max?}`(Q-5), 켜진 시각 `startedAt`(Unix 초), 익스텐션이 `server.list_enrich`에서 `status_line`을 주면 `statusLine`(X-1)
pub(super) fn relay_server_entry(server: &Value, usage: Option<&crate::system_monitor::InstanceMetrics>) -> Value {
    let mut entry = serde_json::Map::new();
    for key in ["id", "name", "module", "status"] {
        if let Some(v) = server.get(key) {
            entry.insert(key.to_string(), v.clone());
        }
    }
    if let Some(u) = usage.filter(|u| u.running) {
        let cpu = (f64::from(u.cpu_percent) * 10.0).round() / 10.0;
        entry.insert("usage".into(), json!({ "cpu": cpu, "memMb": u.memory_bytes / (1024 * 1024) }));
    }
    let players = server.get("players").filter(|_| server.get("status").and_then(Value::as_str) == Some("running"));
    if let Some(count) = players.and_then(crate::supervisor::players::parse) {
        entry.insert("players".into(), json!(count));
    }
    // 전용 멤버 서버: 모드와 명단의 디스코드 ID (릴레이가 시작·정지를 그 사람에게도 보인다). 게임 닉네임은 올리지 않는다.
    // 서버 공용은 명단을 쓰지 않으므로 올리지 않는다 (판정은 데몬, `controllers::gate_for`)
    if server.get("access").and_then(Value::as_str) == Some("members") {
        entry.insert("access".into(), json!("members"));
        let controllers: Vec<&str> =
            server.get("controllers").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("discord_id")?.as_str()).collect();
        if !controllers.is_empty() {
            entry.insert("controllers".into(), json!(controllers));
        }
    }
    // 켜진 시각 (Unix 초) — 상황판 가동 시간
    let running = server.get("status").and_then(Value::as_str) == Some("running");
    if let Some(at) = server.get("start_time").and_then(Value::as_u64).filter(|_| running) {
        entry.insert("startedAt".into(), json!(at));
    }
    // 익스텐션 id 순으로 첫 번째 상태 줄. 제어 문자를 빼고 한 줄로 자른다
    let line = server.get("extension_status").and_then(|v| v.as_object()).and_then(|statuses| {
        let mut ids: Vec<&String> = statuses.keys().collect();
        ids.sort();
        ids.into_iter().find_map(|id| {
            let text: String = statuses[id].get("status_line")?.as_str()?.chars().filter(|c| !c.is_control()).take(STATUS_LINE_MAX).collect();
            Some(text.trim().to_string()).filter(|t| !t.is_empty())
        })
    });
    if let Some(line) = line {
        entry.insert("statusLine".into(), json!(line));
    }
    // 채팅 연동 (`supervisor::chat_link`): 릴레이가 디스코드 → 게임 글을 보낼 채널
    let links = server.get("id").and_then(Value::as_str).map(crate::supervisor::chat_link::list).unwrap_or_default();
    if !links.is_empty() {
        entry.insert("chatLinks".into(), json!(links));
    }
    Value::Object(entry)
}
