//! `server settings`·`set`·`properties`: 서버 설정 읽고 쓰기

use super::*;

/// 모듈 설정 칸 (이름 → 칸 정의)
pub(super) fn setting_fields(module: &Value) -> Vec<Value> {
    module
        .pointer("/settings/fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// 모듈이 선언하지 않아도 모든 서버에 있는 칸과 그 형식: 인스턴스 공통 칸과 데몬이 읽는 설정 (데몬과 같은 표)
fn common() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    use saba_chan_updater_lib::constants::{DAEMON_SETTINGS, INSTANCE_FIELDS};
    INSTANCE_FIELDS.iter().chain(DAEMON_SETTINGS)
}

/// KEY=VALUE 하나를 PATCH 키와 형식 맞춘 값으로. `ext.키`는 extension_data, 빈 값은 지우기(null).
/// 모듈 정의에도, 공통 칸에도, 지금 값에도 없는 키 (오타일 가능성이 높다)
fn unknown_setting(fields: &[Value], key: &str) -> anyhow::Error {
    let names: Vec<String> = fields
        .iter()
        .map(|f| s(f, "name").to_string())
        .chain(common().map(|(k, _)| k.to_string()))
        .collect();
    let suggestions = match resolve::find(&names, key, |n| vec![n.clone()], |n| n.clone()) {
        resolve::Found::None(s) => s,
        resolve::Found::One(n) => vec![n],
        resolve::Found::Many(v) => v.into_iter().take(3).collect(),
    };
    CliError::NotFound {
        what: t!("noun.setting"),
        query: key.into(),
        suggestions,
    }
    .into()
}

pub(super) fn typed_setting(fields: &[Value], instance: &Value, key: &str, raw: &str) -> Result<(String, Value)> {
    if let Some(ext_key) = key.strip_prefix("ext.") {
        let v = if raw.is_empty() {
            Value::Null
        } else {
            crate::cmd::guess(raw)
        };
        return Ok((format!("extension_data.{}", ext_key), v));
    }
    let kind = fields
        .iter()
        .find(|f| s(f, "name") == key)
        .map(|f| s(f, "field_type").to_string())
        .or_else(|| common().find(|(k, _)| *k == key).map(|(_, t)| t.to_string()))
        .or_else(|| {
            // 정의에 없는 키는 지금 값의 형식을 따른다
            let cur = instance
                .get(key)
                .or_else(|| instance.pointer(&format!("/module_settings/{}", key)))?;
            Some(match cur {
                Value::Bool(_) => "boolean".into(),
                Value::Number(_) => "number".into(),
                _ => "text".into(),
            })
        })
        .ok_or_else(|| unknown_setting(fields, key))?;
    // 공통 칸은 빈 값이 지우기다 (예: `memory_limit_mb=` → 제한 없음). 데몬이 비우지 않는 칸은 거절한다
    if raw.trim().is_empty() && common().any(|(k, _)| *k == key) && !fields.iter().any(|f| s(f, "name") == key) {
        if saba_chan_updater_lib::constants::UNCLEARABLE_INSTANCE_FIELDS.contains(&key) {
            return Err(usage(t!("set.cannot_clear", key = key)));
        }
        return Ok((key.to_string(), Value::Null));
    }
    let field = fields.iter().find(|f| s(f, "name") == key);
    if let Some(opts) = field.and_then(|f| f.get("options")).and_then(Value::as_array) {
        let opts: Vec<&str> = opts.iter().filter_map(Value::as_str).collect();
        if !opts.is_empty() {
            // 데몬은 선택지를 그대로 비교하므로, 대소문자만 다르게 친 값은 모듈이 정한 선택지로 보낸다 (L58)
            let pick = |raw: &str| {
                opts.iter()
                    .find(|o| **o == raw)
                    .or_else(|| opts.iter().find(|o| o.eq_ignore_ascii_case(raw)))
                    .map(|o| o.to_string())
                    .ok_or_else(|| usage(t!("set.must_be_one_of", key = key, options = opts.join(", "))))
            };
            // 여러 개 고르는 목록은 항목마다
            if kind == "list" {
                let items = typed(raw, &kind)?;
                let picked = items
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|i| pick(i.as_str().unwrap_or_default()).map(Value::String))
                    .collect::<Result<Vec<_>>>()?;
                return Ok((key.to_string(), Value::Array(picked)));
            }
            return Ok((key.to_string(), typed(&pick(raw)?, &kind)?));
        }
    }
    Ok((key.to_string(), typed(raw, &kind)?))
}

/// 선택형 설정의 선택지 (`a | b | c`). 선택지가 없으면 None
pub(super) fn options_line(f: &Value) -> Option<String> {
    let opts: Vec<&str> = f.get("options")?.as_array()?.iter().filter_map(Value::as_str).collect();
    (!opts.is_empty()).then(|| opts.join(" | "))
}

/// 설정 값 한 줄: 목록은 쉼표로, 여러 줄은 ⏎로 잇는다. 비밀번호 같은 값은 가린다
pub(super) fn setting_text(name: &str, val: &Value) -> String {
    match val {
        Value::Array(items) if items.iter().all(Value::is_string) => output::mask(
            name,
            &json!(items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
        ),
        Value::String(text) if text.contains('\n') => {
            output::mask(name, &json!(text.lines().collect::<Vec<_>>().join(" ⏎ ")))
        }
        _ => output::mask(name, val),
    }
}

/// 설정 옆 표시: 켜져 있을 때 바꾸면 다시 켜야 적용되는 설정 (06 §8.2)
pub(super) fn setting_marks(f: &Value) -> String {
    if f.get("restart_required").and_then(Value::as_bool) == Some(true) {
        output::dim(&format!("({})", t!("set.restart_required_mark")))
    } else {
        String::new()
    }
}

/// 저장 응답의 `restart_required`: 켜져 있을 때 값이 바뀌어 다시 켜야 적용되는 설정 이름
pub(super) fn restart_required(r: &Value) -> Vec<String> {
    r.get("restart_required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

pub(super) fn insert_patch(patch: &mut Map<String, Value>, key: &str, value: Value) {
    match key.strip_prefix("extension_data.") {
        Some(k) => {
            let ext = patch.entry("extension_data").or_insert_with(|| json!({}));
            ext.as_object_mut().unwrap().insert(k.to_string(), value);
        }
        None => {
            patch.insert(key.to_string(), value);
        }
    }
}

/// 서버 설정 화면에 쓰는 것을 한 번에 읽은 것 (`server settings`와 TUI 설정 탭이 같이 쓴다)
pub struct SettingsData {
    pub sv: Server,
    fields: Vec<Value>,
    hidden: Vec<String>,
    live: Option<Value>,
    locales: Option<Value>,
    ms: Value,
}

impl SettingsData {
    fn locale(&self, f: &Value, part: &str) -> Option<String> {
        self.locales
            .as_ref()
            .and_then(|l| l.pointer(&format!("/settings/{}/{part}", s(f, "name"))))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    fn label(&self, f: &Value) -> String {
        self.locale(f, "label").unwrap_or_else(|| {
            if s(f, "label").is_empty() {
                s(f, "name").to_string()
            } else {
                s(f, "label").to_string()
            }
        })
    }

    fn current(&self, name: &str) -> Value {
        self.live
            .as_ref()
            .and_then(|l| l.get(name))
            .filter(|v| !v.is_null())
            .or_else(|| self.sv.raw.get(name).filter(|v| !v.is_null()))
            .or_else(|| self.ms.get(name))
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// `--json` 문서: 칸마다 이름·종류·묶음·지금 값·기본값·선택지(번역된 이름도)·설명·범위
    pub fn doc(&self) -> Value {
        let out_fields: Vec<Value> = self
            .fields
            .iter()
            .map(|f| {
                let name = s(f, "name");
                let option_labels: Map<String, Value> = f
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(|o| {
                        self.locales
                            .as_ref()
                            .and_then(|l| l.pointer(&format!("/settings/{name}/options/{o}")))
                            .and_then(Value::as_str)
                            .map(|t| (o.to_string(), json!(t)))
                    })
                    .collect();
                json!({
                    "key": name, "label": self.label(f), "type": s(f, "field_type"), "group": f.get("group"),
                    "value": self.current(name), "default": f.get("default"), "options": f.get("options"),
                    "option_labels": option_labels,
                    "description": self.locale(f, "description").or_else(|| f.get("description").and_then(Value::as_str).map(str::to_string)),
                    "min": f.get("min"), "max": f.get("max"),
                    "hidden": self.hidden.iter().any(|h| h == name),
                    "restart_required": f.get("restart_required").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        json!({ "server": self.sv.name, "live": self.live.is_some(), "fields": out_fields, "instance": self.sv.raw })
    }
}

pub async fn load_settings(ctx: &Ctx, query: Option<&str>) -> Result<SettingsData> {
    let sv = resolve::server(ctx, query).await?;
    let (module, locales, live) = tokio::join!(
        module_info(ctx, &sv.module),
        module_locales(ctx, &sv.module),
        ctx.api.live_settings(&sv.id)
    );
    let module = module.unwrap_or(Value::Null);
    let fields = setting_fields(&module);
    // 다른 설정 값에 따라 지금 숨긴 설정 (저장된 값 기준, 06 §8.2)
    let hidden: Vec<String> = ctx
        .api
        .instance(&sv.id)
        .await
        .ok()
        .and_then(|r| r.get("hidden").cloned())
        .and_then(|h| serde_json::from_value(h).ok())
        .unwrap_or_default();
    // 실제 설정 파일 값이 우선. 못 읽으면 (설치 전, 모듈이 지원 안 함) 저장된 값으로.
    let live = live
        .ok()
        .and_then(|r| r.get("settings").cloned())
        .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    // 데몬이 읽은 값으로 인스턴스(포트·비밀번호 칸 등)도 맞췄으니 다시 받는다
    let sv = match &live {
        Some(_) => resolve::server(ctx, Some(&sv.id)).await.unwrap_or(sv),
        None => sv,
    };
    let ms = sv.raw.get("module_settings").cloned().unwrap_or(json!({}));
    Ok(SettingsData { sv, fields, hidden, live, locales, ms })
}

pub(super) async fn settings(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let data = load_settings(ctx, query).await?;
    let (sv, fields, hidden, ms) = (&data.sv, &data.fields, &data.hidden, &data.ms);
    let label = |f: &Value| data.label(f);
    let current = |name: &str| data.current(name);
    // 꺼진 서버는 다시 켤 일이 없다: "바꾸면 다시 시작" 표시는 켜져 있을 때만
    let running = matches!(s(&sv.raw, "status"), "running" | "starting");
    let v = data.doc();
    ctx.out.data(&v, |_| {
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut push = |group: &str, k: String, v: String| match groups.iter_mut().find(|(g, _)| g == group) {
            Some((_, rows)) => rows.push((k, v)),
            None => groups.push((group.to_string(), vec![(k, v)])),
        };
        let mut seen: Vec<String> = Vec::new();
        for f in fields {
            let name = s(f, "name");
            if hidden.iter().any(|h| h == name) {
                seen.push(name.to_string());
                continue;
            }
            let group = f.get("group").and_then(Value::as_str).unwrap_or("basic");
            let val = current(name);
            let mut shown = setting_text(name, &val);
            if val.is_null() {
                if let Some(d) = f.get("default").filter(|d| !d.is_null()) {
                    shown = output::dim(&format!("{} ({})", setting_text(name, d), t!("set.default")));
                }
            }
            if let Some(line) = options_line(f) {
                shown = format!("{}  {}", shown, output::dim(&format!("({})", line)));
            }
            let marks = if running { setting_marks(f) } else { String::new() };
            if !marks.is_empty() {
                shown = format!("{}  {}", shown, marks);
            }
            push(
                group,
                format!("{} {}", label(f), output::dim(&format!("[{}]", name))),
                shown,
            );
            seen.push(name.to_string());
        }
        for (k, _) in saba_chan_updater_lib::constants::INSTANCE_FIELDS {
            if seen.iter().any(|s| s == k) {
                continue;
            }
            if let Some(val) = sv.raw.get(*k).filter(|v| !v.is_null()) {
                push("saba-chan", output::dim(&format!("[{}]", k)), output::mask(k, val));
            }
        }
        // 데몬이 읽는 설정은 값이 없어도 보인다: 바꿀 수 있다는 것을 알 수 있게
        for (k, _) in saba_chan_updater_lib::constants::DAEMON_SETTINGS {
            if seen.iter().any(|s| s == k) {
                continue;
            }
            let shown = match ms.get(*k).filter(|v| !v.is_null()) {
                Some(val) => output::mask(k, val),
                None => output::dim(&format!("({})", t!("set.default"))),
            };
            push("saba-chan", output::dim(&format!("[{}]", k)), shown);
            seen.push(k.to_string());
        }
        if let Some(map) = ms.as_object() {
            for (k, val) in map.iter().filter(|(k, _)| !seen.contains(k)) {
                push("other", output::dim(&format!("[{}]", k)), output::mask(k, val));
            }
        }
        if let Some(map) = sv.raw.get("extension_data").and_then(Value::as_object) {
            for (k, val) in map {
                push("extension", output::dim(&format!("[ext.{}]", k)), output::mask(k, val));
            }
        }
        for (group, rows) in groups {
            let title = crate::i18n::try_t(&format!("set.group.{}", group)).unwrap_or(group);
            output::heading(&title);
            output::print_kv(&rows);
            say!();
        }
        if !hidden.is_empty() {
            say!("{}", output::dim(&t!("set.hidden", keys = hidden.join(", "))));
        }
        say!(
            "{}",
            output::dim(&format!("saba-chan-cli server set {} KEY=VALUE", shell_word(&sv.name)))
        );
    });
    Ok(())
}

pub(super) async fn set(ctx: &Ctx, query: &str, pairs: &[String]) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let module = module_info(ctx, &sv.module).await.unwrap_or(Value::Null);
    let fields = setting_fields(&module);
    let mut patch = Map::new();
    for (k, v) in split_pairs(pairs)? {
        let (key, value) = typed_setting(&fields, &sv.raw, &k, &v)?;
        insert_patch(&mut patch, &key, value);
    }
    let r = ctx.api.update_instance(&sv.id, Value::Object(patch.clone())).await?;
    let data = json!({ "success": true, "changed": patch, "response": r });
    // 게임 설정 파일에 쓰지 못했으면 끝에 실패로 알린다 (--json은 오류 문서 하나에 결과를 싣는다)
    let sync_error = sync_failure(&r);
    if sync_error.is_none() || !ctx.out.json {
        ctx.out.done(&t!("set.done", name = sv.name, count = pairs.len()), &data);
    }
    let ignored: Vec<&str> = r.get("ignored").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    if !ignored.is_empty() {
        ctx.out.warn(&t!("set.ignored", keys = ignored.join(", ")));
    }
    // 다시 켜야 적용되는 설정은 데몬이 알려 준다 (06 §8.2)
    let needs_restart = restart_required(&r);
    if !needs_restart.is_empty() {
        ctx.out
            .warn(&t!("set.restart_required", keys = needs_restart.join(", ")));
        ctx.out.hint(&t!("set.restart_hint", name = shell_word(&sv.name)));
    }
    match sync_error {
        Some(message) => Err(CliError::Partial { message, data }.into()),
        None => Ok(()),
    }
}

pub(super) async fn properties(ctx: &Ctx, query: Option<&str>, action: Option<PropsCmd>) -> Result<()> {
    let action = action.unwrap_or(PropsCmd::Show);
    // 초기화는 되돌릴 수 없다: 이름이 정확히 맞는 서버만 고른다
    let sv = if matches!(action, PropsCmd::Reset) {
        resolve::server_exact(ctx, query).await?
    } else {
        resolve::server(ctx, query).await?
    };
    match action {
        PropsCmd::Show => {
            let r = ctx.api.properties(&sv.id).await?;
            ctx.out.data(&r, |r| {
                if r.get("exists").and_then(Value::as_bool) == Some(false) {
                    ctx.out.note(&t!("props.defaults_shown"));
                }
                let props = r.get("properties").cloned().unwrap_or(json!({}));
                let mut rows: Vec<(String, String)> = props
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, v)| (k.clone(), output::mask(k, v)))
                    .collect();
                rows.sort();
                output::print_kv(&rows);
            });
        }
        PropsCmd::Set { pairs } => {
            let mut settings = Map::new();
            for (k, v) in split_pairs(&pairs)? {
                settings.insert(k, Value::String(v));
            }
            let r = ctx.api.write_properties(&sv.id, Value::Object(settings)).await?;
            ctx.out.done(&t!("props.saved", name = sv.name), &r);
        }
        PropsCmd::Reset => {
            if let Ok(m) = module_info(ctx, &sv.module).await {
                crate::cmd::module::ensure_supported(&m, "reset_properties")?;
            }
            ctx.ask.ensure(&t!("props.confirm_reset", name = sv.name))?;
            let r = ctx.api.reset_properties(&sv.id).await?;
            ctx.out.done(&t!("props.reset_done", name = sv.name), &r);
        }
    }
    Ok(())
}
