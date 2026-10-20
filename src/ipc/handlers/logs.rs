//! 서버 로그 읽기·내보내기 (디스코드 검수 9번, 계약 06 §3): 원천은 러너가 남기는 콘솔 기록 세션 파일과, 러너 없이 직접 실행했을 때의 `console.log`·회전 파일이다
//!
//! - `GET /api/instance/:id/logs?source=&lines=&before=`: 한 세션의 끝에서부터 줄을 읽는다. `before`는 이전 응답이 준 커서다.
//! - `GET /api/instance/:id/logs/export?source=&since=&until=`: 한 세션을 파일 하나로 (`{fileName, bytes, truncated, content}`).
//!
//! 원격으로 나갈 때의 가리기는 액션 실행이 한다(`actions::strip_local_details`·`mask_console`).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path as FsPath, PathBuf};

use super::super::IPCServer;
use crate::supervisor::managed_process::{console_log_path, sessions_dir, CONSOLE_LOG_ROTATIONS};

/// 한 번에 읽는 줄 수의 기본값과 상한
const DEFAULT_LINES: usize = 200;
const MAX_LINES: usize = 500;
/// 내보내기 내용의 상한, JSON 문자열로 쓴 크기 (넘으면 최근 쪽을 남긴다)
pub const EXPORT_MAX_BYTES: usize = 4 * 1024 * 1024;
/// 끝에서 거꾸로 읽는 조각 크기
const CHUNK: u64 = 64 * 1024;

/// 원천 하나: 러너 세션 파일(id는 시작 시각 `YYYYMMDD-HHMMSS`, UTC) 또는 직접 실행 때의 `console.log`·회전 파일(`console`, `console.1`, …)
struct Session {
    id: String,
    path: PathBuf,
    started_at: Option<u64>,
}

/// 첫 줄의 시각 (직접 실행 기록은 파일 이름에 시작 시각이 없다)
fn first_line_time(path: &FsPath) -> Option<u64> {
    let mut first = String::new();
    std::io::BufReader::new(std::fs::File::open(path).ok()?).read_line(&mut first).ok()?;
    line_time(&first)
}

/// 최근에 쓴 것부터
fn sessions(instance_dir: &FsPath) -> Vec<Session> {
    let runner = std::fs::read_dir(sessions_dir(instance_dir))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .filter_map(|path| {
            let id = path.file_stem()?.to_str()?.to_string();
            let started_at = chrono::NaiveDateTime::parse_from_str(&id, "%Y%m%d-%H%M%S").ok().map(|t| t.and_utc().timestamp().max(0) as u64);
            Some(Session { id, path, started_at })
        });
    // saba-runner 없이 직접 실행하면 콘솔 기록은 `console.log`와 그 회전 파일에 남는다
    let direct = console_log_path(instance_dir);
    let direct = (0..=CONSOLE_LOG_ROTATIONS)
        .map(|i| if i == 0 { ("console".to_string(), direct.clone()) } else { (format!("console.{i}"), PathBuf::from(format!("{}.{i}", direct.display()))) })
        .filter(|(_, path)| path.is_file())
        .map(|(id, path)| Session { started_at: first_line_time(&path), id, path });
    let modified = |p: &FsPath| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut list: Vec<(Option<std::time::SystemTime>, Session)> = runner.chain(direct).map(|s| (modified(&s.path), s)).collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.id.cmp(&a.1.id)));
    list.into_iter().map(|(_, s)| s).collect()
}

fn not_found(what: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("{} not found", what), "error_code": "not_found" }))).into_response()
}

/// 인스턴스 폴더와 이름. 없는 인스턴스면 None
async fn instance_of(state: &IPCServer, id: &str) -> Option<(PathBuf, String)> {
    let sup = state.supervisor.read().await;
    let inst = sup.instance_store.get(id)?;
    Some((sup.instance_store.instance_dir(&inst.id), inst.name.clone()))
}

/// `source`(없거나 `latest`면 가장 최근)에 맞는 세션
fn pick<'a>(list: &'a [Session], source: Option<&str>) -> Option<&'a Session> {
    match source.filter(|s| !s.is_empty() && *s != "latest") {
        None => list.first(),
        Some(s) => list.iter().find(|x| x.id == s),
    }
}

/// `end`(바이트) 앞의 마지막 `n`줄과, 그 첫 줄이 시작하는 바이트(더 앞이 없으면 None)
fn lines_before(path: &FsPath, end: Option<u64>, n: usize) -> std::io::Result<(Vec<String>, Option<u64>)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let end = end.unwrap_or(len).min(len);
    // 마지막 줄바꿈은 줄의 끝이지 빈 줄의 시작이 아니다
    let mut buf: Vec<u8> = Vec::new();
    let mut pos = end;
    loop {
        let newlines = buf.iter().filter(|&&b| b == b'\n').count();
        let trailing = buf.last() == Some(&b'\n');
        if newlines > n + usize::from(trailing) || pos == 0 {
            break;
        }
        let start = pos.saturating_sub(CHUNK);
        let mut chunk = vec![0u8; (pos - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&buf);
        buf = chunk;
        pos = start;
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.is_empty() && pos == 0 {
        return Ok((Vec::new(), None));
    }
    let mut parts: Vec<&[u8]> = buf.split(|&b| b == b'\n').collect();
    // 맨 앞 조각은 파일 처음에서 시작하지 않았으면 잘린 줄이라 버린다(다음 읽기가 가져간다)
    let mut first_start = pos;
    if parts.len() > n {
        let drop = parts.len() - n;
        first_start += parts[..drop].iter().map(|p| p.len() as u64 + 1).sum::<u64>();
        parts.drain(..drop);
    } else if pos > 0 {
        first_start += parts[0].len() as u64 + 1;
        parts.remove(0);
    }
    let lines = parts.iter().map(|p| String::from_utf8_lossy(p).trim_end_matches('\r').to_string()).collect();
    Ok((lines, (first_start > 0).then_some(first_start)))
}

/// 세션 줄의 시각 (`[2026-03-05 12:34:56] [STDOUT/INFO] …`, UTC). 없으면 None
fn line_time(line: &str) -> Option<u64> {
    let stamp = line.strip_prefix('[')?.get(..19)?;
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").ok().map(|t| t.and_utc().timestamp().max(0) as u64)
}

/// 파일 이름에 못 쓰는 글자를 `_`로
fn file_safe(s: &str) -> String {
    s.chars().map(|c| if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect()
}

fn sources_json(list: &[Session]) -> Vec<Value> {
    list.iter()
        .map(|s| json!({ "id": s.id, "label": s.started_at.map(crate::actions::summary::local_time).unwrap_or_else(|| s.id.clone()), "startedAt": s.started_at }))
        .collect()
}

/// GET /api/instance/:id/logs?source=&lines=&before=
pub async fn logs_handler(Path(id): Path<String>, State(state): State<IPCServer>, Query(params): Query<HashMap<String, String>>) -> axum::response::Response {
    let Some((dir, _)) = instance_of(&state, &id).await else { return not_found("Instance") };
    let list = sessions(&dir);
    let source = params.get("source").map(String::as_str);
    let Some(session) = pick(&list, source) else {
        // 아직 한 번도 켜지 않은 서버는 원천이 없다: 고른 것이 없으면 빈 목록으로 답한다
        if source.is_none_or(|s| s.is_empty() || s == "latest") {
            return Json(json!({ "success": true, "sources": [], "source": null, "lines": [] })).into_response();
        }
        return not_found("Log source");
    };
    let n = params.get("lines").and_then(|v| v.parse::<usize>().ok()).unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    let before = params.get("before").and_then(|v| v.parse::<u64>().ok());
    let path = session.path.clone();
    let read = tokio::task::spawn_blocking(move || lines_before(&path, before, n)).await;
    match read {
        Ok(Ok((lines, cursor))) => {
            let mut body = json!({ "success": true, "sources": sources_json(&list), "source": session.id, "lines": lines });
            if let Some(c) = cursor {
                body["before"] = json!(c);
            }
            Json(body).into_response()
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": "Could not read the log", "error_code": "log_read_failed" }))).into_response(),
    }
}

/// 줄 하나(+ 줄바꿈)가 JSON 문자열 안에서 차지하는 바이트: 릴레이로 갈 때의 크기
fn json_len(line: &str) -> usize {
    serde_json::to_string(line).map_or(line.len() * 6, |s| s.len() - 2) + 2
}

/// 한 세션을 시각으로 걸러 JSON으로 쓴 크기가 최근 `EXPORT_MAX_BYTES`까지 모은다 → (내용, 넘쳐서 앞을 버렸는가)
fn export_lines(path: &FsPath, since: Option<u64>, until: Option<u64>) -> std::io::Result<(String, bool)> {
    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut kept: VecDeque<String> = VecDeque::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    let mut current: Option<u64> = None;
    for raw in reader.split(b'\n') {
        let raw = raw?;
        let line = String::from_utf8_lossy(&raw).trim_end_matches('\r').to_string();
        // 시각이 없는 줄(여러 줄 출력의 이어지는 줄)은 앞 줄을 따른다
        if let Some(t) = line_time(&line) {
            current = Some(t);
        }
        let inside = match current {
            Some(t) => since.is_none_or(|s| t >= s) && until.is_none_or(|u| t <= u),
            None => since.is_none(),
        };
        if !inside {
            continue;
        }
        bytes += json_len(&line);
        kept.push_back(line);
        while bytes > EXPORT_MAX_BYTES {
            let Some(old) = kept.pop_front() else { break };
            bytes -= json_len(&old);
            truncated = true;
        }
    }
    let mut content = Vec::from(kept).join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    Ok((content, truncated))
}

/// GET /api/instance/:id/logs/export?source=&since=&until=
pub async fn export_handler(Path(id): Path<String>, State(state): State<IPCServer>, Query(params): Query<HashMap<String, String>>) -> axum::response::Response {
    let Some((dir, name)) = instance_of(&state, &id).await else { return not_found("Instance") };
    let list = sessions(&dir);
    let Some(session) = pick(&list, params.get("source").map(String::as_str)) else { return not_found("Log source") };
    let since = params.get("since").and_then(|v| v.parse::<u64>().ok());
    let until = params.get("until").and_then(|v| v.parse::<u64>().ok());
    let path = session.path.clone();
    match tokio::task::spawn_blocking(move || export_lines(&path, since, until)).await {
        Ok(Ok((content, truncated))) => {
            let file_name = format!("{}-{}-{}.log", file_safe(&name), file_safe(&session.id), chrono::Local::now().format("%Y%m%d-%H%M"));
            Json(json!({ "success": true, "fileName": file_name, "bytes": content.len(), "truncated": truncated, "content": content })).into_response()
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": "Could not read the log", "error_code": "log_read_failed" }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    async fn server_with_sessions(files: &[(&str, &str)]) -> (IPCServer, String, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("내 서버:1", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let dir = sessions_dir(&sup.instance_store.instance_dir(&id));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(format!("{name}.log")), body).unwrap();
        }
        (IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new()), id, tmp)
    }

    async fn get(state: &IPCServer, id: &str, query: &[(&str, &str)], export: bool) -> (StatusCode, Value) {
        let params: HashMap<String, String> = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let resp = if export {
            export_handler(Path(id.to_string()), State(state.clone()), Query(params)).await
        } else {
            logs_handler(Path(id.to_string()), State(state.clone()), Query(params)).await
        };
        let r = crate::ipc::internal::into_internal(resp).await;
        (r.status, r.body)
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("[2026-10-20 11:{:02}:00] [STDOUT/INFO] line {i}\n", i % 60)).collect()
    }

    #[tokio::test]
    async fn reading_pages_back_through_a_whole_session() {
        let (state, id, _tmp) = server_with_sessions(&[("20261019-100000", "old\n"), ("20261020-110000", &numbered(1200))]).await;
        let (status, first) = get(&state, &id, &[("lines", "500")], false).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["source"], "20261020-110000", "latest by default");
        let ids: Vec<&str> = first["sources"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["20261020-110000", "20261019-100000"]);
        assert_eq!(first["sources"][0]["startedAt"], 1792494000u64);
        let lines = first["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 500);
        assert!(lines[499].as_str().unwrap().ends_with("line 1200"));

        let mut seen = 500;
        let mut cursor = first["before"].as_u64();
        let mut oldest = String::new();
        while let Some(c) = cursor {
            let (_, page) = get(&state, &id, &[("lines", "500"), ("before", &c.to_string())], false).await;
            let lines = page["lines"].as_array().unwrap();
            seen += lines.len();
            oldest = lines[0].as_str().unwrap().to_string();
            cursor = page["before"].as_u64();
        }
        assert_eq!(seen, 1200, "every line exactly once");
        assert!(oldest.ends_with("line 1"));

        let (_, capped) = get(&state, &id, &[("lines", "99999")], false).await;
        assert_eq!(capped["lines"].as_array().unwrap().len(), 500);
        let (_, old) = get(&state, &id, &[("source", "20261019-100000")], false).await;
        assert_eq!(old["lines"], json!(["old"]));
        assert!(old.get("before").is_none());
        assert_eq!(get(&state, &id, &[("source", "nope")], false).await.0, StatusCode::NOT_FOUND);
        assert_eq!(get(&state, "missing", &[], false).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_server_never_started_has_no_sources_yet() {
        let (state, id, _tmp) = server_with_sessions(&[]).await;
        let (status, body) = get(&state, &id, &[], false).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["sources"], json!([]));
        assert_eq!(get(&state, &id, &[], true).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn exporting_filters_by_time_and_keeps_the_newest_part() {
        let session = "[2026-10-20 11:00:00] [STDOUT/INFO] boot\n\
                       [2026-10-20 11:05:00] [STDERR/ERROR] Exception in thread\n\
                       \tat Foo.bar\n\
                       [2026-10-20 11:10:00] [STDOUT/INFO] done\n";
        let (state, id, _tmp) = server_with_sessions(&[("20261020-110000", session)]).await;
        let (_, all) = get(&state, &id, &[], true).await;
        assert_eq!(all["content"], session);
        assert_eq!(all["bytes"], session.len());
        assert_eq!(all["truncated"], false);
        let name = all["fileName"].as_str().unwrap();
        assert!(name.starts_with("내 서버_1-20261020-110000-") && name.ends_with(".log"), "{name}");

        // 11:05 이후 11:09까지: 시각 없는 줄은 앞 줄을 따른다
        let from = (1792494000u64 + 5 * 60).to_string();
        let to = (1792494000u64 + 9 * 60).to_string();
        let (_, part) = get(&state, &id, &[("since", &from), ("until", &to)], true).await;
        assert_eq!(part["content"], "[2026-10-20 11:05:00] [STDERR/ERROR] Exception in thread\n\tat Foo.bar\n");

        let big = format!("{}[2026-10-20 12:00:00] [STDOUT/INFO] last\n", "[2026-10-20 11:00:00] [STDOUT/INFO] xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n".repeat(EXPORT_MAX_BYTES / 64 + 10));
        let (state, id, _tmp2) = server_with_sessions(&[("20261020-110000", &big)]).await;
        let (_, cut) = get(&state, &id, &[], true).await;
        assert_eq!(cut["truncated"], true);
        let content = cut["content"].as_str().unwrap();
        assert!(content.len() <= EXPORT_MAX_BYTES && content.ends_with("last\n"));
        assert!(content.starts_with("[2026-10-20 11:00:00]"), "whole lines only");
    }

    #[tokio::test]
    async fn a_server_run_without_the_runner_has_its_console_log_as_sources() {
        let (state, id, _tmp) = server_with_sessions(&[("20261019-100000", "[2026-10-19 10:00:00] [STDOUT/INFO] runner\n")]).await;
        let logs = state.supervisor.read().await.instance_store.instance_dir(&id).join("logs");
        let day = std::time::Duration::from_secs(86400);
        let now = std::time::SystemTime::now();
        for (name, line, age) in [("console.log", "[2026-10-20 12:00:00] [STDOUT/INFO] direct", 0u32), ("console.log.1", "[2026-10-20 08:00:00] [STDOUT/INFO] rotated", 1)] {
            std::fs::write(logs.join(name), format!("{line}\n")).unwrap();
            std::fs::File::options().append(true).open(logs.join(name)).unwrap().set_modified(now - day * age).unwrap();
        }
        let session = sessions_dir(&logs.parent().unwrap().to_path_buf()).join("20261019-100000.log");
        std::fs::File::options().append(true).open(session).unwrap().set_modified(now - day * 2).unwrap();

        let (status, body) = get(&state, &id, &[], false).await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<&str> = body["sources"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["console", "console.1", "20261019-100000"], "newest written first");
        assert_eq!(body["lines"], json!(["[2026-10-20 12:00:00] [STDOUT/INFO] direct"]));
        assert_eq!(body["sources"][0]["startedAt"], 1792497600u64, "first line's time");
        let (_, rotated) = get(&state, &id, &[("source", "console.1")], true).await;
        assert_eq!(rotated["content"], "[2026-10-20 08:00:00] [STDOUT/INFO] rotated\n");
    }

    #[tokio::test]
    async fn an_export_stays_under_the_limit_once_written_as_json() {
        // 따옴표·역슬래시는 JSON에서 두 배가 된다: 내보내기 상한은 JSON 문자열로 잰다
        let quoted = format!("[2026-10-20 11:00:00] [STDOUT/INFO] {}\n", "\"\\".repeat(32));
        let (state, id, _tmp) = server_with_sessions(&[("20261020-110000", &quoted.repeat(EXPORT_MAX_BYTES / quoted.len() + 10))]).await;
        let (_, cut) = get(&state, &id, &[], true).await;
        assert_eq!(cut["truncated"], true);
        assert!(serde_json::to_string(&cut["content"]).unwrap().len() <= EXPORT_MAX_BYTES);
    }
}
