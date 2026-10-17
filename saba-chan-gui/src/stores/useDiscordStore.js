import { create } from 'zustand';
import { createTranslateError, safeShowToast } from '../utils/helpers';

// i18n translate function — set after store creation
let _t = (key, fallback) => fallback || key;
let _translateError = (msg) => msg;

export const setDiscordI18n = (t) => {
    _t = t;
    _translateError = createTranslateError(t);
};

const RELAY_URL_FALLBACK = 'https://saba-chan.online';
const DEFAULT_NODE_MUSIC_SETTINGS = { queueLines: 5, refreshInterval: 4000, normalize: true };
let statusRequestId = 0;

function mergeNodeMusicState(prevNode, patch) {
    const next = { ...(prevNode || {}) };
    if ('musicEnabled' in patch) next.musicEnabled = patch.musicEnabled;
    if ('musicUIEnabled' in patch) next.musicUIEnabled = patch.musicUIEnabled;
    if ('musicSettings' in patch) {
        next.musicSettings = {
            ...(next.musicSettings || DEFAULT_NODE_MUSIC_SETTINGS),
            ...(patch.musicSettings || {}),
        };
    }
    return next;
}

// 클라우드 전용 Discord Bot 설정 store.
// 로컬 봇 기능은 제거되었음. 봇은 릴레이 서버에서 호스팅되며,
// 이 호스트는 prefix/aliases/nodeSettings 동기화 + 릴레이 페어링만 담당한다.
export const useDiscordStore = create((set, get) => ({
    // ── Bot config (릴레이 서버에 동기화되는 호스트별 설정) ──
    discordPrefix: '!saba',
    discordModuleAliases: {},
    discordCommandAliases: {},

    // ── Cloud relay pairing ──
    discordCloudRelayUrl: '',
    discordCloudHostId: '',

    // ── Node & cloud ──
    // nodeSettings는 노드(길드) 단위 음악 설정 전용.
    // 인스턴스 권한/별명은 instanceSettings(인스턴스 단위)가 담당한다.
    nodeSettings: {},
    cloudNodes: [],
    cloudMembers: {},

    // ── 인스턴스별 디스코드 설정 (서버별 설정 팝업) ──
    // { [instanceId]: { aliases?, commandAliases?, allowedNodes?, blockedUsers? } }
    // 릴레이로 전송하지 않음 — bot-config.json에 저장되고 데몬 디스패처가 판정한다.
    instanceSettings: {},

    // ── Runtime status ──
    relayConnected: false,
    relayConnecting: false,
    /// 최소 1회 상태 확인이 완료되었는지 — 완료 전에는 UI가 "연결 중"으로,
    /// 완료 후 미연결이면 "연결 끊김"으로 구분해 표시할 수 있다.
    relayStatusChecked: false,
    /// 연속 상태확인 실패 횟수 — 히스테리시스용 (일시적 429/타임아웃 깜박임 방지)
    _statusFailStreak: 0,

    // ── Internal ──
    _settingsReady: false,
    _botConfigLoaded: false,
    _statusInterval: null,
    _statusTimeout: null,

    // ── Actions ──
    update: (partial) => {
        // hostId가 바뀌면 이전 상태 확인 결과는 무효 — 리셋 후 즉시 재확인해서
        // 다음 5초 폴링까지 stale 상태(끊김/연결됨)가 표시되는 것을 방지한다.
        const state = get();
        const targetChanged = ['discordCloudHostId', 'discordCloudRelayUrl'].some(
            (key) => key in partial && partial[key] !== state[key],
        );
        if (targetChanged) {
            statusRequestId += 1;
            set({
                ...partial,
                relayConnected: false,
                relayConnecting: false,
                relayStatusChecked: false,
                _statusFailStreak: 0,
            });
            if (get().discordCloudHostId) get().checkStatus();
            return;
        }
        set(partial);
    },

    loadConfig: async () => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        // 데몬은 무결성 검증·프리플라이트 등으로 IPC 준비가 수 초 늦어질 수 있다.
        // 로드 실패 시 _botConfigLoaded=false로 남아 저장이 차단되므로, 충분히 재시도한다.
        const maxAttempts = isTest ? 1 : 15;
        const retryDelay = 1000;

        for (let attempt = 1; attempt <= maxAttempts; attempt++) {
            try {
                // botConfigLoad는 데몬 미응답 시 null을 반환한다 (가짜 기본값 아님)
                const botCfg = await window.api.botConfigLoad();
                if (!botCfg) {
                    if (attempt < maxAttempts) {
                        if (!isTest)
                            console.warn(`[Settings] Bot config unavailable, retrying (${attempt}/${maxAttempts})...`);
                        await new Promise((r) => setTimeout(r, retryDelay));
                        continue;
                    }
                    console.error('[Settings] Bot config could not be loaded — keeping defaults, auto-save disabled');
                    return;
                }

                const isFullConfig =
                    'prefix' in botCfg ||
                    'cloud' in botCfg ||
                    'moduleAliases' in botCfg ||
                    'commandAliases' in botCfg ||
                    'nodeSettings' in botCfg;

                if (!isFullConfig && attempt < maxAttempts) {
                    if (!isTest)
                        console.log(`[Settings] Got partial bot config, retrying (${attempt}/${maxAttempts})...`);
                    await new Promise((r) => setTimeout(r, retryDelay));
                    continue;
                }

                const patch = {
                    discordPrefix: botCfg.prefix || '!saba',
                    discordModuleAliases: botCfg.moduleAliases || {},
                    discordCommandAliases: botCfg.commandAliases || {},
                    discordCloudRelayUrl: botCfg.cloud?.relayUrl || '',
                    discordCloudHostId: botCfg.cloud?.hostId || '',
                    _botConfigLoaded: isFullConfig,
                };

                if (botCfg.nodeSettings && typeof botCfg.nodeSettings === 'object') {
                    patch.nodeSettings = botCfg.nodeSettings;
                }
                if (botCfg.instanceSettings && typeof botCfg.instanceSettings === 'object') {
                    patch.instanceSettings = botCfg.instanceSettings;
                }
                if (Array.isArray(botCfg.cloudNodes)) patch.cloudNodes = botCfg.cloudNodes;
                if (botCfg.cloudMembers && typeof botCfg.cloudMembers === 'object')
                    patch.cloudMembers = botCfg.cloudMembers;

                set(patch);

                // ★ 로드 직후 자가치유 재저장 — 과거 버전의 저장 버그(예: main.js가
                //   instanceSettings를 저장 화이트리스트에서 누락)로 디스크에 반영되지
                //   못한 필드가 있어도, 사용자가 설정 화면에서 값을 다시 건드리지 않아도
                //   다음 실행부터 고쳐진 저장 경로로 자동 복구되게 한다. 실패해도 로드
                //   자체는 이미 완료된 상태이므로 조용히 무시한다.
                if (isFullConfig) {
                    get()
                        .saveConfig()
                        .catch((err) => {
                            if (!isTest) console.warn('[Settings] Self-heal resave failed:', err);
                        });
                }
                return;
            } catch (err) {
                if (attempt === maxAttempts) {
                    console.error('Failed to load bot config after retries:', err);
                } else {
                    if (!isTest) console.warn(`[Settings] Bot config load attempt ${attempt} failed, retrying...`);
                    await new Promise((r) => setTimeout(r, retryDelay));
                }
            }
        }
    },

    saveConfig: async (newPrefix) => {
        const state = get();
        try {
            let base = {};
            if (!state._botConfigLoaded) {
                base = await window.api.botConfigLoad();
                if (!base) throw new Error('Bot config is unavailable; saving defaults would overwrite existing settings');
            }

            const {
                token: _legacyToken,
                autoStart: _legacyAutoStart,
                mode: _legacyMode,
                musicEnabled: _legacyMusicEnabled,
                musicChannelId: _legacyMusicChannelId,
                musicUISettings: _legacyMusicUISettings,
                musicUiSettings: _legacyMusicUiSettings,
                ...baseClean
            } = base || {};

            // 설정 로드에 성공한 적 없으면(_botConfigLoaded=false) 스토어의 초기값
            // (빈 문자열/빈 객체)이 디스크의 hostId·별명을 덮어쓰지 않도록,
            // 비어있는 state 값 대신 데몬에서 읽은 base 값을 우선한다.
            const loaded = state._botConfigLoaded;
            const pickStr = (stateVal, baseVal) => (loaded ? (stateVal ?? baseVal ?? '') : stateVal || baseVal || '');
            const pickObj = (stateVal, baseVal) => {
                if (loaded) return stateVal ?? baseVal ?? {};
                if (stateVal && Object.keys(stateVal).length > 0) return stateVal;
                return baseVal ?? {};
            };
            const pickArr = (stateVal, baseVal) => {
                if (loaded) return stateVal ?? baseVal ?? [];
                if (Array.isArray(stateVal) && stateVal.length > 0) return stateVal;
                return baseVal ?? [];
            };
            let prefix = newPrefix || state.discordPrefix;
            if (!newPrefix && !loaded && state.discordPrefix === '!saba') {
                prefix = baseClean.prefix || prefix;
            }

            const payload = {
                ...baseClean,
                prefix: prefix || '!saba',
                cloud: {
                    relayUrl: pickStr(state.discordCloudRelayUrl, baseClean.cloud?.relayUrl),
                    hostId: pickStr(state.discordCloudHostId, baseClean.cloud?.hostId),
                },
                moduleAliases: pickObj(state.discordModuleAliases, baseClean.moduleAliases),
                commandAliases: pickObj(state.discordCommandAliases, baseClean.commandAliases),
                nodeSettings: pickObj(state.nodeSettings, baseClean.nodeSettings),
                instanceSettings: pickObj(state.instanceSettings, baseClean.instanceSettings),
                cloudNodes: pickArr(state.cloudNodes, baseClean.cloudNodes),
                cloudMembers: pickObj(state.cloudMembers, baseClean.cloudMembers),
            };
            const res = await window.api.botConfigSave(payload);
            if (res?.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
                safeShowToast(_t('settings.save_error'), 'error');
            } else {
                console.log('[Settings] Bot config saved, prefix:', payload.prefix);
            }
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
            safeShowToast(_t('settings.save_error'), 'error');
        }
    },

    // ── 릴레이 연결 상태 확인 ──
    // 일시적 실패(레이트리밋·타임아웃)로 UI가 "연결 끊김"으로 깜박이지 않도록
    // 연속 3회 실패한 경우에만 연결 끊김으로 판정한다 (5s 폴링 기준 ≈15s).
    checkStatus: async () => {
        const STATUS_FAIL_THRESHOLD = 3;
        const state = get();
        if (state.relayConnecting) return;
        const requestId = ++statusRequestId;
        if (!state.discordCloudHostId) {
            set({ relayConnected: false, relayStatusChecked: true, _statusFailStreak: 0 });
            return;
        }
        const relayUrl = state.discordCloudRelayUrl || RELAY_URL_FALLBACK;
        let ok = false;
        try {
            set({ relayConnecting: true });
            const result = await window.api.relayCheckHostStatus(state.discordCloudHostId, relayUrl);
            ok = !!(result && !result.error);
        } catch {
            ok = false;
        }
        if (requestId !== statusRequestId) return;
        set({ relayConnecting: false });

        if (ok) {
            set({ relayConnected: true, relayStatusChecked: true, _statusFailStreak: 0 });
        } else {
            const streak = get()._statusFailStreak + 1;
            if (!get().relayConnected || streak >= STATUS_FAIL_THRESHOLD) {
                set({ relayConnected: false, relayStatusChecked: true, _statusFailStreak: streak });
            } else {
                set({ relayStatusChecked: true, _statusFailStreak: streak });
            }
        }
    },

    syncNodeMusicSettings: async (guildId, relayUrlOverride) => {
        const state = get();
        const relayUrl = relayUrlOverride || state.discordCloudRelayUrl || RELAY_URL_FALLBACK;
        const data = await window.api.relayGetNodeSettings(guildId, relayUrl);
        if (!data || data.error) {
            throw new Error(data?.error || 'Failed to load node music settings');
        }

        const musicPatch = {
            musicEnabled: data.musicEnabled !== false,
            musicUIEnabled: data.musicUIEnabled !== false,
            musicSettings: data.musicSettings || DEFAULT_NODE_MUSIC_SETTINGS,
        };
        set((prev) => ({
            nodeSettings: {
                ...prev.nodeSettings,
                [guildId]: mergeNodeMusicState(prev.nodeSettings[guildId], musicPatch),
            },
        }));
        return data;
    },

    updateNodeMusicSettings: async (guildId, patch, relayUrlOverride) => {
        const state = get();
        const relayUrl = relayUrlOverride || state.discordCloudRelayUrl || RELAY_URL_FALLBACK;
        const data = await window.api.relayUpdateNodeSettings(guildId, patch, relayUrl);
        if (!data || data.error) {
            return { success: false, error: data?.error || 'Failed to update node music settings' };
        }

        set((prev) => ({
            nodeSettings: {
                ...prev.nodeSettings,
                [guildId]: mergeNodeMusicState(prev.nodeSettings[guildId], patch),
            },
        }));
        return data;
    },

    startStatusPolling: () => {
        const state = get();
        if (state._statusInterval) return;
        const timeout = setTimeout(() => {
            set({ _statusTimeout: null });
            get().checkStatus();
        }, 200);
        const interval = setInterval(() => get().checkStatus(), 5000);
        set({ _statusInterval: interval, _statusTimeout: timeout });
    },

    stopStatusPolling: () => {
        const state = get();
        clearInterval(state._statusInterval);
        clearTimeout(state._statusTimeout);
        statusRequestId += 1;
        set({ _statusInterval: null, _statusTimeout: null, relayConnecting: false });
    },

    // 로컬 봇이 제거되었으므로 별도의 process listener 초기화는 없음.
    initListeners: () => {},

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        if (state._statusInterval) clearInterval(state._statusInterval);
        clearTimeout(state._statusTimeout);
        statusRequestId += 1;
        set({
            discordPrefix: '!saba',
            discordModuleAliases: {},
            discordCommandAliases: {},
            discordCloudRelayUrl: '',
            discordCloudHostId: '',
            nodeSettings: {},
            instanceSettings: {},
            cloudNodes: [],
            cloudMembers: {},
            relayConnected: false,
            relayConnecting: false,
            relayStatusChecked: false,
            _statusFailStreak: 0,
            _settingsReady: false,
            _botConfigLoaded: false,
            _statusInterval: null,
            _statusTimeout: null,
        });
    },
}));

// ── Auto-save subscription ──
let botConfigSaveTimer = null;
const _discordSaveKeys = [
    'discordPrefix',
    'discordCloudRelayUrl',
    'discordCloudHostId',
    'nodeSettings',
    'instanceSettings',
    'cloudNodes',
    'cloudMembers',
    'discordModuleAliases',
    'discordCommandAliases',
];
useDiscordStore.subscribe((state, prevState) => {
    if (!state._settingsReady) return;
    const changed = _discordSaveKeys.some((k) => state[k] !== prevState[k]);
    if (!changed) return;
    if (!state._botConfigLoaded) {
        console.warn('[Settings] Discord config changed but bot config not loaded — skipping auto-save');
        return;
    }
    console.log('[Settings] Discord config changed, saving...');
    clearTimeout(botConfigSaveTimer);
    botConfigSaveTimer = setTimeout(() => {
        useDiscordStore.getState().saveConfig();
    }, 500);
});

// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useDiscordStore.getState();
        data.prevState = {
            discordPrefix: s.discordPrefix,
            discordModuleAliases: s.discordModuleAliases,
            discordCommandAliases: s.discordCommandAliases,
            discordCloudRelayUrl: s.discordCloudRelayUrl,
            discordCloudHostId: s.discordCloudHostId,
            nodeSettings: s.nodeSettings,
            instanceSettings: s.instanceSettings,
            cloudNodes: s.cloudNodes,
            cloudMembers: s.cloudMembers,
            relayConnected: s.relayConnected,
            relayStatusChecked: s.relayStatusChecked,
            _settingsReady: s._settingsReady,
            _botConfigLoaded: s._botConfigLoaded,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useDiscordStore.setState(import.meta.hot.data.prevState);
    }
}
