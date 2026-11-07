/**
 * preload.js의 window.api·window.electron을 Tauri 위에서 같은 이름·인자·결과로 구현한다.
 * 응답 가공은 main.js의 각 ipcMain 핸들러를 그대로 옮겼다. 네이티브 작업은 Rust 명령(계약 §2)으로 간다.
 */
import i18n from 'i18next';
import { safeShowToast } from '../utils/helpers';
import type { BackdropSupport, WindowBackdropConfig } from '../utils/themeManager';
import type {
    ApiBody,
    AgentStatus,
    ApiResult,
    DaemonError,
    ExtensionListResponse,
    GuiSettings,
    ModuleListResponse,
    RequestOptions,
    ServerListResponse,
} from '../types/daemon';
import { createDaemonClient, type Invoke, queryString } from './daemon';
import type { EventHub, Listener } from './events';
import { hasRemotePathPicker, pickRemotePath } from './remotePathPicker';

// 저장 파일 이름의 날짜는 이 PC 시간대로 (toISOString은 UTC라 한국 오전 9시 전에는 어제가 된다)
const localDate = (d = new Date()) =>
    `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;

/** 서버 시작 요청을 기다리는 시간 (CLI `post_long`과 같다) */
const START_TIMEOUT_MS = 30 * 60_000;

/** 정지·재시작 요청을 기다리는 시간: 데몬은 인스턴스 stop_timeout(최대 3600초)까지 기다린 뒤 답한다 (CLI와 같다, L74) */
const STOP_TIMEOUT_MS = 70 * 60_000;

// 오류·상태 문구는 main.js처럼 common 네임스페이스
const t = (key: string, vars: Record<string, unknown> = {}) => i18n.t(key, { ns: 'common', ...vars });

/** 대화상자 인자 (Rust `dialog_*`) */
interface DialogArgs {
    defaultPath?: string;
    filters?: { name: string; extensions?: string[] }[] | null;
    [key: string]: unknown;
}

export function createApi({ invoke, hub }: { invoke: Invoke; hub: Pick<EventHub, 'on' | 'dispatch'> }) {
    const { request, errorBody, daemonCall, daemonJson } = createDaemonClient(invoke);
    const get = <T = ApiBody>(path: string, opts?: RequestOptions) => request<T>('get', path, undefined, opts);
    const post = (path: string, body?: unknown, opts?: RequestOptions) => request('post', path, body, opts);
    const put = (path: string, body?: unknown, opts?: RequestOptions) => request('put', path, body, opts);
    const del = (path: string, opts?: RequestOptions) => request('delete', path, undefined, opts);

    // 경로 고르기. 원격 데몬을 다루는 중이면 이 PC의 대화상자 대신 그 데몬의 폴더에서 고른다:
    // 이 PC의 경로는 그 PC 데몬에 가면 엉뚱한 곳을 가리킨다 (G1, D2)
    const KINDS = { dialog_open_file: 'file', dialog_open_folder: 'folder', dialog_save_file: 'save' } as const;
    const pickLocal = async (cmd: keyof typeof KINDS, args?: DialogArgs): Promise<string | null> => {
        const current = await Promise.resolve(invoke('daemon_target_get')).catch(() => null);
        if (current?.target) {
            if (!hasRemotePathPicker()) {
                safeShowToast(t('errors.remote_local_path'), 'warning', 5000);
                return null;
            }
            const extensions = args?.filters ? args.filters.flatMap((f) => f.extensions || []) : null;
            return pickRemotePath({ kind: KINDS[cmd], extensions, defaultName: args?.defaultPath ?? null });
        }
        return invoke(cmd, args);
    };

    // ipcRenderer.send처럼 기다리지 않는 호출
    const send = (cmd: string, args?: Record<string, unknown>) => {
        try {
            Promise.resolve(invoke(cmd, args)).catch((e) => console.warn(`[Bridge] ${cmd} failed:`, e));
        } catch (e) {
            console.warn(`[Bridge] ${cmd} failed:`, e);
        }
    };

    // main.js sendStatus: 렌더러 안에서 바로 나눠 준다 (마지막 초기화 상태로는 기억하지 않는다)
    const sendStatus = (step: string, message: string) => hub.dispatch('status:update', { step, message, ts: Date.now() });

    const connectionError = (error: DaemonError): ApiBody =>
        error.code === 'ECONNREFUSED'
            ? { error: t('network.connection_refused') }
            : { error: `${t('error')}: ${error.message}` };

    const extensionFailure = (extId: string, verb: string, error: DaemonError, related?: boolean): ApiBody => {
        const data = error.response?.data;
        console.warn(`[Extension] Failed to ${verb} '${extId}':`, data || error.message);
        const body: ApiBody = {
            ...data,
            success: false,
            error: data?.error || error.message,
            error_code: data?.error_code || 'network',
        };
        if (related) body.related = data?.related || [];
        return body;
    };

    // 결과가 {ok:false, error} 하나뿐인 업데이터 호출
    const updaterCall = async (fn: () => Promise<{ data: ApiBody }>): Promise<ApiBody> => {
        try {
            return (await fn()).data;
        } catch (caught) {
            const err = caught as DaemonError;
            return { ok: false, error: err.message };
        }
    };

    async function loadLocalSettings() {
        return invoke('settings_local_load');
    }

    // 데몬이 응답하면 바꾼 키만 PATCH한다: 데몬이 잠근 채 합치므로, 미리 읽어 통째로 보내면 그 사이 CLI가 바꾼 값을 되돌린다.
    // 데몬이 아예 응답하지 않을 때만 로컬에 쓴다(셸이 저장 직전 상태에 합친다)
    async function saveSettings(settings: Partial<GuiSettings>): Promise<boolean> {
        try {
            await request('patch', '/api/config/gui', settings, { timeout: 5000 });
            return true;
        } catch (caught) {
            const err = caught as DaemonError;
            if (err.response) {
                console.error('[saveSettings] Daemon rejected settings:', err.message);
                return false;
            }
        }
        try {
            const res = await invoke('settings_local_save', { settings });
            return res !== false;
        } catch (caught) {
            const e = caught as DaemonError;
            console.error('[saveSettings] Local save failed:', e);
            return false;
        }
    }

    // 겹치면 늦게 끝난 앞 저장이 뒤 저장을 덮으므로 차례로 한다
    // IPC 포트를 바꿔도 연결 주소는 그대로 둔다: 지금 데몬은 옛 포트에 있고, 셸이 데몬을 다시 띄울 때 맞춘다
    let settingsQueue: Promise<unknown> = Promise.resolve();

    const permPath = (guildId: string, discordId?: string) =>
        `/api/permissions/${encodeURIComponent(guildId)}${discordId === undefined ? '' : `/members/${encodeURIComponent(discordId)}`}`;
    const unisolatedPath = (component: string) => `/api/sandbox/unisolated/${encodeURIComponent(component)}`;
    const extensionGuiPath = (component: string) => `/api/sandbox/extension-gui/${encodeURIComponent(component)}`;
    // 번들은 텍스트로 받으므로 403 본문도 글자다
    const guiUnapproved = (error: DaemonError) => {
        if (error.response?.status !== 403) return false;
        let body = error.response.data;
        if (typeof body === 'string') {
            try {
                body = JSON.parse(body);
            } catch {
                return false;
            }
        }
        return body?.error_code === 'extension_gui_unapproved';
    };
    const discordSettingsPath = (guild: string) => `/api/discord/${encodeURIComponent(guild)}/settings`;
    const instancePath = (id: string, rest = '') => `/api/instance/${encodeURIComponent(id)}${rest}`;
    const relayQs = (relayUrl: string) => (relayUrl ? `?relayUrl=${encodeURIComponent(relayUrl)}` : '');
    // 큰 월드는 묶고 푸는 데 몇 분 걸린다
    const BACKUP_TIMEOUT = 10 * 60 * 1000;
    // 모듈·익스텐션 폴더 옮기기: 다른 드라이브면 복사와 검증까지 한다
    const PATHS_MOVE_TIMEOUT = 30 * 60 * 1000;
    // 네트워크 권한: 관리자 확인 창을 사용자가 누를 때까지 데몬이 기다린다
    const NET_TRACE_TIMEOUT = 3 * 60 * 1000;

    const minimizeWindow = () => send('window_minimize');
    const maximizeWindow = () => send('window_maximize');
    const closeWindow = () => send('window_close');
    // 테마 바탕색(#rrggbb)을 창 바탕에 칠한다 (크기를 바꾸는 중 흰 바탕이 비치지 않게)
    const windowSetBackground = (color: string) => send('window_set_background', { color });
    // 창 바탕 반투명을 이 창에 적용하고, 이 컴퓨터에서 쓸 수 있는 것과 켜졌는지를 받는다
    const windowSetBackdrop = (config: WindowBackdropConfig) =>
        Promise.resolve(invoke('window_set_backdrop', { config })) as Promise<BackdropSupport>;

    // 본 마지막 포트 충돌 정지 알림 id. 데몬은 읽어도 알림을 지우지 않으므로 그 뒤 것만 받는다 (L50)
    let noticesAfter = 0;

    const api = {
        serverList: async (): Promise<ApiResult<ServerListResponse>> => {
            const notifyConflicts = (data: ApiBody) => {
                // 포트 충돌로 강제 정지된 서버가 있으면 OS 알림
                for (const evt of data?.port_conflict_stops || []) {
                    send('notify', {
                        title: t('port_conflict.force_stop_title', { name: evt.stopped_name }),
                        body: t('port_conflict.force_stop_body', {
                            stopped: evt.stopped_name,
                            existing: evt.existing_name,
                            port: evt.port,
                        }),
                    });
                }
            };
            try {
                const { data } = await get<ServerListResponse>(`/api/servers?notices_after=${noticesAfter}`);
                if (Array.isArray(data?.port_conflict_stops)) {
                    // 겹친 조회는 같은 커서로 같은 알림을 받는다: 먼저 처리한 응답 뒤의 것만 남긴다
                    const seen = noticesAfter;
                    data.port_conflict_stops = data.port_conflict_stops.filter(
                        (evt) => !Number.isSafeInteger(evt.id) || (evt.id ?? 0) > seen,
                    );
                    for (const { id } of data.port_conflict_stops) {
                        if (id !== undefined && Number.isSafeInteger(id) && id > noticesAfter) noticesAfter = id;
                    }
                }
                notifyConflicts(data);
                return data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const { status, data } = error.response;
                    // 401 재시도(토큰 다시 읽기)는 daemon_request가 이미 했다
                    if (status === 401) return { error: 'Authentication failed. Daemon token may have changed.' };
                    return { error: t('server.list_failed', { status, error: data?.error || error.message }) };
                }
                return connectionError(error);
            }
        },
        // 시작 방식(managed 여부)과 포트 충돌 검사는 데몬이 인스턴스·전역 설정으로 정한다.
        // 첫 시작은 준비에 몇 분이 걸릴 수 있어 CLI처럼 30분까지 기다린다 (중간에 끊으면 시작 중인데 실패로 보인다)
        instanceStart: async (instanceId: string): Promise<ApiBody> => {
            try {
                return (
                    await post(
                        `/api/instance/${encodeURIComponent(instanceId)}/start`,
                        {},
                        { timeout: START_TIMEOUT_MS },
                    )
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const status = error.response.status;
                    const data = error.response.data || {};
                    const message = data.error_message || data.error || data.message;
                    switch (status) {
                        case 404:
                            return { ...data, error: t('server.not_found', { name: instanceId }) };
                        case 409:
                            return { ...data, error: message || t('server.already_running', { name: instanceId }) };
                        default:
                            return { ...data, error: t('server.start_failed', { error: message || error.message }) };
                    }
                }
                return connectionError(error);
            }
        },
        // 상태는 id로 묻는다. 이 길이 없는 옛 데몬(원격 0.1.11, 라우트가 없어 error_code 없는 404)이면 이름으로
        instanceStatus: async (id: string, name = id): Promise<ApiBody> => {
            const ask = async (path: string) => (await get(path)).data;
            try {
                try {
                    return await ask(`/api/instance/${encodeURIComponent(id)}/status`);
                } catch (caught) {
                    const error = caught as DaemonError;
                    if (error.response?.status !== 404 || error.response.data?.error_code) throw error;
                    return await ask(`/api/server/${encodeURIComponent(name)}/status`);
                }
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const status = error.response.status;
                    const data = error.response.data || {};
                    switch (status) {
                        case 404:
                            return { error: t('server.not_found', { name }) };
                        case 500:
                            return { error: `${t('error')}: ${data.error || data.message}` };
                        default:
                            return {
                                error: t('server.status_check_failed', { status, error: data.error || error.message }),
                            };
                    }
                }
                return connectionError(error);
            }
        },
        moduleList: async (): Promise<ApiResult<ModuleListResponse>> => {
            try {
                return (await get<ModuleListResponse>('/api/modules')).data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const { status, data } = error.response;
                    return { error: t('server.list_failed', { status, error: data?.error || error.message }) };
                }
                return connectionError(error);
            }
        },
        moduleRefresh: async () => {
            try {
                sendStatus('modules', t('modules.refreshing'));
                const { data } = await post('/api/modules/refresh');
                sendStatus('modules', t('modules.refresh_complete'));
                return data;
            } catch (caught) {
                const error = caught as DaemonError;
                let errorMsg = `${t('modules.refreshing')}: `;
                if (error.response) {
                    const { status, data } = error.response;
                    errorMsg = t('server.list_failed', { status, error: data?.error || error.message });
                } else if (error.code === 'ECONNREFUSED') {
                    errorMsg = t('network.connection_refused');
                } else {
                    errorMsg += error.message;
                }
                sendStatus('modules', errorMsg);
                return { error: errorMsg };
            }
        },
        moduleGetMetadata: async (moduleName: string): Promise<ApiBody> => {
            try {
                return (await get(`/api/module/${encodeURIComponent(moduleName)}`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const { status, data } = error.response;
                    if (status === 404) return { error: t('server.module_not_found', { module: moduleName }) };
                    return { error: t('server.status_check_failed', { status, error: data?.error || error.message }) };
                }
                return connectionError(error);
            }
        },
        // 모듈 코드가 쓰는 위험한 능력과 module.toml [permissions] 선언 비교 (데몬 응답 그대로)
        moduleAudit: async (moduleName: string) => {
            try {
                return (await get(`/api/module/${encodeURIComponent(moduleName)}/audit`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        // 최근 종료 이력
        instanceEvents: async (id: string) => {
            try {
                return (await get(`/api/instance/${encodeURIComponent(id)}/events`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error, { events: [] });
            }
        },
        // 지난 로그: 한 실행분의 끝에서부터 (`before`로 더 앞을 읽는다)
        instanceLogs: (
            id: string,
            { source, lines, before }: { source?: string; lines?: number; before?: string | number } = {},
        ) =>
            daemonCall('get', instancePath(id, `/logs${queryString({ source, lines, before })}`), undefined, 15000),
        // 한 실행분 전체를 내용으로 받는다 (웹은 그 기기에 저장한다)
        instanceLogsExport: (id: string, { source }: { source?: string } = {}) =>
            daemonCall('get', instancePath(id, `/logs/export${queryString({ source })}`), undefined, 60000),
        // 저장 창에서 고른 곳에 데몬이 쓴다. 저장 창을 닫으면 {canceled:true}
        instanceLogsSave: async (id: string, { source, defaultName }: { source?: string; defaultName?: string } = {}) => {
            const filePath = await pickLocal('dialog_save_file', {
                defaultPath: defaultName || `saba-chan-log-${localDate()}.log`,
                filters: [{ name: 'Log', extensions: ['log', 'txt'] }],
            });
            if (!filePath) return { canceled: true };
            return daemonCall('post', instancePath(id, '/logs/export'), { path: filePath, source }, 60000);
        },
        moduleGetLocales: async (moduleName: string) => {
            try {
                return (await get(`/api/module/${encodeURIComponent(moduleName)}/locales`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.error(`Failed to load locales for module ${moduleName}:`, error.message);
                return {};
            }
        },
        moduleListVersions: async (
            moduleName: string,
            options: { include_snapshots?: boolean; page?: number; per_page?: number; settings?: ApiBody } = {},
        ) => {
            try {
                const qs = queryString({
                    include_snapshots: options?.include_snapshots ? 'true' : undefined,
                    page: options?.page || undefined,
                    per_page: options?.per_page || undefined,
                    // 고른 설정(서버 종류 등): 그 설정으로 받을 수 있는 버전만
                    settings: options?.settings ? JSON.stringify(options.settings) : undefined,
                });
                return (await get(`/api/module/${encodeURIComponent(moduleName)}/versions${qs}`, { timeout: 15000 }))
                    .data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        moduleInstallServer: async (moduleName: string, installConfig: ApiBody) => {
            try {
                // 데몬이 install_server에 30분까지 허용한다 — 그보다 먼저 끊으면 설치가 도는 중에 실패로 보인다 (1분 여유)
                return (
                    await post(`/api/module/${encodeURIComponent(moduleName)}/install`, installConfig, {
                        timeout: 1860000,
                    })
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        instanceGetInstalledVersion: async (instanceId: string) => {
            try {
                return (
                    await get(`/api/instance/${encodeURIComponent(instanceId)}/installed-version`, { timeout: 10000 })
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        /** GET /api/maker — 모듈 메이커 안내 (저장소·가이드 주소, 이 PC의 경로) */
        makerInfo: async () => {
            try {
                return (await get('/api/maker')).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error, { ok: false });
            }
        },
        moduleManifest: async () => {
            try {
                return (await get('/api/modules/manifest', { timeout: 15000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error, { ok: false });
            }
        },
        moduleInstallFromManifest: async (moduleId: string) => {
            try {
                return (
                    await post(`/api/modules/manifest/${encodeURIComponent(moduleId)}/install`, {}, { timeout: 120000 })
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error, { ok: false });
            }
        },
        moduleRemove: async (moduleId: string) => {
            try {
                return (await del(`/api/modules/${encodeURIComponent(moduleId)}`, { timeout: 15000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error, { ok: false });
            }
        },
        instanceCreate: async (data: ApiBody) => {
            try {
                // 새 설치는 데몬이 백그라운드로 처리하므로 짧은 타임아웃으로 충분. 기존 서버 가져오기는 큰 폴더에 권한을 주고
                // 설정을 읽느라 오래 걸릴 수 있어 시작처럼 기다린다 (CLI `post_long`과 같다)
                const timeout = data?.migration_source ? START_TIMEOUT_MS : 30000;
                return (await post('/api/instances', data, { timeout })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const status = error.response.status;
                    const errData = error.response.data;
                    const body = errData && typeof errData === 'object' ? errData : {};
                    // error_code(working_dir_in_use 등)·instance를 렌더러가 쓸 수 있게 본문을 그대로 넘긴다
                    switch (status) {
                        case 400:
                            return { ...body, error: body.error || t('errors.invalid_input') };
                        case 409:
                            return { ...body, error: body.error || t('errors.instance_name_taken') };
                        case 500:
                            return {
                                ...body,
                                error: t('errors.create_error', {
                                    error: body.error || body.message || t('errors.internal'),
                                }),
                            };
                        default:
                            return {
                                ...body,
                                error: t('errors.create_http', {
                                    status: String(status),
                                    error: body.error || error.message,
                                }),
                            };
                    }
                }
                if (error.code === 'ECONNREFUSED') return { error: t('network.connection_refused') };
                return { error: t('server.creation_failed', { error: error.message }) };
            }
        },
        // 만들기 전 확인: 이름·서버 폴더·포트가 다른 서버와 겹치는지 (아무것도 만들지 않는다)
        instanceCheck: async (data: ApiBody) => {
            try {
                return (await post('/api/instances/check', data, { timeout: 15000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        instanceProvisionProgress: async (name: string) => {
            try {
                return (await get(`/api/provision-progress/${encodeURIComponent(name)}`, { timeout: 3000 })).data;
            } catch (_error) {
                return { active: false };
            }
        },
        instanceDismissProvision: async (name: string) => {
            try {
                return (await del(`/api/provision-progress/${encodeURIComponent(name)}`, { timeout: 3000 })).data;
            } catch (_error) {
                return { success: false };
            }
        },
        instanceCheckUpdate: async (id: string) => {
            try {
                // SteamCMD app_info_print는 느릴 수 있다
                return (await get(`/api/instance/${encodeURIComponent(id)}/check-update`, { timeout: 30000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.error('instance:checkUpdate error:', error.message);
                return { update_available: false, error: error.message };
            }
        },
        instanceApplyUpdate: async (id: string) => {
            try {
                return (await post(`/api/instance/${encodeURIComponent(id)}/apply-update`, {}, { timeout: 10000 }))
                    .data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.error('instance:applyUpdate error:', error.message);
                return errorBody(error, { success: false });
            }
        },
        instanceDelete: async (id: string, { keepSettings = false } = {}) => {
            try {
                // 데몬은 정지 대기·pre_delete 훅·폴더 삭제(재시도 포함)를 마치고 답한다.
                // keepSettings: 가져온 서버의 게임 설정 파일을 가져오기 전 내용으로 되돌리지 않는다
                const query = keepSettings ? '?keep_settings=true' : '';
                return (await del(`/api/instance/${encodeURIComponent(id)}${query}`, { timeout: 120000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    const status = error.response.status;
                    const data = error.response.data || {};
                    switch (status) {
                        case 404:
                            return { error: t('errors.instance_not_found') };
                        case 409:
                            // 백업·복원·업데이트와 겹치면 instance_busy: 정지해도 풀리지 않는다
                            return data.error_code === 'instance_busy'
                                ? { error: t('errors.instance_busy'), error_code: data.error_code }
                                : { error: t('errors.instance_running'), error_code: data.error_code };
                        case 500:
                            return {
                                error: t('errors.delete_error', {
                                    error: data.error || data.message || t('errors.internal'),
                                }),
                            };
                        default:
                            return {
                                error: t('errors.delete_http', {
                                    status: String(status),
                                    error: data.error || error.message,
                                }),
                            };
                    }
                }
                if (error.code === 'ECONNREFUSED') return { error: t('network.connection_refused') };
                // 시간이 다 돼도 데몬은 계속 지우고 있을 수 있다: 화면이 목록으로 다시 확인한다
                return { error: t('server.delete_failed', { error: error.message }), error_code: error.code };
            }
        },
        instanceReorder: async (orderedIds: string[]) => {
            try {
                return (await put('/api/instances/reorder', { order: orderedIds })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response) {
                    return {
                        error:
                            error.response.data?.error ||
                            t('errors.reorder_failed', { error: `HTTP ${error.response.status}` }),
                    };
                }
                return { error: t('errors.reorder_failed', { error: error.message }) };
            }
        },
        // 인스턴스 하나의 지금 설정과 revision (GET /api/instance/:id)
        instanceGet: (id: string) => daemonCall('get', instancePath(id)),
        instanceUpdateSettings: async (id: string, settings: ApiBody) => {
            try {
                // 값에는 비밀번호가 들어갈 수 있어 키 이름만 남긴다
                console.log(`[Bridge] Updating settings for instance ${id}:`, Object.keys(settings || {}));
                // 데몬은 잠금을 기다리고 모듈 configure를 돌린 뒤 답한다: CLI(SLOW)처럼 기다린다
                return (await request('patch', `/api/instance/${encodeURIComponent(id)}`, settings, { timeout: 60000 }))
                    .data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.error('[Bridge] Error updating settings:', error.message);
                if (error.response) {
                    const status = error.response.status;
                    const data = error.response.data || {};
                    if (data.module && data.function) return errorBody(error);
                    switch (status) {
                        case 400:
                            // 데몬의 검증 결과를 그대로: error_code가 invalid_format이면 renderer가 형식 오류를 띄운다
                            return {
                                status,
                                error: t('errors.settings_invalid', {
                                    error: data.message || data.error || t('errors.invalid_input'),
                                }),
                                error_code: data.error_code,
                                details: data.details,
                                fields: data.fields,
                            };
                        case 404:
                            return { error: t('errors.instance_not_found') };
                        case 500:
                            return {
                                error: t('errors.settings_error', {
                                    error: data.error || data.message || t('errors.internal'),
                                }),
                            };
                        default:
                            // 409(working_dir_in_use·instance_busy·settings_changed)는 이유를 넘겨 설정 화면이 문구로 보이게 한다
                            return {
                                error: t('errors.settings_http', {
                                    status: String(status),
                                    error: data.error || error.message,
                                }),
                                error_code: data.error_code,
                                instance: data.instance,
                            };
                    }
                }
                if (error.code === 'ECONNREFUSED') return { error: t('network.connection_refused') };
                return { error: t('server.settings_save_failed', { error: error.message }) };
            }
        },
        // 실행 인자 미리보기: extraArgs를 넘기지 않으면 저장된 추가 인자로 본다
        instanceLaunchPreview: (id: string, settings?: ApiBody | null, extraArgs?: string | string[] | null) => {
            const body: ApiBody = { settings: settings || {} };
            if (extraArgs !== undefined && extraArgs !== null) body.extra_args = extraArgs;
            return daemonCall('post', `/api/instance/${encodeURIComponent(id)}/launch-preview`, body, 15000);
        },
        instanceResetServer: async (instanceId: string) => {
            try {
                return (
                    await post(`/api/instance/${encodeURIComponent(instanceId)}/server/reset`, {}, { timeout: 30000 })
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        // 게임 명령 — 해석과 라우팅은 데몬이 한다
        executeCommand: async (id: string, command: { command?: string; args?: Record<string, unknown> | null }): Promise<ApiBody> => {
            const payload = {
                command: (command.command || '').trim(),
                args: command.args && Object.keys(command.args).length > 0 ? command.args : null,
            };
            try {
                return (await post(`/api/instance/${encodeURIComponent(id)}/command`, payload, { timeout: 30000 }))
                    .data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.error('[Bridge] Error executing command:', error.message, error.response?.data || '');
                if (error.response) {
                    const data = error.response.data || {};
                    // 데몬이 모듈 [errors] 문구(error_message) 또는 원인(error)을 준다
                    return {
                        ...(data.module && data.function ? data : {}),
                        error:
                            data.error_message ||
                            data.error ||
                            t('errors.http', { status: String(error.response.status) }),
                        error_code: data.error_code,
                        field: data.field,
                    };
                }
                const networkErrors: Record<string, string> = {
                    ECONNREFUSED: t('network.connection_refused'),
                    ETIMEDOUT: t('network.timeout'),
                };
                return { error: networkErrors[error.code ?? ''] || t('errors.command_failed', { error: error.message }) };
            }
        },
        instanceStop: (id: string, opts: { force?: boolean } = {}) =>
            daemonCall(
                'post',
                `/api/instance/${encodeURIComponent(id)}/stop`,
                { force: !!opts?.force },
                STOP_TIMEOUT_MS,
            ),
        // 재시작은 데몬이 프로세스 종료를 stop_timeout까지 기다린 뒤 다시 켠다
        instanceRestart: (id: string, opts: { force?: boolean } = {}) =>
            daemonCall(
                'post',
                `/api/instance/${encodeURIComponent(id)}/restart`,
                { force: !!opts?.force },
                STOP_TIMEOUT_MS,
            ),
        instanceRename: (id: string, name: string) => daemonCall('post', `/api/instance/${encodeURIComponent(id)}/rename`, { name }),
        instanceCommands: (id: string) => daemonCall('get', `/api/instance/${encodeURIComponent(id)}/commands`),
        // 지금 접속한 사람 이름 (모듈 players 함수가 게임에 묻는다). 부를 때만 묻고 주기적으로 부르지 않는다
        instancePlayers: (id: string) => daemonCall('get', `/api/instance/${encodeURIComponent(id)}/players`),
        // 서버 점검: 모듈 validate(켜기 전 점검)·diagnose(최근 로그 진단). 모듈 Python을 돌리므로 넉넉히 기다린다
        instanceValidate: (id: string) => daemonCall('post', `/api/instance/${encodeURIComponent(id)}/validate`, {}, 60000),
        instanceDiagnose: (id: string) => daemonCall('post', `/api/instance/${encodeURIComponent(id)}/diagnose`, {}, 60000),
        // 포트포워딩 확인: 릴레이가 밖에서 게임 포트에 접속해 본다 (몇 초). 마지막 결과는 자동 확인 것도 같이 본다
        instancePortCheck: (id: string) => daemonCall('post', `/api/instance/${encodeURIComponent(id)}/port-check`, {}, 30000),
        instancePortCheckLast: (id: string) => daemonCall('get', `/api/instance/${encodeURIComponent(id)}/port-check`),
        // 설정 화면용: 디스크의 실제 설정 (실패하면 success:false)
        instanceReadLiveSettings: (id: string) =>
            daemonCall('get', `/api/instance/${encodeURIComponent(id)}/settings/live`, undefined, 60000),
        runtimePreflight: () => daemonCall('get', '/api/runtime/preflight', undefined, 5000),
        // sizes=false면 데몬이 인스턴스 폴더 크기를 새로 재지 않는다. gpu=true일 때만 GPU를 읽는다(NVIDIA 라이브러리를 올린다)
        systemMetrics: (opts: { sizes?: boolean; gpu?: boolean } = {}) => {
            const query = [opts?.sizes === false && 'sizes=false', opts?.gpu && 'gpu=true'].filter(Boolean).join('&');
            return daemonCall('get', `/api/system/metrics${query ? `?${query}` : ''}`, undefined, 5000);
        },
        // keys를 생략하면 kind 전체를 기본값으로 되돌린다
        configReset: (kind: string, keys?: string[]) =>
            daemonCall('post', `/api/config/${encodeURIComponent(kind)}/reset`, keys ? { keys } : {}),

        // ── 익스텐션 ──
        extensionList: async (): Promise<ApiResult<ExtensionListResponse>> => {
            try {
                return (await get<ExtensionListResponse>('/api/extensions')).data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.warn('[Extension] Failed to list extensions:', error.message);
                // error가 있으면 받은 목록이 아니다: 받아 둔 목록을 빈 목록으로 덮지 않게
                return { extensions: [], error: error.message || 'extension list unavailable' };
            }
        },
        extensionEnable: async (extId: string) => {
            try {
                return (await post(`/api/extensions/${encodeURIComponent(extId)}/enable`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return extensionFailure(extId, 'enable', error, true);
            }
        },
        extensionDisable: async (extId: string) => {
            try {
                return (await post(`/api/extensions/${encodeURIComponent(extId)}/disable`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return extensionFailure(extId, 'disable', error, true);
            }
        },
        extensionAudit: async (extId: string) => {
            try {
                return (await get(`/api/extensions/${encodeURIComponent(extId)}/audit`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        // 익스텐션 설정 (manifest config_fields). Set은 값 객체를 그대로 보낸다(키 단위 병합, null이면 삭제)
        extensionConfigGet: (extId: string) => daemonCall('get', `/api/extensions/${encodeURIComponent(extId)}/config`),
        extensionConfigSet: (extId: string, config: ApiBody) =>
            daemonCall('put', `/api/extensions/${encodeURIComponent(extId)}/config`, config),
        extensionI18n: async (extId: string, locale: string) => {
            try {
                return (await get(`/api/extensions/${encodeURIComponent(extId)}/i18n/${encodeURIComponent(locale)}`))
                    .data;
            } catch (caught) {
                const error = caught as DaemonError;
                // 404는 해당 로케일이 없는 것이므로 경고 없이 null
                if (error.response?.status !== 404) {
                    console.warn(`[Extension] Failed to load i18n for '${extId}' (${locale}):`, error.message);
                }
                return null;
            }
        },
        // 익스텐션 GUI 번들 (JS 소스 텍스트). 이 웹뷰에서 실행하므로 번들·스타일은 원격 데몬을 고른 동안에도 이 PC 데몬에서 받는다
        extensionGuiBundle: async (extId: string) => {
            try {
                const { data } = await get(`/api/extensions/${encodeURIComponent(extId)}/gui`, {
                    responseType: 'text',
                    target: 'local',
                });
                return typeof data === 'string' ? data : null;
            } catch (caught) {
                const error = caught as DaemonError;
                // 확인되지 않았고 이 PC에서 허락하지 않은 번들 (허락 뒤 파일이 바뀐 경우 포함)
                if (guiUnapproved(error)) return { error_code: 'extension_gui_unapproved' };
                if (error.response?.status !== 404) {
                    console.warn(`[Extension] Failed to load GUI bundle for '${extId}':`, error.message);
                }
                return null;
            }
        },
        extensionGuiStyles: async (extId: string) => {
            try {
                const { data } = await get(`/api/extensions/${encodeURIComponent(extId)}/gui/styles`, {
                    responseType: 'text',
                    target: 'local',
                });
                return typeof data === 'string' ? data : null;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response?.status !== 404) {
                    console.warn(`[Extension] Failed to load GUI styles for '${extId}':`, error.message);
                }
                return null;
            }
        },
        // 익스텐션 아이콘 (PNG → data:image/png;base64)
        extensionIcon: async (extId: string) => {
            try {
                const { data } = await get(`/api/extensions/${encodeURIComponent(extId)}/icon`, {
                    responseType: 'base64',
                });
                return typeof data === 'string' ? `data:image/png;base64,${data}` : null;
            } catch (caught) {
                const error = caught as DaemonError;
                if (error.response?.status !== 404) {
                    console.warn(`[Extension] Failed to load icon for '${extId}':`, error.message);
                }
                return null;
            }
        },
        extensionFetchManifest: async () => {
            try {
                return (await get('/api/extensions/manifest', { timeout: 15000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.warn('[Extension] Failed to fetch manifest:', error.message);
                return { success: false, error: error.message, extensions: [], updates: [] };
            }
        },
        extensionInstall: async (extId: string, opts = {}) => {
            try {
                return (
                    await post(`/api/extensions/${encodeURIComponent(extId)}/install`, opts || {}, { timeout: 60000 })
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return extensionFailure(extId, 'install', error, false);
            }
        },
        extensionRemove: async (extId: string) => {
            try {
                return (await del(`/api/extensions/${encodeURIComponent(extId)}`, { timeout: 15000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return extensionFailure(extId, 'remove', error, false);
            }
        },
        extensionCheckUpdates: async () => {
            try {
                return (await get('/api/extensions/updates', { timeout: 15000 })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.warn('[Extension] Failed to check updates:', error.message);
                return { success: false, error: error.message, updates: [], count: 0 };
            }
        },
        extensionRescan: async () => {
            try {
                return (await post('/api/extensions/rescan')).data;
            } catch (caught) {
                const error = caught as DaemonError;
                console.warn('[Extension] Failed to rescan extensions:', error.message);
                return { success: false, error: error.message, newly_found: [] };
            }
        },
        extensionInitStatus: async () => {
            try {
                return (await get('/api/extensions/init-status', { timeout: 5000 })).data;
            } catch (_error) {
                // 데몬 미연결: 스피너가 무한히 돌지 않게 initializing:false
                return { initializing: false, in_progress: {}, completed: [], daemon_unreachable: true };
            }
        },

        // ── 관리 프로세스 콘솔 ──
        // opts.spans: 데몬이 나눈 하이라이트 조각도 받는다
        managedConsole: async (instanceId: string, since = 0, count = 200, opts: { spans?: boolean } = {}) => {
            try {
                const qs = queryString({ since, count, spans: opts?.spans ? 1 : undefined });
                return (await get(`/api/instance/${encodeURIComponent(instanceId)}/console${qs}`)).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return errorBody(error);
            }
        },
        managedStdin: async (instanceId: string, command: string) => {
            try {
                return (await post(`/api/instance/${encodeURIComponent(instanceId)}/stdin`, { command })).data;
            } catch (caught) {
                const error = caught as DaemonError;
                // 답을 못 받았으면 보냈는지 모른다: 화면이 다른 길로 다시 보내지 않게 연결 오류 코드를 넘긴다
                if (!error.response) return { error: error.message, error_code: error.code };
                return errorBody(error);
            }
        },

        // ── 콘솔 팝아웃 (창은 Rust가 만든다) ──
        consolePopout: (instanceId: string, serverName: string) => invoke('console_popout', { instanceId, serverName }),
        consolePopoutList: () => invoke('console_popout_list'),
        daemonConsole: (opts?: { since?: number; count?: number }) => {
            const { since, count } = opts || {};
            const qs = queryString({ since, count: count || undefined });
            return daemonCall('get', `/api/daemon/console${qs}`, undefined, 10000);
        },
        consolePopin: (instanceId: string) => invoke('console_popin', { instanceId }),
        consoleToggleAlwaysOnTop: (instanceId: string, pinned: boolean) =>
            invoke('console_toggle_always_on_top', { instanceId, pinned }),
        // 셸 이벤트 구독은 그 구독만 떼는 함수를 돌려준다 (다른 화면의 구독은 남는다)
        onConsolePopoutOpened: (callback: Listener) => hub.on('console:popoutOpened', callback),
        onConsolePopoutClosed: (callback: Listener) => hub.on('console:popoutClosed', callback),
        onConsolePopinRequest: (callback: Listener) => hub.on('console:popinRequest', callback),

        // ── 업데이터 (데몬 HTTP API) ──
        updaterCheck: async () => {
            try {
                const { data } = await post('/api/updates/check', {}, { timeout: 30000 });
                // 배너·알림 모달용 이벤트. Locales는 백그라운드 자동 적용이라 뺀다. 받을 것이 없으면 count 0으로
                // 알려 남은 업데이트 알림을 지운다
                const visibleNames = (data?.update_names || []).filter((n: string) => n !== 'Locales');
                const visibleComponents = (data?.components || []).filter((c: { display_name?: string }) => c.display_name !== 'Locales');
                if (data?.ok && !data?.error) {
                    hub.dispatch('updates:available', {
                        count: visibleNames.length,
                        updates_available: visibleNames.length,
                        names: visibleNames,
                        update_names: visibleNames,
                        components: visibleComponents,
                    });
                }
                return data;
            } catch (caught) {
                const err = caught as DaemonError;
                return { ok: false, error: err.message };
            }
        },
        updaterStatus: () => updaterCall(() => get('/api/updates/status', { timeout: 5000 })),
        updaterDownload: (components: string[]) =>
            updaterCall(() =>
                post(
                    '/api/updates/download',
                    { components: Array.isArray(components) ? components : [] },
                    { timeout: 600000 },
                ),
            ),
        updaterDownloadProgress: () => updaterCall(() => get('/api/updates/download/progress', { timeout: 5000 })),
        // 모듈은 데몬이 직접 적용, 데몬/GUI/CLI는 needs_updater에 들어간다
        updaterApply: async (components: string[]) => {
            try {
                const body = { components: Array.isArray(components) ? components : [] };
                const { data } = await post('/api/updates/apply', body, { timeout: 120000 });
                // 일부만 적용됐으면(ok:false) 성공 알림을 띄우지 않는다: 실패는 부른 화면이 보인다 (G-U9)
                if (data?.ok !== false && data?.applied && data.applied.length > 0) {
                    hub.dispatch('updates:completed', {
                        count: data.applied.length,
                        names: data.applied,
                        requiresUpdater: !!data.requires_updater,
                        needsUpdater: data.needs_updater || [],
                    });
                }
                return data;
            } catch (caught) {
                const err = caught as DaemonError;
                return { ok: false, error: err.message };
            }
        },
        // 앱을 열 때 한 번: '받고 다음 실행 때 적용'이면 데몬이 받아 둔 업데이트를 적용한다.
        // 켜진 서버가 있으면 재시작이 필요한 것은 데몬이 미룬다(held). 적용 결과는 updaterApply와 같은 모양
        updaterApplyOnLaunch: async () => {
            try {
                const { data } = await post('/api/updates/apply-on-launch', {}, { timeout: 120000 });
                if (data?.ok !== false && data?.applied && data.applied.length > 0) {
                    hub.dispatch('updates:completed', {
                        count: data.applied.length,
                        names: data.applied,
                        requiresUpdater: !!data.requires_updater,
                        needsUpdater: data.needs_updater || [],
                    });
                }
                return data;
            } catch (caught) {
                const err = caught as DaemonError;
                return { ok: false, error: err.message };
            }
        },
        // 업데이터 실행은 데몬이 한다. 다시 띄울 exe 경로와 종료는 Rust가 맡는다
        updaterLaunchApply: (_targets?: unknown) => invoke('updater_launch_apply'),
        // clear면 확인 처리, 아니면 이 언어의 패치노트를 고른다
        updaterLastResult: async ({ clear = false } = {}) => {
            if (clear) return updaterCall(() => del('/api/updates/last-result', { timeout: 5000 }));
            let lang: string | undefined;
            try {
                lang = await invoke('language_get');
            } catch (_) {
                lang = i18n.language;
            }
            return updaterCall(() => get(`/api/updates/last-result${queryString({ lang })}`, { timeout: 5000 }));
        },
        updaterGetConfig: () => updaterCall(() => get('/api/updates/config', { timeout: 5000 })),
        updaterSetConfig: (config: ApiBody) => updaterCall(() => put('/api/updates/config', config, { timeout: 5000 })),
        onUpdatesAvailable: (callback: Listener) => {
            hub.on('updates:available', callback);
        },
        onUpdateCompleted: (callback: Listener) => {
            hub.on('updates:completed', callback);
        },

        // ── 설정: 데몬 가동 중이면 API, 부트스트랩이면 로컬 ──
        settingsLoad: async (): Promise<Partial<GuiSettings> | null> => {
            try {
                return (await get<GuiSettings>('/api/config/gui', { timeout: 5000 })).data;
            } catch (caught) {
                const err = caught as DaemonError;
                console.warn('[settings:load] Daemon API failed, falling back to local:', err.message);
            }
            return loadLocalSettings();
        },
        settingsSave: (settings: Partial<GuiSettings>): Promise<boolean> => {
            const run = settingsQueue.then(() => saveSettings(settings));
            settingsQueue = run.catch(() => {});
            return run;
        },
        settingsGetPath: () => invoke('settings_get_path'),

        // ── 대화상자 ──
        openFileDialog: (options?: { filters?: DialogArgs['filters']; title?: string }) => pickLocal('dialog_open_file', { filters: options?.filters ?? null }),
        openFolderDialog: () => pickLocal('dialog_open_folder'),

        // 고른 폴더가 어느 게임 서버 폴더인지 (데몬의 게임 폴더 판정만 쓴다)
        migrationScanDir: async (dirPath: string): Promise<ApiBody> => {
            try {
                if (!dirPath || typeof dirPath !== 'string') return { error: 'Invalid directory path' };
                const { data } = await post('/api/modules/detect', { path: dirPath });
                return { ok: true, ...data };
            } catch (caught) {
                const error = caught as DaemonError;
                return { error: error.response?.data?.error || error.message };
            }
        },

        // ── 디스코드 릴레이 에이전트 (데몬 내장) ──
        discordBotStatus: async (): Promise<AgentStatus> => {
            try {
                const { data } = await get('/api/relay/agent', { timeout: 3000 });
                // 릴레이가 이 앱 버전을 받지 않는다: 세션은 살아 있어도 명령이 오지 않는다
                if (data?.lastErrorCode === 'agent_too_old')
                    return { status: 'too_old', running: !!data.running, minVersion: data.minVersion ?? null };
                if (data?.running) return 'running';
                // 같은 디스코드 계정이 다른 PC를 페어링해서 데몬이 에이전트를 멈췄다
                if (data?.lastErrorCode === 'token_replaced') return 'replaced';
                // 저장된 릴레이 주소가 https(루프백만 http)가 아니어서 데몬이 에이전트를 띄우지 않았다
                return data?.lastErrorCode === 'invalid_relay_url' ? 'invalid_relay_url' : 'stopped';
            } catch (_e) {
                // 늦거나 닿지 못했다: 꺼졌는지 모른다 (스토어가 두 번 연달아 모를 때만 꺼짐으로 본다)
                return 'unknown';
            }
        },
        discordBotStart: async () => {
            try {
                return (await post('/api/relay/agent/start', {}, { timeout: 10000 })).data;
            } catch (caught) {
                const e = caught as DaemonError;
                const errData = e.response?.data;
                if (errData?.error_code === 'node_token_missing') return { error: 'cloud_token_not_found' };
                if (errData?.error_code === 'invalid_relay_url') return { error: 'invalid_relay_url' };
                return { error: errData?.error || e.message };
            }
        },
        discordBotStop: async () => {
            try {
                return (await post('/api/relay/agent/stop', {}, { timeout: 10000 })).data;
            } catch (caught) {
                const e = caught as DaemonError;
                return errorBody(e);
            }
        },
        // 못 읽으면 던진다. 기본값을 돌려주면 호출한 쪽이 그 값을 저장해 디스크의 설정을 지운다
        botConfigLoad: async () => (await get('/api/config/bot')).data,
        // 데몬이 bot-config의 원천. `config`는 바뀐 칸만 담은 JSON Merge Patch다 (null은 지우기)
        botConfigSave: async (config: ApiBody) => {
            try {
                // 데몬 본문을 그대로: 권한 키라 저장하지 않은 경로는 `ignored`로 온다 (L61)
                return (await request('patch', '/api/config/bot', config || {})).data || { success: true };
            } catch (caught) {
                const error = caught as DaemonError;
                console.error('Failed to save bot config via daemon:', error.message);
                // error_code(invalid_relay_url 등)를 렌더러가 쓸 수 있게 본문을 넘긴다
                return errorBody(error);
            }
        },
        saveNodeToken: async (token: string): Promise<boolean> => {
            try {
                await put('/api/config/node-token', { token });
                return true;
            } catch (caught) {
                const e = caught as DaemonError;
                console.error('[NodeToken] Failed to save via daemon:', e.message);
                return false;
            }
        },
        // 연결 해제: 데몬이 릴레이 에이전트를 멈추고 토큰을 지운다
        deleteNodeToken: () => daemonCall('delete', '/api/config/node-token'),

        auditList: (opts?: { limit?: number; instance?: string; since?: string | number; until?: string | number }) => {
            const { limit, instance, since, until } = opts || {};
            const qs = queryString({ limit: limit || undefined, instance: instance || undefined, since, until });
            return daemonCall('get', `/api/audit${qs}`, undefined, 10000);
        },
        // 기간({since, until} unix 초)의 기록을 데몬이 CSV로 쓴다. 저장 창을 닫으면 {canceled:true}
        auditExport: async (opts: { instance?: string; since?: string | number; until?: string | number } = {}) => {
            const { instance, since, until } = opts;
            const filePath = await pickLocal('dialog_save_file', {
                defaultPath: `saba-chan-activity-${localDate()}.csv`,
                filters: [{ name: 'CSV', extensions: ['csv'] }],
            });
            if (!filePath) return { canceled: true };
            return daemonCall('post', '/api/audit/export', { path: filePath, instance, since, until }, 60000);
        },
        permissionsGet: (guildId: string) => daemonCall('get', permPath(guildId)),
        permissionsUpdate: (guildId: string, patch: ApiBody) => daemonCall('patch', permPath(guildId), patch || {}),
        permissionsCommands: (guildId: string, locale: string) =>
            daemonCall('get', `${permPath(guildId)}/commands${queryString({ locale: locale || undefined })}`),
        permissionsSetMember: (guildId: string, discordId: string, level: string | number) => daemonCall('put', permPath(guildId, discordId), { level }),
        permissionsRemoveMember: (guildId: string, discordId: string) => daemonCall('delete', permPath(guildId, discordId)),
        sandboxListUnisolated: () => daemonCall('get', '/api/sandbox/unisolated'),
        // 동의 기록은 셸이 네이티브 확인 창을 띄운 뒤에 보낸다. 취소하면 {success:false, error_code:'canceled'}
        sandboxAllowUnisolated: (component: string) => invoke('sandbox_consent_allow', { kind: 'unisolated', component }),
        sandboxRevokeUnisolated: (component: string) => daemonCall('delete', unisolatedPath(component)),
        sandboxListExtensionGui: () => daemonCall('get', '/api/sandbox/extension-gui'),
        sandboxAllowExtensionGui: (component: string) => invoke('sandbox_consent_allow', { kind: 'extension-gui', component }),
        sandboxRevokeExtensionGui: (component: string) => daemonCall('delete', extensionGuiPath(component)),
        actionsList: (opts?: { surface?: string; instance?: string; locale?: string }) => {
            const { surface = 'gui', instance, locale } = opts || {};
            const qs = queryString({ surface, instance: instance || undefined, locale: locale || undefined });
            return daemonCall('get', `/api/actions${qs}`);
        },
        actionsPlan: (id: string, body: ApiBody) => daemonCall('post', `/api/actions/${encodeURIComponent(id)}/plan`, body || {}),
        actionsRun: (id: string, body: ApiBody) => daemonCall('post', `/api/actions/${encodeURIComponent(id)}/run`, body || {}),
        // 인스턴스별 디스코드 부르는 말 (어느 디스코드 서버에서나 같다)
        instanceWords: (id: string) => daemonCall('get', `/api/instance/${encodeURIComponent(id)}/words`),
        instanceWordsSave: (id: string, body: ApiBody) =>
            daemonCall('put', `/api/instance/${encodeURIComponent(id)}/words`, body || {}),
        // 길드가 쓸 서버 제한 `{instances: [id] | null}` (null = 제한 없음) → `{guildId, allowedInstances}`
        guildAllowedInstancesSave: (guild: string, instances: string[] | null) =>
            daemonCall('put', `/api/guilds/${encodeURIComponent(guild)}/allowed-instances`, { instances }),
        // 켜짐 알림을 켠 서버 → `{guildId, instances, allowedInstances}` (allowedInstances null = 모든 서버)
        guildStartNoticeGet: (guild: string) => daemonCall('get', `/api/guilds/${encodeURIComponent(guild)}/start-notice`),
        // 켜짐 알림 `{enabled}` → `{success, id, guildId, enabled, instances}` (그 길드에서 켠 서버 목록)
        guildStartNoticeSave: (id: string, guild: string, enabled: boolean) =>
            daemonCall('put', `/api/instance/${encodeURIComponent(id)}/start-notice/${encodeURIComponent(guild)}`, {
                enabled,
            }),
        // 자리 꽉 참 알림을 켠 서버 → `{guildId, instances, allowedInstances}`
        guildFullNoticeGet: (guild: string) => daemonCall('get', `/api/guilds/${encodeURIComponent(guild)}/full-notice`),
        // 자리 꽉 참 알림 `{enabled}` → `{success, id, guildId, enabled, instances}` (켜짐 알림과 같은 모양)
        guildFullNoticeSave: (id: string, guild: string, enabled: boolean) =>
            daemonCall('put', `/api/instance/${encodeURIComponent(id)}/full-notice/${encodeURIComponent(guild)}`, {
                enabled,
            }),
        discordSettingsGet: (guild: string) => daemonCall('get', discordSettingsPath(guild)),
        discordSettingsSet: (guild: string, patch: ApiBody) => daemonCall('patch', discordSettingsPath(guild), patch || {}),
        // GUI ⇄ CLI 전환: 데몬이 이 PC에서 띄울 수 있는 인터페이스 `{cli, gui}`, 그리고 띄우기 (고른 서버로 연다)
        clientsLaunchable: () => daemonCall('get', '/api/clients/launch'),
        clientLaunch: (opts?: { kind?: string; instance?: string; daemon?: string }) => {
            const { kind, instance, daemon } = opts || {};
            return daemonCall('post', '/api/clients/launch', { kind, instance, ...(daemon ? { daemon } : {}) });
        },
        // 데몬 이벤트 스트림: cb({id, type, data}). 돌려주는 함수로 해제
        onDaemonEvent: (callback: Listener) => hub.on('events:message', callback),

        // 내보내기/가져오기: 파일은 데몬이 읽고 쓴다. 여기서는 경로만 고른다
        dataExport: async (opts: { secrets?: boolean } = {}) => {
            const filePath = await pickLocal('dialog_save_file', {
                defaultPath: `saba-chan-export-${localDate()}.zip`,
                filters: [{ name: 'Zip', extensions: ['zip'] }],
            });
            if (!filePath) return { canceled: true };
            return daemonCall('post', '/api/migration/export', { path: filePath, secrets: !!opts?.secrets }, 120000);
        },
        // path 없이 부르면 파일을 고르게 하고, 결과에 path를 붙인다 (dryRun 뒤 같은 path로 다시 부른다)
        dataImport: async (opts: { path?: string; dryRun?: boolean; overwrite?: boolean } = {}): Promise<ApiBody> => {
            let filePath = typeof opts?.path === 'string' && opts.path ? opts.path : null;
            if (!filePath) {
                filePath = await pickLocal('dialog_open_file', { filters: [{ name: 'Zip', extensions: ['zip'] }] });
                if (!filePath) return { canceled: true };
            }
            const body = await daemonCall(
                'post',
                '/api/migration/import',
                { path: filePath, dryRun: !!opts?.dryRun, overwrite: !!opts?.overwrite },
                120000,
            );
            return { ...body, path: filePath };
        },

        // ── 릴레이 (데몬 경유) ──
        relayCheckHostStatus: async (hostId: string, relayUrl: string) => {
            try {
                return (
                    await get(`/api/relay/host/${encodeURIComponent(hostId)}/status${relayQs(relayUrl)}`, {
                        timeout: 8000,
                    })
                ).data;
            } catch (caught) {
                const err = caught as DaemonError;
                return { ok: false, error: err.message };
            }
        },
        relayListHostNodes: async (hostId: string, relayUrl: string) => {
            try {
                return (
                    await get(`/api/relay/host/${encodeURIComponent(hostId)}/nodes${relayQs(relayUrl)}`, {
                        timeout: 8000,
                    })
                ).data;
            } catch (caught) {
                const err = caught as DaemonError;
                return errorBody(err);
            }
        },
        // `{members, partial, reason?}`: 디스코드 전체 목록을 못 받으면 권한을 등록한 멤버만 (partial)
        relayListNodeMembers: async (guildId: string, relayUrl: string) => {
            const qs = relayUrl ? `&relayUrl=${encodeURIComponent(relayUrl)}` : '';
            try {
                return (
                    await get(`/api/relay/node/${encodeURIComponent(guildId)}/members?withStatus=1${qs}`, {
                        // 데몬이 디스코드 전체 목록을 15초까지 기다린 뒤 폴백한다
                        timeout: 20000,
                    })
                ).data;
            } catch (caught) {
                const err = caught as DaemonError;
                return errorBody(err);
            }
        },
        // 릴레이에 남은 권한 부여값 `[{id, username, displayName, level}]` (가져오기 후보)
        relayListLegacyMembers: async (guildId: string) => {
            try {
                return (await get(`/api/relay/node/${encodeURIComponent(guildId)}/legacy-members`, { timeout: 15000 }))
                    .data;
            } catch (caught) {
                const err = caught as DaemonError;
                return errorBody(err);
            }
        },
        relayBotInvite: async (relayUrl: string) => {
            try {
                return (await get(`/api/relay/bot-invite${relayQs(relayUrl)}`, { timeout: 8000 })).data;
            } catch (caught) {
                const err = caught as DaemonError;
                return errorBody(err);
            }
        },
        relayNodeChannels: (guildId: string) =>
            daemonCall('get', `/api/relay/node/${encodeURIComponent(guildId)}/channels`, undefined, 15000),
        relayNodeChannelsSave: (guildId: string, patch: ApiBody) =>
            daemonCall('put', `/api/relay/node/${encodeURIComponent(guildId)}/channels`, patch, 15000),
        // 서버 관리: 채널 상태·음소거 목록, 채널 수정(이름·주제·슬로우·잠금·자동 청소), 청소, 음소거
        relayNodeModeration: (guildId: string) =>
            daemonCall('get', `/api/relay/node/${encodeURIComponent(guildId)}/moderation`, undefined, 20000),
        relayModChannel: (guildId: string, channelId: string, patch: ApiBody) =>
            daemonCall(
                'patch',
                `/api/relay/node/${encodeURIComponent(guildId)}/moderation/channels/${encodeURIComponent(channelId)}`,
                patch,
                20000,
            ),
        relayModClear: (guildId: string, channelId: string, body: ApiBody) =>
            daemonCall(
                'post',
                `/api/relay/node/${encodeURIComponent(guildId)}/moderation/channels/${encodeURIComponent(channelId)}/clear`,
                body,
                70000,
            ),
        relayModClearCheck: (guildId: string, channelId: string, body: ApiBody) =>
            daemonCall(
                'post',
                `/api/relay/node/${encodeURIComponent(guildId)}/moderation/channels/${encodeURIComponent(channelId)}/clear/check`,
                body,
                20000,
            ),
        relayModMute: (guildId: string, userId: string, body: ApiBody) =>
            daemonCall(
                'put',
                `/api/relay/node/${encodeURIComponent(guildId)}/moderation/mutes/${encodeURIComponent(userId)}`,
                body,
                20000,
            ),

        // ── 동의·온보딩·백업·예약·챗봇 ──
        instanceAgreements: (id: string, accept: boolean | string[]) => daemonJson('post', instancePath(id, '/agreements'), { accept }),
        onboarding: () => daemonJson('get', '/api/onboarding'),
        // 문제를 물어볼 때 붙여 넣는 데몬 진단 (비밀은 데몬이 싣지 않는다)
        daemonDiagnostics: () => daemonJson('get', '/api/diagnostics'),
        // 폴더 위치: 데이터·프로그램·인스턴스·모듈·익스텐션 폴더와 기본값
        systemPaths: () => daemonJson('get', '/api/system/paths'),
        // 고른 데몬 PC의 폴더 보기 (원격 경로 고르기, D2): path가 비면 드라이브·루트. extensions 없으면 폴더만, ['*']면 모든 파일
        systemBrowse: (path: string, extensions?: string[] | null) =>
            daemonJson('get', `/api/system/browse${queryString({ path: path || undefined, files: extensions?.length ? extensions.join(',') : undefined })}`),
        // 모듈·익스텐션 폴더 옮기기: 데몬이 복사·검증 뒤 옮긴다 (큰 폴더는 오래 걸린다)
        systemPathsMove: (kind: string, target: string) =>
            daemonJson('post', '/api/system/paths/move', { kind, target }, PATHS_MOVE_TIMEOUT),
        // 원격 데몬 (01 §1.4.2): 고른 데몬과 상관없이 이 PC의 데몬이 답한다. 지우기는 204라 본문이 없다
        remoteSettings: () => daemonJson('get', '/api/remote/settings'),
        remoteSettingsSet: (patch: ApiBody) => daemonJson('put', '/api/remote/settings', patch),
        remoteClients: () => daemonJson('get', '/api/remote/clients'),
        remoteRevoke: async (hostId: string) =>
            (await daemonJson('delete', `/api/remote/clients/${encodeURIComponent(hostId)}`)) || { ok: true },
        remotePeers: () => daemonJson('get', '/api/remote/peers', undefined, 30000),
        remoteDaemons: () => daemonJson('get', '/api/remote/daemons'),
        remoteConnect: (hostId: string) => daemonJson('post', '/api/remote/daemons', { hostId }, 30000),
        remoteForget: async (hostId: string) =>
            (await daemonJson('delete', `/api/remote/daemons/${encodeURIComponent(hostId)}`)) || { ok: true },
        // 합쳐 보기 (G1c): 이 PC와 원격 데몬마다의 서버 {daemons:[{hostId(이 PC는 null), name, ok, error_code?, servers}]}
        // 데몬이 원격 데몬 하나를 30초까지 기다린다 (01 §1.4.2 8)
        remoteServers: () => daemonJson('get', '/api/remote/servers', undefined, 40000),
        // 합쳐 보기에서 다른 데몬의 서버 하나를 켜고 끈다. hostId가 없으면 이 PC
        remoteInstanceAction: async (hostId: string | null, id: string, action: string): Promise<ApiBody> => {
            if (!['start', 'stop', 'restart'].includes(action)) return { success: false, error: `unknown action ${action}` };
            const body = action === 'start' ? {} : { force: false };
            try {
                return (
                    await request('post', `/api/instance/${encodeURIComponent(id)}/${action}`, body, {
                        timeout: action === 'start' ? START_TIMEOUT_MS : STOP_TIMEOUT_MS,
                        target: hostId || 'local',
                    })
                ).data;
            } catch (caught) {
                const error = caught as DaemonError;
                return error.response ? errorBody(error) : connectionError(error);
            }
        },
        // 다룰 데몬 (G1): 셸이 데몬 요청마다 X-Saba-Target을 싣고 설정 gui.daemonTarget에 저장한다
        daemonTarget: () => invoke('daemon_target_get'),
        daemonTargetSet: (target: string | null) => invoke('daemon_target_set', { target: target || null }),
        // 로그인할 때 자동 실행: `{supported, mode: 'off'|'daemon'|'full'}`
        systemAutostart: () => daemonJson('get', '/api/system/autostart'),
        systemAutostartSet: (mode: string) => daemonJson('put', '/api/system/autostart', { mode }),
        // 포트·서버별 네트워크 권한: `{ok, access: 'ok'|'needs_grant'|'needs_relogin'|'unsupported', enabled}`.
        // grant·revoke는 데몬 PC에 관리자 확인 창을 띄우고 닫힐 때까지 기다린다
        systemNetTrace: () => daemonJson('get', '/api/system/net-trace'),
        systemNetTraceGrant: () => daemonJson('post', '/api/system/net-trace/grant', {}, NET_TRACE_TIMEOUT),
        systemNetTraceRevoke: () => daemonJson('post', '/api/system/net-trace/revoke', {}, NET_TRACE_TIMEOUT),
        instanceBackups: (id: string) => daemonJson('get', instancePath(id, '/backups')),
        // body: 없으면 지금 상태, `{ source: 'snapshot', snapshot? }`이면 게임 자동 백업 보관
        instanceBackupCreate: (id: string, body?: ApiBody) => daemonJson('post', instancePath(id, '/backups'), body, BACKUP_TIMEOUT),
        instanceBackupDelete: (id: string, name: string) =>
            daemonJson('delete', instancePath(id, `/backups/${encodeURIComponent(name)}`)),
        instanceBackupRestore: (id: string, name: string) =>
            daemonJson(
                'post',
                instancePath(id, `/backups/${encodeURIComponent(name)}/restore`),
                undefined,
                BACKUP_TIMEOUT,
            ),
        instanceScheduleGet: (id: string) => daemonJson('get', instancePath(id, '/schedule')),
        instanceScheduleSave: (id: string, body: ApiBody) => daemonJson('put', instancePath(id, '/schedule'), body),
        chatbotGet: () => daemonJson('get', '/api/chatbot'),
        chatbotSave: (patch: ApiBody) => daemonJson('put', '/api/chatbot', patch),
        // 길드에 저장된 사용자 키로 릴레이가 제공자에게 묻는다 (chatbot-settings-v2 §1)
        chatbotModels: (guildId: string) =>
            daemonJson('get', `/api/relay/node/${encodeURIComponent(guildId)}/chatbot/models`, undefined, 20000),
        // 고를 수 있는 도구 목록 — 카테고리·쓸 수 있는지 (원본은 릴레이)
        chatbotTools: (guildId: string) => daemonJson('get', `/api/relay/node/${encodeURIComponent(guildId)}/chatbot/tools`),
        // 남은 크레딧(제공사가 알려 줄 때만)과 이번 달 사용량 — 릴레이 답 그대로
        chatbotCredit: (guildId: string) =>
            daemonJson('get', `/api/relay/node/${encodeURIComponent(guildId)}/chatbot/credit`, undefined, 20000),
        // 서버(길드) 하나의 말하는 사바쨩 설정. 키는 오가지 않는다 ({settings, hasApiKey})
        chatbotGuildGet: (guildId: string) => daemonJson('get', `/api/chatbot/guilds/${encodeURIComponent(guildId)}`),
        chatbotGuildSave: (guildId: string, patch: ApiBody) =>
            daemonJson('patch', `/api/chatbot/guilds/${encodeURIComponent(guildId)}`, patch || {}),
        chatbotMemoTemplate: (locale: string) =>
            daemonJson('get', `/api/chatbot/memo-template${locale ? `?locale=${encodeURIComponent(locale)}` : ''}`),
        chatbotClearMemory: (guildId: string) =>
            daemonJson('delete', `/api/chatbot/memory${guildId ? `?guildId=${encodeURIComponent(guildId)}` : ''}`),
        relayInitiatePairing: async (payload: ApiBody) => {
            try {
                return (await post('/api/relay/pair/initiate', payload, { timeout: 8000 })).data;
            } catch (caught) {
                const err = caught as DaemonError;
                return errorBody(err);
            }
        },
        relayPollPairingStatus: async (code: string, secret: string, relayUrl: string) => {
            try {
                // 코드·secret은 본문으로 보낸다 (경로·쿼리는 로그에 남는다)
                const body = { code, secret: secret || undefined, relayUrl: relayUrl || undefined };
                return (await post('/api/relay/pair/status', body, { timeout: 8000 })).data;
            } catch (caught) {
                const err = caught as DaemonError;
                return errorBody(err);
            }
        },
        // 디스코드에서 코드 없이 온 연결 요청. 수락·거절 응답은 데몬 본문 그대로
        relayLinkRequests: () => daemonJson('get', '/api/relay/link-requests'),
        relayLinkRequestRespond: (requestId: string, accept: boolean) =>
            daemonJson('post', `/api/relay/link-requests/${encodeURIComponent(requestId)}`, { accept: !!accept }),
        // 원격 MCP 에이전트 연결 (계정마다 하나): 보기 `{grant|null}`, 끊기는 빈 본문(204)
        relayMcpGrant: () => daemonJson('get', '/api/relay/mcp/grant'),
        // id: GET이 준 연결 id (없으면 모든 연결)
        relayMcpGrantRevoke: (id?: string) =>
            daemonJson('delete', id ? `/api/relay/mcp/grant?id=${encodeURIComponent(id)}` : '/api/relay/mcp/grant'),
        // 개인정보 모두 지우기: `{url}`(웹사이트 동의 페이지)을 받고, 끝났는지는 `{status}`로 묻는다
        relayPrivacyErase: (locale: string) => daemonJson('post', '/api/relay/privacy/erase', { locale }),
        relayPrivacyEraseStatus: () => daemonJson('get', '/api/relay/privacy/erase'),

        // ── 셸·로그·앱 ──
        shellOpenPath: async (folderPath: string) => {
            if (!folderPath || typeof folderPath !== 'string') return { error: t('errors.path_missing') };
            return invoke('shell_open_path', { path: folderPath });
        },
        logsOpenFolder: () => invoke('logs_open_folder'),
        launchUninstaller: () => invoke('launch_uninstaller'),
        getAppVersion: () => invoke('app_get_version'),
        getComponentInfo: async () => {
            try {
                return (await get('/api/system/components')).data;
            } catch (caught) {
                const e = caught as DaemonError;
                console.warn('[App] Failed to get component info from daemon:', e.message);
                let version = null;
                try {
                    version = await invoke('app_get_version');
                } catch (_) {
                    /* 버전을 모르면 null */
                }
                return { version, components: {}, lastUpdated: null };
            }
        },

        // ── 앱 수명 ──
        onCloseRequest: (callback: Listener) => hub.on('app:closeRequest', callback),
        // 다른 인터페이스가 이 GUI를 띄우며 고른 서버 `{instanceId}`
        onOpenServer: (callback: Listener) => hub.on('app:openServer', callback),
        closeResponse: (choice: string) => send('app_close_response', { choice }),
        onStatusUpdate: (callback: Listener) => hub.on('status:update', callback),
        daemonStatus: () => invoke('daemon_status'),
        daemonRestart: () => invoke('daemon_restart'),

        // ── 트레이 팝업 ──
        // open_window | switch_to_cli | start_daemon | stop_daemon | quit_all | close. 팝업은 닫힌다
        trayAction: (action: string) => invoke('tray_action', { action }),
        // 잰 내용 높이(px)에 맞춰 셸이 팝업을 아이콘 옆에 놓고 보인다
        trayPopupFit: (height: number) => invoke('tray_popup_fit', { height }),

        // ── 창 제어 (호출한 창에 적용) ──
        minimizeWindow,
        maximizeWindow,
        closeWindow,
        windowSetBackground,
        windowSetBackdrop,
    };

    const electron = {
        minimizeWindow,
        maximizeWindow,
        closeWindow,
        getLanguage: () => invoke('language_get'),
        setLanguage: (language: string) => invoke('language_set', { language }),
        shell: {
            openExternal: async (url: string) => {
                if (!url || typeof url !== 'string') return { error: t('errors.url_missing') };
                return invoke('shell_open_external', { url });
            },
        },
    };

    return { api, electron };
}

/** window.api */
export type SabaApi = ReturnType<typeof createApi>['api'];
/** window.electron */
export type SabaElectron = ReturnType<typeof createApi>['electron'];
