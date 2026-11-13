//! 하트비트: 60초 주기·즉시 보내기, 본문과 메타데이터, 이 PC의 OS·부하·공인 주소

use super::*;

/// 이 PC의 리눅스 배포판 `{id, version?, name?}` (`/etc/os-release`의 ID·VERSION_ID·PRETTY_NAME). 리눅스가 아니거나 읽지 못하면 없다
static LINUX_DISTRO: std::sync::LazyLock<Option<Value>> = std::sync::LazyLock::new(|| {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).ok()?;
    parse_os_release(&text)
});

/// os-release 본문 → `{id, version?, name?}`. ID는 릴레이가 받는 모양(소문자·숫자·`.-_`, 32자)일 때만
pub(super) fn parse_os_release(text: &str) -> Option<Value> {
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix(key)?.strip_prefix('=')?.trim();
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
            Some(value.to_string()).filter(|v| !v.is_empty())
        })
    };
    let id = field("ID")?;
    if id.len() > 32 || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')) {
        return None;
    }
    let mut distro = json!({ "id": id });
    if let Some(v) = field("VERSION_ID").filter(|v| v.chars().count() <= 32) {
        distro["version"] = json!(v);
    }
    if let Some(n) = field("PRETTY_NAME").map(|n| n.chars().take(64).collect::<String>()) {
        distro["name"] = json!(n);
    }
    Some(distro)
}

/// 하트비트 답의 `address`: 릴레이가 본 이 PC의 공인 주소. IP로 읽히는 값만 받는다
pub(super) fn relay_seen_address(data: &Value) -> Option<String> {
    data.get("address").and_then(Value::as_str).and_then(|a| a.parse::<std::net::IpAddr>().ok()).map(|ip| ip.to_string())
}

/// 하트비트마다 보내는 메타데이터 키: 서버 목록(CPU·RAM·접속자가 매번 바뀐다)과 릴레이가 매번 캐시를 맞추는 권한,
/// PC 부하와 예약 목록(다음 실행이 지나면 바뀐다)
const HEARTBEAT_ALWAYS: [&str; 5] = ["servers", "permissions", "permissionsAt", "host", "schedules"];

/// 그중 지문에 넣지 않는 키 (매번 바뀌어도 큰 값을 다시 보내지 않게)
const HEARTBEAT_VOLATILE: [&str; 4] = ["servers", "permissionsAt", "host", "schedules"];

/// 하트비트 예약 목록 상한 (맨 위 글은 가까운 순으로 10개까지 보인다)
const SCHEDULES_MAX: usize = 10;

impl Session {
    pub(super) async fn heartbeat_loop(&self) {
        use tokio::sync::broadcast::error::RecvError;
        let mut rx = crate::events::bus().subscribe(None).rx;
        loop {
            self.heartbeat().await;
            let sent_at = tokio::time::Instant::now();
            let next = tokio::time::sleep(HEARTBEAT_INTERVAL);
            tokio::pin!(next);
            loop {
                tokio::select! {
                    _ = self.cancel.cancelled() => return,
                    _ = &mut next => break,
                    r = rx.recv() => match r {
                        Ok(e) if kicks_heartbeat(&e.kind) => {
                            // 같은 때 몰린 시작·준비·정지를 한 번에 싣는다
                            let at = (tokio::time::Instant::now() + HEARTBEAT_KICK_DELAY).max(sent_at + HEARTBEAT_MIN_GAP);
                            tokio::select! {
                                _ = self.cancel.cancelled() => return,
                                _ = tokio::time::sleep_until(at) => {}
                            }
                            while rx.try_recv().is_ok() {}
                            break;
                        }
                        Ok(_) | Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => {
                            tokio::select! {
                                _ = self.cancel.cancelled() => return,
                                _ = &mut next => break,
                            }
                        }
                    },
                }
            }
        }
    }

    /// 릴레이가 메타데이터 전체를 다시 달라고 하면(다시 켜졌거나 지문을 잊었다) 곧바로 한 번 더 보낸다
    pub(super) async fn heartbeat(&self) {
        if self.heartbeat_once().await {
            self.heartbeat_once().await;
        }
    }

    /// 하트비트 한 번. 릴레이가 메타데이터 전체를 달라고 했거나(`needMetadata`) 새로 묶인 길드에 열 서버를 정했으면 true
    pub(super) async fn heartbeat_once(&self) -> bool {
        // 업데이트로 내려가는 중: 새 데몬의 heartbeat가 "업데이트 중" 창을 닫는다
        if self.agent.going_down.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        let sent = self.metadata_sent.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (metadata, hash, unchanged) = heartbeat_metadata(self.metadata().await, sent.as_deref());
        let acks = self.linked_acks.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let body = heartbeat_body(metadata, &hash, unchanged, &acks);
        let mut rejected: Option<HeartbeatReject> = None;
        let result = async {
            let resp = self.send_signed(reqwest::Method::POST, "/heartbeat", Some(body.to_string())).await?;
            let status = resp.status();
            let data: Value = resp.json().await.unwrap_or(Value::Null);
            if token_replaced(status, &data) {
                return Err(REPLACED.to_string());
            }
            if let Some(min) = agent_too_old(status, &data) {
                self.report_too_old(&min);
                return Err(TOO_OLD.to_string());
            }
            if !status.is_success() {
                let hint = if status.as_u16() == 401 || status.as_u16() == 403 { " (node token rejected)" } else { "" };
                rejected = data.get("reason").cloned().and_then(|reason| serde_json::from_value(reason).ok());
                let field = rejected.as_ref().map(|r| format!(" ({})", r.field)).unwrap_or_default();
                return Err(format!("heartbeat {}{}: {}{}", status, hint, data.get("error").and_then(|v| v.as_str()).unwrap_or(""), field));
            }
            Ok(data)
        }
        .await;

        // 릴레이가 지문을 되돌려 주면 받아 둔 것이다: 다음 하트비트는 그대로면 바뀌는 부분만 보낸다.
        // 되돌려 주지 않으면(실패, 다시 달라고 함) 다음에도 전체를 보낸다
        let need_full = result.as_ref().is_ok_and(|data| data.get("needMetadata").and_then(Value::as_bool) == Some(true));
        let kept = result.as_ref().is_ok_and(|data| data.get("metadataHash").and_then(Value::as_str) == Some(hash.as_str()));
        *self.metadata_sent.lock().unwrap_or_else(|e| e.into_inner()) = kept.then_some(hash);
        let mut send_again = false;
        match result {
            Ok(data) => {
                if data.get("warning").and_then(|v| v.as_str()) == Some("UPDATE_REQUIRED") {
                    tracing::warn!("[RelayAgent] relay asks for agent update (min {})", data.get("minVersion").and_then(|v| v.as_str()).unwrap_or("?"));
                }
                let seen = self.friend_address(relay_seen_address(&data)).await;
                let mut seen_changed = false;
                self.agent.update(self.generation, |s| {
                    s.last_heartbeat_at = Some(now_ms());
                    s.last_error = None;
                    s.last_error_code = None;
                    s.min_version = None;
                    s.heartbeat_rejected = None;
                    // 사설 주소면 없다
                    seen_changed = s.seen_address != seen;
                    s.seen_address = seen;
                });
                // 원격 포트의 직접 연결 주소를 따로 적지 않았으면 릴레이가 본 공인 IP를 올린다
                if seen_changed {
                    crate::remote_daemon::on_seen_address(&self.server);
                }
                // 릴레이가 받은 확인 응답은 지운다. 그 사이 새로 적용한 길드는 남는다
                self.linked_acks.lock().unwrap_or_else(|e| e.into_inner()).retain(|g| !acks.contains(g));
                // 새로 묶인 길드에 열 서버를 정했으면 곧바로 한 번 더 보내 확인 응답과 그 목록을 릴레이에 알린다
                // (릴레이는 확인 응답을 받기 전까지 그 길드의 명령을 막고 목록을 다시 보낸다)
                if self.apply_linked_guilds(&data).await {
                    send_again = true;
                }
                // 채널별 최신 릴리즈 id: 이 데몬이 받는 채널의 값이 바뀌었을 때만 확인한다 (ipc/updates.rs).
                // 그것이 없는 옛 릴레이는 새 릴리즈 알림(`updateHint`)을 대조한다
                if let Some(ids) = data.get("latestReleases").filter(|v| v.is_object()) {
                    crate::ipc::updates::on_latest_releases(&self.server.update_state, ids, self.cancel.clone());
                } else if let Some(hint) = data.get("updateHint") {
                    crate::ipc::updates::on_update_hint(&self.server.update_state, hint, self.cancel.clone());
                }
                // 챗봇 설정은 첫 하트비트 응답(새로 묶인 길드 받기) 뒤에 한 번 릴레이에 맞춘다.
                // 세션이 끝나면(연결 해제·토큰 교체) 같이 멈춘다: 예전 토큰으로 설정을 올리지 않게
                if self.chatbot_sync_due.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    let (server, cancel) = (self.server.clone(), self.cancel.clone());
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = cancel.cancelled() => {}
                            _ = crate::service::chatbot::sync_on_connect(&server) => {}
                        }
                    });
                }
                // 연결이 끊긴 사이 릴레이에 못 보낸 디스코드 길드 설정을 다시 보낸다 (세션과 함께 멈춘다)
                if crate::service::discord::take_pending_sync() {
                    let (server, cancel) = (self.server.clone(), self.cancel.clone());
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = cancel.cancelled() => {}
                            _ = crate::service::discord::sync_on_connect(&server) => {}
                        }
                    });
                }
            }
            Err(e) if e == REPLACED => self.give_up_replaced(),
            // 이미 상태에 남겼다
            Err(e) if e == TOO_OLD => {}
            Err(e) => {
                tracing::warn!("[RelayAgent] {}", e);
                self.agent.update(self.generation, |s| s.heartbeat_rejected = rejected);
                self.report_error(e);
            }
        }
        (need_full && unchanged) || send_again
    }

    /// 친구에게 안내할 이 PC의 공인 주소. 하트비트가 IPv6로 나가 릴레이가 IPv6를 봤으면 포트 확인과 같은 IPv4 전용 연결로
    /// 한 번 다시 물어 공인 IPv4를 쓴다 (포트 확인은 IPv4로 두드리고, 친구는 대개 IPv4로 붙는다). 얻지 못하면 IPv6 그대로
    async fn friend_address(&self, seen: Option<String>) -> Option<String> {
        use crate::port_check::Ipv4Lookup;
        let Some(v6) = seen.as_deref().filter(|a| a.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_ipv6())) else { return seen };
        if let Some(SeenV4 { v6: asked, v4, retry_at }) = self.seen_v4.lock().unwrap_or_else(|e| e.into_inner()).clone()
            && asked == v6
            && retry_at.is_none_or(|at| std::time::Instant::now() < at)
        {
            return v4.or(seen);
        }
        let relay = crate::port_check::Relay { base: self.base.clone(), token: self.token.clone(), host: None };
        let (v4, retry_at) = match crate::port_check::public_ipv4(&relay).await {
            Ipv4Lookup::Found(ip) => (Some(ip.to_string()), None),
            Ipv4Lookup::Unusable => (None, None),
            Ipv4Lookup::Later => (None, Some(std::time::Instant::now() + IPV4_RETRY)),
        };
        *self.seen_v4.lock().unwrap_or_else(|e| e.into_inner()) = Some(SeenV4 { v6: v6.to_string(), v4: v4.clone(), retry_at });
        v4.or(seen)
    }

    /// 하트비트 메타데이터: 릴레이 봇이 자동완성·접두사 표시에 쓴다
    pub(super) async fn metadata(&self) -> Value {
        use crate::chat::api::DaemonApi;
        // `/사바쨩 상태`의 CPU·RAM과 PC 부하 (폴더 크기는 재지 않는다)
        let roots = self.server.supervisor.read().await.sample_roots();
        let monitor = self.server.system_monitor.clone();
        let snapshot = tokio::task::spawn_blocking(move || monitor.sample(&roots, &[], false, false)).await.ok();
        let usage = snapshot.as_ref().map(|s| s.instances.as_slice()).unwrap_or_default();
        let servers: Vec<Value> = match crate::ipc::internal::into_internal(
            crate::service::servers::list(&self.server, 0).await,
        )
        .await
        .body
        .get("servers")
        {
            Some(Value::Array(list)) => list
                .iter()
                .map(|s| relay_server_entry(s, usage.iter().find(|u| s.get("id").and_then(|v| v.as_str()) == Some(u.id.as_str()))))
                .collect(),
            _ => Vec::new(),
        };
        let schedules = {
            let supervisor = self.server.supervisor.read().await;
            crate::scheduler::runner::upcoming_runs(supervisor.instance_store.list(), SCHEDULES_MAX)
        };
        // 모듈 locales 파일은 하트비트마다 읽으므로 async 작업자 밖에서 읽는다
        let views = self.server.modules().await;
        let commands = remote_commands(&views);
        let modules: Vec<Value> = tokio::task::spawn_blocking(move || views.iter().map(relay_module_entry).collect())
        .await
        .unwrap_or_else(|e| if e.is_panic() { std::panic::resume_unwind(e.into_panic()) } else { Vec::new() });

        // 권한을 읽기 직전의 시각 (권한 변경 응답의 `permissionsAt`과 같은 PC 시계)
        let permissions_at = now_ms();
        let bot = self.server.config_store.get_bot_config().await;

        let mut meta = json!({
            "servers": servers,
            // 알림 채널 맨 위 글의 예약 목록
            "schedules": schedules,
            "modules": modules,
            // 읽는 곳은 없지만 2026-11-12 전 릴레이 스키마가 요구한다. 그 요구를 뺀 릴레이가 배포되면 지운다
            "moduleDetails": {},
            "botConfig": {
                // 비어 있으면 릴레이가 응답 언어의 기본값을 쓴다
                "prefixes": bot.prefixes,
            },
        });
        // 길드별 허용 인스턴스. 목록이 있는 길드만 싣는다 (없으면 릴레이는 제한 없음으로 본다)
        if let Some(allowed) = allowed_instances_by_guild(&bot.node_settings) {
            meta["allowedInstances"] = allowed;
        }
        // PC 과부하·디스크 부족 알림 (릴레이가 주인에게)
        if let Some(snapshot) = &snapshot {
            meta["host"] = host_load(snapshot);
        }
        dm_notice_meta(&mut meta, &bot);
        // 길드별 자리 꽉 참 알림을 켠 서버 (`/api/instance/:id/full-notice/:guild`). 켠 길드만 싣는다
        if let Some(on) = ids_by_guild(&bot.node_settings, crate::service::discord::FULL_NOTICE_KEY) {
            meta["fullNotice"] = on;
        }
        // 길드별 켜짐 알림을 켠 서버 (`/api/instance/:id/start-notice/:guild`). 켠 길드만 싣는다
        if let Some(on) = ids_by_guild(&bot.node_settings, crate::service::discord::START_NOTICE_KEY) {
            meta["startNotice"] = on;
        }
        // 인스턴스별로 서버와 명령을 부르는 낱말 (어느 길드에서나 같다, 릴레이는 찾기·위험 확인에 쓴다).
        // 비어도 싣는다: 이 키가 있으면 릴레이는 옛 길드별 `instanceWords`·전역 별명을 보지 않는다
        meta["serverWords"] = crate::chat::words::heartbeat(&bot.server_words);
        // 디스코드·챗봇에서 쓸 수 있는 액션 (P2). 상한을 넘으면 잘라 표시한다
        let loaded = self.server.supervisor.read().await.list_modules().unwrap_or_default();
        let (actions, truncated) = crate::actions::heartbeat_actions(&crate::chat::i18n::languages(), &loaded);
        // 권한 원본은 데몬이다: 릴레이는 이 값으로 바깥 검사용 캐시를 맞춘다. 요구 레벨은 위 서버·모듈·액션 목록 그대로 센다
        let targets = crate::actions::RemoteTargets { actions, commands, instances: remote_instances(&servers) };
        meta["permissions"] = permissions_by_guild(&bot.node_settings, &targets);
        meta["permissionsAt"] = json!(permissions_at);
        // `/사바쨩 상태` 끝 줄의 다음 할 일 (J1-a, GUI·CLI와 같은 단계)
        meta["onboarding"] = crate::service::onboarding::now(&self.server).await;
        // 언어가 `auto`인 길드는 이 언어로 답한다 (I2)
        // (릴레이 스키마는 10자까지. 넘으면 heartbeat 전체가 거절되므로 싣지 않는다)
        if let Some(language) = Some(self.server.config_store.pc_language().await).filter(|l| l.len() <= 10) {
            meta["language"] = Value::String(language);
        }
        meta["actions"] = Value::Array(targets.actions);
        if truncated {
            meta["actionsTruncated"] = Value::Bool(true);
        }
        meta
    }
}

/// 하트비트의 PC 부하 `{cpu, mem, diskFree?}` (정수 %). 디스크는 서버·백업 폴더가 있는 드라이브 중 가장 덜 남은 곳.
/// 드라이브 이름·경로는 싣지 않는다
pub(super) fn host_load(snapshot: &crate::system_monitor::MetricsSnapshot) -> Value {
    let pct = |part: f64, whole: f64| if whole > 0.0 { (part * 100.0 / whole).round().clamp(0.0, 100.0) as u64 } else { 0 };
    let mut load = json!({
        "cpu": (snapshot.cpu.total_percent as f64).round().clamp(0.0, 100.0) as u64,
        "mem": pct(snapshot.memory.used_bytes as f64, snapshot.memory.total_bytes as f64),
    });
    if let Some(free) = snapshot.disks.iter().filter(|d| d.total_bytes > 0).map(|d| pct(d.available_bytes as f64, d.total_bytes as f64)).min() {
        load["diskFree"] = json!(free);
    }
    load
}

/// 하트비트 본문. 적용한 새 길드가 있으면 그 연결 id로 확인 응답을 싣는다(`linkedGuildsAckIds`)
/// 이 PC의 OS·CPU와 리눅스 배포판 `{os, distro?}` (하트비트, 앱의 PC 카드)
pub fn platform() -> Value {
    let mut v = json!({ "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH) });
    // 리눅스 배포판: 내 다른 PC의 앱 카드가 배포판 로고를 보인다
    if let Some(distro) = LINUX_DISTRO.as_ref() {
        v["distro"] = distro.clone();
    }
    v
}

pub(super) fn heartbeat_body(metadata: Value, hash: &str, unchanged: bool, acks: &[String]) -> Value {
    let mut body = json!({
        "agentVersion": AGENT_VERSION,
        "metadata": metadata,
        "metadataHash": hash,
    });
    if let Value::Object(platform) = platform() {
        body.as_object_mut().expect("object").extend(platform);
    }
    if unchanged {
        body["metadataUnchanged"] = Value::Bool(true);
    }
    if !acks.is_empty() {
        body["linkedGuildsAckIds"] = json!(acks);
    }
    body
}

/// 하트비트 메타데이터를 보낼 모양으로: (보낼 메타데이터, 지문, 바뀌지 않아 줄였는가).
/// 지문은 `HEARTBEAT_VOLATILE`을 뺀 나머지(권한 포함)의 SHA-256이다. 릴레이가 받아 둔 지문(`sent`)과 같으면
/// `HEARTBEAT_ALWAYS`만 보낸다 (모듈 명령·액션·낱말처럼 큰 값은 바뀔 때만)
/// PC 주인 DM 알림 설정 (봇 설정 `backupNotice`·`dmNotice`). 기본값이면 싣지 않는다
pub(super) fn dm_notice_meta(meta: &mut Value, bot: &crate::config_store::BotConfig) {
    use crate::config_store::BackupNotice;
    match bot.backup_notice {
        BackupNotice::All => {}
        BackupNotice::Failures => meta["backupNotice"] = json!("failures"),
        BackupNotice::Off => meta["backupNotice"] = json!("off"),
    }
    let off = bot.dm_notice.off_kinds();
    if !off.is_empty() {
        meta["dmOff"] = json!(off);
    }
    let on = bot.dm_notice.on_kinds();
    if !on.is_empty() {
        meta["dmOn"] = json!(on);
    }
}

pub(super) fn heartbeat_metadata(mut meta: Value, sent: Option<&str>) -> (Value, String, bool) {
    use sha2::{Digest, Sha256};
    let Value::Object(map) = &mut meta else { return (meta, String::new(), false) };
    let volatile: Vec<(&str, Value)> = HEARTBEAT_VOLATILE.iter().filter_map(|k| Some((*k, map.remove(*k)?))).collect();
    let hash = hex::encode(Sha256::digest(Value::Object(map.clone()).to_string().as_bytes()));
    let unchanged = sent == Some(hash.as_str());
    if unchanged {
        map.retain(|k, _| HEARTBEAT_ALWAYS.contains(&k.as_str()));
    }
    for (key, value) in volatile {
        map.insert(key.to_string(), value);
    }
    (meta, hash, unchanged)
}
