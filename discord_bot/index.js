/**
 * 🚀 사바쨩 Discord Bot — 메인 진입점
 *
 * 동작 모드:
 *   A) 로컬 모드 (기본)  — Discord 로그인 + 메시지 처리
 *   B) 릴레이 에이전트 모드 — Discord 로그인 없이 릴레이 서버 폴링만
 *      (RELAY_URL + RELAY_NODE_TOKEN 설정 시 자동 전환)
 *
 * 아키텍처:
 *   index.js          진입점 · 프로세스 관리
 *   core/ipc.js       IPC 통신 (토큰, axios, API 래퍼)
 *   core/resolver.js  별명/매핑 통합 (botConfig, moduleMetadata)
 *   core/processor.js 명령어 해석 · 디스패치
 *   core/handler.js   봇 자체 기능 (익스텐션 파이프라인)
 *   core/relayAgent.js 릴레이 서버 ↔ 로컬 데몬 브릿지
 */

const ipc = require('./core/ipc');
const resolver = require('./core/resolver');
const processor = require('./core/processor');
const relayAgent = require('./core/relayAgent');

// ── 릴레이 에이전트 모드 판별 ──
const RELAY_AGENT_MODE = !!(process.env.RELAY_URL && process.env.RELAY_NODE_TOKEN);

// ── GUI ↔ 봇 IPC 메시지 핸들러 (로컬 모드 전용) ──
function sendIpcResponse(data) {
    // stdout에 __IPC__ 접두사로 JSON 전송 (일반 로그와 구분)
    process.stdout.write('__IPC__:' + JSON.stringify(data) + '\n');
}

// ── Guild member 캐시 (rate limit 방지) ──
// IPC 요청 시에는 항상 discord.js 내부 캐시만 사용하고,
// Gateway fetch는 봇 startup 시 한 번만 수행합니다.
let _guildMembersCache = null;
let _guildMembersCacheTime = 0;
const GUILD_MEMBERS_CACHE_TTL = 120_000; // 120초
let _initialFetchDone = false;

function _membersToList(members) {
    return members
        .filter(m => !m.user.bot)
        .map(m => ({
            id: m.user.id,
            username: m.user.username,
            displayName: m.displayName || m.user.username,
        }));
}

/**
 * 봇 ready 직후 한 번만 호출 — guild별 순차 fetch로 rate limit 회피.
 * GuildMembers intent 덕분에 이후 업데이트는 이벤트로 자동 캐시됩니다.
 */
async function prefetchGuildMembers(client) {
    const guilds = [...client.guilds.cache.values()];
    for (const guild of guilds) {
        try {
            await guild.members.fetch();
            console.log(`[Bot] Prefetched ${guild.members.cache.size} members for guild ${guild.id} (${guild.name})`);
        } catch (e) {
            console.warn(`[Bot] Prefetch failed for guild ${guild.id}: ${e.message}`);
            // rate limit 시 retry_after만큼 대기 후 재시도 1회
            const retryMatch = e.message.match(/[Rr]etry after ([\d.]+)/);
            if (retryMatch) {
                const waitMs = Math.ceil(parseFloat(retryMatch[1]) * 1000) + 1000;
                console.log(`[Bot] Waiting ${Math.ceil(waitMs / 1000)}s before retry...`);
                await new Promise(r => setTimeout(r, waitMs));
                try {
                    await guild.members.fetch();
                    console.log(`[Bot] Prefetch retry OK for guild ${guild.id} (${guild.members.cache.size} members)`);
                } catch (e2) {
                    console.warn(`[Bot] Prefetch retry also failed for guild ${guild.id}: ${e2.message}`);
                }
            }
        }
        // guild간 1초 간격으로 rate limit 여유 확보
        if (guilds.length > 1) {
            await new Promise(r => setTimeout(r, 1000));
        }
    }
    _initialFetchDone = true;
    console.log('[Bot] Guild member prefetch complete');
}

async function handleIpcMessage(msg, client) {
    const id = msg.id || null;
    try {
        switch (msg.type) {
            case 'getGuildMembers': {
                if (!client || !client.isReady()) {
                    sendIpcResponse({ id, type: 'guildMembers', error: 'BOT_NOT_READY', data: {} });
                    return;
                }
                // 직렬화 캐시 유효하면 재사용
                const now = Date.now();
                if (_guildMembersCache && (now - _guildMembersCacheTime) < GUILD_MEMBERS_CACHE_TTL) {
                    sendIpcResponse({ id, type: 'guildMembers', data: _guildMembersCache });
                    return;
                }
                // discord.js 내부 캐시만 읽기 — gateway fetch 절대 안함
                const result = {};
                for (const [guildId, guild] of client.guilds.cache) {
                    const members = guild.members.cache;
                    result[guildId] = {
                        guildName: guild.name,
                        members: _membersToList(members),
                    };
                }
                _guildMembersCache = result;
                _guildMembersCacheTime = now;
                sendIpcResponse({ id, type: 'guildMembers', data: result });
                break;
            }
            default:
                sendIpcResponse({ id, type: 'error', error: 'UNKNOWN_TYPE', message: `Unknown IPC type: ${msg.type}` });
        }
    } catch (e) {
        sendIpcResponse({ id, type: 'error', error: 'HANDLER_ERROR', message: e.message });
    }
}

// ── 프로세스 에러 핸들링 ──
process.on('unhandledRejection', (reason, promise) => {
    console.error('[Bot] Unhandled rejection at:', promise, 'reason:', reason);
});
process.on('uncaughtException', (error) => {
    console.error('[Bot] Uncaught exception:', error);
});

if (RELAY_AGENT_MODE) {
    // ═══════════════════════════════════════════
    //  모드 B: 릴레이 에이전트 (Discord 로그인 없음)
    // ═══════════════════════════════════════════
    (async () => {
        console.log('[Bot] Relay agent mode — Discord 로그인 생략');

        // 1. IPC 초기화
        ipc.init();

        // 2. 봇 설정 + 모듈 메타데이터 로드
        await resolver.init();

        const cfg = resolver.getConfig();
        console.log(`[Bot] Prefix: ${cfg.prefix}`);

        // 3. 릴레이 에이전트 시작
        const started = await relayAgent.start();
        if (!started) {
            console.error('[Bot] Relay agent failed to start');
            process.exit(1);
        }

        console.log('[Bot] Relay agent ready');
    })().catch(e => {
        console.error('[Bot] Fatal:', e);
        process.exit(1);
    });

    process.on('SIGINT', () => { relayAgent.stop(); process.exit(0); });
    process.on('SIGTERM', () => { relayAgent.stop(); process.exit(0); });

} else {
    // ═══════════════════════════════════════════
    //  모드 A: 로컬 모드 (Discord 클라이언트)
    // ═══════════════════════════════════════════
    const { Client, GatewayIntentBits } = require('discord.js');

    const client = new Client({
        intents: [
            GatewayIntentBits.Guilds,
            GatewayIntentBits.GuildMembers,
            GatewayIntentBits.GuildMessages,
            GatewayIntentBits.MessageContent,
            GatewayIntentBits.GuildVoiceStates,
        ],
    });

    // ── stdin JSON IPC (GUI ↔ 봇 프로세스 양방향 통신) ──
    let stdinBuf = '';
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (chunk) => {
        stdinBuf += chunk;
        let nlIdx;
        while ((nlIdx = stdinBuf.indexOf('\n')) !== -1) {
            const line = stdinBuf.slice(0, nlIdx).trim();
            stdinBuf = stdinBuf.slice(nlIdx + 1);
            if (!line) continue;
            try {
                const msg = JSON.parse(line);
                // GUI에서 종료 요청 시 graceful shutdown
                if (msg.type === 'shutdown') {
                    console.log('[Bot] Shutdown requested via IPC');
                    client.destroy();
                    process.exit(0);
                    return;
                }
                handleIpcMessage(msg, client);
            } catch (e) {
                console.error('[Bot:IPC] Invalid JSON on stdin:', e.message);
            }
        }
    });
    process.stdin.on('error', () => {}); // stdin 닫힘 무시

    // 메시지 → processor
    client.on('messageCreate', (message) => processor.process(message));

    // 음성 채널 상태 변경 → 음악 봇 자동 퇴장 (채널 비면)
    const musicExtension = require('./extensions/music');
    client.on('voiceStateUpdate', (oldState, newState) => {
        try {
            musicExtension.handleVoiceStateUpdate(oldState, newState);
        } catch (e) {
            console.error('[Bot] voiceStateUpdate handler error:', e.message);
        }
    });

    // Discord 클라이언트 에러 핸들링
    client.on('error', (err) => {
        console.error('[Bot] Discord client error:', err.message);
    });
    client.on('warn', (info) => {
        console.warn('[Bot] Discord client warning:', info);
    });

    // 부팅 시퀀스
    client.once('clientReady', async () => {
        console.log(`[Bot] Logged in as ${client.user.tag}`);

        ipc.init();
        try {
            await resolver.init();
        } catch (e) {
            console.error('[Bot] Resolver init failed — commands may not work:', e.message);
        }

        const cfg = resolver.getConfig();
        console.log(`[Bot] Prefix: ${cfg.prefix}`);

        // Guild 멤버를 startup 시 한 번만 prefetch (비차단)
        prefetchGuildMembers(client).catch(e => {
            console.warn('[Bot] Guild prefetch error:', e.message);
        });

        console.log('[Bot] Ready (local mode)');
    });

    process.on('SIGINT', () => { client.destroy(); process.exit(0); });
    process.on('SIGTERM', () => { client.destroy(); process.exit(0); });

    client.login(process.env.DISCORD_TOKEN).catch(e => {
        console.error('[Bot] Login failed:', e.message);
        process.exit(1);
    });
}
