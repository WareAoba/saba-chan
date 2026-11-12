import { create } from 'zustand';
import type { CommandAliasInfo, ModuleInfo, ModuleLoadError, ServerInfo } from '../types/daemon';
import { debugWarn, retryWithBackoff, safeShowToast, waitForDaemon } from '../utils/helpers';
import { branchesUnderParents } from '../utils/serverView';
import { resetServerActions, type ServerActions, serverActions } from './serverActions';
import { serverI18n, t, translateError } from './serverI18n';
import { useUIStore } from './useUIStore';

let modulesFetch: Promise<void> | null = null;

/** 화면이 쓰는 서버: 데몬 목록 항목에 펼침 상태를 더한다 (serverActions) */
export type Server = ServerInfo & { expanded?: boolean };

/** 모듈 명령 하나의 별명·설명 (모듈 메타데이터 command_aliases + commands.fields) */
export interface ModuleCommandAliases {
    aliases: string[];
    description: string;
    label: string;
}

export interface ModuleAliases {
    module_aliases: string[];
    commands: Record<string, ModuleCommandAliases>;
}

type Updater<T> = T | ((prev: T) => T);

export interface ServerState extends ServerActions {
    servers: Server[];
    modules: ModuleInfo[];
    failedModules: ModuleLoadError[];
    modulesLoaded: boolean;
    loading: boolean;
    moduleAliasesPerModule: Record<string, ModuleAliases>;
    daemonReady: boolean;
    initStatus: string;
    initProgress: number;
    serversInitializing: boolean;
    nowEpoch: number;
    _uptimeInterval: ReturnType<typeof setInterval> | null;

    _resetForTest: () => void;
    setServers: (serversOrUpdater: Updater<Server[]>) => void;
    setModules: (modules: ModuleInfo[]) => void;
    setDaemonReady: (val: boolean) => void;
    setInitStatus: (val: string) => void;
    setInitProgress: (val: Updater<number>) => void;
    setServersInitializing: (val: boolean) => void;
    startUptimeClock: () => void;
    stopUptimeClock: () => void;
    /** 시작 시각(epoch 초)부터 지금까지 hh:mm:ss */
    formatUptime: (startTime: number | null | undefined) => string | null;
    /** quiet: 실패해도 알리지 않는다 (받을 때까지 다시 받는 중) */
    fetchModules: (opts?: { quiet?: boolean }) => Promise<void>;
    _fetchModulesOnce: (quiet?: boolean) => Promise<void>;
}

export const useServerStore = create<ServerState>()((set, get) => ({
    ...serverActions(set, get),

    // ── State ──
    servers: [],
    modules: [],
    failedModules: [], // 데몬이 불러오지 못한 모듈 [{ path, error }]
    modulesLoaded: false, // 모듈 목록을 한 번이라도 받았는지
    loading: true,
    moduleAliasesPerModule: {},

    // ── Init state ──
    daemonReady: false,
    initStatus: '', // 비어 있으면 로딩 화면이 첫 단계 문구를 쓴다
    initProgress: 0,
    serversInitializing: true,

    // ── Uptime ──
    nowEpoch: Math.floor(Date.now() / 1000),
    _uptimeInterval: null,

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        if (state._uptimeInterval) clearInterval(state._uptimeInterval);
        resetServerActions();
        set({
            ...serverActions(set, get),
            servers: [],
            modules: [],
            failedModules: [], // 데몬이 불러오지 못한 모듈 [{ path, error }]
            modulesLoaded: false,
            loading: true,
            moduleAliasesPerModule: {},
            daemonReady: false,
            initStatus: '', // 비어 있으면 로딩 화면이 첫 단계 문구를 쓴다
            initProgress: 0,
            serversInitializing: true,
            nowEpoch: Math.floor(Date.now() / 1000),
            _uptimeInterval: null,
        });
    },

    // ── Actions ──

    // 브랜치 서버는 늘 상위 서버 바로 아래에 둔다 (목록과 끌어 옮기기가 같은 순서를 본다)
    setServers: (serversOrUpdater) => {
        if (typeof serversOrUpdater === 'function') {
            set((state) => ({ servers: branchesUnderParents(serversOrUpdater(state.servers)) }));
        } else {
            set({ servers: branchesUnderParents(serversOrUpdater) });
        }
    },

    setModules: (modules) => set({ modules }),

    setDaemonReady: (val) => set({ daemonReady: val }),
    setInitStatus: (val) => set({ initStatus: val }),
    setInitProgress: (val) => {
        if (typeof val === 'function') {
            set((state) => ({ initProgress: val(state.initProgress) }));
        } else {
            set({ initProgress: val });
        }
    },
    setServersInitializing: (val) => set({ serversInitializing: val }),

    startUptimeClock: () => {
        const state = get();
        if (state._uptimeInterval) return;
        const interval = setInterval(() => {
            set({ nowEpoch: Math.floor(Date.now() / 1000) });
        }, 1000);
        set({ _uptimeInterval: interval });
    },

    stopUptimeClock: () => {
        const state = get();
        if (state._uptimeInterval) {
            clearInterval(state._uptimeInterval);
            set({ _uptimeInterval: null });
        }
    },

    formatUptime: (startTime) => {
        if (!startTime) return null;
        const nowEpoch = get().nowEpoch;
        const elapsed = Math.max(0, nowEpoch - startTime);
        const h = String(Math.floor(elapsed / 3600)).padStart(2, '0');
        const m = String(Math.floor((elapsed % 3600) / 60)).padStart(2, '0');
        const s = String(elapsed % 60).padStart(2, '0');
        return `${h}:${m}:${s}`;
    },

    // 데몬이 늦게 뜨거나 다시 켜지면 여러 곳에서 부를 수 있다: 이미 받는 중이면 그걸 기다린다
    fetchModules: (opts) => {
        if (!modulesFetch)
            modulesFetch = get()
                ._fetchModulesOnce(opts?.quiet)
                .finally(() => {
                    modulesFetch = null;
                });
        return modulesFetch;
    },

    _fetchModulesOnce: async (quiet = false) => {
        try {
            console.log('Fetching modules...');
            try {
                await waitForDaemon(5000);
            } catch (err) {
                debugWarn('Daemon not ready, but continuing:', (err as Error).message);
            }

            const data = await retryWithBackoff(() => window.api.moduleList(), 3, 800);
            console.log('Module data received:', data);

            if (data && data.modules) {
                console.log('Setting modules:', data.modules.length, 'modules');
                set({ modules: data.modules, failedModules: data.failed || [], modulesLoaded: true });

                // 번역과 메타데이터(별명)를 모듈마다 차례로 기다리지 않고 한꺼번에 받는다
                const i18n = serverI18n();
                const loadLocales = async (mod: { name: string }) => {
                    try {
                        if (i18n && window.api.moduleGetLocales) {
                            const locales = await window.api.moduleGetLocales(mod.name);
                            if (locales && typeof locales === 'object') {
                                for (const [lang, localeData] of Object.entries(locales)) {
                                    i18n.addResourceBundle(lang, `mod_${mod.name}`, localeData, true, true);
                                }
                                console.log(`Module locales registered for ${mod.name}:`, Object.keys(locales));
                            }
                        }
                    } catch (e) {
                        console.warn(`Failed to load locales for module ${mod.name}:`, e);
                    }
                };
                const loadAliases = async (mod: { name: string }): Promise<ModuleAliases | null> => {
                    try {
                        const metadata = await window.api.moduleGetMetadata(mod.name);
                        // 데몬이 파싱한 메타데이터만 쓴다 (원본 toml은 응답에서 빠질 예정)
                        const meta = metadata?.metadata;
                        if (!meta) return null;
                        const mergedCommands: Record<string, ModuleCommandAliases> = {};
                        const commandAliases = (meta.command_aliases || {}) as Record<string, CommandAliasInfo>;
                        for (const [cmdName, cmdData] of Object.entries(commandAliases)) {
                            mergedCommands[cmdName] = {
                                aliases: cmdData.aliases || [],
                                description: cmdData.description || '',
                                label: cmdName,
                            };
                        }
                        for (const cmdField of meta.commands?.fields || []) {
                            const cmdName = cmdField.name;
                            if (!mergedCommands[cmdName]) {
                                mergedCommands[cmdName] = {
                                    aliases: [],
                                    description: cmdField.description || '',
                                    label: cmdField.label || cmdName,
                                };
                            } else {
                                if (!mergedCommands[cmdName].description && cmdField.description)
                                    mergedCommands[cmdName].description = cmdField.description;
                                if (cmdField.label) mergedCommands[cmdName].label = cmdField.label;
                            }
                        }
                        return { module_aliases: meta.module_aliases || [], commands: mergedCommands };
                    } catch (e) {
                        console.warn(`Failed to load metadata for module ${mod.name}:`, e);
                        return null;
                    }
                };
                const mods: { name: string }[] = data.modules;
                const [, aliases] = await Promise.all([
                    Promise.all(mods.map(loadLocales)),
                    Promise.all(mods.map(loadAliases)),
                ]);
                const aliasesMap: Record<string, ModuleAliases> = {};
                mods.forEach((mod, i) => {
                    const entry = aliases[i];
                    if (entry) aliasesMap[mod.name] = entry;
                });
                set({ moduleAliasesPerModule: aliasesMap });
                console.log('Module aliases loaded:', aliasesMap);
            } else if (data && data.error) {
                console.error('Module fetch error:', data.error);
                const errMsg = translateError(data.error);
                if (errMsg && !quiet) safeShowToast(t('modules.load_failed_toast', { error: errMsg }), 'error', 4000);
            } else {
                debugWarn('No modules data:', data);
                if (!quiet) safeShowToast(t('modules.list_empty'), 'warning', 3000);
            }
        } catch (error) {
            console.error('Failed to fetch modules:', error);
            if (quiet) return;
            const errMsg = translateError((error as Error).message);
            if (errMsg) safeShowToast(t('modules.fetch_failed_toast', { error: errMsg }), 'error', 5000);
            useUIStore.getState().openModal({
                type: 'failure',
                title: t('modules.load_error_title'),
                message: translateError((error as Error).message),
            });
        }
    },
}));
// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useServerStore.getState();
        data.prevState = {
            servers: s.servers,
            modules: s.modules,
            failedModules: s.failedModules,
            loading: s.loading,
            moduleAliasesPerModule: s.moduleAliasesPerModule,
            daemonReady: s.daemonReady,
            initStatus: s.initStatus,
            initProgress: s.initProgress,
            serversInitializing: s.serversInitializing,
            nowEpoch: s.nowEpoch,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useServerStore.setState(import.meta.hot.data.prevState);
    }
}
