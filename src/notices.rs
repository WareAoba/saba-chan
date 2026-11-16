//! 알림함: 사용자가 확인할 일(크래시·시작 실패·응답 없음·복원·업데이트 결과 …)을 데몬이 보관한다.
//! `<데이터 폴더>/notices.json`에 최근 `KEPT`개를 두고, 앱·CLI·웹 콘솔·MCP가 같은 목록과 읽음 상태를 본다
//! (`GET /api/notices`). 무엇을 넣을지는 여기 한 곳(`from_event`)에서 정한다.
//!
//! 정상 시작·정지는 어디서 시켰든(앱·CLI·웹·디스코드·예약·자동 재시작·게임 안) 넣지 않는다 (사용자 2026-11-15).
//! 문장은 요청한 언어로 그때 만든다(`render`): 알림에는 문구 키와 값만 남긴다 (daemon.json `notices`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 보관하는 알림 수. 넘치면 오래된 것부터 지운다
pub const KEPT: usize = 200;
/// 알림함이 바뀌면 붙어 있는 클라이언트에게 보내는 이벤트 (`{unread}`)
pub const CHANGED: &str = "notices.changed";
/// 누르면 패치노트를 여는 업데이트 결과 알림. 지우면 업데이트 결과를 확인한 것으로 남긴다
pub const PATCH_NOTES_ACTION: &str = "openPatchNotes";
/// 누르면 업데이트 화면을 여는 모듈 새 버전 알림
pub const UPDATE_MODAL_ACTION: &str = "openUpdateModal";
/// 누르면 그 게임 서버를 업데이트하는 알림 (`data: {instanceId, name}`)
pub const SERVER_UPDATE_ACTION: &str = "applyServerUpdate";
/// 누르면 그 서버의 백업 화면을 여는 알림 (예약 백업 실패)
pub const OPEN_BACKUPS_ACTION: &str = "openBackups";
/// 누르면 그 서버의 예약 작업 화면을 여는 알림 (예약 작업 실패)
pub const OPEN_SCHEDULE_ACTION: &str = "openSchedule";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Success,
    Error,
}

/// daemon.json `notices.<key>`와 채울 값
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub key: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
}

impl Text {
    fn new(key: &str, args: Value) -> Self {
        Self { key: key.to_string(), args: args.as_object().cloned().unwrap_or_default() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: u64,
    /// unix 초
    pub at: u64,
    pub level: Level,
    pub text: Text,
    /// 이유·다시 켜기 안내처럼 둘째 줄에 " · "로 잇는 것
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// 알릴 때의 서버 이름 (지운 서버도 묶어 보이게)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
    #[serde(default)]
    pub read: bool,
    /// 같은 값의 알림은 하나만 둔다: 새 것이 오면 예전 것을 지우고 맨 위에 넣는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup: Option<String>,
}

/// 넣을 알림 (번호·시각은 넣을 때 붙인다)
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub level: Level,
    pub text: Text,
    pub detail: Vec<Text>,
    pub instance_id: Option<String>,
    pub instance: Option<String>,
    pub action: Option<String>,
    pub data: Value,
    pub read: bool,
    pub dedup: Option<String>,
}

impl Draft {
    fn new(level: Level, key: &str, args: Value) -> Self {
        Self {
            level,
            text: Text::new(key, args),
            detail: Vec::new(),
            instance_id: None,
            instance: None,
            action: None,
            data: Value::Null,
            read: false,
            dedup: None,
        }
    }

    /// 서버 하나에 묶는다. 이름이 없으면 id
    fn server(mut self, d: &Value) -> Self {
        let id = str_of(d, "instanceId");
        self.instance = str_of(d, "instance").or_else(|| id.clone());
        self.instance_id = id;
        self
    }
}

/// 이벤트가 알림함에 하는 일
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Add(Draft),
    /// 이 dedup 값의 알림을 지운다 (더 할 일이 없어졌다)
    Drop(String),
}

fn str_of(d: &Value, key: &str) -> Option<String> {
    d.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// 서버 종료 근거 코드 → 이유 문구. 콘솔 줄이 근거이거나 모르는 코드는 없다
const EXIT_REASONS: &[&str] = &["memory_limit", "exit_code", "signal", "crash_file", "ended_early"];
/// 알림함에 넣는 종료: 정상 정지(`user_stop`·`in_game_stop`)는 넣지 않는다
const EXIT_KINDS: &[&str] = &["crash", "start_failed", "unknown_exit"];

fn server_update_dedup(id: &str) -> String {
    format!("server_update:{id}")
}

/// 문장에 넣을 서버 이름. 없으면 id
fn server_name(d: &Value) -> Option<String> {
    str_of(d, "instance").or_else(|| str_of(d, "instanceId"))
}

/// 종료 근거 → 이유 한 줄 (`instance.exit`·`instance.restart_failed`의 `evidenceCode`·`evidenceArgs`)
fn exit_reason(d: &Value) -> Option<Text> {
    let code = d.get("evidenceCode").and_then(Value::as_str).filter(|c| EXIT_REASONS.contains(c))?;
    Some(Text::new(&format!("reason_{code}"), d.get("evidenceArgs").cloned().unwrap_or(Value::Null)))
}

/// 복구 시도 하나의 카드: 크래시 카드와 그 시도의 재시작 실패 카드가 같은 값 (`restartAttempt`)
fn restart_dedup(d: &Value) -> Option<String> {
    let attempt = d.get("restartAttempt").and_then(Value::as_u64)?;
    Some(format!("restart:{}:{attempt}", str_of(d, "instanceId")?))
}

/// 데몬 이벤트 → 알림함에 할 일 (없으면 None). 앱에서 한 일(`source: gui`)은 그 자리에서 이미 봤으니 읽은 것으로 넣는다
pub fn from_event(kind: &str, d: &Value) -> Option<Change> {
    let from_app = d.get("source").and_then(Value::as_str) == Some("gui");
    let draft = match kind {
        "instance.exit" => {
            let exit = d.get("kind").and_then(Value::as_str)?;
            if !EXIT_KINDS.contains(&exit) {
                return None;
            }
            let mut n = Draft::new(Level::Error, &format!("exit_{exit}"), json!({})).server(d);
            n.detail.extend(exit_reason(d));
            if let Some(secs) = d.get("restartInSecs").filter(|v| !v.is_null()) {
                n.detail.push(Text::new("restart_in", json!({ "secs": secs })));
                // 다시 켜지 못하면(`instance.restart_failed`) 이 카드를 실패 카드로 바꾼다
                n.dedup = restart_dedup(d);
            } else {
                match d.get("noteCode").and_then(Value::as_str) {
                    Some("auto_restart_off") => n.detail.push(Text::new("note_auto_off", json!({}))),
                    Some("gave_up") => n.detail.push(Text::new("note_gave_up", d.get("noteArgs").cloned().unwrap_or(Value::Null))),
                    _ => {}
                }
            }
            n
        }
        // 크래시 뒤 다시 켜지 못했다: 그 크래시 카드("N초 뒤 다시 켬")를 같은 자리 표시(`dedup`)로 바꾼다
        "instance.restart_failed" => {
            let exit = d.get("kind").and_then(Value::as_str).filter(|k| EXIT_KINDS.contains(k)).unwrap_or("crash");
            let mut n = Draft::new(Level::Error, "not_restarted", json!({ "exit": { "key": format!("exit_{exit}") } })).server(d);
            n.detail.extend(exit_reason(d));
            n.detail.push(Text::new("error", json!({ "message": d.pointer("/noteArgs/reason") })));
            n.dedup = restart_dedup(d);
            n
        }
        // 예약 백업만: 직접 한 백업은 그 자리에서 결과를 봤고, 업데이트·복원 전 백업의 실패는 그 작업의 실패로 알린다
        "backup.done" if d.get("ok").and_then(Value::as_bool) == Some(false) && d.get("kind").and_then(Value::as_str) == Some("scheduled") => {
            let mut n = Draft::new(Level::Error, "scheduled_backup_failed", json!({ "name": server_name(d) })).server(d);
            n.detail.push(Text::new("error", json!({ "code": d.get("code"), "message": d.get("error") })));
            n.action = Some(OPEN_BACKUPS_ACTION.into());
            n
        }
        // 실패가 따로 카드를 남기는 작업(백업 → 예약 백업 실패, 업데이트 → 게임 서버 업데이트 실패)은 넣지 않는다
        "schedule.failed" => {
            let action = d.get("action").and_then(Value::as_str)?;
            let code = d.get("code").and_then(Value::as_str);
            if action == "backup" || code == Some("update_failed") {
                return None;
            }
            let mut n = Draft::new(Level::Error, "scheduled_task_failed", json!({ "name": server_name(d) })).server(d);
            n.detail.push(Text::new(&format!("task_{action}"), json!({})));
            if code.is_some() {
                n.detail.push(Text::new("error", json!({ "code": code })));
            }
            n.action = Some(OPEN_SCHEDULE_ACTION.into());
            n
        }
        // 사바쨩이 띄우지 않은 프로세스로 켜졌다. 사바쨩이 켠 것은 넣지 않는다
        "instance.started" if d.get("unexpected").and_then(Value::as_bool) == Some(true) => {
            Draft::new(Level::Info, "started_external", json!({})).server(d)
        }
        "instance.unresponsive" => Draft::new(Level::Error, "unresponsive", json!({ "minutes": d.get("minutes") })).server(d),
        "instance.port_conflict_stop" => {
            Draft::new(Level::Error, "port_conflict", json!({ "port": d.get("port"), "existing": d.get("existing") })).server(d)
        }
        "action.run" if d.get("action").and_then(Value::as_str) == Some("restore") => {
            str_of(d, "instanceId").or_else(|| str_of(d, "instance"))?;
            let mut n = if d.get("ok").and_then(Value::as_bool) == Some(true) {
                let source = str_of(d, "source").unwrap_or_default();
                let mut args = json!({ "source": { "key": format!("source_{source}") } });
                if let Some(user) = str_of(d, "userName") {
                    args["user"] = json!(user);
                }
                Draft::new(Level::Success, "restored", args)
            } else {
                let mut n = Draft::new(Level::Error, "restore_failed", json!({}));
                if let Some(code) = str_of(d, "code") {
                    n.detail.push(Text::new("error", json!({ "code": code })));
                }
                n
            };
            n.read = from_app;
            n.server(d)
        }
        "instance.update_notice" => match d.get("kind").and_then(Value::as_str)? {
            "module" => {
                let mut n = Draft::new(
                    Level::Info,
                    "module_update",
                    json!({ "name": server_name(d), "module": d.get("module"), "version": d.get("latest") }),
                );
                n.action = Some(UPDATE_MODAL_ACTION.into());
                n.dedup = Some("module_update".into());
                n
            }
            "server" => server_update(d, "server_update", json!({})),
            _ => return None,
        },
        "instance.update_waiting" => match d.get("players").filter(|p| p.is_u64()) {
            Some(count) => server_update(d, "server_update_waiting", json!({ "count": count })),
            None => server_update(d, "server_update_waiting_unknown", json!({})),
        },
        "instance.update_failed" => {
            Draft::new(Level::Error, "server_update_failed", json!({ "name": server_name(d), "error": str_of(d, "error").unwrap_or_default() })).server(d)
        }
        // 업데이트를 마쳤다: 누를 업데이트 알림이 더는 없다
        "instance.updated" => return str_of(d, "instanceId").map(|id| Change::Drop(server_update_dedup(&id))),
        _ => return None,
    };
    Some(Change::Add(draft))
}

/// 누르면 그 서버를 업데이트하는 알림. 서버마다 하나
fn server_update(d: &Value, key: &str, mut args: Value) -> Draft {
    let id = str_of(d, "instanceId").unwrap_or_default();
    let name = str_of(d, "instance").unwrap_or_else(|| id.clone());
    args["name"] = json!(name);
    let mut n = Draft::new(Level::Info, key, args).server(d);
    n.action = Some(SERVER_UPDATE_ACTION.into());
    n.data = json!({ "instanceId": id, "name": name });
    n.dedup = Some(server_update_dedup(&id));
    n
}

/// 업데이트 완료 마커 → 알림. 사용자 되돌리기는 데몬이 확인한 것으로 남겨도 한 번 알린다. 확인한 결과·복구는 넣지 않는다
pub fn from_update_result(marker: &saba_chan_updater_lib::UpdateCompletionMarker) -> Option<Draft> {
    let mut n = if marker.result.as_deref() == Some("reverted") {
        let args = json!({ "from": marker.replaced_by, "version": marker.version });
        Draft::new(if marker.success { Level::Success } else { Level::Error }, if marker.success { "reverted" } else { "revert_incomplete" }, args)
    } else if marker.acknowledged {
        return None;
    } else {
        let mut n = if marker.success {
            let version = marker.release.as_ref().map(|r| json!(r.version)).unwrap_or_else(|| json!({ "key": "unknown_version" }));
            Draft::new(Level::Success, "update_done", json!({ "version": version }))
        } else {
            Draft::new(Level::Error, "update_failed", json!({ "count": marker.failed.len().max(1) }))
        };
        n.action = Some(PATCH_NOTES_ACTION.into());
        n
    };
    n.dedup = Some("update_result".into());
    Some(n)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default)]
    next: u64,
    /// 최신이 먼저
    #[serde(default)]
    notices: Vec<Notice>,
    /// 알린 마지막 업데이트 결과의 시각 (같은 결과를 데몬이 켜질 때마다 다시 넣지 않게)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    noted_update: Option<String>,
}

pub struct Store {
    path: Option<PathBuf>,
    saved: Mutex<Saved>,
}

impl Store {
    /// 파일에서 읽는다. 없거나 깨졌으면 빈 알림함
    pub fn open(path: PathBuf) -> Self {
        let saved = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| match serde_json::from_str(&text) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("[Notices] {} is unreadable, starting empty: {}", path.display(), e);
                    None
                }
            })
            .unwrap_or_default();
        Self { path: Some(path), saved: Mutex::new(saved) }
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self { path: None, saved: Mutex::new(Saved::default()) }
    }

    /// 바꾸고 저장한 뒤 바뀌었으면 알린다
    fn change<R>(&self, f: impl FnOnce(&mut Saved) -> (R, bool)) -> R {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let (out, changed) = f(&mut saved);
        if changed {
            if let Some(path) = &self.path {
                let text = serde_json::to_vec_pretty(&*saved).unwrap_or_default();
                if let Err(e) = saba_atomic_file::write(path, &text, saba_atomic_file::Mode::Plain) {
                    tracing::warn!("[Notices] Could not save {}: {}", path.display(), e);
                }
            }
            let unread = saved.notices.iter().filter(|n| !n.read).count();
            crate::events::emit_live(CHANGED, json!({ "unread": unread }));
        }
        out
    }

    pub fn add(&self, draft: Draft) -> u64 {
        self.change(|s| {
            if let Some(key) = &draft.dedup {
                s.notices.retain(|n| n.dedup.as_ref() != Some(key));
            }
            s.next = s.next.max(s.notices.iter().map(|n| n.id).max().unwrap_or(0)) + 1;
            let id = s.next;
            s.notices.insert(
                0,
                Notice {
                    id,
                    at: saba_runner::now(),
                    level: draft.level,
                    text: draft.text,
                    detail: draft.detail,
                    instance_id: draft.instance_id,
                    instance: draft.instance,
                    action: draft.action,
                    data: draft.data,
                    read: draft.read,
                    dedup: draft.dedup,
                },
            );
            s.notices.truncate(KEPT);
            (id, true)
        })
    }

    pub fn apply(&self, change: Change) {
        match change {
            Change::Add(draft) => {
                self.add(draft);
            }
            Change::Drop(key) => {
                self.change(|s| {
                    let before = s.notices.len();
                    s.notices.retain(|n| n.dedup.as_deref() != Some(key.as_str()));
                    ((), s.notices.len() != before)
                });
            }
        }
    }

    /// 업데이트 결과를 한 번만 넣는다. 성공한 업데이트 뒤에는 모듈 새 버전 알림이 지난 일이다
    pub fn note_update_result(&self, marker: &saba_chan_updater_lib::UpdateCompletionMarker) {
        let Some(draft) = from_update_result(marker) else { return };
        let fresh = self.change(|s| {
            let fresh = s.noted_update.as_deref() != Some(marker.timestamp.as_str());
            if fresh {
                s.noted_update = Some(marker.timestamp.clone());
                if marker.success && marker.result.is_none() {
                    s.notices.retain(|n| n.dedup.as_deref() != Some("module_update"));
                }
            }
            (fresh, fresh)
        });
        if fresh {
            self.add(draft);
        }
    }

    /// 최신이 먼저
    pub fn list(&self) -> Vec<Notice> {
        self.saved.lock().unwrap_or_else(|e| e.into_inner()).notices.clone()
    }

    pub fn unread(&self) -> usize {
        self.saved.lock().unwrap_or_else(|e| e.into_inner()).notices.iter().filter(|n| !n.read).count()
    }

    pub fn mark_all_read(&self) {
        self.change(|s| {
            let changed = s.notices.iter().any(|n| !n.read);
            s.notices.iter_mut().for_each(|n| n.read = true);
            ((), changed)
        })
    }

    /// 한 건을 지운다. 지운 것을 돌려준다 (없으면 None)
    pub fn remove(&self, id: u64) -> Option<Notice> {
        self.change(|s| {
            let at = s.notices.iter().position(|n| n.id == id);
            let removed = at.map(|i| s.notices.remove(i));
            let changed = removed.is_some();
            (removed, changed)
        })
    }

    /// 모두 지운다. 지운 것을 돌려준다
    pub fn clear(&self) -> Vec<Notice> {
        self.change(|s| {
            let removed = std::mem::take(&mut s.notices);
            let changed = !removed.is_empty();
            (removed, changed)
        })
    }
}

static STORE: OnceLock<Store> = OnceLock::new();

/// 데몬이 켜질 때 한 번: 데이터 폴더의 notices.json을 연다
pub fn init(data_dir: &std::path::Path) {
    let _ = STORE.set(Store::open(data_dir.join("notices.json")));
}

/// 데몬의 알림함. `init` 전(테스트 등)에는 없다
pub fn store() -> Option<&'static Store> {
    STORE.get()
}

/// 데몬 버스에 낸 이벤트를 알림함에도 (`events::emit`이 부른다)
pub fn on_event(kind: &str, data: &Value) {
    if let (Some(store), Some(change)) = (store(), from_event(kind, data)) {
        store.apply(change);
    }
}

/// 지운 알림 중 업데이트 결과가 있으면 결과를 확인한 것으로 남긴다 (다음 실행에 다시 뜨지 않게)
pub fn acknowledge_removed(removed: &[Notice]) {
    if removed.iter().any(|n| n.action.as_deref() == Some(PATCH_NOTES_ACTION)) {
        let path = saba_chan_updater_lib::UpdateCompletionMarker::path();
        if let Err(e) = saba_chan_updater_lib::UpdateCompletionMarker::acknowledge_at(&path) {
            tracing::warn!("[Notices] Could not acknowledge the update result: {}", e);
        }
    }
}

/// 값 하나 → 문장에 넣을 글자. `{key}`면 그 문구
fn arg_text(cat: &crate::chat::i18n::Catalog, v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Object(o) => o.get("key").and_then(Value::as_str).map(|k| cat.t(&format!("notices.{k}"), &[])).unwrap_or_default(),
        other => other.to_string(),
    }
}

fn render_text(cat: &crate::chat::i18n::Catalog, text: &Text) -> String {
    let args: Vec<(String, String)> = text.args.iter().map(|(k, v)| (k.clone(), arg_text(cat, v))).collect();
    let vars: Vec<(&str, &str)> = args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let line = cat.t(&format!("notices.{}", text.key), &vars);
    // 복원한 사람: "디스코드 · 홍길동"
    match text.args.get("user").and_then(Value::as_str) {
        Some(user) if text.key == "restored" => format!("{line} · {user}"),
        _ => line.trim().to_string(),
    }
}

/// 한 건 → 화면에 보일 모양. `source`는 묶음 이름(서버 이름, 아니면 "사바쨩")
pub fn render(cat: &crate::chat::i18n::Catalog, n: &Notice) -> Value {
    let detail: Vec<String> = n
        .detail
        .iter()
        .map(|t| {
            if t.key == "error" {
                // 실패 코드 → 데몬 오류 문구. 모르는 코드면 데몬이 남긴 문장(`message`), 그것도 없으면 코드 그대로
                let code = t.args.get("code").and_then(Value::as_str).unwrap_or_default();
                let message = t.args.get("message").and_then(Value::as_str).filter(|m| !m.is_empty());
                let key = format!("errors.{code}");
                match cat.t(&key, &[]) {
                    text if !code.is_empty() && text != key => text,
                    _ => message.unwrap_or(code).to_string(),
                }
            } else {
                render_text(cat, t)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = json!({
        "id": n.id,
        "at": n.at,
        "level": n.level,
        "message": render_text(cat, &n.text),
        "source": n.instance.clone().unwrap_or_else(|| cat.t("notices.main", &[])),
        "read": n.read,
    });
    if !detail.is_empty() {
        out["detail"] = json!(detail.join(" · "));
    }
    if let Some(id) = &n.instance_id {
        out["instanceId"] = json!(id);
    }
    if let Some(action) = &n.action {
        out["action"] = json!(action);
    }
    if !n.data.is_null() {
        out["data"] = n.data.clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(kind: &str, d: Value) -> Option<Draft> {
        match from_event(kind, &d) {
            Some(Change::Add(draft)) => Some(draft),
            _ => None,
        }
    }

    /// 사용자 규칙 (2026-11-15, 여러 번): 정상 시작·정지는 어디서 시켰든 알림함에 넣지 않는다.
    /// 크래시·시작 실패·원인 모를 종료·사바쨩 밖의 시작만 넣는다
    #[test]
    fn plain_start_and_stop_never_go_into_the_inbox() {
        for source in ["gui", "cli", "web", "discord", "chatbot", "schedule", "restart", "idle", "mcp", "remote", "daemon"] {
            assert_eq!(added("instance.started", json!({ "instanceId": "i1", "instance": "mc", "source": source })), None, "started from {source}");
            assert_eq!(added("instance.stopped", json!({ "instanceId": "i1", "instance": "mc", "source": source })), None, "stopped from {source}");
            assert_eq!(added("instance.stopped", json!({ "instanceId": "i1", "kind": "in_game_stop", "source": source })), None);
            for kind in ["user_stop", "in_game_stop"] {
                assert_eq!(added("instance.exit", json!({ "instanceId": "i1", "kind": kind, "source": source })), None, "{kind} exit");
            }
            let run = |action: &str| added("action.run", json!({ "instanceId": "i1", "action": action, "ok": true, "source": source }));
            for action in ["start", "stop", "restart"] {
                assert_eq!(run(action), None, "{action} from {source}");
            }
        }
        for kind in EXIT_KINDS {
            assert!(added("instance.exit", json!({ "instanceId": "i1", "kind": kind })).is_some(), "{kind}");
        }
        assert!(added("instance.started", json!({ "instanceId": "i1", "unexpected": true })).is_some());
    }

    /// 백업·예약 작업은 예약해 둔 것이 실패했을 때만 넣는다. 직접 한 백업의 실패는 그 자리에서 이미 봤다.
    /// 예약 백업이 실패하면 백업 카드 하나만, 업데이트 실패는 업데이트 카드 하나만 남긴다
    #[test]
    fn only_scheduled_failures_of_backups_and_tasks_go_into_the_inbox() {
        for kind in ["manual", "snapshot", "pre-update", "pre-restore"] {
            assert_eq!(added("backup.done", json!({ "instanceId": "i1", "kind": kind, "ok": false, "error": "x" })), None, "{kind} backup");
        }
        assert_eq!(added("backup.done", json!({ "instanceId": "i1", "kind": "scheduled", "ok": true, "name": "a.zip" })), None);
        assert!(added("backup.done", json!({ "instanceId": "i1", "kind": "scheduled", "ok": false, "error": "x" })).is_some());
        for action in ["backup", "restart", "start", "stop", "save", "announce", "command", "update"] {
            assert_eq!(added("schedule.ran", json!({ "instanceId": "i1", "action": action, "seconds": 1 })), None, "{action} ran");
        }
        assert_eq!(added("schedule.failed", json!({ "instanceId": "i1", "action": "backup", "code": "backup_failed" })), None);
        assert_eq!(added("schedule.failed", json!({ "instanceId": "i1", "action": "update", "code": "update_failed" })), None);
        assert!(added("schedule.failed", json!({ "instanceId": "i1", "action": "update", "code": "check_failed" })).is_some());
    }

    /// 크래시 뒤 다시 켜지 못하면 그 크래시 카드를 고친다: 새 카드를 더하지 않고, 다시 읽지 않은 것으로 돌린다
    #[test]
    fn a_failed_restart_rewrites_its_crash_card() {
        let store = Store::in_memory();
        let ko = crate::chat::i18n::Catalog::for_language("ko");
        let exit = json!({ "instanceId": "i1", "instance": "mc", "kind": "crash", "evidenceCode": "exit_code", "evidenceArgs": { "code": 1 }, "restartInSecs": 10, "restartAttempt": 3 });
        store.apply(from_event("instance.exit", &exit).unwrap());
        store.apply(from_event("instance.unresponsive", &json!({ "instanceId": "i2", "minutes": 3 })).unwrap());
        store.mark_all_read();
        let failed = json!({ "instanceId": "i1", "instance": "mc", "kind": "crash", "evidenceCode": "exit_code", "evidenceArgs": { "code": 1 },
            "noteCode": "restart_failed", "noteArgs": { "reason": "Java를 찾을 수 없음" }, "restartAttempt": 3 });
        store.apply(from_event("instance.restart_failed", &failed).unwrap());
        let list = store.list();
        assert_eq!(list.len(), 2, "the crash card is rewritten, not joined by another");
        let card = render(&ko, list.iter().find(|n| n.instance_id.as_deref() == Some("i1")).unwrap());
        assert_eq!(card["message"], "크래시로 종료됨 · 다시 켜지 못함");
        assert_eq!(card["detail"], "종료 코드 1 · Java를 찾을 수 없음");
        assert_eq!(card["read"], false);
        assert_eq!(store.unread(), 1);
        // 카드를 지운 뒤에 실패해도 실패는 남는다. 다른 시도의 실패는 그 시도의 카드만 고친다
        store.clear();
        store.apply(from_event("instance.restart_failed", &failed).unwrap());
        store.apply(from_event("instance.exit", &json!({ "instanceId": "i1", "kind": "crash", "restartInSecs": 10, "restartAttempt": 4 })).unwrap());
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(render(&ko, &list[0])["detail"], "10초 뒤 다시 켬");
    }

    /// 예약 백업·예약 작업 실패 카드: 서버 이름과 실패, 이유(모르는 코드면 데몬이 준 문장), 누르면 그 서버의 백업·예약 화면
    #[test]
    fn scheduled_failures_say_what_failed_and_open_its_screen() {
        let store = Store::in_memory();
        let ko = crate::chat::i18n::Catalog::for_language("ko");
        store.apply(from_event("backup.done", &json!({ "instanceId": "i1", "instance": "mc", "kind": "scheduled", "ok": false, "code": "backup_save_rejected", "error": "save rejected" })).unwrap());
        let backup = render(&ko, &store.list()[0]);
        assert_eq!(backup["message"], "mc · 예약 백업 실패");
        assert_eq!(backup["detail"], "서버가 저장 명령을 거절함");
        assert_eq!(backup["action"], OPEN_BACKUPS_ACTION);
        assert_eq!(backup["instanceId"], "i1");
        assert_eq!(backup["level"], "error");
        store.apply(from_event("backup.done", &json!({ "instanceId": "i1", "instance": "mc", "kind": "scheduled", "ok": false, "code": "backup_failed", "error": "No space left on device" })).unwrap());
        assert_eq!(render(&ko, &store.list()[0])["detail"], "No space left on device");
        store.apply(from_event("schedule.failed", &json!({ "instanceId": "i2", "instance": "pal", "action": "save", "seconds": 1, "code": "command_failed" })).unwrap());
        let task = render(&ko, &store.list()[0]);
        assert_eq!(task["message"], "pal · 예약 작업 실패");
        assert_eq!(task["detail"], "저장 · 서버가 명령을 거절함");
        assert_eq!(task["action"], OPEN_SCHEDULE_ACTION);
        let en = render(&crate::chat::i18n::Catalog::for_language("en"), &store.list()[0]);
        assert_eq!(en["message"], "pal · Scheduled task failed");
    }

    #[test]
    fn a_crash_says_why_and_when_it_restarts_in_the_asked_language() {
        let d = json!({ "instanceId": "i1", "instance": "survival", "kind": "crash", "evidenceCode": "exit_code", "evidenceArgs": { "code": 1 }, "restartInSecs": 10 });
        let store = Store::in_memory();
        store.apply(from_event("instance.exit", &d).unwrap());
        let shown = render(&crate::chat::i18n::Catalog::for_language("ko"), &store.list()[0]);
        assert_eq!(shown["message"], "크래시로 종료됨");
        assert_eq!(shown["detail"], "종료 코드 1 · 10초 뒤 다시 켬");
        assert_eq!(shown["source"], "survival");
        assert_eq!(shown["instanceId"], "i1");
        assert_eq!(shown["level"], "error");
        let en = render(&crate::chat::i18n::Catalog::for_language("en"), &store.list()[0]);
        assert_eq!(en["message"], "Exited by crash");
        store.apply(from_event("action.run", &json!({ "instanceId": "i1", "action": "restore", "ok": true, "source": "discord", "userName": "길동" })).unwrap());
        let restored = render(&crate::chat::i18n::Catalog::for_language("ko"), &store.list()[0]);
        assert_eq!(restored["message"], "복원 · 디스코드 · 길동");
        assert_eq!(restored["level"], "success");
    }

    /// 서버 업데이트 알림은 서버마다 하나이고, 업데이트를 마치면 사라진다
    #[test]
    fn a_server_update_notice_is_kept_once_and_goes_away_when_updated() {
        let store = Store::in_memory();
        store.apply(from_event("instance.update_notice", &json!({ "instanceId": "i1", "instance": "mc", "kind": "server" })).unwrap());
        store.apply(from_event("instance.update_waiting", &json!({ "instanceId": "i1", "instance": "mc", "players": 2 })).unwrap());
        store.apply(from_event("instance.update_notice", &json!({ "instanceId": "i2", "instance": "pal", "kind": "server" })).unwrap());
        assert_eq!(store.list().len(), 2);
        let cat = crate::chat::i18n::Catalog::for_language("ko");
        let mc = render(&cat, &store.list()[1]);
        assert!(mc["message"].as_str().unwrap().contains("2명"), "{mc}");
        assert_eq!(mc["action"], SERVER_UPDATE_ACTION);
        assert_eq!(mc["data"], json!({ "instanceId": "i1", "name": "mc" }));
        store.apply(from_event("instance.updated", &json!({ "instanceId": "i1", "instance": "mc" })).unwrap());
        assert_eq!(store.list().iter().map(|n| n.instance_id.clone().unwrap()).collect::<Vec<_>>(), vec!["i2"]);
    }

    #[test]
    fn only_the_latest_kept_notices_stay_and_reading_is_shared() {
        let store = Store::in_memory();
        for _ in 0..KEPT + 5 {
            store.apply(from_event("instance.unresponsive", &json!({ "instanceId": "i1", "minutes": 3 })).unwrap());
        }
        let list = store.list();
        assert_eq!(list.len(), KEPT);
        assert!(list[0].id > list[1].id, "newest first");
        assert_eq!(store.unread(), KEPT);
        store.mark_all_read();
        assert_eq!(store.unread(), 0);
        assert!(store.remove(list[0].id).is_some());
        assert!(store.remove(list[0].id).is_none());
        assert_eq!(store.clear().len(), KEPT - 1);
        assert!(store.list().is_empty());
    }

    /// 데몬을 다시 켜도 알림함이 남는다
    #[test]
    fn the_inbox_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notices.json");
        let first = Store::open(path.clone());
        first.apply(from_event("instance.exit", &json!({ "instanceId": "i1", "kind": "start_failed" })).unwrap());
        let id = first.list()[0].id;
        let again = Store::open(path);
        assert_eq!(again.list()[0].id, id);
        again.apply(from_event("instance.unresponsive", &json!({ "instanceId": "i1", "minutes": 3 })).unwrap());
        assert!(again.list()[0].id > id, "numbers keep counting up after a restart");
    }

    /// 업데이트 결과는 같은 결과를 한 번만, 확인한 결과는 넣지 않는다. 되돌리기는 확인한 것으로 남아도 한 번 알린다
    #[test]
    fn an_update_result_is_noted_once() {
        use saba_chan_updater_lib::UpdateCompletionMarker;
        let store = Store::in_memory();
        store.apply(from_event("instance.update_notice", &json!({ "instanceId": "i1", "kind": "module", "module": "minecraft", "latest": "2.0" })).unwrap());
        let done = UpdateCompletionMarker::applied(vec!["saba-core".into()], Vec::new(), Vec::new(), None);
        store.note_update_result(&done);
        store.note_update_result(&done);
        let list = store.list();
        assert_eq!(list.len(), 1, "the module update notice is past and the result is noted once");
        assert_eq!(list[0].action.as_deref(), Some(PATCH_NOTES_ACTION));
        let mut seen = done.clone();
        seen.acknowledged = true;
        seen.timestamp = "later".into();
        assert_eq!(from_update_result(&seen), None);
        let mut reverted = seen.clone();
        reverted.result = Some("reverted".into());
        reverted.replaced_by = Some("0.1.14".into());
        reverted.version = Some("0.1.13".into());
        let n = from_update_result(&reverted).unwrap();
        assert_eq!(n.text.key, "reverted");
        assert_eq!(n.action, None);
    }
}
