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

async function handleIpcMessage(msg, client) {
    const id = msg.id || null;
    try {
        switch (msg.type) {
            case 'getGuildMembers': {
                if (!client || !client.isReady()) {
                    sendIpcResponse({ id, type: 'guildMembers', error: 'BOT_NOT_READY', data: {} });
                    return;
                }
                const result = {};
                for (const [guildId, guild] of client.guilds.cache) {
                    try {
                        // fetch() 로 전체 멤버 목록 확보 (캐시만으로는 부족)
                        const fetched = await guild.members.fetch();
                        result[guildId] = {
                            guildName: guild.name,
                            members: fetched
                                .filter(m => !m.user.bot)
                                .map(m => ({
                                    id: m.user.id,
                                    username: m.user.username,
                                    displayName: m.displayName || m.user.username,
                                })),
                        };
                    } catch (e) {
                        console.warn(`[Bot:IPC] Failed to fetch members for guild ${guildId}:`, e.message);
                        result[guildId] = { guildName: guild.name, members: [] };
                    }
                }
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
                handleIpcMessage(msg, client);
            } catch (e) {
                console.error('[Bot:IPC] Invalid JSON on stdin:', e.message);
            }
        }
    });
    process.stdin.on('error', () => {}); // stdin 닫힘 무시

    // 메시지 → processor
    client.on('messageCreate', (message) => processor.process(message));

    // Discord 클라이언트 에러 핸들링
    client.on('error', (err) => {
        console.error('[Bot] Discord client error:', err.message);
    });
    client.on('warn', (info) => {
        console.warn('[Bot] Discord client warning:', info);
    });

    // 부팅 시퀀스
    client.once('ready', async () => {
        console.log(`[Bot] Logged in as ${client.user.tag}`);

        ipc.init();
        try {
            await resolver.init();
        } catch (e) {
            console.error('[Bot] Resolver init failed — commands may not work:', e.message);
        }

        const cfg = resolver.getConfig();
        console.log(`[Bot] Prefix: ${cfg.prefix}`);
        console.log('[Bot] Ready (local mode)');
    });

    process.on('SIGINT', () => { client.destroy(); process.exit(0); });
    process.on('SIGTERM', () => { client.destroy(); process.exit(0); });

    client.login(process.env.DISCORD_TOKEN).catch(e => {
        console.error('[Bot] Login failed:', e.message);
        process.exit(1);
    });
}
