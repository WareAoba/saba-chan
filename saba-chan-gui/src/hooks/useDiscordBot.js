import { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast, createTranslateError } from '../utils/helpers';

/**
 * Manages Discord bot status polling, start/stop, auto-start, and bot relaunch.
 *
 * @param {Object} params
 * @param {string} params.discordToken
 * @param {string} params.discordPrefix
 * @param {boolean} params.discordAutoStart
 * @param {Object} params.discordModuleAliases
 * @param {Object} params.discordCommandAliases
 * @param {string} params.discordBotMode - 'local' | 'cloud'
 * @param {string} params.discordCloudRelayUrl
 * @param {string} params.discordCloudHostId
 * @param {boolean} params.settingsReady
 * @param {React.MutableRefObject<string>} params.discordTokenRef
 * @param {Function} params.setModal
 * @returns {Object} Bot status and handlers
 */
export function useDiscordBot({
    discordToken,
    discordPrefix,
    discordAutoStart,
    discordModuleAliases,
    discordCommandAliases,
    discordBotMode,
    discordCloudRelayUrl,
    discordCloudHostId,
    settingsReady,
    discordTokenRef,
    setModal,
}) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    const [discordBotStatus, setDiscordBotStatus] = useState('stopped');
    const [botStatusReady, setBotStatusReady] = useState(false);
    const autoStartDoneRef = useRef(false);

    // ── Status polling ──────────────────────────────────────
    // 클라우드 모드: 릴레이 서버 /api/hosts/{hostId} 확인
    // 로컬 모드: window.api.discordBotStatus() 확인
    const DEFAULT_RELAY_URL = 'http://localhost:3000';

    useEffect(() => {
        let mounted = true;

        const checkStatus = async () => {
            if (discordBotMode === 'cloud') {
                // ── 클라우드 모드: 로컬 릴레이 에이전트 프로세스 상태 확인 ──
                try {
                    const status = await window.api.discordBotStatus();
                    if (mounted) setDiscordBotStatus(status === 'running' ? 'running' : 'stopped');
                } catch {
                    if (mounted) setDiscordBotStatus('stopped');
                }
            } else {
                // ── 로컬 모드: 로컬 봇 프로세스 상태 ──
                try {
                    const status = await window.api.discordBotStatus();
                    if (mounted) {
                        const botRunning = status === 'running';
                        setDiscordBotStatus(botRunning ? 'running' : 'stopped');
                    }
                } catch {
                    if (mounted) setDiscordBotStatus('stopped');
                }
            }
        };

        const init = async () => {
            await new Promise(resolve => setTimeout(resolve, 200));
            await checkStatus();
            if (mounted) {
                setBotStatusReady(true);
                console.log('[Init] BotStatusReady flag set to true, mode:', discordBotMode);
            }
        };

        init();

        const interval = setInterval(checkStatus, 5000);

        return () => {
            mounted = false;
            clearInterval(interval);
        };
    }, [discordBotMode, discordCloudRelayUrl, discordCloudHostId]);

    // ── Start bot ───────────────────────────────────────────
    const handleStartDiscordBot = async () => {
        const isCloud = discordBotMode === 'cloud';

        // 로컬 모드에서만 Discord 토큰 필수
        if (!isCloud && !discordToken) {
            setModal({ type: 'failure', title: t('discord_bot.token_missing_title'), message: t('discord_bot.token_missing_message') });
            return;
        }
        if (!discordPrefix) {
            setModal({ type: 'failure', title: t('discord_bot.prefix_missing_title'), message: t('discord_bot.prefix_missing_message') });
            return;
        }
        try {
            const botConfig = {
                token: discordToken,
                prefix: discordPrefix,
                moduleAliases: discordModuleAliases,
                commandAliases: discordCommandAliases,
                mode: discordBotMode || 'local',
                cloud: {
                    relayUrl: discordCloudRelayUrl || '',
                    hostId: discordCloudHostId || '',
                },
            };
            const result = await window.api.discordBotStart(botConfig);
            if (result.error) {
                safeShowToast(t('discord_bot.start_failed_toast', { error: translateError(result.error) }), 'error', 4000);
            } else {
                setDiscordBotStatus('running');
                safeShowToast(t('discord_bot.started_toast'), 'discord', 3000, { isNotice: true, source: 'Discord Bot' });
            }
        } catch (e) {
            safeShowToast(t('discord_bot.start_error_toast', { error: translateError(e.message) }), 'error', 4000);
        }
    };

    // ── Auto-start (when both settings and bot status are ready) ─
    useEffect(() => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        if (!isTest) {
            console.log('[Auto-start] Effect triggered', {
                botStatusReady,
                settingsReady,
                autoStartDone: autoStartDoneRef.current,
                discordAutoStart,
                tokenExists: !!discordToken,
                prefixExists: !!discordPrefix,
                botStatus: discordBotStatus
            });
        }

        if (botStatusReady && settingsReady && !autoStartDoneRef.current) {
            autoStartDoneRef.current = true;
            // ★ 클라우드 모드: hostId가 있으면 릴레이 에이전트 자동시작
            if (discordBotMode === 'cloud') {
                if (discordCloudHostId && discordPrefix && discordBotStatus === 'stopped') {
                    if (!isTest) console.log('[Auto-start] Cloud mode — starting relay agent');
                    handleStartDiscordBot();
                } else {
                    if (!isTest) console.log('[Auto-start] Cloud mode — setup incomplete, skipping');
                }
            } else if (discordAutoStart && discordToken && discordPrefix && discordBotStatus === 'stopped') {
                if (!isTest) console.log('[Auto-start] Starting Discord bot automatically!');
                handleStartDiscordBot();
            }
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [botStatusReady, settingsReady, discordAutoStart, discordToken, discordPrefix, discordBotStatus]);

    // ── Stop bot ────────────────────────────────────────────
    const handleStopDiscordBot = async () => {
        try {
            const result = await window.api.discordBotStop();
            if (result.error) {
                safeShowToast(t('discord_bot.stop_failed_toast', { error: translateError(result.error) }), 'error', 4000);
            } else {
                setDiscordBotStatus('stopped');
                safeShowToast(t('discord_bot.stopped_toast'), 'discord', 3000, { isNotice: true, source: 'Discord Bot' });
            }
        } catch (e) {
            safeShowToast(t('discord_bot.stop_error_toast', { error: translateError(e.message) }), 'error', 4000);
        }
    };

    // ── Bot relaunch listener (language change) ─────────────
    useEffect(() => {
        if (!window.api?.onBotRelaunch) return;

        const handler = (botConfig) => {
            console.log('[Bot Relaunch] Received signal to relaunch bot with new language settings');
            setTimeout(async () => {
                const configWithToken = { ...botConfig, token: discordTokenRef.current };
                const result = await window.api.discordBotStart(configWithToken);
                if (result.error) {
                    console.error('[Bot Relaunch] Failed to relaunch bot:', result.error);
                } else {
                    console.log('[Bot Relaunch] Bot relaunched successfully');
                    setDiscordBotStatus('running');
                    safeShowToast(t('discord_bot.relaunched_toast'), 'discord', 3000, { isNotice: true, source: 'Discord Bot' });
                }
            }, 1000);
        };

        window.api.onBotRelaunch(handler);
        return () => {
            if (window.api.offBotRelaunch) window.api.offBotRelaunch();
        };
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    return {
        discordBotStatus,
        setDiscordBotStatus,
        botStatusReady,
        handleStartDiscordBot,
        handleStopDiscordBot,
    };
}
