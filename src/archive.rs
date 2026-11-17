//! 내려받은 ZIP(모듈·익스텐션 패키지)을 안전하게 푼다.
//! 항목 수와 풀린 크기에 상한을 두고, 폴더 밖을 가리키는 경로와 링크는 거부하며, 권한 비트는 옮기지 않는다

use anyhow::Result;
use std::io::{Read, Seek};
use std::path::Path;

/// 압축 파일 하나의 상한 (`what`은 오류 문구의 주어다: "Module archive", "Extension package").
/// 상한 검사·경로 규칙·복사는 업데이터와 같은 것(updater-lib `check_zip`·`zip_entry_path`·`copy_zip_entry`)을 쓴다
pub use saba_chan_updater_lib::{check_zip as check, ZipLimits as Limits};

/// `dest`(새로 만든 빈 준비 폴더) 아래에 푼다. 보통 이름 조각만 받고(`..`·절대 경로·드라이브 거부), 링크 항목은 거부하며,
/// 실제로 풀린 바이트로 크기 상한을 센다. 파일은 기본 권한으로 만든다 (SUID 같은 비트를 옮기지 않는다)
pub fn extract<R: Read + Seek>(mut archive: zip::ZipArchive<R>, dest: &Path, limits: &Limits) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let output = dest.join(saba_chan_updater_lib::zip_entry_path(&entry, limits)?);
        if entry.is_dir() {
            std::fs::create_dir_all(output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        saba_chan_updater_lib::copy_zip_entry(&mut entry, &mut std::fs::File::create(output)?, &mut expanded, limits)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const LIMITS: Limits = Limits { what: "Test archive", entries: 100, expanded: 1024 };

    fn archive(build: impl FnOnce(&mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>)) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        build(&mut writer);
        zip::ZipArchive::new(std::io::Cursor::new(writer.finish().unwrap().into_inner())).unwrap()
    }

    #[test]
    fn files_and_folders_come_out_under_the_destination() {
        let zip = archive(|w| {
            w.add_directory("mod/", SimpleFileOptions::default()).unwrap();
            w.start_file("mod/module.toml", SimpleFileOptions::default()).unwrap();
            w.write_all(b"name = \"x\"").unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        extract(zip, dest.path(), &LIMITS).unwrap();
        assert_eq!(std::fs::read_to_string(dest.path().join("mod/module.toml")).unwrap(), "name = \"x\"");
    }

    #[test]
    fn a_link_entry_is_refused() {
        let zip = archive(|w| {
            w.add_symlink("mod/escape", "/etc/passwd", SimpleFileOptions::default()).unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        let err = extract(zip, dest.path(), &LIMITS).unwrap_err();
        assert_eq!(err.to_string(), "Test archive contains a link");
        assert!(!dest.path().join("mod/escape").exists());
    }

    #[test]
    fn a_path_outside_the_destination_is_refused() {
        let zip = archive(|w| {
            w.start_file("mod/../../escape.txt", SimpleFileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        assert_eq!(extract(zip, dest.path(), &LIMITS).unwrap_err().to_string(), "Test archive contains an invalid path");
    }

    #[test]
    fn a_name_with_a_colon_is_a_stream_on_windows() {
        let zip = archive(|w| {
            // zip 9.2부터 `a:b`처럼 한 글자 뒤 콜론은 드라이브로 보아 모든 OS에서 거부한다: 스트림 이름으로 시험한다
            w.start_file("mod/level.dat:hidden", SimpleFileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        assert_eq!(extract(zip, dest.path(), &LIMITS).is_err(), cfg!(windows));
    }

    #[test]
    fn the_size_limit_counts_the_bytes_that_come_out() {
        let zip = archive(|w| {
            w.start_file("big.bin", SimpleFileOptions::default()).unwrap();
            w.write_all(&[0u8; 2048]).unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        assert_eq!(extract(zip, dest.path(), &LIMITS).unwrap_err().to_string(), "Test archive expands beyond the size limit");
    }

    #[cfg(unix)]
    #[test]
    fn permission_bits_from_the_archive_are_not_applied() {
        use std::os::unix::fs::PermissionsExt;
        let zip = archive(|w| {
            w.start_file("run.sh", SimpleFileOptions::default().unix_permissions(0o4777)).unwrap();
            w.write_all(b"#!/bin/sh").unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        extract(zip, dest.path(), &LIMITS).unwrap();
        let mode = std::fs::metadata(dest.path().join("run.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o4000, 0, "setuid bit must not be copied");
    }

    #[test]
    fn the_header_check_refuses_too_many_entries() {
        let mut zip = archive(|w| {
            for i in 0..3 {
                w.start_file(format!("{i}.txt"), SimpleFileOptions::default()).unwrap();
            }
        });
        let few = Limits { what: "Test archive", entries: 2, expanded: 1024 };
        assert_eq!(check(&mut zip, &few).unwrap_err().to_string(), "Test archive has too many files");
        assert!(check(&mut zip, &LIMITS).is_ok());
    }
}
