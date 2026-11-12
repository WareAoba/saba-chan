//! 오래 사는 작업자 (module.toml `[runtime] persistent = true`, 익스텐션 manifest `runtime.persistent`).
//!
//! 구성요소(진입 파일)마다 Python 하나를 `python <진입 파일> --worker`로 띄워 두고, 호출마다 요청 한 줄
//! `{"function", "config"}`을 보내 답 JSON 한 줄을 받는다. 함수 이름·config·답은 단발 실행과 같다(SDK `run_lifecycle`).
//! 띄우는 준비(환경·작업 폴더·격리)는 단발 실행의 `Launch`를 그대로 쓰고, 준비가 바뀌면(업데이트·설정·언어) 새로 띄운다.
//! 작업자가 뜨지 못하거나 답하기 전에 끝나면 그 호출은 단발로 돌린다. 다른 호출을 하는 중이면 기다리지 않고 단발로 돌린다

use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

/// SDK `daemon_rcon.WORKER_ARG`·`WORKER_READY`·`WORKER_DONE`과 같은 값
pub(super) const ARG: &str = "--worker";
const READY: &str = r#"{"saba_worker": 1}"#;
/// 답을 쓰기 직전에 stderr에 쓰는 줄: 그 호출의 stderr를 다 받았다는 표시
const DONE: &str = "WORKER:done";
/// 띄운 작업자가 READY를 쓰기까지 기다리는 한도. 넘기면 이 준비로는 띄우지 않고 단발로 돌린다
const START_SECS: u64 = 20;

/// 작업자 하나. 놓으면 프로세스 트리를 끝낸다 (Unix 프로세스 그룹, Windows Job·AppContainer)
struct Worker {
    fingerprint: String,
    process: Process,
    pid: Option<u32>,
    requests: Writer,
    replies: BufReader<Reader>,
    /// 지금 호출의 stderr를 받는 곳. 호출 사이에 쓴 줄은 debug 로그로만 남긴다
    current: Arc<Mutex<Option<StderrSink>>>,
    /// stderr에서 본 `DONE` 수. stderr가 닫히면 보내는 쪽이 사라진다
    done: tokio::sync::watch::Receiver<u64>,
    stderr_task: tokio::task::AbortHandle,
    unverified_extensions: Vec<String>,
    loopback_dir: Option<std::path::PathBuf>,
    /// 작업자의 `SABA_HOST_DIR` (호출마다 만들지 않고 작업자와 함께 산다)
    host_dir: tempfile::TempDir,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.process.kill_now(self.pid);
        self.stderr_task.abort();
    }
}

/// 진입 파일 하나의 작업자 자리. 호출 하나가 쥐는 동안 다른 호출은 단발로 돈다
#[derive(Default)]
struct Slot(tokio::sync::Mutex<Option<Worker>>);

static WORKERS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, Arc<Slot>>>> = std::sync::LazyLock::new(Default::default);
/// 작업자로 띄우지 못한 진입 파일 → 그때의 준비. 준비가 바뀔 때까지 단발로만 돌리고 다시 알리지 않는다
static UNUSABLE: std::sync::LazyLock<Mutex<std::collections::HashMap<String, String>>> = std::sync::LazyLock::new(Default::default);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Windows 경로는 대소문자를 가리지 않는다
fn is_under(entry: &str, dir: &std::path::Path) -> bool {
    if cfg!(windows) {
        let dir = dir.to_string_lossy().to_lowercase();
        std::path::Path::new(&entry.to_lowercase()).starts_with(dir)
    } else {
        std::path::Path::new(entry).starts_with(dir)
    }
}

/// `dir` 아래(None이면 모두)의 작업자를 끝낸다. 구성요소를 바꾸거나 지우거나 옮기기 전과 데몬이 끝날 때 부른다.
/// 호출 중인 작업자는 그 호출이 끝나면 끝난다
pub fn stop_workers(dir: Option<&std::path::Path>) {
    let under = |entry: &str| dir.is_none_or(|dir| is_under(entry, dir));
    let stopped: Vec<Arc<Slot>> = {
        let mut workers = lock(&WORKERS);
        let entries: Vec<String> = workers.keys().filter(|entry| under(entry)).cloned().collect();
        entries.iter().filter_map(|entry| workers.remove(entry)).collect()
    };
    lock(&UNUSABLE).retain(|entry, _| !under(entry));
    for slot in stopped {
        if let Ok(mut held) = slot.0.try_lock() {
            if held.take().is_some() {
                tracing::debug!("Plugin worker stopped");
            }
        }
    }
}

pub(super) struct Request<'a> {
    pub function: &'a str,
    pub config: &'a Value,
    pub server: Option<u32>,
    pub timeout_secs: u64,
    /// 구성요소의 module.toml·manifest.json (바뀌면 새로 띄운다)
    pub declaration: &'a std::path::Path,
}

// 호출마다 하나라 크기는 상관없다
#[allow(clippy::large_enum_variant)]
pub(super) enum Outcome {
    Done(Result<Value>),
    /// 작업자로 부르지 못했다: 단발로 돌린다. 진행 콜백과, 띄우는 데 쓰지 않았으면 그 준비를 돌려준다
    OneShot(Option<Progress>, Option<Launch>),
}

#[allow(clippy::large_enum_variant)]
enum Start {
    Ready(Worker),
    /// 격리할 수 없어 막혔다: 단발로 돌려도 같은 답이다
    Blocked(Value),
    /// 이 준비로는 작업자가 되지 못한다 (SDK 진입점이 아님 등)
    Unusable(String),
    /// 뜨는 사이에 호출 기한이 지났다
    TimedOut,
}

/// 호출 하나를 작업자로 부른다
pub(super) async fn call(mut launch: Launch, request: Request<'_>, deadline: &CallDeadline, on_progress: Option<Progress>) -> Outcome {
    let entry = launch.entry.clone();
    let fingerprint = launch.fingerprint(request.declaration);
    if lock(&UNUSABLE).get(&entry) == Some(&fingerprint) {
        return Outcome::OneShot(on_progress, Some(launch));
    }
    let slot = lock(&WORKERS).entry(entry.clone()).or_default().clone();
    let Ok(held) = slot.0.try_lock() else { return Outcome::OneShot(on_progress, Some(launch)) };
    let mut busy = Busy { held, finished: false };
    if busy.held.as_ref().is_some_and(|worker| worker.fingerprint != fingerprint) {
        tracing::info!("Restarting the worker of {}: its files or settings changed", module_name(&entry));
        busy.held.take();
    }
    // 격리 실행이 인스턴스 폴더의 데몬 항목 자리에 만든 것은 단발 실행처럼 호출마다 치운다
    let isolation = launch.isolation.clone();
    let _clear_entries = isolation.as_ref().map(crate::sandbox::Isolation::clear_daemon_entries_on_drop);
    let started = busy.held.is_none();
    if started {
        match start(&mut launch, fingerprint.clone(), request.function, deadline).await {
            Start::Ready(worker) => *busy.held = Some(worker),
            Start::Blocked(reply) => {
                busy.finished = true;
                return Outcome::Done(Ok(reply));
            }
            Start::Unusable(reason) => {
                tracing::warn!("{} cannot run as a persistent worker ({}); calling it once per request", module_name(&entry), reason);
                lock(&UNUSABLE).insert(entry, fingerprint);
                busy.finished = true;
                return Outcome::OneShot(on_progress, None);
            }
            Start::TimedOut => {
                busy.finished = true;
                return Outcome::Done(Err(timed_out(&entry, request.function, request.timeout_secs, Vec::new())));
            }
        }
    }
    let worker = busy.held.as_mut().expect("worker started");

    let log = Arc::new(Mutex::new(StderrLog::default()));
    let host = host::Proxy {
        dir: worker.host_dir.path().into(),
        context: HOST_CONTEXT.try_with(Clone::clone).ok().flatten(),
        server: request.server,
        sandboxed: launch.sandboxed.clone(),
    };
    let loopback = worker.loopback_dir.clone().map(|dir| loopback::Proxy { dir, ports: loopback::instance_ports(request.config), server: request.server });
    *lock(&worker.current) = Some(StderrSink::new(on_progress, loopback, Some(host), CALL_DEADLINE.try_with(Clone::clone).ok(), log.clone()));
    let line = serde_json::json!({ "function": request.function, "config": request.config }).to_string() + "\n";
    let before = *worker.done.borrow();
    let exchange = async {
        worker.requests.write_all(line.as_bytes()).await?;
        worker.requests.flush().await?;
        let reply = read_line(&mut worker.replies).await?;
        // 답 앞에 쓴 stderr(예외의 traceback 등)까지 이 호출의 것으로 받는다. stderr가 닫혔으면 기다리지 않는다
        let stderr_open = worker.done.wait_for(|done| *done > before).await.is_ok();
        Ok::<_, std::io::Error>((reply, stderr_open))
    };
    let result = deadline.timeout(exchange).await;
    let sink = lock(&worker.current).take();
    let (stderr, stderr_tail) = {
        let log = lock(&log);
        (log.text(), log.tail())
    };
    let outcome = match result {
        Ok(Ok((reply, stderr_open))) => {
            let reply = plugin_reply(&entry, request.function, &reply, &stderr, &worker.unverified_extensions);
            if !stderr_open {
                busy.held.take();
            }
            Outcome::Done(reply)
        }
        Ok(Err(error)) if error.get_ref().is_some_and(|error| error.is::<OutputTooLarge>()) => {
            busy.held.take();
            Outcome::Done(Err(call_error(&entry, request.function, "module_output_too_large", "run", &error, stderr_tail)))
        }
        Ok(Err(error)) => {
            tracing::warn!("The worker of {} stopped before answering {} ({}); calling it once", module_name(&entry), request.function, error);
            busy.held.take();
            Outcome::OneShot(sink.and_then(|sink| sink.on_progress), (!started).then_some(launch))
        }
        Err(()) => {
            busy.held.take();
            Outcome::Done(Err(timed_out(&entry, request.function, request.timeout_secs, stderr_tail)))
        }
    };
    // 호출하는 사이에 끝내라는 요청(`stop_workers`)을 받았으면 지금 끝낸다
    if !lock(&WORKERS).get(&entry).is_some_and(|current| Arc::ptr_eq(current, &slot)) {
        busy.held.take();
    }
    busy.finished = true;
    outcome
}

/// 작업자를 쥔 호출. 끝내지 못하고 놓이면(호출이 버려지면) 주고받던 줄이 어긋났을 수 있어 작업자를 끝낸다
struct Busy<'a> {
    held: tokio::sync::MutexGuard<'a, Option<Worker>>,
    finished: bool,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.held.take();
        }
    }
}

async fn start(launch: &mut Launch, fingerprint: String, function: &str, deadline: &CallDeadline) -> Start {
    let host_dir = match std::fs::create_dir_all(&launch.host_root).and_then(|()| tempfile::Builder::new().prefix("host-").tempdir_in(&launch.host_root)) {
        Ok(dir) => dir,
        Err(error) => return Start::Unusable(error.to_string()),
    };
    let mut process = match launch.spawn(function, true, host_dir.path(), deadline).await {
        Ok(Ok(process)) => process,
        Ok(Err(blocked)) => return Start::Blocked(blocked),
        Err(error) => return Start::Unusable(error.to_string()),
    };
    let pid = process.pid();
    let (Some(requests), Some(replies), Some(stderr)) = process.pipes() else {
        process.kill_now(pid);
        return Start::Unusable("no pipes".to_string());
    };
    let current: Arc<Mutex<Option<StderrSink>>> = Arc::default();
    let (done_tx, done) = tokio::sync::watch::channel(0);
    let stderr_task = tokio::spawn({
        let current = current.clone();
        async move {
            let mut reader = BufReader::new(stderr);
            let mut lines = saba_runner::Lines::default();
            while let Ok(Some((line, ended))) = next_stderr_line(&mut reader, &mut lines).await {
                if line == DONE {
                    done_tx.send_modify(|done| *done += 1);
                    continue;
                }
                match lock(&current).as_mut() {
                    Some(sink) => sink.line(line, ended),
                    None => tracing::debug!("Plugin worker stderr: {}", line),
                }
            }
        }
    })
    .abort_handle();
    let mut worker = Worker {
        fingerprint,
        process,
        pid,
        requests,
        replies: BufReader::new(replies),
        current,
        done,
        stderr_task,
        unverified_extensions: launch.unverified_extensions.clone(),
        loopback_dir: launch.loopback_dir.clone(),
        host_dir,
    };
    let greeting = tokio::time::timeout(std::time::Duration::from_secs(START_SECS), read_line(&mut worker.replies));
    match deadline.timeout(greeting).await {
        Ok(Ok(Ok(line))) if line == READY => Start::Ready(worker),
        Ok(Ok(Ok(_))) => Start::Unusable("its entry point is not the SDK run_lifecycle".to_string()),
        Ok(Ok(Err(error))) => Start::Unusable(error.to_string()),
        Ok(Err(_)) => Start::Unusable(format!("it did not start in {} s", START_SECS)),
        Err(()) => Start::TimedOut,
    }
}

/// 답 한 줄 (stdout 상한 안에서)
async fn read_line(replies: &mut BufReader<Reader>) -> std::io::Result<String> {
    let mut line = Vec::new();
    (&mut *replies).take(STDOUT_LIMIT as u64 + 1).read_until(b'\n', &mut line).await?;
    if line.last() != Some(&b'\n') {
        return Err(if line.len() > STDOUT_LIMIT {
            std::io::Error::other(OutputTooLarge)
        } else {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the worker closed its output")
        });
    }
    Ok(String::from_utf8_lossy(&line).trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 모듈 폴더 아래에 `[runtime] persistent = true`인 모듈을 만든다 (격리 없이 돌리기로 동의해 둔다)
    fn persistent_module(code: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::Builder::new().prefix("worker-").tempdir_in(resolve_modules_dir()).unwrap();
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            dir.path().join("module.toml"),
            format!("[module]\nname = \"{name}\"\nversion = \"0.0.1\"\nentry = \"lifecycle.py\"\n\n[runtime]\npersistent = true\n"),
        )
        .unwrap();
        let entry = dir.path().join("lifecycle.py");
        std::fs::write(&entry, code).unwrap();
        crate::sandbox::consent_test_component(&entry);
        (dir, entry.to_string_lossy().into_owned())
    }

    const SDK_MODULE: &str = "import os, sys\nfrom daemon_rcon import run_lifecycle\n\
        def pid(config):\n    return {'success': True, 'pid': os.getpid(), 'argv': sys.argv[1]}\n\
        def boom(config):\n    raise RuntimeError('worker boom')\n\
        if __name__ == '__main__':\n    run_lifecycle({'pid': pid, 'boom': boom})\n";

    async fn call(entry: &str, function: &str) -> Value {
        run_plugin(entry, function, serde_json::json!({})).await.unwrap_or_else(|e| error_response(&e))
    }

    /// 함수 예외의 traceback(답 앞에 쓴 stderr)이 그 호출의 `stderr_tail`에 실리고, 작업자는 다음 호출에도 남는다
    #[tokio::test]
    async fn a_worker_keeps_serving_after_a_function_raises_and_reports_its_traceback() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let first = call(&entry, "pid").await;
        assert_eq!(first["argv"], ARG, "{first}");
        let failed = call(&entry, "boom").await;
        assert_eq!(failed["error_code"], "module_exception", "{failed}");
        let tail = failed["stderr_tail"].as_array().unwrap();
        assert!(tail.iter().any(|line| line.as_str().is_some_and(|l| l.contains("RuntimeError: worker boom"))), "{failed}");
        assert_eq!(call(&entry, "pid").await["pid"], first["pid"]);
        stop_workers(Some(dir.path()));
    }

    /// 진입점이 SDK가 아니면 작업자가 되지 못한다: 단발로 답한다
    #[tokio::test]
    async fn a_module_without_the_sdk_entry_point_runs_one_shot() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module("import json, sys\nprint(json.dumps({'success': True, 'argv': sys.argv[1]}))\n");
        assert_eq!(call(&entry, "probe").await["argv"], "probe");
        assert_eq!(call(&entry, "probe").await["argv"], "probe");
        stop_workers(Some(dir.path()));
    }

    /// 진입 파일이 바뀌면(업데이트) 작업자를 새로 띄운다
    #[tokio::test]
    async fn a_changed_module_gets_a_new_worker() {
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let before = call(&entry, "pid").await;
        assert_eq!(before["argv"], ARG, "{before}");
        std::fs::write(&entry, format!("{SDK_MODULE}# changed\n")).unwrap();
        // 수정 시각 해상도가 거친 파일 시스템에서도 바뀐 것으로 보이게
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&entry).unwrap().set_modified(later).unwrap();
        crate::sandbox::consent_test_component(std::path::Path::new(&entry));
        let after = call(&entry, "pid").await;
        assert_eq!(after["argv"], ARG, "{after}");
        assert_ne!(after["pid"], before["pid"]);
        stop_workers(Some(dir.path()));
    }
}
