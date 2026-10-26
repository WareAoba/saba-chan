//! 원격 데몬 설정과 짝 맺은 PC (config.json `remote` 한 곳).
//!
//!   enabled·port·address·transport   원격 포트를 열지, 어느 포트로, 릴레이에 알릴 공인 주소, 연결 방식
//!   clients[]                        이 PC에 붙도록 짝 맺은 같은 주인의 PC. 토큰은 SHA-256만 둔다

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
            enabled: false,
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
    pub token_hash: String,
    pub paired_at: String,
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
pub fn issue_in(file: &ConfigFile, host_id: &str, name: &str) -> std::io::Result<String> {
    let token = format!("{TOKEN_PREFIX}{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let client = Client {
        host_id: host_id.to_string(),
        name: name.to_string(),
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
    fn settings_default_to_off_on_the_default_port_and_keep_other_sections() {
        let (_d, f) = file();
        f.set_section("node", json!({"token": "sbn_x.y"})).unwrap();
        assert_eq!(settings_in(&f), Settings::default());
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
        let token = issue_in(&f, "host-a", "집 PC").unwrap();
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
        let old = issue_in(&f, "host-a", "집 PC").unwrap();
        let other = issue_in(&f, "host-b", "노트북").unwrap();
        let new = issue_in(&f, "host-a", "집 PC").unwrap();
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
}
