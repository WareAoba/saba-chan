/**
 * 🌐 릴레이 에이전트 — 릴레이 서버 ↔ 로컬 데몬 브릿지
 *
 * 역할:
 *   1. 릴레이 서버 GET /poll 롱폴링으로 명령어 수신
 *   2. 수신된 raw_command를 로컬 프로세서 체인으로 실행
 *   3. 실행 결과를 POST /result/:requestId로 릴레이 서버에 반환
 *   4. 주기적 POST /heartbeat 전송으로 온라인 상태 유지
 *
 * 필요 환경변수:
 *   RELAY_URL        — 릴레이 서버 주소 (예: http://localhost:3000)
 *   RELAY_NODE_TOKEN — 노드 인증 토큰 (호스트 등록 시 발급)
 *
 * 사용법:
 *   const relayAgent = require('./relayAgent');
 *   relayAgent.start();   // ipc/resolver 초기화 후 폴링 시작
 *   relayAgent.stop();    // 중지
 */

const ipc = require('./ipc');
const resolver = require('./resolver');
const processor = require('./processor');
const os = require('os');
const crypto = require('crypto');

// ── 설정 ──
const RELAY_URL = process.env.RELAY_URL || '';
const NODE_TOKEN = process.env.RELAY_NODE_TOKEN || '';
const HEARTBEAT_INTERVAL = 60_000;   // 60초
const POLL_RETRY_DELAY = 3_000;      // 폴링 실패 시 재시도 대기

let _running = false;
let _heartbeatTimer = null;
let _pollAbort = null;

// ── 토큰 파싱 ──
function parseToken(token) {
    const m = token.match(/^sbn_([A-Za-z0-9_-]+)\.(.+)$/);
    if (!m) return null;
    return { nodeId: m[1], secret: m[2] };
}

const _parsed = NODE_TOKEN ? parseToken(NODE_TOKEN) : null;

// ── 서명 유틸 ──

/**
 * authenticateNode 미들웨어가 요구하는 헤더를 생성합니다:
 *   Authorization: Bearer <token>
 *   x-request-timestamp: <unix seconds>
 *   x-request-signature: HMAC-SHA256(method + url + ts + body, secret)
 */
function signedHeaders(method, urlPath, body) {
    const ts = Math.floor(Date.now() / 1000);
    const bodyStr = body ? JSON.stringify(body) : '';
    const payload = [method.toUpperCase(), urlPath, ts.toString(), bodyStr].join('\n');
    const sig = crypto.createHmac('sha256', _parsed.secret).update(payload).digest('hex');

    return {
        'Authorization': `Bearer ${NODE_TOKEN}`,
        'Content-Type': 'application/json',
        'x-request-timestamp': String(ts),
        'x-request-signature': sig,
    };
}

function delay(ms) {
    return new Promise(r => setTimeout(r, ms));
}

// ── 하트비트 ──

async function sendHeartbeat() {
    try {
        let metadata;
        try {
            const servers = await ipc.getServers();
            const modules = await ipc.getModules();
            metadata = { servers, modules, moduleDetails: {} };
        } catch { metadata = undefined; }

        const hbBody = {
            agentVersion: '0.1.0',
            os: `${os.platform()} ${os.release()}`,
            metadata,
        };

        const res = await fetch(`${RELAY_URL}/heartbeat`, {
            method: 'POST',
            headers: signedHeaders('POST', '/heartbeat', hbBody),
            body: JSON.stringify(hbBody),
        });

        if (!res.ok) {
            const data = await res.json().catch(() => ({}));
            console.error(`[RelayAgent] Heartbeat failed (${res.status}):`, data.error || res.statusText);
        }
    } catch (e) {
        console.error('[RelayAgent] Heartbeat error:', e.message);
    }
}

// ── 목 메시지 팩토리 ──

function createMockMessage(text, requestedBy, guildId, channelId) {
    const replies = [];
    const botConfig = resolver.getConfig();
    const content = `${botConfig.prefix} ${text}`;

    const msg = {
        id: `relay-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        content,
        author: { bot: false, tag: 'relay-agent', id: requestedBy || 'system', username: 'relay-agent' },
        guildId: guildId || null,
        channel: { id: channelId || 'relay' },
        reply: async (textOrObj) => {
            const replyContent = typeof textOrObj === 'string' ? textOrObj : (textOrObj?.content ?? String(textOrObj));
            replies.push(replyContent);
            const idx = replies.length - 1;
            return {
                edit: async (editTextOrObj) => {
                    replies[idx] = typeof editTextOrObj === 'string' ? editTextOrObj : (editTextOrObj?.content ?? String(editTextOrObj));
                },
                delete: async () => {},
            };
        },
    };

    return { msg, getReplies: () => [...replies] };
}

// ── 명령어 처리 ──

async function processRelayCommand(commandPayload, requestedBy, guildId, channelId) {
    const { action, text } = commandPayload;

    if (action === 'raw_command' && text) {
        const { msg, getReplies } = createMockMessage(text, requestedBy, guildId, channelId);

        try {
            await processor.process(msg);
        } catch (e) {
            console.error('[RelayAgent] Processor error:', e.message);
            return { success: false, data: { error: e.message } };
        }

        const replies = getReplies();
        const resultText = replies.length > 0 ? replies[replies.length - 1] : '✅ 완료';
        return { success: true, data: { text: resultText } };
    }

    return { success: false, data: { error: `Unknown action: ${action}` } };
}

// ── 결과 전송 ──

async function postResult(requestId, result) {
    try {
        const urlPath = `/result/${encodeURIComponent(requestId)}`;
        const res = await fetch(`${RELAY_URL}${urlPath}`, {
            method: 'POST',
            headers: signedHeaders('POST', urlPath, result),
            body: JSON.stringify(result),
        });

        if (!res.ok) {
            const data = await res.json().catch(() => ({}));
            console.error(`[RelayAgent] POST result failed (${res.status}):`, data.error || res.statusText);
        }
    } catch (e) {
        console.error('[RelayAgent] POST result error:', e.message);
    }
}

// ── 폴링 루프 ──

async function pollLoop() {
    console.log('[RelayAgent] Poll loop started');

    while (_running) {
        try {
            _pollAbort = new AbortController();
            const res = await fetch(`${RELAY_URL}/poll`, {
                method: 'GET',
                headers: signedHeaders('GET', '/poll', null),
                signal: _pollAbort.signal,
            });

            if (!res.ok) {
                if (res.status === 204) {
                    continue; // 대기 명령 없음 — 즉시 재폴링
                }
                const data = await res.json().catch(() => ({}));
                console.error(`[RelayAgent] Poll failed (${res.status}):`, data.error || res.statusText);
                await delay(POLL_RETRY_DELAY);
                continue;
            }

            if (res.status === 204) {
                continue; // 대기 명령 없음
            }

            const body = await res.json();
            const commands = body.commands || [];

            if (commands.length === 0) {
                continue; // 타임아웃 — 즉시 재폴링
            }

            console.log(`[RelayAgent] Received ${commands.length} command(s)`);

            for (const cmd of commands) {
                const { id, payload, requestedBy, guildId, channelId } = cmd;
                console.log(`[RelayAgent] Processing: ${id}`, JSON.stringify(payload));

                const result = await processRelayCommand(
                    payload,
                    requestedBy,
                    guildId,
                    channelId,
                );

                await postResult(id, result);
                console.log(`[RelayAgent] Result posted: ${id} (success=${result.success})`);
            }
        } catch (e) {
            if (e.name === 'AbortError') {
                console.log('[RelayAgent] Poll aborted');
                break;
            }
            console.error('[RelayAgent] Poll error:', e.message);
            await delay(POLL_RETRY_DELAY);
        }
    }

    console.log('[RelayAgent] Poll loop stopped');
}

// ── 공개 API ──

/**
 * 릴레이 에이전트 시작.
 * ipc.init() / resolver.init() 은 호출자가 사전에 수행해야 합니다.
 */
async function start() {
    if (!RELAY_URL || !NODE_TOKEN) {
        console.log('[RelayAgent] RELAY_URL 또는 RELAY_NODE_TOKEN 미설정 — 에이전트 비활성');
        return false;
    }

    if (!_parsed) {
        console.error('[RelayAgent] RELAY_NODE_TOKEN 형식 오류 (sbn_<hostId>.<secret> 필요)');
        return false;
    }

    if (_running) {
        console.log('[RelayAgent] Already running');
        return true;
    }

    _running = true;

    // 초기 하트비트 (온라인 전환)
    await sendHeartbeat();
    _heartbeatTimer = setInterval(sendHeartbeat, HEARTBEAT_INTERVAL);

    // 폴링 루프 (비동기 — 중단 전까지 계속)
    pollLoop().catch(e => console.error('[RelayAgent] Fatal poll error:', e));

    console.log(`[RelayAgent] Started (relay=${RELAY_URL})`);
    return true;
}

/**
 * 릴레이 에이전트 중지
 */
function stop() {
    _running = false;

    if (_heartbeatTimer) {
        clearInterval(_heartbeatTimer);
        _heartbeatTimer = null;
    }

    if (_pollAbort) {
        _pollAbort.abort();
        _pollAbort = null;
    }

    console.log('[RelayAgent] Stopped');
}

/**
 * 에이전트 상태 조회
 */
function getStatus() {
    return {
        running: _running,
        relayUrl: RELAY_URL || null,
        hasToken: !!NODE_TOKEN,
    };
}

module.exports = { start, stop, getStatus };
