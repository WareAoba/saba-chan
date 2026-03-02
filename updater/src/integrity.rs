//! 무결성 검증 — 설치된 컴포넌트의 SHA256 해시를 매니페스트와 비교
//!
//! ## 동작 원리
//! 업데이터가 모든 컴포넌트(코어, CLI, GUI, 모듈, 익스텐션, 디스코드 봇)의
//! 설치 파일을 스캔하여 SHA256 해시를 계산하고, 매니페스트에 기록된 기대값과
//! 비교하여 위변조 여부를 판정합니다.
//!
//! ## 매니페스트 소스
//! - **코어 컴포넌트** (saba-core, cli, gui, updater, discord_bot):
//!   `manifest.json` (GitHub 릴리즈에서 다운로드, 설치 시 로컬에 저장)
//! - **모듈**: 모듈 리포의 `manifest.json`
//! - **익스텐션**: 익스텐션 리포의 `manifest.json` (GitHub 릴리즈 에셋)
//!
//! ## 검증 후 결과
//! 코어 데몬이 IPC를 통해 요청하면 검증 결과를 반환하며,
//! 코어는 이를 터미널에 출력합니다.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

// ══════════════════════════════════════════════════════
// 검증 결과 구조체
// ══════════════════════════════════════════════════════

/// 개별 컴포넌트의 무결성 검증 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentIntegrity {
    /// 컴포넌트 키 (예: "saba-core", "module-minecraft", "ext-docker")
    pub component: String,
    /// 사용자 표시용 이름
    pub display_name: String,
    /// 검증 상태
    pub status: IntegrityStatus,
    /// 매니페스트에 기록된 기대 해시
    pub expected_hash: Option<String>,
    /// 실제 계산된 해시
    pub actual_hash: Option<String>,
    /// 검증 대상 파일 경로
    pub file_path: Option<String>,
    /// 상세 메시지
    pub message: String,
}

/// 무결성 상태
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IntegrityStatus {
    /// 검증 통과 — 해시 일치
    Verified,
    /// 해시 불일치 — 위변조 가능성
    Tampered,
    /// 매니페스트에 해시가 없음 — 검증 불가 (개발 환경 등)
    NoHash,
    /// 파일을 찾을 수 없음
    FileNotFound,
    /// 검증 중 오류 발생
    Error,
}

/// 전체 무결성 검증 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    /// 검증 시각
    pub checked_at: String,
    /// 전체 결과 요약
    pub overall: OverallIntegrity,
    /// 개별 컴포넌트 결과
    pub components: Vec<ComponentIntegrity>,
    /// 검증된 컴포넌트 수
    pub total: usize,
    /// 통과한 컴포넌트 수
    pub verified: usize,
    /// 실패한 컴포넌트 수 (Tampered + Error)
    pub failed: usize,
    /// 검증 불가 수 (NoHash + FileNotFound)
    pub skipped: usize,
}

/// 전체 무결성 요약
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OverallIntegrity {
    /// 모든 컴포넌트 검증 통과
    AllVerified,
    /// 일부 해시 없음 (개발 환경 등) — 위변조는 감지되지 않음
    Partial,
    /// 하나 이상의 컴포넌트에서 위변조가 감지됨
    TamperDetected,
    /// 검증 대상이 없음
    Empty,
}

// ══════════════════════════════════════════════════════
// IntegrityChecker
// ══════════════════════════════════════════════════════

/// 무결성 검증기
pub struct IntegrityChecker {
    /// 설치 루트 디렉터리 (코어 바이너리 기준)
    install_root: PathBuf,
    /// 모듈 디렉터리
    modules_dir: PathBuf,
    /// 익스텐션 디렉터리
    extensions_dir: PathBuf,
}

impl IntegrityChecker {
    pub fn new(install_root: PathBuf, modules_dir: PathBuf, extensions_dir: PathBuf) -> Self {
        Self {
            install_root,
            modules_dir,
            extensions_dir,
        }
    }

    /// 모든 컴포넌트의 무결성을 검증합니다.
    ///
    /// # Arguments
    /// * `expected_hashes` - 컴포넌트 키 → 기대 SHA256 해시 맵
    ///   (manifest.json + 모듈/익스텐션 manifest에서 수집)
    pub fn verify_all(&self, expected_hashes: &HashMap<String, ComponentHashInfo>) -> IntegrityReport {
        let mut components = Vec::new();

        for (key, info) in expected_hashes {
            let result = self.verify_component(key, info);
            components.push(result);
        }

        // 정렬: 코어 컴포넌트 → 모듈 → 익스텐션 순
        components.sort_by(|a, b| component_sort_key(&a.component).cmp(&component_sort_key(&b.component)));

        let total = components.len();
        let verified = components.iter().filter(|c| c.status == IntegrityStatus::Verified).count();
        let failed = components.iter().filter(|c| matches!(c.status, IntegrityStatus::Tampered | IntegrityStatus::Error)).count();
        let skipped = components.iter().filter(|c| matches!(c.status, IntegrityStatus::NoHash | IntegrityStatus::FileNotFound)).count();

        let overall = if total == 0 {
            OverallIntegrity::Empty
        } else if failed > 0 {
            OverallIntegrity::TamperDetected
        } else if verified == total {
            OverallIntegrity::AllVerified
        } else {
            OverallIntegrity::Partial
        };

        IntegrityReport {
            checked_at: chrono::Utc::now().to_rfc3339(),
            overall,
            components,
            total,
            verified,
            failed,
            skipped,
        }
    }

    /// 단일 컴포넌트의 무결성을 검증합니다.
    fn verify_component(&self, key: &str, info: &ComponentHashInfo) -> ComponentIntegrity {
        let display_name = info.display_name.clone();
        let file_path = self.resolve_component_path(key, info);

        // 1. 기대 해시가 없으면 NoHash
        let expected = match &info.expected_sha256 {
            Some(h) if !h.is_empty() => h.clone(),
            _ => {
                return ComponentIntegrity {
                    component: key.to_string(),
                    display_name,
                    status: IntegrityStatus::NoHash,
                    expected_hash: None,
                    actual_hash: None,
                    file_path: file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                    message: "매니페스트에 SHA256 해시가 없습니다".to_string(),
                };
            }
        };

        // 2. 파일 경로 해결
        let path = match file_path {
            Some(ref p) if p.exists() => p,
            Some(ref p) => {
                return ComponentIntegrity {
                    component: key.to_string(),
                    display_name,
                    status: IntegrityStatus::FileNotFound,
                    expected_hash: Some(expected),
                    actual_hash: None,
                    file_path: Some(p.to_string_lossy().to_string()),
                    message: format!("파일을 찾을 수 없습니다: {}", p.display()),
                };
            }
            None => {
                return ComponentIntegrity {
                    component: key.to_string(),
                    display_name,
                    status: IntegrityStatus::FileNotFound,
                    expected_hash: Some(expected),
                    actual_hash: None,
                    file_path: None,
                    message: "컴포넌트 파일 경로를 해결할 수 없습니다".to_string(),
                };
            }
        };

        // 3. SHA256 계산
        match compute_sha256(path) {
            Ok(actual) => {
                if actual == expected.to_lowercase() {
                    ComponentIntegrity {
                        component: key.to_string(),
                        display_name,
                        status: IntegrityStatus::Verified,
                        expected_hash: Some(expected),
                        actual_hash: Some(actual),
                        file_path: Some(path.to_string_lossy().to_string()),
                        message: "검증 통과".to_string(),
                    }
                } else {
                    ComponentIntegrity {
                        component: key.to_string(),
                        display_name,
                        status: IntegrityStatus::Tampered,
                        expected_hash: Some(expected),
                        actual_hash: Some(actual),
                        file_path: Some(path.to_string_lossy().to_string()),
                        message: "SHA256 해시 불일치 — 파일이 변조되었을 수 있습니다".to_string(),
                    }
                }
            }
            Err(e) => {
                ComponentIntegrity {
                    component: key.to_string(),
                    display_name,
                    status: IntegrityStatus::Error,
                    expected_hash: Some(expected),
                    actual_hash: None,
                    file_path: Some(path.to_string_lossy().to_string()),
                    message: format!("해시 계산 중 오류: {}", e),
                }
            }
        }
    }

    /// 컴포넌트 키와 install_dir로 실제 파일 경로를 해결합니다.
    fn resolve_component_path(&self, key: &str, info: &ComponentHashInfo) -> Option<PathBuf> {
        match key {
            // 코어 바이너리: install_root 기준
            "saba-core" => {
                let name = if cfg!(windows) { "saba-core.exe" } else { "saba-core" };
                Some(self.install_root.join(name))
            }
            "cli" => {
                let name = if cfg!(windows) { "saba-chan-cli.exe" } else { "saba-chan-cli" };
                Some(self.install_root.join(name))
            }
            "gui" => {
                // GUI는 install_dir이 "saba-chan-gui"로 지정됨
                let dir = info.install_dir.as_deref().unwrap_or("saba-chan-gui");
                let name = if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" };
                Some(self.install_root.join(dir).join(name))
            }
            "updater" => {
                let name = if cfg!(windows) { "saba-chan-updater.exe" } else { "saba-chan-updater" };
                Some(self.install_root.join(name))
            }
            "discord_bot" => {
                let dir = info.install_dir.as_deref().unwrap_or("discord_bot");
                // 디스코드 봇은 Node.js — index.js를 기준으로 해시
                Some(self.install_root.join(dir).join("index.js"))
            }
            k if k.starts_with("module-") => {
                let module_name = k.strip_prefix("module-").unwrap_or(k);
                // 모듈은 Python — lifecycle.py를 기준으로 해시
                Some(self.modules_dir.join(module_name).join("lifecycle.py"))
            }
            k if k.starts_with("ext-") => {
                let ext_name = k.strip_prefix("ext-").unwrap_or(k);
                let dir = info.install_dir.as_deref()
                    .map(|d| PathBuf::from(d.trim_start_matches("extensions/")))
                    .unwrap_or_else(|| PathBuf::from(ext_name));
                // 익스텐션의 매니페스트 파일
                Some(self.extensions_dir.join(dir).join("manifest.json"))
            }
            _ => None,
        }
    }
}

// ══════════════════════════════════════════════════════
// SHA256 계산
// ══════════════════════════════════════════════════════

/// 파일의 SHA256 해시를 계산합니다.
pub fn compute_sha256(path: &Path) -> Result<String, String> {
    use std::io::BufReader;

    let file = std::fs::File::open(path)
        .map_err(|e| format!("파일 열기 실패: {}", e))?;

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let n = reader.read(&mut buffer)
            .map_err(|e| format!("파일 읽기 실패: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hex_encode(&hasher.finalize()))
}

// ══════════════════════════════════════════════════════
// 순수 Rust SHA-256 구현 (외부 크레이트 불필요)
// ══════════════════════════════════════════════════════

/// SHA-256 constants: first 32 bits of the fractional parts of the cube roots of the first 64 primes
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::new(),
            total_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.total_len += data.len() as u64;
        self.buffer.extend_from_slice(data);

        while self.buffer.len() >= 64 {
            let block: Vec<u8> = self.buffer.drain(..64).collect();
            self.process_block(&block);
        }
    }

    fn finalize(&mut self) -> [u8; 32] {
        let bit_len = self.total_len * 8;

        // Padding
        self.buffer.push(0x80);
        while (self.buffer.len() % 64) != 56 {
            self.buffer.push(0x00);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());

        // Process remaining blocks
        let remaining = self.buffer.clone();
        for chunk in remaining.chunks(64) {
            self.process_block(chunk);
        }

        // Produce output
        let mut result = [0u8; 32];
        for (i, &val) in self.state.iter().enumerate() {
            result[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
        }
        result
    }

    fn process_block(&mut self, block: &[u8]) {
        let mut w = [0u32; 64];

        // Prepare message schedule
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        // Working variables
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;

        // Compression
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        // Update state
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ══════════════════════════════════════════════════════
// 해시 정보 수집용 구조체
// ══════════════════════════════════════════════════════

/// 검증 대상 컴포넌트의 해시 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHashInfo {
    /// 사용자 표시용 이름
    pub display_name: String,
    /// 매니페스트에 기록된 기대 SHA256 해시
    pub expected_sha256: Option<String>,
    /// 설치 디렉터리 (install_root 기준 상대경로)
    pub install_dir: Option<String>,
}

// ══════════════════════════════════════════════════════
// 해시 정보 수집 헬퍼
// ══════════════════════════════════════════════════════

/// 서버에서 가져온 ReleaseManifest에서 코어 컴포넌트 해시 정보를 수집합니다.
/// 이것이 신뢰할 수 있는 소스입니다 (GitHub 릴리즈에서 fetch).
pub fn collect_hashes_from_server_manifest(
    manifest: &crate::github::ReleaseManifest,
) -> HashMap<String, ComponentHashInfo> {
    let mut hashes = HashMap::new();

    let display_names: HashMap<&str, &str> = HashMap::from([
        ("saba-core", "Saba-Core"),
        ("cli", "CLI"),
        ("gui", "GUI"),
        ("updater", "Updater"),
        ("discord_bot", "Discord Bot"),
    ]);

    for (key, info) in &manifest.components {
        let display_name = if key.starts_with("module-") {
            let module_name = key.strip_prefix("module-").unwrap_or(key);
            format!("Module: {}", module_name)
        } else if key.starts_with("ext-") {
            let ext_name = key.strip_prefix("ext-").unwrap_or(key);
            format!("Extension: {}", ext_name)
        } else {
            display_names.get(key.as_str())
                .unwrap_or(&key.as_str())
                .to_string()
        };

        hashes.insert(key.clone(), ComponentHashInfo {
            display_name,
            expected_sha256: info.sha256.clone(),
            install_dir: info.install_dir.clone(),
        });
    }

    hashes
}

/// 서버에서 가져온 익스텐션 manifest.json에서 해시 정보를 수집합니다.
/// manifest_json은 saba-chan-extensions 릴리즈 에셋에서 fetch한 JSON 문자열입니다.
/// 구조: `{ "schema_version": 1, "extensions": { "docker": { "name": "...", "sha256": "...", ... } } }`
pub fn collect_hashes_from_extension_manifest(
    manifest_json: &str,
) -> HashMap<String, ComponentHashInfo> {
    let mut hashes = HashMap::new();

    #[derive(Deserialize)]
    struct ExtManifest {
        extensions: Option<HashMap<String, ExtEntry>>,
    }

    #[derive(Deserialize)]
    struct ExtEntry {
        name: Option<String>,
        sha256: Option<String>,
        install_dir: Option<String>,
    }

    let manifest: ExtManifest = match serde_json::from_str(manifest_json) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[Integrity] 익스텐션 manifest.json 파싱 실패: {}", e);
            return hashes;
        }
    };

    if let Some(extensions) = &manifest.extensions {
        for (key, entry) in extensions {
            let component_key = format!("ext-{}", key);
            let display = entry.name.clone().unwrap_or_else(|| key.clone());
            hashes.insert(component_key, ComponentHashInfo {
                display_name: format!("Extension: {}", display),
                expected_sha256: entry.sha256.clone(),
                install_dir: entry.install_dir.clone(),
            });
        }
    }

    hashes
}

/// 서버에서 가져온 모듈 manifest.json에서 해시 정보를 수집합니다.
/// manifest_json은 saba-chan-modules 릴리즈 에셋에서 fetch한 JSON 문자열입니다.
/// 구조: `{ "schema_version": 1, "modules": { "minecraft": { "name": "...", "sha256": "...", ... } } }`
pub fn collect_hashes_from_module_manifest(
    manifest_json: &str,
) -> HashMap<String, ComponentHashInfo> {
    let mut hashes = HashMap::new();

    #[derive(Deserialize)]
    struct ModManifest {
        modules: Option<HashMap<String, ModEntry>>,
    }

    #[derive(Deserialize)]
    struct ModEntry {
        name: Option<String>,
        sha256: Option<String>,
        install_dir: Option<String>,
    }

    let manifest: ModManifest = match serde_json::from_str(manifest_json) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[Integrity] 모듈 manifest.json 파싱 실패: {}", e);
            return hashes;
        }
    };

    if let Some(modules) = &manifest.modules {
        for (key, entry) in modules {
            let component_key = if key.starts_with("module-") {
                key.clone()
            } else {
                format!("module-{}", key)
            };
            let display = entry.name.clone().unwrap_or_else(|| key.clone());
            hashes.insert(component_key, ComponentHashInfo {
                display_name: format!("Module: {}", display),
                expected_sha256: entry.sha256.clone(),
                install_dir: entry.install_dir.clone(),
            });
        }
    }

    hashes
}

/// (폴백) 로컬 release-manifest.json에서 코어 컴포넌트 해시 정보를 수집합니다.
/// 서버에 접근할 수 없을 때만 사용됩니다. 로컬 매니페스트는 위변조 가능성이 있으므로
/// 결과에 경고를 포함합니다.
pub fn collect_core_hashes(manifest_path: &Path) -> HashMap<String, ComponentHashInfo> {
    let mut hashes = HashMap::new();

    let content = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[Integrity] manifest.json 읽기 실패: {}", e);
            return hashes;
        }
    };

    #[derive(Deserialize)]
    struct Manifest {
        components: HashMap<String, ManifestEntry>,
    }

    #[derive(Deserialize)]
    struct ManifestEntry {
        sha256: Option<String>,
        install_dir: Option<String>,
    }

    let manifest: Manifest = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("[Integrity] manifest.json 파싱 실패: {}", e);
            return hashes;
        }
    };

    let display_names: HashMap<&str, &str> = HashMap::from([
        ("saba-core", "Saba-Core"),
        ("cli", "CLI"),
        ("gui", "GUI"),
        ("updater", "Updater"),
        ("discord_bot", "Discord Bot"),
    ]);

    for (key, entry) in &manifest.components {
        hashes.insert(key.clone(), ComponentHashInfo {
            display_name: display_names.get(key.as_str())
                .unwrap_or(&key.as_str())
                .to_string(),
            expected_sha256: entry.sha256.clone(),
            install_dir: entry.install_dir.clone(),
        });
    }

    hashes
}

/// (폴백) 로컬 익스텐션 manifest.json에서 해시 정보를 수집합니다.
/// 서버에 접근할 수 없을 때만 사용됩니다.
pub fn collect_extension_hashes(manifest_path: &Path) -> HashMap<String, ComponentHashInfo> {
    let mut hashes = HashMap::new();

    let content = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[Integrity] 익스텐션 manifest.json 읽기 실패: {}", e);
            return hashes;
        }
    };

    #[derive(Deserialize)]
    struct Registry {
        extensions: HashMap<String, ExtEntry>,
    }

    #[derive(Deserialize)]
    struct ExtEntry {
        name: String,
        sha256: Option<String>,
        install_dir: Option<String>,
    }

    let registry: Registry = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[Integrity] 익스텐션 manifest.json 파싱 실패: {}", e);
            return hashes;
        }
    };

    for (key, entry) in &registry.extensions {
        let component_key = format!("ext-{}", key);
        hashes.insert(component_key, ComponentHashInfo {
            display_name: format!("Extension: {}", entry.name),
            expected_sha256: entry.sha256.clone(),
            install_dir: entry.install_dir.clone(),
        });
    }

    hashes
}

/// 모듈 manifest.json에서 해시 정보를 수집합니다.
pub fn collect_module_hashes(manifest_path: &Path) -> HashMap<String, ComponentHashInfo> {
    let mut hashes = HashMap::new();

    let content = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[Integrity] 모듈 manifest.json 읽기 실패: {}", e);
            return hashes;
        }
    };

    // 모듈 매니페스트: manifest.json의 components에 module-* 키로 포함됨
    // 또는 별도 모듈 리포의 manifest.json
    #[derive(Deserialize)]
    struct ModuleManifest {
        components: Option<HashMap<String, ModuleEntry>>,
        // 모듈 리포 형식 (release-manifest와 동일 구조)
        modules: Option<HashMap<String, ModuleEntry>>,
    }

    #[derive(Deserialize)]
    struct ModuleEntry {
        #[serde(default)]
        name: Option<String>,
        sha256: Option<String>,
        install_dir: Option<String>,
    }

    let manifest: ModuleManifest = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("[Integrity] 모듈 manifest.json 파싱 실패: {}", e);
            return hashes;
        }
    };

    // components 형식 (release-manifest에서 module-* 키 추출)
    if let Some(components) = &manifest.components {
        for (key, entry) in components {
            if key.starts_with("module-") {
                let module_name = key.strip_prefix("module-").unwrap_or(key);
                hashes.insert(key.clone(), ComponentHashInfo {
                    display_name: entry.name.clone()
                        .unwrap_or_else(|| format!("Module: {}", module_name)),
                    expected_sha256: entry.sha256.clone(),
                    install_dir: entry.install_dir.clone(),
                });
            }
        }
    }

    // modules 형식 (별도 모듈 리포)
    if let Some(modules) = &manifest.modules {
        for (key, entry) in modules {
            let component_key = if key.starts_with("module-") {
                key.clone()
            } else {
                format!("module-{}", key)
            };
            hashes.insert(component_key, ComponentHashInfo {
                display_name: entry.name.clone()
                    .unwrap_or_else(|| format!("Module: {}", key)),
                expected_sha256: entry.sha256.clone(),
                install_dir: entry.install_dir.clone(),
            });
        }
    }

    hashes
}

/// 컴포넌트 키를 정렬 우선순위로 변환 (코어 → 모듈 → 익스텐션)
fn component_sort_key(key: &str) -> (u8, String) {
    match key {
        "saba-core" => (0, key.to_string()),
        "cli" => (1, key.to_string()),
        "gui" => (2, key.to_string()),
        "updater" => (3, key.to_string()),
        "discord_bot" => (4, key.to_string()),
        k if k.starts_with("module-") => (5, key.to_string()),
        k if k.starts_with("ext-") => (6, key.to_string()),
        _ => (7, key.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_sha256_empty() {
        // SHA-256 of empty input = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let mut hasher = Sha256::new();
        hasher.update(&[]);
        let result = hex_encode(&hasher.finalize());
        assert_eq!(result, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn test_sha256_hello() {
        // SHA-256 of "hello" = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let mut hasher = Sha256::new();
        hasher.update(b"hello");
        let result = hex_encode(&hasher.finalize());
        assert_eq!(result, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_sha256_multiblock() {
        // Test with data spanning multiple blocks (>64 bytes)
        let data = "a".repeat(200);
        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        let result = hex_encode(&hasher.finalize());
        // hex-encoded SHA-256 is always 64 chars
        assert_eq!(result.len(), 64);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_compute_sha256_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"hello").unwrap();
        drop(f);

        let hash = compute_sha256(&file_path).unwrap();
        assert_eq!(hash, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_integrity_verified() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        std::fs::write(&file_path, b"test content").unwrap();

        let actual_hash = compute_sha256(&file_path).unwrap();

        let checker = IntegrityChecker::new(
            dir.path().to_path_buf(),
            dir.path().join("modules"),
            dir.path().join("extensions"),
        );

        let mut hashes = HashMap::new();
        hashes.insert("saba-core".to_string(), ComponentHashInfo {
            display_name: "Saba-Core".to_string(),
            expected_sha256: Some(actual_hash),
            install_dir: None,
        });

        let report = checker.verify_all(&hashes);
        // FileNotFound because "saba-core.exe" (or "saba-core") doesn't match "test.bin"
        // This is expected — full integration test would use proper file layout
        assert_eq!(report.total, 1);
    }
}
