//! 명령 받기: 롱폴, 받은 명령 id 기억(다시 와도 한 번만 실행), payload 처리, 길드 연결

use super::*;

/// 받기(poll)가 실패한 뒤 다시 묻기까지. 버전 미달은 업데이트 전에는 같은 답이라 heartbeat 주기로만 묻는다
pub(super) fn poll_retry(error: &str, failures: u32) -> Duration {
    if error == TOO_OLD {
        HEARTBEAT_INTERVAL
    } else {
        backoff(failures)
    }
}

/// 받은 명령을 처음 본 것인지, 아직 처리 중인지, 이미 끝났는지
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Received {
    New,
    /// 받아 두고 아직 시작하지 않았다
    Waiting,
    Running,
    Done(Value),
}

/// 최근에 받은 명령 id. 끝난 명령은 결과도 기억해 다시 보낼 수 있게 한다 (`SEEN_FOR` 동안, 최대 `SEEN_MAX`개).
/// 기다리거나 실행 중인 명령은 끝날 때까지(또는 세션이 끝나 `forget`할 때까지) 기억한다
#[derive(Default)]
pub(super) struct SeenCommands {
    pub(super) entries: std::collections::VecDeque<Seen>,
}

pub(super) struct Seen {
    id: String,
    at: std::time::Instant,
    started: bool,
    result: Option<Value>,
}

impl SeenCommands {
    /// 받은 id를 기록하고, 이 명령을 어떻게 다룰지 돌려준다. 처음 본 id는 처리 중으로 기록한다
    /// 끝난 명령만 시간·개수로 잊는다. 기다리거나 실행 중인 명령은 작업량 상한 안에 있고, 잊으면 다시 오는 같은 명령을 또 실행한다
    pub(super) fn receive(&mut self, id: &str, now: std::time::Instant) -> Received {
        self.entries.retain(|e| e.result.is_none() || now.saturating_duration_since(e.at) < SEEN_FOR);
        if let Some(e) = self.entries.iter().find(|e| e.id == id) {
            return match &e.result {
                Some(r) => Received::Done(r.clone()),
                None if e.started => Received::Running,
                None => Received::Waiting,
            };
        }
        self.entries.push_back(Seen { id: id.to_string(), at: now, started: false, result: None });
        let mut over = self.entries.len().saturating_sub(SEEN_MAX);
        self.entries.retain(|e| {
            let drop = over > 0 && e.result.is_some();
            over -= usize::from(drop);
            !drop
        });
        Received::New
    }

    /// 세션이 끝나 시작하지 않고 버린 명령: 다시 오면 다음 세션이 실행한다
    pub(super) fn forget(&mut self, ids: &[&str]) {
        self.entries.retain(|e| !ids.contains(&e.id.as_str()));
    }

    pub(super) fn start(&mut self, id: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.started = true;
        }
    }

    /// 끝난 명령의 보관 기간은 끝난 때부터 센다. 받은 때부터 세면 오래 기다린 명령을 끝나자마자 잊는다
    pub(super) fn finish(&mut self, id: &str, result: &Value) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.at = std::time::Instant::now();
            e.result = Some(result.clone());
        }
    }
}

/// 시작한 명령 id로 만든 `POST /ack` 본문들 (`ACK_BATCH`개씩)
pub(super) fn ack_bodies(commands: &[Value]) -> Vec<String> {
    let ids: Vec<&str> = commands.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).collect();
    ids.chunks(ACK_BATCH).map(|chunk| json!({ "ids": chunk }).to_string()).collect()
}

impl Session {
    /// 받은 명령은 동시에 처리하고 그동안에도 계속 받는다: 오래 걸리는 명령(재시작·백업 등) 하나가
    /// 뒤의 명령을 릴레이 대기 시간 너머로 붙잡지 않게 (한 번에 `COMMANDS_IN_FLIGHT`개까지)
    pub(super) async fn poll_loop(&self) {
        use futures_util::StreamExt;
        type Poll<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>, String>> + Send + 'a>>;
        let mut failures: u32 = 0;
        type Side<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
        let mut running = futures_util::stream::FuturesUnordered::new();
        // ack와 다시 온 명령의 결과 재전송: 명령 자리를 차지하지 않고 따로 돈다
        let mut side: futures_util::stream::FuturesUnordered<Side<'_>> = futures_util::stream::FuturesUnordered::new();
        let mut waiting: Vec<Value> = Vec::new();
        let mut next: Option<Poll<'_>> = Some(Box::pin(self.poll_once()));
        while !self.cancel.is_cancelled() {
            // 시작하는 명령만 ack한다: 받아 두고 시작하지 못한 명령은 연결이 끝나면 릴레이가 다음 연결로 다시 보낸다
            let mut started = Vec::new();
            let backlogged = self.unsent_full();
            while running.len() < COMMANDS_IN_FLIGHT && !waiting.is_empty() && !backlogged {
                let cmd = waiting.remove(0);
                if let Some(id) = cmd.get("id").and_then(Value::as_str) {
                    self.seen.lock().unwrap_or_else(|e| e.into_inner()).start(id);
                }
                started.push(json!({ "id": cmd.get("id").cloned().unwrap_or(Value::Null) }));
                running.push(self.run_command(cmd));
            }
            for body in ack_bodies(&started) {
                side.push(Box::pin(self.post_ack(body)));
            }
            if next.is_none() && waiting.is_empty() && running.len() < COMMANDS_IN_FLIGHT {
                next = Some(Box::pin(self.poll_once()));
            }
            tokio::select! {
                polled = async { next.as_mut().expect("poll").await }, if next.is_some() => {
                    next = None;
                    match polled {
                        Ok(commands) => {
                            // 실패 뒤 다시 받았다: 하트비트를 기다리지 않고 붙어 있다고 보인다 (`connected`)
                            if failures > 0 {
                                self.agent.update(self.generation, |s| s.last_error = None);
                            }
                            failures = 0;
                            let now = std::time::Instant::now();
                            let mut reack = Vec::new();
                            for cmd in commands {
                                let Some(id) = cmd.get("id").and_then(Value::as_str).map(str::to_string) else { continue };
                                // 결과를 아직 보내지 못한 명령: 다시 실행하지 않는다 (`unsent_loop`가 결과를 보낸다)
                                if self.has_unsent(&id) {
                                    reack.push(json!({ "id": id }));
                                    continue;
                                }
                                let received = self.seen.lock().unwrap_or_else(|e| e.into_inner()).receive(&id, now);
                                match received {
                                    Received::New => waiting.push(cmd),
                                    // 같은 명령을 다시 받았다: 실행하지 않는다. 실행 중이면 ack가 닿지 않은 것이니 다시 ack하고,
                                    // 끝난 것이면 결과만 다시 보낸다
                                    Received::Waiting => {}
                                    Received::Running => reack.push(json!({ "id": id })),
                                    Received::Done(result) => side.push(Box::pin(async move { self.post_result(&id, &result).await })),
                                }
                            }
                            for body in ack_bodies(&reack) {
                                side.push(Box::pin(self.post_ack(body)));
                            }
                        }
                        Err(e) if e == REPLACED => self.give_up_replaced(),
                        Err(e) => {
                            failures += 1;
                            let backoff = poll_retry(&e, failures);
                            tracing::warn!("[RelayAgent] poll failed ({}), retry in {}s", e, backoff.as_secs());
                            if e != TOO_OLD {
                                self.report_error(e);
                            }
                            next = Some(Box::pin(async move {
                                self.pause(backoff).await;
                                self.poll_once().await
                            }));
                        }
                    }
                }
                Some(()) = running.next(), if !running.is_empty() => {}
                Some(()) = side.next(), if !side.is_empty() => {}
                // 받지 않은 결과가 줄었는지 가끔 본다 (그동안 받아 둔 명령은 시작하지 않는다)
                _ = self.pause(result_retry_delay(RESULT_RETRIES)), if backlogged && !waiting.is_empty() => {}
                _ = self.cancel.cancelled() => {}
            }
        }
        // 연결 해제(취소) 뒤에는 받아 두고 시작하지 않은 명령은 바로 잊어 다음 연결이 다시 받으면 실행하게 하고,
        // 이미 시작한 것은 끝까지 마쳐 결과를 올린다
        {
            let ids: Vec<&str> = waiting.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).collect();
            self.seen.lock().unwrap_or_else(|e| e.into_inner()).forget(&ids);
        }
        while running.next().await.is_some() {}
    }

    async fn run_command(&self, cmd: Value) {
        let Some(id) = cmd.get("id").and_then(Value::as_str).map(str::to_string) else { return };
        let result = self.handle_payload(&cmd).await;
        // 결과를 올리지 못해도 기억해 둔다: 릴레이가 명령을 다시 보내면 이 결과를 다시 올린다
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).finish(&id, &result);
        self.post_result(&id, &result).await;
        self.agent.update(self.generation, |s| s.commands_handled += 1);
    }

    pub(super) async fn pause(&self, d: Duration) {
        tokio::select! {
            _ = self.cancel.cancelled() => {}
            _ = tokio::time::sleep(d) => {}
        }
    }

    pub(super) async fn poll_once(&self) -> Result<Vec<Value>, String> {
        let req = self.signed(reqwest::Method::GET, "/poll", None)?;
        let resp = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(Vec::new()),
            r = req.send() => r.map_err(|e| e.to_string())?,
        };
        let status = resp.status();
        if status.as_u16() == 204 {
            return Ok(Vec::new());
        }
        let text = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(Vec::new()),
            b = resp.text() => b.unwrap_or_default(),
        };
        let body = decode_poll(&text);
        if token_replaced(status, &body) {
            return Err(REPLACED.to_string());
        }
        if let Some(min) = agent_too_old(status, &body) {
            self.report_too_old(&min);
            return Err(TOO_OLD.to_string());
        }
        if !status.is_success() {
            return Err(format!("poll {}: {}", status, body.get("error").and_then(|v| v.as_str()).unwrap_or("")));
        }
        Ok(body.get("commands").and_then(|v| v.as_array()).cloned().unwrap_or_default())
    }

    pub(super) async fn handle_payload(&self, cmd: &Value) -> Value {
        let payload = cmd.get("payload").cloned().unwrap_or(Value::Null);
        let action = payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "permission" => {
                let guild_id = cmd.get("guildId").and_then(|v| v.as_str()).map(str::to_string);
                let user_id = cmd.get("requestedBy").and_then(|v| v.as_str()).map(str::to_string);
                let who = self.requester(cmd, &payload, guild_id, user_id).await;
                let origin = crate::audit::Origin::new("discord");
                crate::audit::with_origin(origin, permission_action(&self.server, &who, &payload)).await
            }
            // 원격 MCP (계정 주인의 AI 에이전트): 길드 없이, 이 PC의 주인만. 앱과 같은 액션과 그 목록(`catalog`)
            "plan" | "run_plan" | "query" | "catalog" if payload.get("via").and_then(Value::as_str) == Some(crate::actions::MCP_VIA) => {
                let locale = payload.get("locale").and_then(Value::as_str).unwrap_or("en").to_string();
                let caller = match mcp_caller(cmd, &payload, locale.clone()) {
                    Ok(caller) => caller,
                    Err(e) => return action_result(Err(e)),
                };
                // 문장 계획은 디스코드 채팅 경로다 (MCP는 액션 id로만 부른다)
                if payload.get("text").is_some() {
                    let e = crate::actions::ActionError { status: 400, code: "invalid_input", message: "Text plans are not taken from MCP".into(), data: json!({ "field": "text" }) };
                    return action_result(Err(e));
                }
                let result = match action {
                    "catalog" => {
                        let instance = payload.get("instance").and_then(Value::as_str);
                        Ok(crate::actions::list(&self.server, Some(crate::actions::Surface::Gui), instance, &caller.locale).await)
                    }
                    _ => crate::audit::with_origin(caller.origin.clone(), action_request(&self.server, action, &payload, &caller)).await,
                };
                action_result(result)
            }
            // 액션 카탈로그 (P2·P3): 계획·확인한 계획 실행·읽기 전용 조회·확인 전 권한 검사
            "plan" | "run_plan" | "query" | "check" => {
                let guild_id = cmd.get("guildId").and_then(|v| v.as_str()).map(str::to_string);
                let user_id = cmd.get("requestedBy").and_then(|v| v.as_str()).map(str::to_string);
                let who = self.requester(cmd, &payload, guild_id.clone(), user_id.clone()).await;
                let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_string);
                let mut origin = crate::audit::Origin::new("discord");
                origin.user = user_id;
                origin.guild = guild_id;
                origin.user_name = text("userName");
                origin.guild_name = text("guildName");
                origin.via = who.via_chatbot.then(|| "chatbot".to_string());
                let caller = crate::actions::Caller {
                    surface: if who.via_chatbot { crate::actions::Surface::Chatbot } else { crate::actions::Surface::Discord },
                    remote: Some(who),
                    origin: origin.clone(),
                    locale: text("locale").unwrap_or_else(|| "en".into()),
                };
                let result = crate::audit::with_origin(origin, action_request(&self.server, action, &payload, &caller)).await;
                action_result(result)
            }
            // 사바쨩 웹: 릴레이에 웹 세션이 열렸다. 릴레이 웹소켓에 붙는다 (세션이 없어지면 데몬이 끊는다)
            "web_session" => {
                crate::remote::link::ensure(self.server.clone(), &self.base, &self.token);
                json!({ "success": true, "data": { "connecting": true } })
            }
            // 원격 데몬 터널 (T1): 같은 주인의 PC가 릴레이를 거쳐 이 PC의 원격 포트에 붙는다
            "remote_tunnel" => {
                let id = payload.get("tunnelId").and_then(Value::as_str).unwrap_or("");
                if !crate::remote_daemon::relay::valid_ticket_id(id) {
                    return json!({ "success": false, "data": { "error": "invalid remote_tunnel" } });
                }
                if !self.server.remote_daemon.status().await.listening {
                    return json!({ "success": false, "data": { "error": "remote_off" } });
                }
                let port = crate::remote_daemon::config::settings_in(&saba_chan_updater_lib::config_file::ConfigFile::default_location()).port;
                let relay = crate::remote_daemon::relay::Target { base: self.base.clone(), token: self.token.clone() };
                tokio::spawn(crate::remote_daemon::tunnel::accept(relay, id.to_string(), port));
                json!({ "success": true, "data": { "connecting": true } })
            }
            other => json!({ "success": false, "data": { "error": format!("Unknown action: {}", other) } }),
        }
    }

    /// 하트비트 응답의 `linkedGuilds`를 적용한다 (`close_linked_guild`). 적용한 길드는 다음 하트비트의 확인 응답에 싣는다.
    /// 한 길드라도 적용했으면 true
    pub(super) async fn apply_linked_guilds(&self, data: &Value) -> bool {
        let Some(list) = data.get("linkedGuilds").and_then(Value::as_array) else { return false };
        let store = &self.server.config_store;
        let policy = store.get_bot_config().await.new_guild_instances;
        let mut applied = false;
        for g in list {
            let (Some(guild), Some(via)) = (g.get("guildId").and_then(Value::as_str), g.get("via").and_then(Value::as_str)) else { continue };
            // 릴레이가 연결마다 준 id로 답해야, 풀고 다시 묶은 사이 늦게 간 답이 새 연결의 확인이 되지 않는다
            let link_id = g.get("linkId").and_then(Value::as_str).map(str::to_string);
            // 이미 적용하고 확인 응답을 기다리는 연결은 길드 설정을 다시 받지 않는다 (그 사이 앱에서 바꾼 값을 덮지 않게)
            let waiting = link_id.as_ref().is_some_and(|id| self.linked_acks.lock().unwrap_or_else(|e| e.into_inner()).contains(id));
            // 풀었다가 다시 묶은 길드는 PC 없는 동안 디스코드에서 바꾼 길드·챗봇 설정을 받는다. 못 받았으면 확인 응답을 미뤄 다음 하트비트에 다시 한다
            if !waiting
                && !(crate::service::discord::adopt_linked_guild(&self.server, guild).await
                    && crate::service::chatbot::adopt_linked_guild(&self.server, guild).await)
            {
                tracing::warn!("[RelayAgent] could not read the settings of relinked guild {}; trying again at the next heartbeat", guild);
                continue;
            }
            match store.update_node_settings(guild, |node| {
                let c = close_linked_guild(node, via, policy);
                (c, c)
            }).await {
                Ok(_) => {
                    if let Some(ack) = link_id {
                        let mut acks = self.linked_acks.lock().unwrap_or_else(|e| e.into_inner());
                        if !acks.contains(&ack) {
                            acks.push(ack);
                        }
                    }
                    applied = true;
                }
                Err(e) => tracing::warn!("[RelayAgent] could not set servers for new guild {}: {}", guild, e),
            }
        }
        applied
    }

    /// 릴레이 명령의 요청자. 노드 주인은 처음 받은 값으로 고정하고(`nodeSettings[guild].ownerDiscordId`),
    /// 다른 값이 오면 무시하고 경고를 남긴다
    async fn requester(&self, cmd: &Value, payload: &Value, guild_id: Option<String>, user_id: Option<String>) -> crate::chat::permissions::Requester {
        let reported = cmd.get("ownerDiscordId").and_then(Value::as_str);
        let owner_id = match guild_id.as_deref() {
            Some(guild) => {
                let pinned = self
                    .server
                    .config_store
                    .update_node_settings(guild, |node| {
                        let (owner, changed, conflict) = crate::chat::permissions::pin_owner(node, reported);
                        ((owner, conflict), changed)
                    })
                    .await;
                match pinned {
                    Ok((owner, conflict)) => {
                        if let Some(conflict) = conflict {
                            tracing::warn!("[RelayAgent] guild {}: relay reported a different owner ({}); keeping the pinned owner", guild, conflict);
                        }
                        owner
                    }
                    Err(e) => {
                        // 고정할 수 없으면 주인을 가정하지 않는다
                        tracing::warn!("[RelayAgent] could not read node settings for {}: {}", guild, e);
                        None
                    }
                }
            }
            // 길드 없는 요청(봇 DM `/설정`)은 릴레이가 확인한 이 PC의 주인(`hostOwnerDiscordId`)을 주인으로 본다
            None => cmd.get("hostOwnerDiscordId").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        };
        crate::chat::permissions::Requester {
            guild_id,
            user_id,
            owner_id,
            via_chatbot: crate::chat::permissions::via_chatbot(payload.get("via").and_then(Value::as_str), reported.is_some()),
            confirmed_by: payload.get("confirmedBy").and_then(Value::as_str).map(str::to_string),
            guild_owner_id: cmd.get("guildOwnerDiscordId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
        }
    }
}

/// 하트비트 응답의 새로 묶인 길드 하나(`linkedGuilds[]`, `via` `link`|`code`)에 열 서버를 정한다. 바꿨으면 true.
/// 코드 없이 묶였거나(`link`) PC 설정이 `manual`이면 아무 서버도 열지 않는다(`allowedInstances: []`, 나중에 고른다).
/// 이미 목록을 정해 둔 길드(다시 묶인 길드)는 그대로 둔다
pub(super) fn close_linked_guild(node: &mut serde_json::Map<String, Value>, via: &str, policy: crate::config_store::NewGuildInstances) -> bool {
    let manual = match via {
        "link" => true,
        "code" => policy == crate::config_store::NewGuildInstances::Manual,
        _ => false,
    };
    if !manual || node.contains_key("allowedInstances") {
        return false;
    }
    node.insert("allowedInstances".into(), json!([]));
    true
}

/// poll 응답 본문. 릴레이가 이모지를 반으로 잘라 짝 없는 서로게이트(`\ud83d`)가 오면 serde_json이
/// 본문 전체를 거부해 그 묶음의 명령이 다 사라진다: 짝 없는 서로게이트만 U+FFFD로 바꿔 다시 읽는다
pub(super) fn decode_poll(text: &str) -> Value {
    match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => match serde_json::from_str(&replace_lone_surrogates(text)) {
            Ok(v) => {
                tracing::warn!("[RelayAgent] poll body had broken text ({}); decoded with replacement characters", e);
                v
            }
            Err(e) => {
                tracing::warn!("[RelayAgent] poll body is not JSON ({}), {} bytes dropped", e, text.len());
                Value::Null
            }
        },
    }
}

/// JSON 문자열의 `\uXXXX` 중 짝이 없는 서로게이트를 `\ufffd`로
fn replace_lone_surrogates(text: &str) -> String {
    let hex = |s: &str| u16::from_str_radix(s, 16).ok();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        // 이스케이프 하나: `\\`는 건너뛴다
        if bytes.get(i + 1) != Some(&b'u') {
            i += 2;
            continue;
        }
        let Some(unit) = text.get(i + 2..i + 6).and_then(hex) else {
            i += 2;
            continue;
        };
        let low_next = || {
            (text.get(i + 6..i + 8) == Some("\\u")).then(|| text.get(i + 8..i + 12).and_then(hex)).flatten()
        };
        match unit {
            0xD800..=0xDBFF if low_next().is_some_and(|l| (0xDC00..=0xDFFF).contains(&l)) => i += 12,
            0xD800..=0xDFFF => {
                out.push_str(&text[last..i]);
                out.push_str("\\ufffd");
                i += 6;
                last = i;
            }
            _ => i += 6,
        }
    }
    out.push_str(&text[last..]);
    out
}
