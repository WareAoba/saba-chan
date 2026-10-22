//! 설정·인스턴스 내보내기/가져오기 (A9) — PC를 옮길 때 쓴다
//!
//! - `POST /api/migration/export` `{path, secrets?}` — 데몬이 `path`에 zip을 쓴다
//! - `POST /api/migration/import` `{path, dryRun?, overwrite?}` — 데몬이 `path`의 zip을 읽어 반영한다
//!
//! 파일은 데몬이 읽고 쓴다(인터페이스는 경로만 준다). 게임 서버 파일(월드 등)은 넣지 않는다:
//! 인스턴스의 `working_dir`은 옛 PC 경로 그대로라 서버 폴더는 따로 옮긴다.
//!
//! zip 내용:
//! - `saba-export.json` — `{format, formatVersion, appVersion, createdAt, secrets}`
//! - `config.json` — 옮길 설정 네임스페이스 (`ConfigStore::portable_sections`)
//! - `instances.json` — 인스턴스 목록 (순서대로, `module_settings` 포함)
//! - `node_token` — `secrets`일 때만
#![allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::super::IPCServer;
use crate::instance::ServerInstance;

const FORMAT: &str = "saba-chan-export";
const FORMAT_VERSION: u64 = 1;
/// 읽을 항목 하나의 최대 크기 (설정·인스턴스 JSON만 들어 있다)
const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "success": false, "error": message.into(), "error_code": code }))).into_response()
}

/// 상대 경로는 어디를 가리킬지 모호하다: 절대 경로만 받는다
fn absolute_path(path: &str) -> Result<PathBuf, Response> {
    let p = PathBuf::from(path.trim());
    if path.trim().is_empty() || !p.is_absolute() {
        return Err(error(StatusCode::BAD_REQUEST, "INVALID_PATH", "path must be an absolute file path"));
    }
    Ok(p)
}

/// 비밀값을 뺀 인스턴스: REST 비밀번호와 모듈이 `password`로 선언한 설정.
/// 모듈을 읽지 못해 설정 종류를 모르면(`types`가 None) 모듈 설정을 모두 뺀다
pub(crate) fn strip_instance_secrets(instance: &mut ServerInstance, types: Option<&HashMap<String, String>>) {
    instance.rest_password = None;
    match types {
        Some(types) => instance.module_settings.retain(|k, _| types.get(k).map(String::as_str) != Some("password")),
        None => instance.module_settings.clear(),
    }
}

/// 내보낼 묶음
pub(crate) struct Bundle {
    pub meta: Value,
    pub config: Map<String, Value>,
    pub instances: Vec<ServerInstance>,
    pub node_token: Option<String>,
}

pub(crate) fn write_bundle(path: &Path, bundle: &Bundle) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 다 쓴 뒤에 옮긴다: 쓰다 실패하면 반쪽 zip이 남지 않게
    let tmp = path.with_extension("saba-export.tmp");
    let result = (|| -> anyhow::Result<()> {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&tmp)?);
        let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str, bytes: &[u8]| -> anyhow::Result<()> {
            zip.start_file(name, opts)?;
            zip.write_all(bytes)?;
            Ok(())
        };
        put("saba-export.json", &serde_json::to_vec_pretty(&bundle.meta)?)?;
        put("config.json", &serde_json::to_vec_pretty(&bundle.config)?)?;
        put("instances.json", &serde_json::to_vec_pretty(&bundle.instances)?)?;
        if let Some(token) = &bundle.node_token {
            put("node_token", token.as_bytes())?;
        }
        zip.finish()?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub(crate) fn read_bundle(path: &Path) -> anyhow::Result<Bundle> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut entry = |name: &str| -> anyhow::Result<Option<Vec<u8>>> {
        let Ok(file) = zip.by_name(name) else { return Ok(None) };
        let mut buf = Vec::new();
        file.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buf)?;
        anyhow::ensure!(buf.len() as u64 <= MAX_ENTRY_BYTES, "{} is too large", name);
        Ok(Some(buf))
    };
    let meta: Value = serde_json::from_slice(&entry("saba-export.json")?.ok_or_else(|| anyhow::anyhow!("not a saba-chan export"))?)?;
    anyhow::ensure!(meta.get("format").and_then(Value::as_str) == Some(FORMAT), "not a saba-chan export");
    let version = meta.get("formatVersion").and_then(Value::as_u64).unwrap_or(0);
    anyhow::ensure!(version == FORMAT_VERSION, "unsupported export format version {}", version);
    let config = match entry("config.json")? {
        Some(b) => serde_json::from_slice(&b)?,
        None => Map::new(),
    };
    let instances = match entry("instances.json")? {
        Some(b) => serde_json::from_slice(&b)?,
        None => Vec::new(),
    };
    let node_token = entry("node_token")?
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(Bundle { meta, config, instances, node_token })
}

/// 릴레이 연결 정보(`relay.cloud`의 relayUrl·hostId, 노드 캐시)는 노드 토큰과 한 묶음이다.
/// 토큰을 가져오지 않을 때는 이 PC 것을 둔다: 이 PC 토큰에 옛 PC hostId가 붙거나, 낯선 zip이
/// relayUrl을 바꿔 이 PC 토큰을 다른 곳으로 보내지 않게
pub(crate) fn keep_local_pairing(sections: &mut Map<String, Value>) {
    if let Some(Value::Object(relay)) = sections.get_mut("relay") {
        for key in ["cloud", "cloudNodes", "cloudMembers"] {
            relay.remove(key);
        }
    }
}

/// zip의 노드 토큰을 이 PC에 쓸 것인가: 이 PC가 아직 페어링되지 않았을 때만
fn takes_node_token(bundle: &Bundle) -> bool {
    bundle.node_token.is_some() && !std::fs::read_to_string(super::relay::node_token_path()).is_ok_and(|t| !t.trim().is_empty())
}

/// 가져올 인스턴스마다 넣을지 건너뛸지
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    Add,
    Replace,
    Skip(&'static str),
}

/// `instance_dir`는 id의 인스턴스 폴더 (`InstanceStore::instance_dir`)
pub(crate) fn judge(
    incoming: &ServerInstance,
    existing: &[ServerInstance],
    running: &HashSet<String>,
    overwrite: bool,
    instance_dir: &dyn Fn(&str) -> PathBuf,
) -> Verdict {
    if !crate::utils::is_single_path_component(&incoming.id) {
        return Verdict::Skip("invalid_id");
    }
    let same = existing.iter().find(|e| e.id == incoming.id);
    // 다른 서버의 서버 폴더나 인스턴스 폴더와 같거나 그 안·바깥이면 받지 않는다 (`InstanceStore::folder_user`와 같은 판정)
    if let Some(dir) = incoming.working_dir.as_deref().map(Path::new) {
        let other = existing.iter().find(|o| {
            o.id != incoming.id
                && (o.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::paths_overlap(Path::new(d), dir))
                    || saba_chan_updater_lib::paths_overlap(&instance_dir(&o.id), dir))
        });
        if other.is_some() {
            return Verdict::Skip("working_dir_in_use");
        }
    }
    // 이름으로 서버를 찾는 클라이언트가 있어 같은 이름은 둘 두지 않는다
    if existing.iter().any(|e| e.id != incoming.id && e.name == incoming.name) {
        return Verdict::Skip("name_taken");
    }
    match same {
        None => Verdict::Add,
        Some(_) if !overwrite => Verdict::Skip("exists"),
        Some(_) if running.contains(&incoming.id) => Verdict::Skip("running"),
        Some(_) => Verdict::Replace,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub path: String,
    #[serde(default)]
    pub secrets: bool,
}

/// POST /api/migration/export — `{path, secrets, instances, config: [네임스페이스]}`
pub async fn export(State(state): State<IPCServer>, Json(req): Json<ExportRequest>) -> Response {
    let path = match absolute_path(&req.path) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let instances: Vec<ServerInstance> = {
        let sup = state.supervisor.read().await;
        sup.instance_store
            .list()
            .iter()
            .cloned()
            .map(|mut inst| {
                if !req.secrets {
                    let types = sup.module_loader.get_module(&inst.module_name).ok().map(|m| m.metadata.setting_types());
                    strip_instance_secrets(&mut inst, types.as_ref());
                }
                inst
            })
            .collect()
    };
    let config = state.config_store.portable_sections(req.secrets);
    let node_token = if req.secrets {
        std::fs::read_to_string(super::relay::node_token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    } else {
        None
    };
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bundle = Bundle {
        meta: json!({
            "format": FORMAT,
            "formatVersion": FORMAT_VERSION,
            "appVersion": env!("CARGO_PKG_VERSION"),
            "createdAt": created_at,
            "secrets": req.secrets,
        }),
        config,
        instances,
        node_token,
    };
    if let Err(e) = write_bundle(&path, &bundle) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "WRITE_FAILED", format!("Could not write {}: {}", path.display(), e));
    }
    Json(json!({
        "success": true,
        "path": path.to_string_lossy(),
        "secrets": req.secrets,
        "instances": bundle.instances.iter().map(|i| json!({ "id": i.id, "name": i.name })).collect::<Vec<_>>(),
        "config": bundle.config.keys().collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub path: String,
    #[serde(default)]
    pub dry_run: bool,
    /// 같은 id의 인스턴스를 덮어쓴다 (켜져 있는 것은 건너뛴다)
    #[serde(default)]
    pub overwrite: bool,
}

/// POST /api/migration/import —
/// `{success, dryRun, appVersion, secrets, instances: {added, replaced, skipped: [{id, name, reason}]},
///   missingModules, config: [네임스페이스], nodeToken: "imported"|"kept"|null}`
pub async fn import(State(state): State<IPCServer>, Json(req): Json<ImportRequest>) -> Response {
    let path = match absolute_path(&req.path) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let bundle = match read_bundle(&path) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, "INVALID_ARCHIVE", e.to_string()),
    };
    // 설정이 잘못된 zip이면 인스턴스·노드 토큰을 바꾸기 전에 멈춘다.
    // 토큰을 가져오지 않으면 릴레이 연결 정보는 아래에서 버려지므로 검사하지 않는다
    let takes = takes_node_token(&bundle);
    let mut checked = bundle.config.clone();
    if !takes {
        keep_local_pairing(&mut checked);
    }
    if let Err(e) = state.config_store.check_sections(&checked).await {
        return error(StatusCode::BAD_REQUEST, "INVALID_CONFIG", e.to_string());
    }

    let (mut added, mut replaced, mut skipped, mut missing_modules) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    {
        let mut sup = state.supervisor.write().await;
        // 쓰기 잠금 안에서 본다: 그 사이에 켜진 서버를 덮어쓰지 않게
        let running: HashSet<String> = {
            let mut ids: HashSet<String> = sup.managed_store.running_instance_ids().await.into_iter().collect();
            ids.extend(sup.instance_store.list().iter().filter(|i| sup.tracker.get_pid(&i.id).is_ok()).map(|i| i.id.clone()));
            ids
        };
        // 앞서 더하거나 바꾼 인스턴스까지 보고 판정한다: 미리보기(dry run)도 실제 가져오기와 같은 결과를 내게
        let mut known: Vec<ServerInstance> = sup.instance_store.list().to_vec();
        for inst in bundle.instances {
            // 남이 준 zip이면 이 경로의 프로그램이 실행된다: 미리보기에서 보이게
            let entry = json!({
                "id": inst.id,
                "name": inst.name,
                "working_dir": inst.working_dir,
                "executable_path": inst.executable_path,
            });
            let mut verdict = judge(&inst, &known, &running, req.overwrite, &|id| sup.instance_store.instance_dir(id));
            // 설치·백업·복원이 쥔 인스턴스는 바꾸지 않는다. 잠금은 저장하는 동안만 쥔다
            let guard = match verdict {
                Verdict::Skip(_) => None,
                _ => super::backup::BusyGuard::take(&inst.id),
            };
            if guard.is_none() && !matches!(verdict, Verdict::Skip(_)) {
                verdict = Verdict::Skip("busy");
            }
            let applied = match verdict {
                Verdict::Skip(reason) => {
                    skipped.push(json!({ "id": inst.id, "name": inst.name, "reason": reason }));
                    continue;
                }
                _ if req.dry_run => Ok(()),
                Verdict::Add => sup.instance_store.add(inst.clone()),
                Verdict::Replace => sup.instance_store.update(&inst.id.clone(), inst.clone()),
            };
            if let Err(e) = applied {
                tracing::warn!("Import: could not save instance '{}': {}", inst.name, e);
                skipped.push(json!({ "id": inst.id, "name": inst.name, "reason": "save_failed" }));
                continue;
            }
            known.retain(|k| k.id != inst.id);
            known.push(inst.clone());
            if sup.module_loader.get_module(&inst.module_name).is_err() && !missing_modules.contains(&inst.module_name) {
                missing_modules.push(inst.module_name.clone());
            }
            if verdict == Verdict::Add { added.push(entry) } else { replaced.push(entry) }
        }
    }

    // 이 PC가 이미 페어링돼 있으면 그 토큰을 둔다
    let mut adopted = None;
    let node_token = match bundle.node_token {
        None => Value::Null,
        Some(_) if !takes => json!("kept"),
        Some(_) if req.dry_run => json!("imported"),
        Some(token) => match super::relay::write_node_token(&super::relay::node_token_path(), &token) {
            Ok(changed) => {
                adopted = Some((token, changed));
                json!("imported")
            }
            Err(e) => {
                tracing::warn!("Import: could not write node token: {}", e);
                Value::Null
            }
        },
    };

    let mut sections = bundle.config;
    if node_token != json!("imported") {
        keep_local_pairing(&mut sections);
    }
    let config: Vec<String> = if req.dry_run {
        let mut keys: Vec<String> = sections.keys().cloned().collect();
        keys.retain(|k| matches!(k.as_str(), "gui" | "relay" | "chatbot") || crate::config_store::ConfigStore::is_client_section(k));
        keys
    } else {
        // 챗봇 절은 챗봇 저장과 같은 차례를 지킨다
        let _turn = if sections.contains_key("chatbot") { Some(super::chatbot::SAVE_LOCK.lock().await) } else { None };
        match state.config_store.import_sections(sections).await {
            Ok(keys) => {
                // 일반 설정 저장과 같이 실행 중인 포트 검사도 맞춘다
                if keys.iter().any(|k| k == "gui") {
                    let check = state.config_store.get_gui_settings().await.port_conflict_check;
                    state.supervisor.read().await.skip_port_check.store(!check, std::sync::atomic::Ordering::Relaxed);
                }
                keys
            }
            Err(e) => {
                // 토큰은 이미 저장했다: 다음 기동까지 미루지 않고 지금 그 토큰으로 잇는다
                if let Some((token, _)) = &adopted {
                    super::relay::adopt_node_token(&state, token, true).await;
                }
                return error(StatusCode::BAD_REQUEST, "INVALID_CONFIG", e.to_string());
            }
        }
    };
    // 토큰 저장 API와 같이: 가져온 토큰(과 릴레이 주소)으로 데몬 재시작 없이 연결한다. 토큰이 같아도 주소가 바뀌었을 수 있다
    if let Some((token, _)) = adopted {
        super::relay::adopt_node_token(&state, &token, true).await;
    }

    Json(json!({
        "success": true,
        "dryRun": req.dry_run,
        "appVersion": bundle.meta.get("appVersion").cloned().unwrap_or(Value::Null),
        "secrets": bundle.meta.get("secrets").and_then(Value::as_bool).unwrap_or(false),
        "instances": { "added": added, "replaced": replaced, "skipped": skipped },
        "missingModules": missing_modules,
        "config": config,
        "nodeToken": node_token,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(id: &str, dir: Option<&str>) -> ServerInstance {
        let mut i = ServerInstance::new(id, "minecraft");
        i.id = id.to_string();
        i.working_dir = dir.map(str::to_string);
        i
    }

    #[test]
    fn export_without_secrets_drops_passwords_but_keeps_other_settings() {
        let mut i = inst("a", None);
        i.rest_password = Some("pw".into());
        i.module_settings.insert("AdminPassword".into(), json!("secret"));
        i.module_settings.insert("port".into(), json!(25565));
        let types = HashMap::from([("AdminPassword".to_string(), "password".to_string()), ("port".to_string(), "number".to_string())]);
        strip_instance_secrets(&mut i, Some(&types));
        assert_eq!(i.rest_password, None);
        assert!(!i.module_settings.contains_key("AdminPassword"));
        assert_eq!(i.module_settings.get("port"), Some(&json!(25565)));
    }

    #[test]
    fn export_without_secrets_drops_module_settings_when_the_module_is_missing() {
        let mut i = inst("a", None);
        i.module_settings.insert("AdminPassword".into(), json!("secret"));
        // 모듈을 읽지 못하면 어느 설정이 비밀번호인지 모른다
        strip_instance_secrets(&mut i, None);
        assert!(i.module_settings.is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_reports_what_the_real_import_would_do() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saba.zip");
        let (mut first, mut second) = (inst("a", None), inst("b", None));
        first.name = "same".into();
        second.name = "same".into();
        let meta = json!({ "format": FORMAT, "formatVersion": FORMAT_VERSION, "appVersion": "0.1.5", "secrets": false });
        write_bundle(&path, &Bundle { meta, config: Map::new(), instances: vec![first, second], node_token: None }).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            dir.path().join("modules").to_str().unwrap(),
            dir.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let run = |dry: bool| {
            let req: ImportRequest = serde_json::from_value(json!({ "path": path.to_string_lossy(), "dryRun": dry })).unwrap();
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(import(State(state), Json(req)).await).await.body }
        };
        let preview = run(true).await;
        let done = run(false).await;
        assert_eq!(preview["instances"]["added"].as_array().unwrap().len(), 1, "{preview}");
        assert_eq!(preview["instances"]["skipped"][0]["reason"], "name_taken");
        assert_eq!(preview["instances"], done["instances"]);
    }

    /// 가져오기 시험용 데몬 상태와 zip 경로. `existing`은 미리 있는 인스턴스
    async fn import_state(dir: &std::path::Path, bundle: Bundle, existing: Option<ServerInstance>) -> (IPCServer, String) {
        let path = dir.join("saba.zip");
        write_bundle(&path, &bundle).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            dir.join("modules").to_str().unwrap(),
            dir.join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        if let Some(i) = existing {
            sup.instance_store.add(i).unwrap();
        }
        let state = IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        (state, path.to_string_lossy().into_owned())
    }

    fn meta() -> Value {
        json!({ "format": FORMAT, "formatVersion": FORMAT_VERSION, "appVersion": "0.1.5", "secrets": false })
    }

    /// 설정 저장에서 거부될 zip(잘못된 접두사)은 인스턴스를 덮어쓰기 전에 멈춘다
    #[tokio::test]
    async fn a_config_the_save_would_refuse_leaves_the_instances_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (mut old, mut new) = (inst("a", None), inst("a", None));
        old.name = "old".into();
        new.name = "new".into();
        let mut config = Map::new();
        config.insert("relay".into(), json!({ "prefixes": ["has space"] }));
        let bundle = Bundle { meta: meta(), config, instances: vec![new], node_token: None };
        let (state, path) = import_state(dir.path(), bundle, Some(old)).await;
        let req: ImportRequest = serde_json::from_value(json!({ "path": path, "overwrite": true })).unwrap();
        let res = crate::ipc::internal::into_internal(import(State(state.clone()), Json(req)).await).await;
        assert_eq!(res.body["error_code"], "INVALID_CONFIG", "{}", res.body);
        let sup = state.supervisor.read().await;
        assert_eq!(sup.instance_store.get("a").unwrap().name, "old");
    }

    /// 토큰 없이 가져오면 zip의 릴레이 연결 정보는 버려진다: 그 안의 옛 http 주소로 가져오기 전체를 막지 않는다
    #[tokio::test]
    async fn a_relay_address_the_import_drops_is_not_checked() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Map::new();
        config.insert("relay".into(), json!({ "prefixes": ["!s"], "cloud": { "relayUrl": "http://old.example", "hostId": "old" } }));
        let bundle = Bundle { meta: meta(), config, instances: vec![], node_token: None };
        let (state, path) = import_state(dir.path(), bundle, None).await;
        let req: ImportRequest = serde_json::from_value(json!({ "path": path })).unwrap();
        let res = crate::ipc::internal::into_internal(import(State(state.clone()), Json(req)).await).await;
        assert_eq!(res.body["success"], true, "{}", res.body);
        let bot = state.config_store.get_bot_config().await;
        assert_eq!(bot.prefixes, vec!["!s"]);
        assert_ne!(bot.cloud.relay_url, "http://old.example");
    }

    /// 설치·백업·복원이 쥔 인스턴스는 덮어쓰지 않는다: 작업이 옛 설정과 폴더로 계속 돈다
    #[tokio::test]
    async fn an_instance_held_by_a_task_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let (mut old, mut new) = (inst("held-by-import-test", None), inst("held-by-import-test", None));
        old.name = "old".into();
        new.name = "new".into();
        let bundle = Bundle { meta: meta(), config: Map::new(), instances: vec![new], node_token: None };
        let (state, path) = import_state(dir.path(), bundle, Some(old)).await;
        let _task = super::super::backup::BusyGuard::take("held-by-import-test").unwrap();
        for dry in [true, false] {
            let req: ImportRequest = serde_json::from_value(json!({ "path": path, "overwrite": true, "dryRun": dry })).unwrap();
            let res = crate::ipc::internal::into_internal(import(State(state.clone()), Json(req)).await).await;
            assert_eq!(res.body["instances"]["skipped"][0]["reason"], "busy", "{}", res.body);
        }
        assert_eq!(state.supervisor.read().await.instance_store.get("held-by-import-test").unwrap().name, "old");
    }

    /// 가져오기의 챗봇 절도 챗봇 저장과 한 번에 하나씩: 겹치면 한쪽이 옛 값으로 다른 쪽 저장을 지운다
    #[tokio::test]
    async fn an_imported_chatbot_section_waits_for_a_chatbot_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Map::new();
        // 빈 절은 지금 값을 그대로 다시 쓴다 (같은 프로세스의 다른 시험과 설정 파일을 같이 쓴다)
        config.insert("chatbot".into(), json!({}));
        let bundle = Bundle { meta: meta(), config, instances: vec![], node_token: None };
        let (state, path) = import_state(dir.path(), bundle, None).await;
        let turn = super::super::chatbot::SAVE_LOCK.lock().await;
        let req: ImportRequest = serde_json::from_value(json!({ "path": path })).unwrap();
        let run = tokio::spawn(import(State(state), Json(req)));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!run.is_finished(), "the import wrote the chatbot settings while a chatbot save was in progress");
        drop(turn);
        let res = tokio::time::timeout(std::time::Duration::from_secs(5), run).await.unwrap().unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// 가져온 포트 충돌 검사 설정은 일반 설정 저장처럼 바로 실행 중인 검사에 반영된다
    #[tokio::test]
    async fn imported_port_check_setting_applies_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Map::new();
        config.insert("gui".into(), json!({ "portConflictCheck": false }));
        let bundle = Bundle { meta: meta(), config, instances: vec![], node_token: None };
        let (state, path) = import_state(dir.path(), bundle, None).await;
        let req: ImportRequest = serde_json::from_value(json!({ "path": path })).unwrap();
        let res = crate::ipc::internal::into_internal(import(State(state.clone()), Json(req)).await).await;
        let skipped = state.supervisor.read().await.skip_port_check.load(std::sync::atomic::Ordering::Relaxed);
        // 같은 프로세스의 다른 시험이 같은 설정 파일을 쓰므로 되돌린다
        state.config_store.set_gui_settings_from_json(json!({ "portConflictCheck": true })).await.unwrap();
        assert_eq!(res.body["success"], true, "{}", res.body);
        assert!(skipped, "the running port check still uses the old setting");
    }

    #[test]
    fn bundle_round_trips_through_the_zip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out").join("saba.zip");
        let mut config = Map::new();
        config.insert("gui".into(), json!({ "language": "ko" }));
        let bundle = Bundle {
            meta: json!({ "format": FORMAT, "formatVersion": FORMAT_VERSION, "appVersion": "0.1.3", "secrets": true }),
            config,
            instances: vec![inst("a", Some("C:/srv/a")), inst("b", None)],
            node_token: Some("tok".into()),
        };
        write_bundle(&path, &bundle).unwrap();
        let back = read_bundle(&path).unwrap();
        assert_eq!(back.instances.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(back.instances[0].working_dir.as_deref(), Some("C:/srv/a"));
        assert_eq!(back.config.get("gui"), Some(&json!({ "language": "ko" })));
        assert_eq!(back.node_token.as_deref(), Some("tok"));
        assert!(!path.with_extension("saba-export.tmp").exists());
    }

    #[test]
    fn a_zip_that_is_not_an_export_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("saba-export.json", zip::write::FileOptions::default()).unwrap();
        zip.write_all(br#"{"format":"something-else","formatVersion":1}"#).unwrap();
        zip.finish().unwrap();
        assert!(read_bundle(&path).is_err());
    }

    #[test]
    fn pairing_fields_stay_local_unless_the_token_comes_along() {
        let mut sections = Map::new();
        sections.insert(
            "relay".into(),
            json!({ "prefix": "!s", "cloud": { "hostId": "old", "relayUrl": "https://x" }, "cloudNodes": [1], "cloudMembers": {} }),
        );
        keep_local_pairing(&mut sections);
        assert_eq!(sections["relay"], json!({ "prefix": "!s" }));
    }

    #[test]
    fn existing_running_and_folder_clashing_instances_are_not_imported() {
        let existing = vec![inst("a", Some("/srv/a")), inst("b", Some("/srv/b")), inst("e", None)];
        let running = HashSet::from(["b".to_string()]);
        let judge = |i: &ServerInstance, existing: &[ServerInstance], running: &HashSet<String>, overwrite: bool| {
            judge(i, existing, running, overwrite, &|id: &str| Path::new("/data/instances").join(id))
        };
        assert_eq!(judge(&inst("c", Some("/srv/c")), &existing, &running, false), Verdict::Add);
        assert_eq!(judge(&inst("a", Some("/srv/a")), &existing, &running, false), Verdict::Skip("exists"));
        assert_eq!(judge(&inst("a", Some("/srv/a")), &existing, &running, true), Verdict::Replace);
        assert_eq!(judge(&inst("b", Some("/srv/b")), &existing, &running, true), Verdict::Skip("running"));
        assert_eq!(judge(&inst("c", Some("/srv/a")), &existing, &running, false), Verdict::Skip("working_dir_in_use"));
        // 다른 서버 폴더나 인스턴스 폴더의 안·바깥도 같다 (만들기·설정 저장과 같은 판정)
        for dir in ["/srv/a/sub", "/srv", "/data/instances/e/server"] {
            assert_eq!(judge(&inst("c", Some(dir)), &existing, &running, false), Verdict::Skip("working_dir_in_use"), "{dir}");
        }
        assert_eq!(judge(&inst("c", Some("/srv/a2")), &existing, &running, false), Verdict::Add);
        assert_eq!(judge(&inst("../x", None), &existing, &running, false), Verdict::Skip("invalid_id"));
        let mut same_name = inst("d", Some("/srv/d"));
        same_name.name = "a".into();
        assert_eq!(judge(&same_name, &existing, &running, false), Verdict::Skip("name_taken"), "two servers with one name are told apart by name");
    }
}
