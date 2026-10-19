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
    /// "v1.2.3" 또는 "1.2.3-beta.1" 형식을 파싱. `+build`는 비교에 쓰지 않으므로 버린다
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let s = s.split_once('+').map_or(s, |(v, _)| v);
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

/// 카탈로그가 내민 `offered`가 설치된 `installed`보다 낮은가. 둘 다 읽혀야 참이다
/// (예전 서명 manifest를 다시 보내 옛 버전을 깔게 하는 것을 막는다)
pub fn is_downgrade(installed: &str, offered: &str) -> bool {
    matches!((SemVer::parse(installed), SemVer::parse(offered)), (Some(i), Some(o)) if i > o)
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
            (Some(a), Some(b)) => compare_prerelease(a, b),
        }
    }
}

/// 점으로 나눈 식별자별 비교: 숫자끼리는 값으로, 숫자는 문자보다 낮고, 앞이 같으면 짧은 쪽이 낮다 (SemVer 11절)
fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let mut a_ids = a.split('.');
    let mut b_ids = b.split('.');
    loop {
        let ord = match (a_ids.next(), b_ids.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                // 값이 같으면 글자로 가른다 ("01"과 "1"이 같다고 보지 않는다: Eq와 맞춘다)
                (Ok(m), Ok(n)) => m.cmp(&n).then_with(|| x.cmp(y)),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => x.cmp(y),
            },
        };
        if ord != Ordering::Equal {
            return ord;
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
    fn older_catalog_versions_are_downgrades() {
        assert!(is_downgrade("1.2.0", "1.1.9"));
        assert!(!is_downgrade("1.2.0", "1.2.0"));
        assert!(!is_downgrade("1.2.0", "1.3.0"));
        // 읽히지 않는 버전은 막지 않는다
        assert!(!is_downgrade("dev", "1.0.0"));
    }

    // L13: 프리릴리스는 점으로 나눈 식별자별로, 숫자는 숫자로 비교한다 (SemVer 11절)
    #[test]
    fn prerelease_identifiers_compare_numerically() {
        let v = |s| SemVer::parse(s).unwrap();
        assert!(v("1.0.0-beta.10") > v("1.0.0-beta.9"));
        assert!(v("1.0.0-beta.2") > v("1.0.0-beta"));
        assert!(v("1.0.0-beta") > v("1.0.0-alpha.5"));
        assert!(v("1.0.0-alpha.beta") > v("1.0.0-alpha.1"), "numeric identifiers rank below text");
    }

    // L13: `+build`는 버전 비교에 쓰지 않는다 (예전에는 patch를 못 읽어 0이 됐다)
    #[test]
    fn build_metadata_is_ignored() {
        let v = SemVer::parse("1.2.3+build.7").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
        assert_eq!(v, SemVer::parse("1.2.3").unwrap());
        let pre = SemVer::parse("1.2.3-rc.1+sha.abc").unwrap();
        assert_eq!(pre.prerelease.as_deref(), Some("rc.1"));
    }

    #[test]
    fn prerelease_less_than_release() {
        let pre = SemVer::parse("1.0.0-beta.1").unwrap();
        let rel = SemVer::parse("1.0.0").unwrap();
        assert!(rel.is_newer_than(&pre));
    }
}
