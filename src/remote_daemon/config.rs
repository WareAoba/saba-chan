//! 원격 데몬 설정과 짝 맺은 PC (config.json `remote` 한 곳).
//!
//!   enabled·port·address·transport   원격 포트를 열지, 어느 포트로, 릴레이에 알릴 공인 주소, 연결 방식
//!   enabled 기본 켬                   디스코드 계정과 PC를 함께 빼앗기지 않는 한 짝을 맺을 수 없다 (사용자 확정 2026-10-30)
//!   clients[]                        이 PC에 붙도록 짝 맺은 같은 주인의 PC. 토큰은 SHA-256만 둔다
//!   daemons[]                        이 PC가 붙는 원격 데몬(D1b): 주소·고정한 지문·받은 토큰

use saba_chan_updater_lib::config_file::ConfigFile;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// 연결 방식. 지금은 직접 연결뿐이다 (공인 주소가 있는 PC). 릴레이 터널은 나중에 이 자리에 더한다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub port: u16,
    pub address: String,
    pub transport: Transport,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            port: saba_chan_updater_lib::constants::DEFAULT_REMOTE_PORT,
            address: String::new(),
            transport: Transport::Direct,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub host_id: String,
    pub name: String,
    /// 그 PC 주인의 디스코드 이름 (릴레이가 알려 준다). 예전에 짝 맺은 PC는 비어 있다가 PC 목록을 읽을 때 채운다
    #[serde(default)]
    pub owner_name: String,
    pub token_hash: String,
    pub paired_at: String,
}

/// 이 PC가 붙는 원격 데몬. 토큰은 보내야 하므로 원문으로 둔다 (노드 토큰처럼 이 PC의 설정 파일에만)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Daemon {
    pub host_id: String,
    pub name: String,
    #[serde(default)]
    pub owner_name: String,
    pub address: String,
    pub port: u16,
    pub cert_sha256: String,
    pub token: String,
    pub paired_at: String,
}

/// 화면에 보일 PC 이름: `디스코드 이름(PC 이름)`. 주인 이름을 모르거나 PC 이름이 곧 주인 이름이면 PC 이름만
pub fn label(owner_name: &str, name: &str) -> String {
    let (owner, name) = (owner_name.trim(), name.trim());
    if owner.is_empty() || owner == name {
        name.to_string()
    } else if name.is_empty() {
        owner.to_string()
    } else {
        format!("{owner}({name})")
    }
}

impl Client {
    pub fn label(&self) -> String {
        label(&self.owner_name, &self.name)
    }
}

impl Daemon {
    pub fn label(&self) -> String {
        label(&self.owner_name, &self.name)
    }
}

const TOKEN_PREFIX: &str = "sbr_";

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn section(doc: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let remote = doc.entry("remote").or_insert_with(|| json!({}));
    if !remote.is_object() {
        *remote = json!({});
    }
    remote.as_object_mut().expect("just made an object")
}

pub fn settings_in(file: &ConfigFile) -> Settings {
    serde_json::from_value(file.peek_section("remote")).unwrap_or_default()
}

/// 설정만 바꾼다 (짝 맺은 PC는 그대로)
pub fn save_settings_in(file: &ConfigFile, settings: &Settings) -> std::io::Result<()> {
    let Value::Object(values) = serde_json::to_value(settings).expect("settings serialize") else { unreachable!() };
    file.update(|doc| section(doc).extend(values)).map(|_| ())
}

pub fn clients_in(file: &ConfigFile) -> Vec<Client> {
    file.peek_section("remote")
        .get("clients")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn write_clients(file: &ConfigFile, edit: impl FnOnce(&mut Vec<Client>)) -> std::io::Result<Vec<Client>> {
    let mut out = Vec::new();
    file.update(|doc| {
        let remote = section(doc);
        let mut clients: Vec<Client> =
            remote.get("clients").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        edit(&mut clients);
        remote.insert("clients".into(), serde_json::to_value(&clients).expect("clients serialize"));
        out = clients;
    })?;
    Ok(out)
}

/// `host_id` PC 앞으로 새 토큰을 낸다. 같은 PC가 다시 짝을 맺으면 예전 토큰은 더는 통하지 않는다
pub fn issue_in(file: &ConfigFile, host_id: &str, name: &str, owner_name: &str) -> std::io::Result<String> {
    let token = format!("{TOKEN_PREFIX}{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let client = Client {
        host_id: host_id.to_string(),
        name: name.to_string(),
        owner_name: owner_name.to_string(),
        token_hash: hash(&token),
        paired_at: chrono::Utc::now().to_rfc3339(),
    };
    write_clients(file, |clients| {
        clients.retain(|c| c.host_id != host_id);
        clients.push(client);
    })?;
    Ok(token)
}

/// 토큰의 주인 PC. 모르는 토큰이면 None
pub fn verify(clients: &[Client], token: &str) -> Option<Client> {
    if !token.starts_with(TOKEN_PREFIX) {
        return None;
    }
    let h = hash(token);
    clients.iter().find(|c| constant_time_eq(c.token_hash.as_bytes(), h.as_bytes())).cloned()
}

pub fn daemons_in(file: &ConfigFile) -> Vec<Daemon> {
    file.peek_section("remote")
        .get("daemons")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn write_daemons(file: &ConfigFile, edit: impl FnOnce(&mut Vec<Daemon>)) -> std::io::Result<Vec<Daemon>> {
    let mut out = Vec::new();
    file.update(|doc| {
        let remote = section(doc);
        let mut daemons: Vec<Daemon> =
            remote.get("daemons").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        edit(&mut daemons);
        remote.insert("daemons".into(), serde_json::to_value(&daemons).expect("daemons serialize"));
        out = daemons;
    })?;
    Ok(out)
}

/// 원격 데몬을 저장한다. 같은 PC면 바꾼다
pub fn save_daemon_in(file: &ConfigFile, daemon: Daemon) -> std::io::Result<()> {
    write_daemons(file, |daemons| {
        daemons.retain(|d| d.host_id != daemon.host_id);
        daemons.push(daemon);
    })
    .map(|_| ())
}

/// 릴레이가 알려 준 새 주소·지문으로 바꾼다 (토큰은 그대로). 없는 PC면 false
pub fn update_endpoint_in(file: &ConfigFile, host_id: &str, address: &str, port: u16, cert_sha256: &str) -> std::io::Result<bool> {
    let mut found = false;
    write_daemons(file, |daemons| {
        if let Some(d) = daemons.iter_mut().find(|d| d.host_id == host_id) {
            (d.address, d.port, d.cert_sha256) = (address.to_string(), port, cert_sha256.to_string());
            found = true;
        }
    })?;
    Ok(found)
}

/// 원격 데몬을 잊는다. 있었으면 true
pub fn forget_daemon_in(file: &ConfigFile, host_id: &str) -> std::io::Result<bool> {
    let before = daemons_in(file).len();
    Ok(write_daemons(file, |daemons| daemons.retain(|d| d.host_id != host_id))?.len() != before)
}

/// 릴레이가 알려 준 PC 이름·주인 이름(`(hostId, name, ownerName)`)으로 짝 맺은 PC와 원격 데몬의 이름을 맞춘다.
/// 바뀐 것이 없으면 쓰지 않는다. 주인 이름을 모르는 옛 릴레이면(빈 값) 주인 이름은 그대로 둔다
pub fn refresh_names_in(file: &ConfigFile, known: &[(String, String, String)]) -> std::io::Result<()> {
    let fresh = |host_id: &str, name: &mut String, owner: &mut String| {
        let Some((_, n, o)) = known.iter().find(|(h, ..)| h == host_id) else { return false };
        let mut changed = false;
        if !n.is_empty() && n != name {
            *name = n.clone();
            changed = true;
        }
        if !o.is_empty() && o != owner {
            *owner = o.clone();
            changed = true;
        }
        changed
    };
    let stale = clients_in(file).into_iter().any(|mut c| fresh(&c.host_id, &mut c.name, &mut c.owner_name))
        || daemons_in(file).into_iter().any(|mut d| fresh(&d.host_id, &mut d.name, &mut d.owner_name));
    if !stale {
        return Ok(());
    }
    write_clients(file, |clients| {
        for c in clients.iter_mut() {
            fresh(&c.host_id, &mut c.name, &mut c.owner_name);
        }
    })?;
    write_daemons(file, |daemons| {
        for d in daemons.iter_mut() {
            fresh(&d.host_id, &mut d.name, &mut d.owner_name);
        }
    })
    .map(|_| ())
}

/// 짝을 푼다. 있었으면 true
pub fn revoke_in(file: &ConfigFile, host_id: &str) -> std::io::Result<bool> {
    let before = clients_in(file).len();
    Ok(write_clients(file, |clients| clients.retain(|c| c.host_id != host_id))?.len() != before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> (tempfile::TempDir, ConfigFile) {
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        (dir, file)
    }

    #[test]
    fn settings_default_to_on_on_the_default_port_and_keep_other_sections() {
        let (_d, f) = file();
        f.set_section("node", json!({"token": "sbn_x.y"})).unwrap();
        assert_eq!(settings_in(&f), Settings::default());
        assert!(settings_in(&f).enabled, "remote opening is on unless turned off");
        save_settings_in(&f, &Settings { enabled: false, ..Settings::default() }).unwrap();
        assert!(!settings_in(&f).enabled, "a saved off stays off");
        assert_eq!(Settings::default().port, saba_chan_updater_lib::constants::DEFAULT_REMOTE_PORT);
        let s = Settings { enabled: true, port: 7443, address: "saba.example.com".into(), transport: Transport::Direct };
        save_settings_in(&f, &s).unwrap();
        assert_eq!(settings_in(&f), s);
        assert_eq!(f.section("node")["token"], "sbn_x.y");
        assert_eq!(f.section("remote")["transport"], "direct");
    }

    #[test]
    fn a_paired_pc_is_known_by_its_token_and_only_the_hash_is_stored() {
        let (_d, f) = file();
        save_settings_in(&f, &Settings { enabled: true, ..Settings::default() }).unwrap();
        let token = issue_in(&f, "host-a", "집 PC", "").unwrap();
        assert!(token.starts_with("sbr_") && token.len() > 60);
        assert!(!std::fs::read_to_string(f.path()).unwrap().contains(&token));
        let clients = clients_in(&f);
        assert_eq!(verify(&clients, &token).map(|c| c.host_id), Some("host-a".into()));
        assert_eq!(verify(&clients, "sbr_wrong"), None);
        assert_eq!(verify(&clients, &clients[0].token_hash), None, "the stored hash is not a token");
        assert!(settings_in(&f).enabled, "pairing keeps the settings");
    }

    #[test]
    fn pairing_again_replaces_the_old_token_and_revoking_forgets_the_pc() {
        let (_d, f) = file();
        let old = issue_in(&f, "host-a", "집 PC", "").unwrap();
        let other = issue_in(&f, "host-b", "노트북", "").unwrap();
        let new = issue_in(&f, "host-a", "집 PC", "").unwrap();
        let clients = clients_in(&f);
        assert_eq!(clients.len(), 2);
        assert_eq!(verify(&clients, &old), None);
        assert!(verify(&clients, &new).is_some());
        assert!(revoke_in(&f, "host-a").unwrap());
        assert!(!revoke_in(&f, "host-a").unwrap());
        let clients = clients_in(&f);
        assert_eq!(verify(&clients, &new), None);
        assert!(verify(&clients, &other).is_some());
    }

    fn daemon(host: &str, token: &str) -> Daemon {
        Daemon {
            host_id: host.into(),
            name: "원격".into(),
            owner_name: String::new(),
            address: "203.0.113.7".into(),
            port: 57480,
            cert_sha256: "ab".repeat(32),
            token: token.into(),
            paired_at: "2026-10-25T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_remote_daemon_is_saved_moved_and_forgotten_without_touching_the_rest() {
        let (_d, f) = file();
        let token = issue_in(&f, "host-a", "집 PC", "").unwrap();
        save_daemon_in(&f, daemon("host-b", "sbr_one")).unwrap();
        save_daemon_in(&f, daemon("host-b", "sbr_two")).unwrap();
        save_daemon_in(&f, daemon("host-c", "sbr_three")).unwrap();
        let daemons = daemons_in(&f);
        assert_eq!(daemons.len(), 2, "the same PC is saved once");
        assert_eq!(daemons.iter().find(|d| d.host_id == "host-b").unwrap().token, "sbr_two");

        assert!(update_endpoint_in(&f, "host-b", "saba.example.com", 7443, &"cd".repeat(32)).unwrap());
        let b = daemons_in(&f).into_iter().find(|d| d.host_id == "host-b").unwrap();
        assert_eq!((b.address.as_str(), b.port, b.token.as_str()), ("saba.example.com", 7443, "sbr_two"));
        assert!(!update_endpoint_in(&f, "host-x", "a", 1, "b").unwrap());

        assert!(forget_daemon_in(&f, "host-b").unwrap());
        assert!(!forget_daemon_in(&f, "host-b").unwrap());
        assert_eq!(daemons_in(&f).len(), 1);
        assert!(verify(&clients_in(&f), &token).is_some(), "paired PCs stay");
    }

    #[test]
    fn a_pc_is_shown_as_its_owners_discord_name_and_its_own_name() {
        assert_eq!(label("김마무", "DESKTOP-1"), "김마무(DESKTOP-1)");
        assert_eq!(label("", "DESKTOP-1"), "DESKTOP-1", "old relays do not send the owner");
        assert_eq!(label("김마무", "김마무"), "김마무", "a PC named after its owner is not doubled");
        assert_eq!(label("김마무", ""), "김마무");
    }

    #[test]
    fn names_from_the_relay_fill_in_saved_pcs_and_leave_unknown_ones_alone() {
        let (_d, f) = file();
        issue_in(&f, "host-a", "집 PC", "").unwrap();
        save_daemon_in(&f, daemon("host-b", "sbr_one")).unwrap();
        save_daemon_in(&f, daemon("host-c", "sbr_two")).unwrap();
        let known = vec![
            ("host-a".to_string(), "집 PC".to_string(), "김마무".to_string()),
            ("host-b".to_string(), "거실 PC".to_string(), "김마무".to_string()),
            ("host-c".to_string(), "노트북".to_string(), String::new()),
        ];
        refresh_names_in(&f, &known).unwrap();
        assert_eq!(clients_in(&f)[0].label(), "김마무(집 PC)");
        let daemons = daemons_in(&f);
        assert_eq!(daemons[0].label(), "김마무(거실 PC)");
        assert_eq!((daemons[1].name.as_str(), daemons[1].owner_name.as_str()), ("노트북", ""));
        assert_eq!(daemons[0].token, "sbr_one", "tokens stay");

        let saved = std::fs::metadata(f.path()).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        refresh_names_in(&f, &known).unwrap();
        assert_eq!(std::fs::metadata(f.path()).unwrap().modified().unwrap(), saved, "nothing changed, nothing written");
    }
}
