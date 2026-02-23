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
            GatewayIntentBits.GuildMessages,
            GatewayIntentBits.MessageContent,
            GatewayIntentBits.GuildVoiceStates,
        ],
    });

    // 메시지 → processor
    client.on('messageCreate', (message) => processor.process(message));

    // 부팅 시퀀스
    client.once('ready', async () => {
        console.log(`[Bot] Logged in as ${client.user.tag}`);

        ipc.init();
        await resolver.init();

        const cfg = resolver.getConfig();
        console.log(`[Bot] Prefix: ${cfg.prefix}`);
        console.log('[Bot] Ready (local mode)');
    });

    process.on('SIGINT', () => { client.destroy(); process.exit(0); });
    process.on('SIGTERM', () => { client.destroy(); process.exit(0); });

    client.login(process.env.DISCORD_TOKEN);
}
