//! 오래 사는 작업자 (module.toml `[runtime] persistent = true`, 익스텐션 manifest `runtime.persistent`).
//!
//! 구성요소(진입 파일)마다 Python 하나를 `python <진입 파일> --worker`로 띄워 두고, 호출마다 요청 한 줄
//! `{"function", "config"}`을 보내 답 JSON 한 줄을 받는다. 함수 이름·config·답은 단발 실행과 같다(SDK `run_lifecycle`).
//! 띄우는 준비(환경·작업 폴더)는 단발 실행의 `Launch`를 그대로 쓰고, 준비가 바뀌면(업데이트·설정·언어) 새로 띄운다.
//! 격리해서 돌리는 구성요소(확인되지 않은 것)는 작업자로 두지 않는다: 격리가 그 호출에 준 권한은 호출이 끝나면 거둬야 한다.
//! 작업자가 뜨지 못하거나 답하기 전에 끝나면 그 호출은 단발로 돌린다. 다른 호출을 하는 중이면 기다리지 않고 단발로 돌린다

use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

/// SDK `daemon_rcon.WORKER_ARG`·`WORKER_READY`·`WORKER_DONE`과 같은 값
pub(super) const ARG: &str = "--worker";
const READY: &str = r#"{"saba_worker": 1}"#;
/// 답을 쓰기 직전에 stderr에 쓰는 줄: 그 호출의 stderr를 다 받았다는 표시. 앞 출력에 붙어 줄 끝에 올 수 있다
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
    /// 이 준비로는 작업자가 되지 못한다 (SDK 진입점이 아님 등)
    Unusable(String),
    /// 뜨는 사이에 호출 기한이 지났다 (이 준비로는 작업자로 띄우지 않는다)
    TimedOut,
}

/// 호출 하나를 작업자로 부른다
pub(super) async fn call(mut launch: Launch, request: Request<'_>, deadline: &CallDeadline, on_progress: Option<Progress>) -> Outcome {
    let entry = launch.entry.clone();
    // 확인되지 않은 것이 된 구성요소(손으로 고친 것 등)의 작업자도 끝낸다
    if launch.isolation.is_some() {
        stop_workers(Some(std::path::Path::new(&entry)));
        return Outcome::OneShot(on_progress, Some(launch));
    }
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
    let started = busy.held.is_none();
    if started {
        match start(&mut launch, fingerprint.clone(), request.function, deadline).await {
            Start::Ready(worker) => *busy.held = Some(worker),
            Start::Unusable(reason) => {
                tracing::warn!("{} cannot run as a persistent worker ({}); calling it once per request", module_name(&entry), reason);
                lock(&UNUSABLE).insert(entry, fingerprint);
                busy.finished = true;
                return Outcome::OneShot(on_progress, None);
            }
            Start::TimedOut => {
                // 인사를 기다리는 동안 기한이 지났다: 기한이 짧은 호출이 매번 같은 자리에서 끝나지 않게 다음 호출부터 단발로 돌린다
                tracing::warn!("{} did not start as a persistent worker before {} timed out; calling it once per request", module_name(&entry), request.function);
                lock(&UNUSABLE).insert(entry.clone(), fingerprint);
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
        sandboxed: None,
    };
    *lock(&worker.current) = Some(StderrSink::new(on_progress, None, Some(host), CALL_DEADLINE.try_with(Clone::clone).ok(), log.clone()));
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
        Ok(Err(blocked)) => return Start::Unusable(blocked.to_string()),
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
            let forward = |line: String, ended: bool| match lock(&current).as_mut() {
                Some(sink) => sink.line(line, ended),
                None => tracing::debug!("Plugin worker stderr: {}", line),
            };
            while let Ok(Some((line, ended))) = next_stderr_line(&mut reader, &mut lines).await {
                // 함수나 자식 프로세스가 줄바꿈 없이 끝낸 출력 뒤에 붙어 올 수 있다
                let Some(output) = line.strip_suffix(DONE) else {
                    forward(line, ended);
                    continue;
                };
                if !output.is_empty() {
                    forward(output.to_string(), true);
                }
                done_tx.send_modify(|done| *done += 1);
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

    /// 모듈 폴더 아래에 `[runtime] persistent = true`인 모듈을 만든다. 카탈로그에서 받은 그대로라고 적어 둔다 (`verify`)
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
        verify(dir.path());
        (dir, entry.to_string_lossy().into_owned())
    }

    /// 설치·업데이트처럼 그 폴더를 카탈로그에서 받은 것으로 기록한다: 확인된 구성요소는 격리 없이 돈다
    fn verify(dir: &std::path::Path) {
        let key = format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, dir.file_name().unwrap().to_string_lossy());
        saba_chan_updater_lib::integrity::record_dir(&key, dir).unwrap();
        saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
    }

    /// 진입 파일의 수정 시각을 뒤로 민다 (수정 시각 해상도가 거친 파일 시스템에서도 바뀐 것으로 보이게)
    fn touch_later(entry: &str) {
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(entry).unwrap().set_modified(later).unwrap();
    }

    async fn gone(pid: &Value) -> bool {
        let pid = pid.as_u64().unwrap() as u32;
        crate::supervisor::process::wait_gone(std::time::Duration::from_secs(5), || crate::supervisor::process::is_process_alive(pid)).await
    }

    const SDK_MODULE: &str = "import os, sys, time\nfrom daemon_rcon import run_lifecycle\n\
        def pid(config):\n    return {'success': True, 'pid': os.getpid(), 'argv': sys.argv[1]}\n\
        def boom(config):\n    raise RuntimeError('worker boom')\n\
        def wait(config):\n    time.sleep(config['secs'])\n    return {'success': True}\n\
        if __name__ == '__main__':\n    run_lifecycle({'pid': pid, 'boom': boom, 'wait': wait})\n";

    /// 작업자 시험(읽기)과 모든 작업자를 끝내는 시험(쓰기)이 겹치지 않게 (작업자도, 단발로 돌리기로 한 기록도 지운다)
    static REUSE: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

    async fn call(entry: &str, function: &str) -> Value {
        run_plugin(entry, function, serde_json::json!({})).await.unwrap_or_else(|e| error_response(&e))
    }

    /// 함수 예외의 traceback(답 앞에 쓴 stderr)이 그 호출의 `stderr_tail`에 실리고, 작업자는 다음 호출에도 남는다
    #[tokio::test]
    async fn a_worker_keeps_serving_after_a_function_raises_and_reports_its_traceback() {
        let _reuse = REUSE.read().await;
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
        assert!(gone(&first["pid"]).await, "stop_workers ended the worker");
    }

    /// 기한을 넘긴 호출은 작업자를 끝내고 `module_timeout`으로 답한다. 다음 호출은 새 작업자를 띄운다
    #[tokio::test]
    async fn a_call_past_its_deadline_ends_the_worker() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let before = call(&entry, "pid").await;
        assert_eq!(before["argv"], ARG, "{before}");
        let late = run_plugin_with_timeout(&entry, "wait", serde_json::json!({"secs": 30}), 2).await.unwrap_or_else(|e| error_response(&e));
        assert_eq!(late["error_code"], "module_timeout", "{late}");
        assert!(gone(&before["pid"]).await, "the late worker ended");
        let after = call(&entry, "pid").await;
        assert_eq!(after["argv"], ARG, "{after}");
        assert_ne!(after["pid"], before["pid"]);
        stop_workers(Some(dir.path()));
    }

    /// 작업자가 다른 호출을 하는 중이면 기다리지 않고 단발로 돈다
    #[tokio::test]
    async fn a_call_while_the_worker_is_busy_runs_one_shot() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let worker = call(&entry, "pid").await;
        assert_eq!(worker["argv"], ARG, "{worker}");
        let busy = tokio::spawn({
            let entry = entry.clone();
            async move { run_plugin(&entry, "wait", serde_json::json!({"secs": 3})).await.unwrap_or_else(|e| error_response(&e)) }
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(call(&entry, "pid").await["argv"], "pid");
        assert_eq!(busy.await.unwrap()["success"], true);
        assert_eq!(call(&entry, "pid").await["pid"], worker["pid"]);
        stop_workers(Some(dir.path()));
    }

    /// 함수가 줄바꿈 없이 끝낸 출력에 호출 끝 표시가 붙어도 그 호출의 답을 기다리지 않고 받는다
    #[tokio::test]
    async fn output_without_a_trailing_newline_does_not_hide_the_end_of_a_call() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(
            "import os\nfrom daemon_rcon import run_lifecycle\n\
             def partial(config):\n    print('downloading...', end='')\n    return {'success': True, 'pid': os.getpid()}\n\
             if __name__ == '__main__':\n    run_lifecycle({'partial': partial})\n",
        );
        let first = run_plugin_with_timeout(&entry, "partial", serde_json::json!({}), 10).await.unwrap_or_else(|e| error_response(&e));
        assert_eq!(first["success"], true, "{first}");
        let second = run_plugin_with_timeout(&entry, "partial", serde_json::json!({}), 10).await.unwrap_or_else(|e| error_response(&e));
        assert_eq!(second["pid"], first["pid"], "{second}");
        stop_workers(Some(dir.path()));
    }

    /// 단발 계약대로 stdin을 먼저 읽는 진입점은 인사하지 않는다: 뜨는 사이에 기한이 지나도 다음 호출부터 단발로 돈다
    #[tokio::test]
    async fn an_entry_point_that_waits_for_stdin_runs_one_shot_after_its_start_timed_out() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module("import json, sys\nsys.stdin.read()\nprint(json.dumps({'success': True, 'argv': sys.argv[1]}))\n");
        let first = run_plugin_with_timeout(&entry, "probe", serde_json::json!({}), 3).await.unwrap_or_else(|e| error_response(&e));
        assert_eq!(first["error_code"], "module_timeout", "{first}");
        let second = call(&entry, "probe").await;
        assert_eq!(second["argv"], "probe", "{second}");
        stop_workers(Some(dir.path()));
    }

    /// 진입점이 SDK가 아니면 작업자가 되지 못한다: 단발로 답한다
    #[tokio::test]
    async fn a_module_without_the_sdk_entry_point_runs_one_shot() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module("import json, sys\nprint(json.dumps({'success': True, 'argv': sys.argv[1]}))\n");
        assert_eq!(call(&entry, "probe").await["argv"], "probe");
        assert_eq!(call(&entry, "probe").await["argv"], "probe");
        stop_workers(Some(dir.path()));
    }

    /// 진입 파일이 바뀌면(업데이트) 작업자를 새로 띄운다
    #[tokio::test]
    async fn a_changed_module_gets_a_new_worker() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let before = call(&entry, "pid").await;
        assert_eq!(before["argv"], ARG, "{before}");
        std::fs::write(&entry, format!("{SDK_MODULE}# changed\n")).unwrap();
        touch_later(&entry);
        verify(dir.path());
        let after = call(&entry, "pid").await;
        assert_eq!(after["argv"], ARG, "{after}");
        assert_ne!(after["pid"], before["pid"]);
        stop_workers(Some(dir.path()));
    }

    /// 익스텐션을 설치·업데이트하면 모든 작업자를 끝낸다: 모듈 작업자가 옛 판을 import해 두었을 수 있다
    #[tokio::test]
    async fn installing_an_extension_stops_every_worker() {
        let _reuse = REUSE.write().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let before = call(&entry, "pid").await;
        assert_eq!(before["argv"], ARG, "{before}");
        let extension = tempfile::Builder::new().prefix("worker-ext-").tempdir_in(resolve_extensions_dir()).unwrap();
        let staged = extension.path().with_file_name(format!(".staging-{}", extension.path().file_name().unwrap().to_string_lossy()));
        std::fs::create_dir(&staged).unwrap();
        crate::utils::swap_dir(&staged, extension.path()).unwrap();
        assert!(gone(&before["pid"]).await, "the worker stopped");
        let after = call(&entry, "pid").await;
        assert_eq!(after["argv"], ARG, "{after}");
        assert_ne!(after["pid"], before["pid"]);
        stop_workers(Some(dir.path()));
    }

    /// 확인되지 않은 구성요소(여기서는 손으로 고친 것)는 격리해서 돌리므로 작업자로 두지 않는다: 단발로 돌고, 떠 있던 작업자는 끝난다
    #[tokio::test]
    async fn an_unverified_module_runs_one_shot_and_its_worker_stops() {
        let _reuse = REUSE.read().await;
        crate::python_env::get_python_path().await.expect("test Python ready");
        let (dir, entry) = persistent_module(SDK_MODULE);
        let before = call(&entry, "pid").await;
        assert_eq!(before["argv"], ARG, "{before}");
        std::fs::write(&entry, format!("{SDK_MODULE}# edited by hand\n")).unwrap();
        touch_later(&entry);
        // 이 컨테이너처럼 격리할 수 없는 곳에서도 돌도록 격리 없이 돌리기로 동의해 둔다
        crate::sandbox::consent_test_component(std::path::Path::new(&entry));
        let after = call(&entry, "pid").await;
        assert_eq!(after["argv"], "pid", "{after}");
        assert!(gone(&before["pid"]).await, "the worker of the unverified module stopped");
        stop_workers(Some(dir.path()));
    }
}
