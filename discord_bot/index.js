/**
 * 🚀 사바쨩 Discord Bot — 메인 진입점 (프로세스 관리)
 *
 * 역할:
 *   - Discord 클라이언트 생성 및 로그인
 *   - 프로세스 에러 핸들링
 *   - 부팅 시 모듈 초기화 오케스트레이션
 *   - 이벤트 ↔ 각 core 모듈 위임
 *
 * 아키텍처:
 *   index.js          진입점 · 프로세스 관리
 *   core/ipc.js       IPC 통신 (토큰, axios, API 래퍼)
 *   core/resolver.js  별명/매핑 통합 (botConfig, moduleMetadata)
 *   core/processor.js 명령어 해석 · 디스패치
 *   core/handler.js   봇 자체 기능 (익스텐션 파이프라인)
 */

const { Client, GatewayIntentBits } = require('discord.js');
const ipc = require('./core/ipc');
const resolver = require('./core/resolver');
const processor = require('./core/processor');

// ── Discord 클라이언트 ──
const client = new Client({
    intents: [
        GatewayIntentBits.Guilds,
        GatewayIntentBits.GuildMessages,
        GatewayIntentBits.MessageContent,
        GatewayIntentBits.GuildVoiceStates,   // 🎵 Music extension
    ],
});

// ── 프로세스 에러 핸들링 ──
process.on('unhandledRejection', (reason, promise) => {
    console.error('[Bot] Unhandled rejection at:', promise, 'reason:', reason);
});
process.on('uncaughtException', (error) => {
    console.error('[Bot] Uncaught exception:', error);
});

// ── 이벤트 등록 ──

// 메시지 → processor
client.on('messageCreate', (message) => processor.process(message));

// 슬래시 커맨드 (레거시 호환)
client.on('interactionCreate', async (interaction) => {
    if (!interaction.isChatInputCommand()) return;
    try {
        if (interaction.commandName === 'server') {
            const servers = await ipc.getServers();
            await interaction.reply({ content: JSON.stringify({ servers }, null, 2), ephemeral: true });
        }
    } catch (error) {
        const reply = { content: `Error: ${error.message}`, ephemeral: true };
        if (interaction.replied || interaction.deferred) {
            await interaction.followUp(reply).catch(() => {});
        } else {
            await interaction.reply(reply).catch(() => {});
        }
    }
});

// ── 부팅 시퀀스 ──
client.once('ready', async () => {
    console.log(`[Bot] Logged in as ${client.user.tag}`);

    // 1. IPC 토큰 · axios 인터셉터 초기화
    ipc.init();

    // 2. 봇 설정 + 모듈 메타데이터 로드
    await resolver.init();

    const cfg = resolver.getConfig();
    console.log(`[Bot] Prefix: ${cfg.prefix}`);
    console.log('[Bot] Ready');
});

// ── 로그인 ──
client.login(process.env.DISCORD_TOKEN);
