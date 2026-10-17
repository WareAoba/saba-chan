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

fn node_token_path() -> PathBuf {
    saba_chan_updater_lib::constants::resolve_data_dir().join(".node_token")
}

/// 비밀값을 뺀 인스턴스: REST 비밀번호와 모듈이 `password`로 선언한 설정
pub(crate) fn strip_instance_secrets(instance: &mut ServerInstance, types: &HashMap<String, String>) {
    instance.rest_password = None;
    instance.module_settings.retain(|k, _| types.get(k).map(String::as_str) != Some("password"));
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

/// 가져올 인스턴스마다 넣을지 건너뛸지
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    Add,
    Replace,
    Skip(&'static str),
}

pub(crate) fn judge(
    incoming: &ServerInstance,
    existing: &[ServerInstance],
    running: &HashSet<String>,
    overwrite: bool,
) -> Verdict {
    if !crate::utils::is_single_path_component(&incoming.id) {
        return Verdict::Skip("invalid_id");
    }
    let same = existing.iter().find(|e| e.id == incoming.id);
    if let Some(dir) = incoming.working_dir.as_deref() {
        let other = existing.iter().find(|o| {
            o.id != incoming.id
                && o.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::same_path(Path::new(d), Path::new(dir)))
        });
        if other.is_some() {
            return Verdict::Skip("working_dir_in_use");
        }
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
                    let types = sup
                        .module_loader
                        .get_module(&inst.module_name)
                        .map(|m| m.metadata.setting_types())
                        .unwrap_or_default();
                    strip_instance_secrets(&mut inst, &types);
                }
                inst
            })
            .collect()
    };
    let config = state.config_store.portable_sections(req.secrets);
    let node_token = if req.secrets {
        std::fs::read_to_string(node_token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
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

    let running: HashSet<String> = {
        let sup = state.supervisor.read().await;
        let mut ids: HashSet<String> = sup.managed_store.running_instance_ids().await.into_iter().collect();
        ids.extend(sup.instance_store.list().iter().filter(|i| sup.tracker.get_pid(&i.id).is_ok()).map(|i| i.id.clone()));
        ids
    };

    let (mut added, mut replaced, mut skipped, mut missing_modules) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    {
        let mut sup = state.supervisor.write().await;
        for inst in bundle.instances {
            // 남이 준 zip이면 이 경로의 프로그램이 실행된다: 미리보기에서 보이게
            let entry = json!({
                "id": inst.id,
                "name": inst.name,
                "working_dir": inst.working_dir,
                "executable_path": inst.executable_path,
            });
            let verdict = judge(&inst, sup.instance_store.list(), &running, req.overwrite);
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
            if sup.module_loader.get_module(&inst.module_name).is_err() && !missing_modules.contains(&inst.module_name) {
                missing_modules.push(inst.module_name.clone());
            }
            if verdict == Verdict::Add { added.push(entry) } else { replaced.push(entry) }
        }
    }

    // 이 PC가 이미 페어링돼 있으면 그 토큰을 둔다
    let node_token = match bundle.node_token {
        None => Value::Null,
        Some(_) if std::fs::read_to_string(node_token_path()).is_ok_and(|t| !t.trim().is_empty()) => json!("kept"),
        Some(_) if req.dry_run => json!("imported"),
        Some(token) => match std::fs::write(node_token_path(), token) {
            Ok(()) => json!("imported"),
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
        match state.config_store.import_sections(sections).await {
            Ok(keys) => keys,
            Err(e) => return error(StatusCode::BAD_REQUEST, "INVALID_CONFIG", e.to_string()),
        }
    };

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
        strip_instance_secrets(&mut i, &types);
        assert_eq!(i.rest_password, None);
        assert!(!i.module_settings.contains_key("AdminPassword"));
        assert_eq!(i.module_settings.get("port"), Some(&json!(25565)));
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
        let existing = vec![inst("a", Some("/srv/a")), inst("b", Some("/srv/b"))];
        let running = HashSet::from(["b".to_string()]);
        assert_eq!(judge(&inst("c", Some("/srv/c")), &existing, &running, false), Verdict::Add);
        assert_eq!(judge(&inst("a", Some("/srv/a")), &existing, &running, false), Verdict::Skip("exists"));
        assert_eq!(judge(&inst("a", Some("/srv/a")), &existing, &running, true), Verdict::Replace);
        assert_eq!(judge(&inst("b", Some("/srv/b")), &existing, &running, true), Verdict::Skip("running"));
        assert_eq!(judge(&inst("c", Some("/srv/a")), &existing, &running, false), Verdict::Skip("working_dir_in_use"));
        assert_eq!(judge(&inst("../x", None), &existing, &running, false), Verdict::Skip("invalid_id"));
    }
}
