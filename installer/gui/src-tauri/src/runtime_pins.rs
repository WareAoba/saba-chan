//! 포터블 런타임 고정 버전과 SHA-256 체크섬
//!
//! 값은 upstream 공식 체크섬 파일에서 한 번 추출해 고정했다.
//! - Python: `python-build-standalone/releases/download/{PYTHON_RELEASE_TAG}/SHA256SUMS`
//! - Node.js: `nodejs.org/dist/v{NODE_VERSION}/SHASUMS256.txt`
//!
//! 버전을 올릴 때는 위 파일에서 5개 타겟의 해시를 다시 옮겨 적는다.

pub const PYTHON_VERSION: &str = "3.12.8";
pub const PYTHON_RELEASE_TAG: &str = "20250106";
pub const NODE_VERSION: &str = "22.14.0";

/// 타겟별 다운로드 이름 조각과 체크섬
#[derive(Debug, Clone, Copy)]
pub struct RuntimePin {
    /// `std::env::consts::OS`
    pub os: &'static str,
    /// `std::env::consts::ARCH`
    pub arch: &'static str,
    pub python_triple: &'static str,
    pub python_sha256: &'static str,
    pub node_os: &'static str,
    pub node_arch: &'static str,
    pub node_ext: &'static str,
    pub node_sha256: &'static str,
}

pub const RUNTIME_PINS: &[RuntimePin] = &[
    RuntimePin {
        os: "windows",
        arch: "x86_64",
        python_triple: "x86_64-pc-windows-msvc",
        python_sha256: "2c6af885033be26a8a2ac2e069aac4889c53ab9d212fdffe0d7c8157b66a3d14",
        node_os: "win",
        node_arch: "x64",
        node_ext: "zip",
        node_sha256: "55b639295920b219bb2acbcfa00f90393a2789095b7323f79475c9f34795f217",
    },
    RuntimePin {
        os: "linux",
        arch: "x86_64",
        python_triple: "x86_64-unknown-linux-gnu",
        python_sha256: "5591da823631467e204baa520ee379ba09b5699b2e2c322c456c44cb2c2e60eb",
        node_os: "linux",
        node_arch: "x64",
        node_ext: "tar.gz",
        node_sha256: "9d942932535988091034dc94cc5f42b6dc8784d6366df3a36c4c9ccb3996f0c2",
    },
    RuntimePin {
        os: "linux",
        arch: "aarch64",
        python_triple: "aarch64-unknown-linux-gnu",
        python_sha256: "29df00dc1435f536c50396d1753fb507f81a81c93a04a7c24b145c9ad68798f8",
        node_os: "linux",
        node_arch: "arm64",
        node_ext: "tar.gz",
        node_sha256: "8cf30ff7250f9463b53c18f89c6c606dfda70378215b2c905d0a9a8b08bd45e0",
    },
    RuntimePin {
        os: "macos",
        arch: "x86_64",
        python_triple: "x86_64-apple-darwin",
        python_sha256: "00e2d29ce8a688f5d2f192718e6f87e4aedae88ac2a0958011b8af9a49e049a7",
        node_os: "darwin",
        node_arch: "x64",
        node_ext: "tar.gz",
        node_sha256: "6698587713ab565a94a360e091df9f6d91c8fadda6d00f0cf6526e9b40bed250",
    },
    RuntimePin {
        os: "macos",
        arch: "aarch64",
        python_triple: "aarch64-apple-darwin",
        python_sha256: "3a31b4f82d589a0ff6efac3cdc1e55284cb67645470b78c5aa50f156c10a93d2",
        node_os: "darwin",
        node_arch: "arm64",
        node_ext: "tar.gz",
        node_sha256: "e9404633bc02a5162c5c573b1e2490f5fb44648345d64a958b17e325729a5e42",
    },
];

impl RuntimePin {
    /// 현재 빌드 타겟의 고정값. 미지원 플랫폼은 None.
    pub fn current() -> Option<&'static RuntimePin> {
        RUNTIME_PINS
            .iter()
            .find(|p| p.os == std::env::consts::OS && p.arch == std::env::consts::ARCH)
    }

    pub fn python_url(&self) -> String {
        format!(
            "https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz",
            tag = PYTHON_RELEASE_TAG,
            ver = PYTHON_VERSION,
            triple = self.python_triple,
        )
    }

    pub fn node_url(&self) -> String {
        format!(
            "https://nodejs.org/dist/v{ver}/node-v{ver}-{os}-{arch}.{ext}",
            ver = NODE_VERSION,
            os = self.node_os,
            arch = self.node_arch,
            ext = self.node_ext,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_sha256_hex(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
    }

    #[test]
    fn every_pin_has_valid_checksums() {
        assert_eq!(RUNTIME_PINS.len(), 5);
        for pin in RUNTIME_PINS {
            assert!(is_sha256_hex(pin.python_sha256), "{:?}", pin);
            assert!(is_sha256_hex(pin.node_sha256), "{:?}", pin);
        }
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    fn windows_x64_pin_builds_expected_urls() {
        let pin = RuntimePin::current().expect("windows x64 pin");
        assert!(pin.python_url().ends_with(
            "/20250106/cpython-3.12.8+20250106-x86_64-pc-windows-msvc-install_only_stripped.tar.gz"
        ));
        assert_eq!(
            pin.node_url(),
            "https://nodejs.org/dist/v22.14.0/node-v22.14.0-win-x64.zip"
        );
    }
}
