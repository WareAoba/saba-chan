//! 빌드에 묶인 버전표 — 저장소 루트 `versions.toml`을 컴파일 시 그대로 넣는다.
//!
//! 설치된 버전의 기준은 `installed-manifest.json`이고, 이 표는 거기에 없는
//! 컴포넌트의 폴백이다. 같은 릴리즈로 빌드된 컴포넌트는 모두 이 표와 버전이 같다.

use std::collections::BTreeMap;
use std::sync::OnceLock;

const SOURCE: &str = include_str!("../../versions.toml");

#[derive(Debug, Clone)]
pub struct BundledVersions {
    /// 묶음 릴리즈 번호 (`[release] version`)
    pub release: String,
    /// 기본 업데이트 채널 (`[release] channel`)
    pub channel: String,
    /// 컴포넌트 키 → 버전
    pub components: BTreeMap<String, String>,
    /// 컴포넌트 키 → 드는 설치 구성 (`profiles`. 표시가 있는 것만, `crate::profile`)
    pub profiles: BTreeMap<String, Vec<String>>,
}

/// `versions.toml`을 파싱한 결과. 형식은 `scripts/versions.py check`가 CI에서 보장한다.
pub fn bundled() -> &'static BundledVersions {
    static CELL: OnceLock<BundledVersions> = OnceLock::new();
    CELL.get_or_init(|| parse(SOURCE).expect("versions.toml is validated in CI"))
}

fn parse(source: &str) -> Result<BundledVersions, toml::de::Error> {
    #[derive(serde::Deserialize)]
    struct File {
        release: Release,
        #[serde(default)]
        components: BTreeMap<String, Component>,
    }
    #[derive(serde::Deserialize)]
    struct Release {
        version: String,
        #[serde(default = "default_channel")]
        channel: String,
    }
    #[derive(serde::Deserialize)]
    struct Component {
        version: String,
        #[serde(default)]
        profiles: Option<Vec<String>>,
    }
    fn default_channel() -> String {
        crate::relay::DEFAULT_CHANNEL.to_string()
    }

    let file: File = toml::from_str(source)?;
    Ok(BundledVersions {
        release: file.release.version,
        channel: file.release.channel,
        profiles: file.components.iter().filter_map(|(k, c)| Some((k.clone(), c.profiles.clone()?))).collect(),
        components: file.components.into_iter().map(|(k, c)| (k, c.version)).collect(),
    })
}

/// 컴포넌트의 빌드 버전. 표에 없으면 `None`.
pub fn component(key: &str) -> Option<&'static str> {
    bundled().components.get(key).map(String::as_str)
}

/// 컴포넌트가 드는 설치 구성. 표시가 없으면 `None`(모든 구성)
pub fn profiles(key: &str) -> Option<&'static [String]> {
    bundled().profiles.get(key).map(Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_table_parses_and_has_core_components() {
        let v = bundled();
        assert!(crate::version::SemVer::parse(&v.release).is_some());
        for key in ["saba-core", "cli", "gui", "installer"] {
            assert!(component(key).is_some(), "{} missing from versions.toml", key);
        }
    }

    #[test]
    // 이 라이브러리는 설치기(업데이트 적용)와 같이 버전을 올린다
    fn this_crate_matches_bundled_installer_version() {
        assert_eq!(component("installer"), Some(env!("CARGO_PKG_VERSION")));
    }
}
