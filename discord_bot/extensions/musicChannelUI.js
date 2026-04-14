/**
 * 🎵 Music Channel UI — 전용 텍스트 채널 실시간 UI
 *
 * 디스코드 텍스트 채널 하나를 점유하여 "실시간 인터랙션 UI"처럼 동작합니다.
 *   - 현재 재생 중인 곡 제목 + 텍스트 진행바 (3~5초 간격 갱신)
 *   - 다음 곡 큐 표시 (사용자 설정 5~10줄)
 *   - 컨트롤 버튼 (볼륨, 이전곡/다음곡, 탐색, 재생/일시정지)
 *   - 전용 채널에서는 prefix 없이 바로 음악 명령어/검색어 입력 가능
 */

const i18n = require('../i18n');
const { ActionRowBuilder, ButtonBuilder, ButtonStyle } = require('discord.js');

// ── 길드별 채널 UI 상태 ──
const guildChannelUI = new Map();

// ── 기본 설정 ──
const DEFAULT_SETTINGS = {
    queueLines: 5,           // 큐 표시 줄 수 (5~10)
    refreshInterval: 4000,   // 진행바 갱신 간격 (ms)
};

/**
 * 초 → "M:SS" 형식 변환
 */
function formatTime(seconds) {
    if (!seconds || seconds < 0) return '0:00';
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
}

/**
 * "M:SS" 또는 "H:MM:SS" → 초 변환
 */
function parseDuration(str) {
    if (!str || str === '??:??') return 0;
    const parts = str.split(':').map(Number);
    if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
    if (parts.length === 2) return parts[0] * 60 + parts[1];
    return 0;
}

/**
 * 텍스트 진행바 생성
 * @param {number} current - 현재 위치 (초)
 * @param {number} total - 전체 길이 (초)
 * @param {number} width - 바 너비 (문자 수)
 * @returns {string}
 */
function buildProgressBar(current, total, width = 20) {
    if (total <= 0) return '▬'.repeat(width);
    const ratio = Math.min(current / total, 1);
    const filled = Math.round(ratio * width);
    const empty = width - filled;
    return '▰'.repeat(filled) + '▱'.repeat(empty);
}

/**
 * 현재 재생 상태 + 큐를 하나의 텍스트로 빌드
 * @param {object} queue - guildQueue 객체
 * @param {object} settings - UI 설정
 * @returns {string}
 */
function buildNowPlayingText(queue, settings) {
    const lines = [];

    if (!queue || !queue.current) {
        lines.push('```');
        lines.push(i18n.t('bot:music_ui.idle_title'));
        lines.push('');
        lines.push(i18n.t('bot:music_ui.idle_hint'));
        lines.push('```');
        return lines.join('\n');
    }

    const track = queue.current;
    const totalSec = parseDuration(track.duration);
    // _startedAt가 null이면 아직 재생 시작 전 → 0초
    const elapsedSec = (queue._startedAt && !queue._paused)
        ? Math.floor((Date.now() - queue._startedAt) / 1000)
        : 0;
    // 일시정지 상태면 _pausedElapsed 사용
    const currentSec = queue._paused
        ? (queue._pausedElapsed || 0)
        : Math.min(elapsedSec, totalSec > 0 ? totalSec : elapsedSec);

    const bar = buildProgressBar(currentSec, totalSec);
    const timeStr = `${formatTime(currentSec)} / ${track.duration || '??:??'}`;

    const loopIcon = queue.loop ? ' 🔂' : '';
    const radioIcon = queue.radio ? ' 📻' : '';
    const normIcon = queue.normalize ? ' 🔊' : '';
    const volIcon = queue.volume === 0 ? '🔇' : queue.volume < 0.5 ? '🔉' : '🔊';

    lines.push('```ansi');
    lines.push(`♪ ${track.title}${loopIcon}${radioIcon}${normIcon}`);
    lines.push(`  ${bar}  ${timeStr}`);
    lines.push(`  ${volIcon} ${Math.round(queue.volume * 100)}%  |  👤 ${track.requester}`);
    lines.push('```');

    return lines.join('\n');
}

/**
 * 큐 텍스트 빌드
 */
function buildQueueText(queue, settings) {
    const maxLines = settings.queueLines || DEFAULT_SETTINGS.queueLines;
    const lines = [];

    lines.push(`📋 **${i18n.t('bot:music_ui.queue_header')}**`);

    if (!queue || queue.tracks.length === 0) {
        lines.push(`> ${i18n.t('bot:music_ui.queue_empty')}`);
        // 빈 줄로 패딩하여 높이 유지
        for (let i = 0; i < maxLines - 1; i++) {
            lines.push('> ');
        }
        return lines.join('\n');
    }

    const display = queue.tracks.slice(0, maxLines);
    display.forEach((track, idx) => {
        const num = `${idx + 1}`.padStart(2, ' ');
        const radioPrefix = (track.requester === '📻 Radio') ? '📻 ' : '';
        lines.push(`> \`${num}.\` ${radioPrefix}**${track.title}** [${track.duration}] — ${track.requester}`);
    });

    // 나머지 줄 패딩
    for (let i = display.length; i < maxLines; i++) {
        lines.push('> ');
    }

    if (queue.tracks.length > maxLines) {
        lines.push(`> *...${i18n.t('bot:music_ui.queue_more', { count: queue.tracks.length - maxLines })}*`);
    }

    return lines.join('\n');
}

/**
 * 컨트롤 버튼 행 빌드 — 큐 메시지 아래에 표시
 * Row 1: 🔉 ⏮ ⏯ ⏭ 🔊
 */
function buildControlButtons(queue) {
    const isPaused = queue?._paused || false;
    const isRadio = queue?.radio || false;
    const isNormalize = queue?.normalize ?? true;

    const row1 = new ActionRowBuilder().addComponents(
        new ButtonBuilder()
            .setCustomId('music_vol_down')
            .setLabel('🔉 -')
            .setStyle(ButtonStyle.Secondary),
        new ButtonBuilder()
            .setCustomId('music_prev')
            .setLabel('⏮')
            .setStyle(ButtonStyle.Primary),
        new ButtonBuilder()
            .setCustomId('music_pause_resume')
            .setLabel(isPaused ? '▶' : '⏸')
            .setStyle(isPaused ? ButtonStyle.Success : ButtonStyle.Primary),
        new ButtonBuilder()
            .setCustomId('music_next')
            .setLabel('⏭')
            .setStyle(ButtonStyle.Primary),
        new ButtonBuilder()
            .setCustomId('music_vol_up')
            .setLabel('🔊 +')
            .setStyle(ButtonStyle.Secondary),
    );

    const row2 = new ActionRowBuilder().addComponents(
        new ButtonBuilder()
            .setCustomId('music_radio')
            .setLabel(isRadio ? '📻 Radio ON' : '📻 Radio')
            .setStyle(isRadio ? ButtonStyle.Success : ButtonStyle.Secondary),
        new ButtonBuilder()
            .setCustomId('music_normalize')
            .setLabel(isNormalize ? '🔊 Normalize ON' : '🔇 Normalize')
            .setStyle(isNormalize ? ButtonStyle.Success : ButtonStyle.Secondary),
    );

    return [row1, row2];
}

// ══════════════════════════════════════
// ── Channel UI 인스턴스 관리 ──
// ══════════════════════════════════════

/**
 * 전용 채널 UI 시작 — 메시지 2개를 전송하고 갱신 루프 시작
 * @param {string} guildId
 * @param {TextChannel} channel - 전용 채널 객체
 * @param {Function} getQueueFn - () => queue 객체를 반환하는 함수
 * @param {object} [settings] - UI 설정
 */
async function startChannelUI(guildId, channel, getQueueFn, settings = {}) {
    // 이미 실행 중이면 정리 후 재시작
    stopChannelUI(guildId);

    const merged = { ...DEFAULT_SETTINGS, ...settings };
    const uiState = {
        channel,
        getQueueFn,
        settings: merged,
        nowPlayingMsg: null,
        queueMsg: null,
        refreshTimer: null,
        destroyed: false,
        _refreshing: false, // M5: refresh race condition 방지
    };

    guildChannelUI.set(guildId, uiState);

    try {
        // 기존 메시지 정리 — 채널을 깔끔하게 유지
        await cleanupOldMessages(channel);

        // 메시지 2개 전송: 현재 재생 + 큐
        const queue = getQueueFn();
        const npText = buildNowPlayingText(queue, merged);
        const qText = buildQueueText(queue, merged);

        uiState.nowPlayingMsg = await channel.send(npText);
        const buttons = buildControlButtons(queue);
        uiState.queueMsg = await channel.send({ content: qText, components: buttons });

        // 갱신 루프 시작
        uiState.refreshTimer = setInterval(() => {
            refreshUI(guildId).catch((e) => {
                console.warn(`[MusicUI] Refresh error (${guildId}):`, e.message);
            });
        }, merged.refreshInterval);

        console.log(`[MusicUI] Started for guild ${guildId} in #${channel.name}`);
    } catch (e) {
        console.error(`[MusicUI] Failed to start for guild ${guildId}:`, e.message);
        stopChannelUI(guildId);
    }
}

/**
 * 전용 채널 UI 정지
 */
function stopChannelUI(guildId) {
    const uiState = guildChannelUI.get(guildId);
    if (!uiState) return;

    uiState.destroyed = true;
    if (uiState.refreshTimer) {
        clearInterval(uiState.refreshTimer);
        uiState.refreshTimer = null;
    }
    guildChannelUI.delete(guildId);
    console.log(`[MusicUI] Stopped for guild ${guildId}`);
}

/**
 * UI 갱신 — 진행바 + 큐 텍스트 수정
 * M5: race condition 방지 — 이전 refresh 완료 전에 다음 refresh 스킵
 */
async function refreshUI(guildId) {
    const uiState = guildChannelUI.get(guildId);
    if (!uiState || uiState.destroyed) return;
    if (uiState._refreshing) return; // 이전 호출 진행 중 → 스킵

    uiState._refreshing = true;
    try {
        const queue = uiState.getQueueFn();
        const settings = uiState.settings;

        // 현재 재생 메시지 갱신
        if (uiState.nowPlayingMsg) {
            const npText = buildNowPlayingText(queue, settings);
            try {
                await uiState.nowPlayingMsg.edit(npText);
            } catch (e) {
                // 메시지가 삭제된 경우 → UI 복구 시도
                if (e.code === 10008) {
                    console.warn(`[MusicUI] NowPlaying message deleted, recreating...`);
                    try {
                        uiState.nowPlayingMsg = await uiState.channel.send(npText);
                    } catch (sendErr) {
                        // 권한 부족 등 — 무한 재시도 방지
                        console.error(`[MusicUI] Failed to recreate NowPlaying message: ${sendErr.message}`);
                    }
                }
            }
        }
    } finally {
        uiState._refreshing = false;
    }
}

/**
 * 강제 UI 갱신 — _refreshing 가드를 무시하고 즉시 갱신
 * 곡 변경, 큐 업데이트 등 이벤트 구동 갱신 시 사용
 */
async function forceRefreshUI(guildId) {
    const uiState = guildChannelUI.get(guildId);
    if (!uiState || uiState.destroyed) return;

    const queue = uiState.getQueueFn();
    const settings = uiState.settings;

    if (uiState.nowPlayingMsg) {
        const npText = buildNowPlayingText(queue, settings);
        try {
            await uiState.nowPlayingMsg.edit(npText);
        } catch (e) {
            if (e.code === 10008) {
                console.warn(`[MusicUI] NowPlaying message deleted (force), recreating...`);
                try {
                    uiState.nowPlayingMsg = await uiState.channel.send(npText);
                } catch (sendErr) {
                    console.error(`[MusicUI] Failed to recreate NowPlaying message: ${sendErr.message}`);
                }
            }
        }
    }
}

/**
 * 큐가 변경되었을 때 호출 — 큐 메시지 + NowPlaying 즉시 갱신 (강제)
 */
async function refreshQueue(guildId) {
    const uiState = guildChannelUI.get(guildId);
    if (!uiState || uiState.destroyed) return;

    const queue = uiState.getQueueFn();
    const settings = uiState.settings;

    if (uiState.queueMsg) {
        const qText = buildQueueText(queue, settings);
        const buttons = buildControlButtons(queue);
        try {
            await uiState.queueMsg.edit({ content: qText, components: buttons });
        } catch (e) {
            if (e.code === 10008) {
                console.warn(`[MusicUI] Queue message deleted, recreating...`);
                try {
                    uiState.queueMsg = await uiState.channel.send({ content: qText, components: buttons });
                } catch (_) {}
            }
        }
    }

    // NowPlaying도 즉시 강제 갱신 (곡 변경 / 큐 업데이트 시)
    await forceRefreshUI(guildId);
}

/**
 * 채널 내 모든 메시지 정리 — 전용 채널을 깨끗하게 비운 뒤 UI만 남긴다
 * bulkDelete는 14일 이내 메시지만 삭제 가능.
 * 오래된 메시지는 속도 문제로 최소한만 개별 삭제하고, 나머지는 무시한다.
 */
async function cleanupOldMessages(channel) {
    const MAX_ROUNDS = 3;       // bulkDelete 최대 반복
    const OLD_DELETE_LIMIT = 5; // 오래된 메시지 개별 삭제 제한
    let totalDeleted = 0;

    try {
        for (let round = 0; round < MAX_ROUNDS; round++) {
            const fetched = await channel.messages.fetch({ limit: 100 });
            if (fetched.size === 0) break;

            console.log(`[MusicUI] Cleanup round ${round + 1}: ${fetched.size} messages`);

            // bulkDelete — 14일 이내 메시지만 삭제 (filterOld=true)
            try {
                const deleted = await channel.bulkDelete(fetched, true);
                totalDeleted += deleted.size;
                console.log(`[MusicUI] Bulk deleted ${deleted.size} messages`);

                // 모든 메시지가 삭제 안 됐으면 (전부 14일 이상 오래된 경우)
                // 소량의 오래된 메시지만 개별 삭제하고 중단
                if (deleted.size === 0) {
                    console.log(`[MusicUI] All messages are >14 days old, deleting up to ${OLD_DELETE_LIMIT} individually`);
                    let oldDeleted = 0;
                    for (const [, msg] of fetched) {
                        if (oldDeleted >= OLD_DELETE_LIMIT) break;
                        try {
                            await msg.delete();
                            oldDeleted++;
                            totalDeleted++;
                        } catch (_) { break; } // 권한 부족 시 즉시 중단
                    }
                    break; // 오래된 메시지 정리는 여기서 끝
                }
            } catch (e) {
                console.warn(`[MusicUI] Bulk delete failed:`, e.message);
                break; // 권한 부족 등 — 클린업 중단
            }

            if (fetched.size < 100) break; // 더 이상 없음
        }

        console.log(`[MusicUI] Cleanup complete: ${totalDeleted} messages deleted`);
    } catch (e) {
        console.warn(`[MusicUI] Cleanup error:`, e.message);
    }
}

/**
 * 유저 입력 메시지를 조용히 삭제 (전용 채널 청결 유지)
 */
async function deleteUserMessage(message) {
    if (message.deletable) {
        try { await message.delete(); } catch (_) {}
    }
}

/**
 * 채널 UI 활성 여부
 */
function isUIActive(guildId) {
    return guildChannelUI.has(guildId);
}

/**
 * 해당 메시지가 전용 음악 채널에서 온 것인지 확인
 */
function isMusicChannel(guildId, channelId, botConfig) {
    const musicChannelId = botConfig?.musicChannelId;
    if (!musicChannelId) return false;
    // 길드별 설정도 지원 (object인 경우)
    if (typeof musicChannelId === 'object') {
        return musicChannelId[guildId] === channelId;
    }
    return musicChannelId === channelId;
}

/**
 * 전용 채널 설정 가져오기
 */
function getMusicChannelSettings(botConfig) {
    return {
        queueLines: botConfig?.musicUISettings?.queueLines || DEFAULT_SETTINGS.queueLines,
        refreshInterval: botConfig?.musicUISettings?.refreshInterval || DEFAULT_SETTINGS.refreshInterval,
    };
}

/**
 * 모든 길드의 채널 UI를 정지
 */
function stopAllChannelUI() {
    for (const guildId of [...guildChannelUI.keys()]) {
        stopChannelUI(guildId);
    }
}

/**
 * 활성 UI의 텍스트 채널 반환 — 에러 알림 등 외부에서 채널 참조가 필요할 때 사용
 * @param {string} guildId
 * @returns {TextChannel|null}
 */
function getChannel(guildId) {
    const uiState = guildChannelUI.get(guildId);
    if (!uiState || uiState.destroyed) return null;
    return uiState.channel || null;
}

module.exports = {
    startChannelUI,
    stopChannelUI,
    stopAllChannelUI,
    refreshUI,
    forceRefreshUI,
    refreshQueue,
    buildControlButtons,
    deleteUserMessage,
    isUIActive,
    isMusicChannel,
    getMusicChannelSettings,
    getChannel,
    DEFAULT_SETTINGS,
    // 테스트용 내부 함수 노출
    _test: {
        buildProgressBar,
        buildNowPlayingText,
        buildQueueText,
        parseDuration,
        formatTime,
    },
};
