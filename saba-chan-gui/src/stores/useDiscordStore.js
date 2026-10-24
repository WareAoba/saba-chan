import { create } from 'zustand';
import { readPrefixes } from '../components/discord/prefixes';
import { createTranslateError, safeShowToast } from '../utils/helpers';
import { mergePatchDiff } from '../utils/mergePatch';

// i18n translate function — set after store creation
let _t = (key, fallback) => fallback || key;
let _translateError = (msg) => msg;

export const setDiscordI18n = (t) => {
    _t = t;
    _translateError = createTranslateError(t);
};

const RELAY_URL_FALLBACK = 'https://saba-chan.online';

// bot-config 최상위 키 ↔ 스토어 필드. 바뀐 칸만 PATCH(JSON Merge Patch)로 보낸다:
// 다른 경로(CLI·원격·다른 길드 설정)가 그사이 바꾼 값을 옛 스냅샷으로 되돌리지 않게
const BOT_KEYS = {
    prefixes: ['discordPrefixes'],
    cloud: ['discordCloudRelayUrl', 'discordCloudHostId'],
    moduleAliases: ['discordModuleAliases'],
    commandAliases: ['discordCommandAliases'],
    nodeSettings: ['nodeSettings'],
    cloudNodes: ['cloudNodes'],
    cloudMembers: ['cloudMembers'],
};
// 마지막으로 데몬과 맞춘 값 (로드·저장 성공 때 갱신). 여기와 다른 필드만 바뀐 것으로 본다
let synced = {};
const sameValue = (a, b) => a === b || JSON.stringify(a) === JSON.stringify(b);
const markSynced = (state, fields) => {
    for (const f of fields) synced[f] = state[f];
};
const ALL_FIELDS = Object.values(BOT_KEYS).flat();
// 저장은 한 번에 하나씩: 데몬 값을 읽는 사이 뒤 저장이 먼저 끝나 옛 값이 마지막에 남지 않게
let saveQueue = Promise.resolve();
// 상태 확인 세대: 연결 대상이 바뀌거나 폴링이 멈추면 올려서, 늦게 온 옛 응답을 버린다
let statusGeneration = 0;
// 지금 확인 중인 대상 세대 (겹친 확인은 하나만 돌려 한 번의 실패를 두 번으로 세지 않는다)
let checkingGeneration = null;

// 디스코드 봇 본체는 클라우드(릴레이 서버)에서 돌고, 로컬에서는 데몬 내장
// 릴레이 에이전트가 명령을 받아 처리한다. 이 스토어는 bot-config와
// 에이전트/릴레이 상태만 다룬다. 에이전트는 노드 토큰이 있으면 데몬이 자동 시작한다.
const initialState = () => ({
    // ── Bot config (bot-config.json, 데몬이 SSOT) ──
    // 비어 있으면 릴레이가 언어별 기본 접두사를 쓴다
    discordPrefixes: [],
    discordModuleAliases: {},
    discordCommandAliases: {},
    discordCloudRelayUrl: '',
    discordCloudHostId: '',
    nodeSettings: {},
    cloudNodes: [],
    cloudMembers: {},

    // ── Runtime status ──
    // stopped: 에이전트 정지 / connecting: 에이전트 실행 중, 릴레이 미확인 / running: 둘 다 OK
    discordBotStatus: 'stopped',
    // 다른 PC가 같은 계정으로 페어링해서 이 PC의 에이전트가 멈췄다
    discordAgentReplaced: false,
    discordAgentRelayUrlInvalid: false,
    // 릴레이가 이 앱 버전을 받지 않을 때 요구하는 최소 버전. 받으면 null
    discordAgentMinVersion: null,
    botStatusReady: false,
    relayConnected: false,
    relayConnecting: false,
    // 릴레이 확인이 연달아 실패한 이유. 비어 있지 않으면 화면이 '연결 중' 대신 실패를 보인다
    relayError: '',
    // 길드 목록을 다시 읽어야 할 때 올린다 (연결 요청 수락 등). 디스코드 화면이 보고 다시 읽는다
    cloudNodesStale: 0,

    // ── Internal ──
    // 연달아 실패한 릴레이 확인 수. 한 번 놓친 것으로 끊김·실패를 보이지 않는다
    _relayFailures: 0,
    // 연달아 답을 받지 못한 에이전트 상태 확인 수. 한 번 늦은 것으로 꺼짐을 보이지 않는다
    _agentFailures: 0,
    // 마지막 봇 설정 저장이 거절된 이유(error_code). 저장에 성공하면 비운다
    _saveErrorCode: null,
    _settingsReady: false,
    _botConfigLoaded: false,
    _statusInterval: null,
    _statusTimeout: null,
});

export const useDiscordStore = create((set, get) => ({
    ...initialState(),

    update: (partial) => {
        const state = get();
        const targetChanged = ['discordCloudHostId', 'discordCloudRelayUrl'].some(
            (k) => k in partial && partial[k] !== state[k],
        );
        if (!targetChanged) return set(partial);
        // 연결 대상이 바뀌면 옛 호스트의 확인 결과는 무효 (다음 폴링까지 잘못된 상태를 보이지 않게)
        statusGeneration += 1;
        set({ ...partial, relayConnected: false, relayConnecting: false, relayError: '', _relayFailures: 0 });
        if (get()._statusInterval) get().checkStatus();
    },

    loadConfig: async ({ attempts } = {}) => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        const maxAttempts = attempts ?? (isTest ? 1 : 5);
        const retryDelay = 800;

        for (let attempt = 1; attempt <= maxAttempts; attempt++) {
            try {
                const botCfg = await window.api.botConfigLoad();
                if (!botCfg) continue;

                const patch = {
                    discordPrefixes: readPrefixes(botCfg),
                    discordModuleAliases: botCfg.moduleAliases || {},
                    discordCommandAliases: botCfg.commandAliases || {},
                    discordCloudRelayUrl: botCfg.cloud?.relayUrl || '',
                    discordCloudHostId: botCfg.cloud?.hostId || '',
                    _botConfigLoaded: true,
                };
                if (botCfg.nodeSettings && typeof botCfg.nodeSettings === 'object') {
                    patch.nodeSettings = botCfg.nodeSettings;
                }
                if (Array.isArray(botCfg.cloudNodes)) patch.cloudNodes = botCfg.cloudNodes;
                if (botCfg.cloudMembers && typeof botCfg.cloudMembers === 'object')
                    patch.cloudMembers = botCfg.cloudMembers;

                // 읽기 전에 화면에서 고친 값은 덮지 않는다 (데몬이 없어 저장되지 못한 값). 데몬 값을 기준으로 두어
                // 바뀐 것으로 남기고 아래에서 저장한다
                const before = get();
                const initial = initialState();
                const edited = ALL_FIELDS.filter((f) => !sameValue(before[f], f in synced ? synced[f] : initial[f]));
                const loaded = { ...before, ...patch };
                for (const f of edited) delete patch[f];
                set(patch);
                synced = {};
                markSynced(loaded, ALL_FIELDS);
                if (edited.length) await get().saveConfig();
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

    saveConfig: () => {
        const run = saveQueue.then(() => get()._saveNow());
        saveQueue = run;
        return run;
    },

    // 큐 차례가 왔을 때의 최신 상태를 저장한다. saveConfig로만 부른다
    _saveNow: async () => {
        const state = get();
        try {
            // 처음 로드에 실패했다면 스토어 값은 기본값이다. 데몬을 읽을 수 없으면 저장하지 않는다
            if (!state._botConfigLoaded && !(await window.api.botConfigLoad())) {
                throw new Error('bot config unavailable; refusing to overwrite it with defaults');
            }
            const initial = initialState();
            const base = (f) => (f in synced ? synced[f] : initial[f]);
            const changed = Object.keys(BOT_KEYS).filter((k) => BOT_KEYS[k].some((f) => !sameValue(state[f], base(f))));
            set({ _saveErrorCode: null });
            if (changed.length === 0) return true;

            const payload = {};
            for (const k of changed) {
                // relayUrl·hostId는 한 키(cloud) 안의 두 칸이다
                const value = (pick) =>
                    k === 'cloud'
                        ? { relayUrl: pick('discordCloudRelayUrl'), hostId: pick('discordCloudHostId') }
                        : pick(BOT_KEYS[k][0]);
                const patch = mergePatchDiff(value(base), value((f) => state[f]));
                if (patch !== undefined) payload[k] = patch;
            }
            const res = await window.api.botConfigSave(payload);
            if (res?.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
                set({ _saveErrorCode: res.error_code || null });
                // 릴레이 주소가 거절되면(R31) 어떤 주소를 쓸 수 있는지 알린다
                const invalidUrl = res.error_code === 'invalid_relay_url';
                safeShowToast(_t(invalidUrl ? 'discord.invalid_relay_url' : 'settings.save_error'), 'error');
                return false;
            }
            markSynced(
                state,
                changed.flatMap((k) => BOT_KEYS[k]),
            );
            // 권한 키는 권한 화면으로만 바뀐다: 데몬이 저장하지 않은 키를 알린다 (L61, CLI와 같게)
            if (Array.isArray(res?.ignored) && res.ignored.length > 0) {
                safeShowToast(_t('discord.permission_keys_ignored'), 'warning', 6000);
                // 화면이 저장되지 않은 값을 저장된 것처럼 보이지 않게 데몬 값으로 다시 읽는다
                await get().loadConfig().catch(() => {});
            }
            return true;
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
            safeShowToast(_t('settings.save_error'), 'error');
            return false;
        }
    },

    /**
     * 데몬 전용 API로 이미 저장한 길드 값을 스토어에 반영한다. 자동 저장이 다시 보내지 않게 동기화 기준도 같이 바꾼다
     */
    applySavedNodeValue: (guildId, key, value) => {
        const put = (all) => {
            const node = { ...(all?.[guildId] || {}) };
            if (value == null) delete node[key];
            else node[key] = value;
            return { ...(all || {}), [guildId]: node };
        };
        synced.nodeSettings = put(synced.nodeSettings ?? initialState().nodeSettings);
        set({ nodeSettings: put(get().nodeSettings) });
    },

    markCloudNodesStale: () => set({ cloudNodesStale: get().cloudNodesStale + 1 }),

    startBot: async () => {
        try {
            const result = await window.api.discordBotStart();
            if (result?.error) {
                safeShowToast(
                    _t('discord_bot.start_failed_toast', { error: _translateError(result.error) }),
                    'error',
                    4000,
                );
            } else {
                set({ discordBotStatus: 'connecting' });
                safeShowToast(_t('discord_bot.started_toast'), 'discord', 3000);
                get().checkStatus();
            }
        } catch (e) {
            safeShowToast(_t('discord_bot.start_error_toast', { error: _translateError(e.message) }), 'error', 4000);
        }
    },

    stopBot: async () => {
        try {
            const result = await window.api.discordBotStop();
            if (result?.error) {
                safeShowToast(
                    _t('discord_bot.stop_failed_toast', { error: _translateError(result.error) }),
                    'error',
                    4000,
                );
            } else {
                set({ discordBotStatus: 'stopped', relayConnected: false, relayError: '', _relayFailures: 0 });
                safeShowToast(_t('discord_bot.stopped_toast'), 'discord', 3000);
            }
        } catch (e) {
            safeShowToast(_t('discord_bot.stop_error_toast', { error: _translateError(e.message) }), 'error', 4000);
        }
    },

    checkStatus: async () => {
        const generation = statusGeneration;
        if (checkingGeneration === generation) return;
        checkingGeneration = generation;
        try {
            await get()._checkStatusNow(generation);
        } finally {
            if (checkingGeneration === generation) checkingGeneration = null;
        }
    },

    _checkStatusNow: async (generation) => {
        // 앱이 데몬보다 먼저 떠서 처음 설정 읽기에 실패했으면 여기서 다시 읽는다.
        // 읽지 못한 채로 두면 호스트가 비어 저장된 연결이 있어도 연결 화면을 보인다
        if (!get()._botConfigLoaded) await get().loadConfig({ attempts: 1 });
        if (generation !== statusGeneration) return;
        const state = get();
        let agent = 'unknown';
        try {
            agent = await window.api.discordBotStatus();
        } catch {
            /* unknown */
        }

        let relayOk = false;
        let relayFailure = '';
        if (state.discordCloudHostId) {
            const relayUrl = state.discordCloudRelayUrl || RELAY_URL_FALLBACK;
            try {
                // 이미 붙어 있거나 실패로 정해졌으면 '연결 중'으로 깜빡이지 않는다 (5초마다 확인)
                if (generation === statusGeneration && !state.relayConnected && !state.relayError)
                    set({ relayConnecting: true });
                const result = await window.api.relayCheckHostStatus(state.discordCloudHostId, relayUrl);
                relayOk = !!result && !result.error;
                if (!relayOk) relayFailure = String(result?.error || _t('errors.network_error'));
            } catch (e) {
                relayFailure = e?.message || String(e);
            }
        }
        // 그 사이 대상이 바뀌었거나 폴링이 멈췄으면 이 결과는 버린다
        if (generation !== statusGeneration) return;

        // 에이전트도 두 번 연달아 모를 때만 꺼짐으로 본다 (그 전까지는 지금 상태를 유지한다)
        const agentFailures = agent === 'unknown' ? get()._agentFailures + 1 : 0;
        const prev = get();
        const keepAgent = agent === 'unknown' && agentFailures < 2;
        // too_old는 { status, running, minVersion }으로 온다
        const tooOld = agent?.status === 'too_old';
        const agentRunning = keepAgent
            ? prev.discordBotStatus !== 'stopped'
            : tooOld
              ? agent.running
              : agent === 'running';
        const agentMinVersion = keepAgent ? prev.discordAgentMinVersion : tooOld ? agent.minVersion || '-' : null;
        const agentReplaced = keepAgent ? prev.discordAgentReplaced : agent === 'replaced';
        const agentRelayUrlInvalid = keepAgent ? prev.discordAgentRelayUrlInvalid : agent === 'invalid_relay_url';

        // 두 번 연달아 실패해야 끊김·실패로 본다 (그 전까지는 지금 상태를 유지한다)
        const failures = relayFailure ? get()._relayFailures + 1 : 0;
        const settled = !relayFailure || failures >= 2;
        if (!settled) relayOk = get().relayConnected;

        set({
            relayConnecting: false,
            relayConnected: relayOk,
            relayError: settled ? relayFailure : get().relayError,
            _relayFailures: failures,
            _agentFailures: agentFailures,
            discordAgentReplaced: agentReplaced,
            discordAgentRelayUrlInvalid: agentRelayUrlInvalid,
            discordAgentMinVersion: agentMinVersion,
            // 릴레이가 거절하는 동안은 명령이 흐르지 않으니 '실행 중'으로 보이지 않는다
            discordBotStatus:
                agentRunning && relayOk && !agentMinVersion ? 'running' : agentRunning ? 'connecting' : 'stopped',
        });
    },

    startStatusPolling: () => {
        if (get()._statusInterval) return;

        // 첫 확인은 잠깐 미룬다. 멈추면 취소되도록 타이머를 들고 있는다
        const timeout = setTimeout(async () => {
            set({ _statusTimeout: null });
            const generation = statusGeneration;
            await get().checkStatus();
            if (generation === statusGeneration) set({ botStatusReady: true });
        }, 200);
        const interval = setInterval(() => get().checkStatus(), 5000);
        set({ _statusInterval: interval, _statusTimeout: timeout });
    },

    stopStatusPolling: () => {
        const state = get();
        clearInterval(state._statusInterval);
        clearTimeout(state._statusTimeout);
        statusGeneration += 1;
        set({
            _statusInterval: null,
            _statusTimeout: null,
            relayConnecting: false,
            relayError: '',
            _relayFailures: 0,
            _agentFailures: 0,
        });
    },

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        clearInterval(state._statusInterval);
        clearTimeout(state._statusTimeout);
        statusGeneration += 1;
        synced = {};
        saveQueue = Promise.resolve();
        set(initialState());
    },
}));

// ── Auto-save subscription (Zustand v5 single-listener) ──
let botConfigSaveTimer = null;
const _discordSaveKeys = [
    'discordPrefixes',
    'discordCloudRelayUrl',
    'discordCloudHostId',
    'nodeSettings',
    'cloudNodes',
    'cloudMembers',
    'discordModuleAliases',
    'discordCommandAliases',
];
useDiscordStore.subscribe((state, prevState) => {
    if (!state._settingsReady) return;
    const changed = _discordSaveKeys.some((k) => state[k] !== prevState[k]);
    if (!changed) return;
    // _botConfigLoaded가 false면 로드 실패 상태 — 자동 저장 시 파일 값 덮어쓰기 방지
    if (!state._botConfigLoaded) {
        console.warn('[Settings] Discord config changed but bot config not loaded — skipping auto-save');
        return;
    }
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
            discordPrefixes: s.discordPrefixes,
            discordModuleAliases: s.discordModuleAliases,
            discordCommandAliases: s.discordCommandAliases,
            discordCloudRelayUrl: s.discordCloudRelayUrl,
            discordCloudHostId: s.discordCloudHostId,
            nodeSettings: s.nodeSettings,
            cloudNodes: s.cloudNodes,
            cloudMembers: s.cloudMembers,
            discordBotStatus: s.discordBotStatus,
            relayConnected: s.relayConnected,
            botStatusReady: s.botStatusReady,
            _settingsReady: s._settingsReady,
            _botConfigLoaded: s._botConfigLoaded,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useDiscordStore.setState(import.meta.hot.data.prevState);
    }
}
