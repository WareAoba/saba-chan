//! 시맨틱 버전 비교 유틸리티 (외부 크레이트 없이)

use std::cmp::Ordering;
use std::fmt;

/// 시맨틱 버전 (major.minor.patch[-prerelease])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl SemVer {
    /// "v1.2.3" 또는 "1.2.3-beta.1" 형식을 파싱
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let (version_part, prerelease) = if let Some(idx) = s.find('-') {
            (&s[..idx], Some(s[idx + 1..].to_string()))
        } else {
            (s, None)
        };

        let parts: Vec<&str> = version_part.split('.').collect();
        if parts.len() < 2 {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);

        Some(Self { major, minor, patch, prerelease })
    }

    /// 현재 버전보다 새로운 버전인지 확인
    pub fn is_newer_than(&self, other: &SemVer) -> bool {
        self > other
    }

    /// 프리릴리스 여부
    pub fn is_prerelease(&self) -> bool {
        self.prerelease.is_some()
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(ref pre) = self.prerelease {
            write!(f, "-{}", pre)?;
        }
        Ok(())
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            ord => return ord,
        }
        // 프리릴리스가 있으면 정식 릴리스보다 낮음
        match (&self.prerelease, &other.prerelease) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,    // 정식 > 프리릴리스
            (Some(_), None) => Ordering::Less,       // 프리릴리스 < 정식
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert!(v.prerelease.is_none());
    }

    #[test]
    fn parse_with_v_prefix() {
        let v = SemVer::parse("v0.1.0").unwrap();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 1);
    }

    #[test]
    fn parse_prerelease() {
        let v = SemVer::parse("1.0.0-beta.1").unwrap();
        assert!(v.is_prerelease());
        assert_eq!(v.prerelease, Some("beta.1".to_string()));
    }

    #[test]
    fn compare_versions() {
        let v1 = SemVer::parse("1.0.0").unwrap();
        let v2 = SemVer::parse("1.0.1").unwrap();
        assert!(v2.is_newer_than(&v1));

        let v3 = SemVer::parse("2.0.0").unwrap();
        assert!(v3.is_newer_than(&v2));
    }

    #[test]
    fn prerelease_less_than_release() {
        let pre = SemVer::parse("1.0.0-beta.1").unwrap();
        let rel = SemVer::parse("1.0.0").unwrap();
        assert!(rel.is_newer_than(&pre));
    }
}
