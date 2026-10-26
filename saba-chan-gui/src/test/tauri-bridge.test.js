/**
 * Tauri 브리지: window.__TAURI__ 위에 preload와 같은 window.api·window.electron을 만든다.
 * 데몬 요청은 daemon_request 명령으로, 네이티브 작업은 Rust 명령으로 간다.
 */
import { describe, expect, it, vi } from 'vitest';
import { installTauriBridge } from '../api/install';
import { setRemotePathPicker } from '../api/remotePathPicker';

/** 가짜 __TAURI__: invoke 응답은 handlers[명령](인자)로, listen은 emit으로 흉내 낸다 */
function fakeTauri(handlers = {}) {
    const listeners = new Map();
    const invoke = vi.fn(async (cmd, args) => {
        const h = handlers[cmd];
        return typeof h === 'function' ? h(args) : h;
    });
    const listen = vi.fn(async (name, cb) => {
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name).add(cb);
        return () => listeners.get(name).delete(cb);
    });
    const emit = (name, payload) => {
        for (const cb of listeners.get(name) || []) cb({ event: name, payload });
    };
    return { core: { invoke }, event: { listen }, invoke, emit };
}

/** daemon_request 응답을 경로별로 정한다 */
const daemon = (routes) => (args) => {
    const r = routes[`${args.method.toUpperCase()} ${args.path}`];
    if (!r) throw new Error(`unexpected request ${args.method} ${args.path}`);
    return typeof r === 'function' ? r(args) : r;
};

async function install(handlers) {
    const tauri = fakeTauri(handlers);
    const target = { __TAURI__: tauri, location: { search: '' } };
    await installTauriBridge(target);
    return { tauri, target, api: target.api };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('Tauri 브리지 설치', () => {
    it('__TAURI__가 있고 api가 없을 때만 설치한다', async () => {
        const plain = { location: { search: '' } };
        expect(await installTauriBridge(plain)).toBe(false);
        expect(plain.api).toBeUndefined();

        const existing = {};
        const withApi = { __TAURI__: fakeTauri(), api: existing, location: { search: '' } };
        expect(await installTauriBridge(withApi)).toBe(false);
        expect(withApi.api).toBe(existing);

        const { tauri, target } = await install({});
        expect(typeof target.api.serverList).toBe('function');
        expect(typeof target.electron.shell.openExternal).toBe('function');
        // 리스너를 단 뒤에 app_ready
        expect(tauri.invoke).toHaveBeenLastCalledWith('app_ready', undefined);
    });
});

describe('데몬 응답 가공', () => {
    it('daemonCall 계열은 HTTP 오류면 데몬 본문을, 연결 실패면 {ok:false, error, error_code}를 돌려준다', async () => {
        const body = { ok: false, error: 'busy', error_code: 'instance_busy' };
        const { api } = await install({
            daemon_request: daemon({
                'POST /api/instance/a/stop': { status: 409, data: body },
                'POST /api/instance/b/stop': { status: 0, error: 'connect refused', code: 'ECONNREFUSED' },
            }),
        });
        expect(await api.instanceStop('a')).toEqual(body);
        expect(await api.instanceStop('b')).toEqual({
            ok: false,
            error: 'connect refused',
            error_code: 'ECONNREFUSED',
        });
    });

    it('bot-config를 못 읽으면 기본값을 꾸며 내지 않고 실패한다 (저장이 디스크 값을 덮지 않게)', async () => {
        const { api } = await install({
            daemon_request: daemon({ 'GET /api/config/bot': { status: 0, error: 'timeout', code: 'ETIMEDOUT' } }),
        });
        await expect(api.botConfigLoad()).rejects.toThrow();
    });

    it('릴레이 노드·멤버 목록이 실패하면 빈 배열 대신 오류를 돌려준다', async () => {
        const { api } = await install({
            daemon_request: daemon({
                'GET /api/relay/host/h1/nodes': { status: 401, data: { error: 'unauthorized' } },
                'GET /api/relay/node/g1/members?withStatus=1': { status: 0, error: 'timeout', code: 'ETIMEDOUT' },
                'GET /api/relay/node/g1/legacy-members': { status: 502, data: { error: 'relay_error' } },
            }),
        });
        expect(await api.relayListHostNodes('h1')).toMatchObject({ error: 'unauthorized' });
        expect(await api.relayListNodeMembers('g1')).toMatchObject({ error: expect.any(String) });
        expect(await api.relayListLegacyMembers('g1')).toMatchObject({ error: 'relay_error' });
    });

    it('GUI 설정 저장은 부른 순서대로 한다 (앞 저장이 늦게 끝나 뒤 저장을 덮지 않게)', async () => {
        let releaseFirst;
        const puts = [];
        const { api } = await install({
            daemon_request: daemon({
                'PATCH /api/config/gui': (args) => {
                    puts.push(args.body.theme);
                    if (puts.length === 1)
                        return new Promise((r) => (releaseFirst = () => r({ status: 200, data: { success: true } })));
                    return { status: 200, data: { success: true } };
                },
            }),
            refresh_ipc_base: null,
        });
        const first = api.settingsSave({ theme: 'old' });
        const second = api.settingsSave({ theme: 'new' });
        await flush();
        expect(puts).toEqual(['old']);
        releaseFirst();
        await Promise.all([first, second]);
        expect(puts).toEqual(['old', 'new']);
    });

    // 데몬이 잠근 채 합친다. GET으로 읽어 통째로 보내면 그 사이 CLI가 바꾼 값을 되돌린다 (Codex 10-21 #2)
    it('데몬이 응답하면 바꾼 키만 PATCH하고 미리 읽지 않는다', async () => {
        const bodies = [];
        const get = vi.fn(() => ({ status: 200, data: { theme: 'light', language: 'en' } }));
        const { api } = await install({
            daemon_request: daemon({
                'GET /api/config/gui': get,
                'PATCH /api/config/gui': (args) => {
                    bodies.push(args.body);
                    return { status: 200, data: { success: true } };
                },
            }),
        });
        expect(await api.settingsSave({ theme: 'dark' })).toBe(true);
        expect(bodies).toEqual([{ theme: 'dark' }]);
        expect(get).not.toHaveBeenCalled();
    });

    // 셸이 저장 직전 상태에 합친다. 여기서 미리 읽어 합치면 그 사이 셸이 바꾼 값(창 위치 등)을 되돌린다
    it('데몬이 응답하지 않으면 바꾼 키만 셸 로컬 저장에 넘긴다', async () => {
        let saved;
        const { api } = await install({
            daemon_request: daemon({
                'PATCH /api/config/gui': { status: 0, error: 'connect refused', code: 'ECONNREFUSED' },
            }),
            settings_local_load: () => {
                throw new Error('미리 읽지 않는다');
            },
            settings_local_save: (args) => {
                saved = args.settings;
                return true;
            },
        });
        expect(await api.settingsSave({ theme: 'dark' })).toBe(true);
        expect(saved).toEqual({ theme: 'dark' });
    });

    it('IPC 포트를 저장해도 지금 데몬과의 연결 주소는 바꾸지 않는다 (데몬은 다시 켤 때 새 포트를 쓴다)', async () => {
        const { api, tauri } = await install({
            daemon_request: daemon({
                'PATCH /api/config/gui': { status: 200, data: { success: true } },
            }),
            refresh_ipc_base: null,
        });
        expect(await api.settingsSave({ ipcPort: 57475 })).toBe(true);
        expect(tauri.invoke.mock.calls.map((c) => c[0])).not.toContain('refresh_ipc_base');
    });

    it('경로에 넣는 이름은 인코딩한다 (공백·빗금이 든 서버 이름)', async () => {
        const { api } = await install({
            daemon_request: daemon({
                'GET /api/server/My%20Server/status': { status: 200, data: { status: 'running' } },
                'POST /api/server/a%2Fb/stop': { status: 200, data: { success: true } },
            }),
        });
        expect(await api.serverStatus('My Server')).toEqual({ status: 'running' });
        expect(await api.serverStop('a/b')).toEqual({ success: true });
    });

    it('instanceStart는 404를 서버 없음 문구로, 409는 데몬 문구를 살려 넘긴다', async () => {
        const { api } = await install({
            daemon_request: daemon({
                'POST /api/instance/x/start': { status: 404, data: { error_code: 'not_found' } },
                'POST /api/instance/y/start': {
                    status: 409,
                    data: { error_message: '포트 사용 중', error_code: 'port' },
                },
                'POST /api/instance/z/start': { status: 409, data: {} },
            }),
        });
        const missing = await api.instanceStart('x');
        expect(missing.error_code).toBe('not_found');
        expect(missing.error).toContain('x');
        expect(missing.error).not.toBe('Request failed with status code 404');

        expect(await api.instanceStart('y')).toEqual({
            error_message: '포트 사용 중',
            error_code: 'port',
            error: '포트 사용 중',
        });
        expect((await api.instanceStart('z')).error).toContain('z');
    });

    it('executeCommand는 명령을 다듬고 빈 args는 null로 보낸다', async () => {
        const seen = [];
        const { api } = await install({
            daemon_request: (args) => {
                seen.push(args);
                return { status: 200, data: { success: true } };
            },
        });
        await api.executeCommand('i1', { command: '  say hi  ', args: {} });
        await api.executeCommand('i1', { command: 'kick', args: { player: 'p' } });
        expect(seen[0]).toMatchObject({
            method: 'POST',
            path: '/api/instance/i1/command',
            body: { command: 'say hi', args: null },
            timeoutMs: 30000,
        });
        expect(seen[1].body).toEqual({ command: 'kick', args: { player: 'p' } });
    });

    it('instanceDelete는 데몬이 정지 대기·훅·폴더 삭제를 마칠 때까지 기다린다 (기본 5초로 끊지 않는다)', async () => {
        const seen = [];
        const { api } = await install({
            daemon_request: (args) => {
                seen.push(args);
                return { status: 200, data: { success: true } };
            },
        });
        await api.instanceDelete('i1');
        expect(seen[0]).toMatchObject({ method: 'DELETE', path: '/api/instance/i1' });
        expect(seen[0].timeoutMs).toBeGreaterThanOrEqual(60000);
        await api.instanceDelete('i1', { keepSettings: true });
        expect(seen[1]).toMatchObject({ method: 'DELETE', path: '/api/instance/i1?keep_settings=true' });
    });

    it('격리 해제·확장 GUI 허락은 셸 명령(네이티브 확인 창)으로만 기록하고, 철회는 데몬에 바로 보낸다', async () => {
        const seen = [];
        const { api, tauri } = await install({
            sandbox_consent_allow: ({ kind }) => ({ success: true, kind }),
            daemon_request: (args) => {
                seen.push(args);
                return { status: 200, data: { success: true } };
            },
        });
        expect(await api.sandboxAllowUnisolated('module-x')).toEqual({ success: true, kind: 'unisolated' });
        await api.sandboxAllowExtensionGui('ext-y');
        expect(tauri.invoke).toHaveBeenCalledWith('sandbox_consent_allow', {
            kind: 'unisolated',
            component: 'module-x',
        });
        expect(tauri.invoke).toHaveBeenCalledWith('sandbox_consent_allow', {
            kind: 'extension-gui',
            component: 'ext-y',
        });
        expect(seen).toEqual([]);
        await api.sandboxRevokeExtensionGui('ext-y');
        expect(seen[0]).toMatchObject({ method: 'DELETE', path: '/api/sandbox/extension-gui/ext-y' });
    });

    it('instanceDelete가 시간 안에 답을 못 받으면 error_code ETIMEDOUT을 함께 돌려준다', async () => {
        const { api } = await install({
            daemon_request: () => ({ status: 0, error: 'operation timed out', code: 'ETIMEDOUT' }),
        });
        expect((await api.instanceDelete('i1')).error_code).toBe('ETIMEDOUT');
    });

    it('managedStdin이 답을 못 받으면(연결 오류) error_code를 함께 돌려준다', async () => {
        const { api } = await install({
            daemon_request: () => ({ status: 0, error: 'operation timed out', code: 'ETIMEDOUT' }),
        });
        expect(await api.managedStdin('i1', 'list')).toMatchObject({ error_code: 'ETIMEDOUT' });
    });

    it('executeCommand는 HTTP 오류에서 데몬 문구와 error_code·field를 넘긴다', async () => {
        const { api } = await install({
            daemon_request: () => ({
                status: 400,
                data: { error_message: '플레이어 이름이 필요해요', error_code: 'missing_arg', field: 'player' },
            }),
        });
        expect(await api.executeCommand('i1', { command: 'kick' })).toEqual({
            error: '플레이어 이름이 필요해요',
            error_code: 'missing_arg',
            field: 'player',
        });
    });
});

describe('원격 데몬 (G1)', () => {
    it('짝 풀기·잊기는 204여도 성공으로, 다룰 데몬은 셸 명령으로 고른다', async () => {
        const { api, tauri } = await install({
            daemon_request: daemon({
                'DELETE /api/remote/clients/host%20a': { status: 204, data: '' },
                'DELETE /api/remote/daemons/host-b': { status: 404, data: { error: 'remote_unknown', error_code: 'remote_unknown' } },
                'POST /api/remote/daemons': (args) => ({ status: 200, data: { hostId: args.body.hostId, name: '게임 PC' } }),
            }),
            daemon_target_set: (args) => ({ success: true, target: args.target }),
        });
        expect(await api.remoteRevoke('host a')).toEqual({ ok: true });
        expect(await api.remoteForget('host-b')).toMatchObject({ error_code: 'remote_unknown' });
        expect(await api.remoteConnect('host-b')).toEqual({ hostId: 'host-b', name: '게임 PC' });
        expect(await api.daemonTargetSet('')).toEqual({ success: true, target: null });
        expect(tauri.invoke).toHaveBeenCalledWith('daemon_target_set', { target: null });
    });

    it('원격 데몬을 다루는 중에는 이 PC의 대화상자 대신 그 데몬의 폴더에서 고른다 (D2)', async () => {
        let target = 'host-b';
        const asked = [];
        setRemotePathPicker(async (req) => {
            asked.push(req);
            return req.kind === 'folder' ? '/srv/game' : req.kind === 'save' ? '/srv/out.zip' : '/srv/game/server.jar';
        });
        const { api, tauri } = await install({
            daemon_target_get: () => ({ target }),
            dialog_open_folder: () => 'C:/local',
            dialog_open_file: () => 'C:/local/a.jar',
            daemon_request: daemon({ 'POST /api/migration/export': { status: 200, data: { ok: true } } }),
        });
        expect(await api.openFolderDialog()).toBe('/srv/game');
        expect(await api.openFileDialog({ filters: [{ name: 'Jar', extensions: ['jar'] }] })).toBe('/srv/game/server.jar');
        expect(await api.dataExport({})).toEqual({ ok: true });
        expect(tauri.invoke.mock.calls.some(([cmd]) => cmd.startsWith('dialog_'))).toBe(false);
        expect(asked[0]).toEqual({ kind: 'folder', extensions: null, defaultName: null });
        expect(asked[1]).toEqual({ kind: 'file', extensions: ['jar'], defaultName: null });
        expect(asked[2].kind).toBe('save');
        expect(asked[2].extensions).toEqual(['zip']);
        expect(asked[2].defaultName).toMatch(/^saba-chan-export-.*\.zip$/);
        const exported = tauri.invoke.mock.calls.find(([cmd, a]) => cmd === 'daemon_request' && a.path === '/api/migration/export');
        expect(exported[1].body.path).toBe('/srv/out.zip');

        // 고르기 창이 없으면(띄우기 전) 이 PC 경로를 보내지 않고 그만둔다
        setRemotePathPicker(null);
        expect(await api.openFolderDialog()).toBeNull();

        target = null;
        expect(await api.openFolderDialog()).toBe('C:/local');
        expect(await api.openFileDialog({})).toBe('C:/local/a.jar');
    });

    it('원격 데몬 폴더 보기는 고른 데몬에 묻는다', async () => {
        const { api } = await install({
            daemon_request: daemon({
                'GET /api/system/browse?path=%2Fsrv&files=zip': { status: 200, data: { ok: true, entries: [] } },
            }),
        });
        expect(await api.systemBrowse('/srv', ['zip'])).toEqual({ ok: true, entries: [] });
    });

    it('합쳐 보기는 모든 데몬의 서버를 이 PC에 묻고, 서버 하나는 그 데몬에 바로 켜고 끈다', async () => {
        const calls = [];
        const { api } = await install({
            daemon_request: (args) => {
                calls.push(args);
                if (args.path === '/api/remote/servers') return { status: 200, data: { daemons: [] } };
                return { status: 200, data: { success: true } };
            },
        });
        expect(await api.remoteServers()).toEqual({ daemons: [] });
        expect(calls[0].target).toBeUndefined();
        expect(await api.remoteInstanceAction('host-b', 'id 1', 'start')).toEqual({ success: true });
        expect(await api.remoteInstanceAction(null, 'id-2', 'stop')).toEqual({ success: true });
        expect(calls[1]).toMatchObject({ method: 'POST', path: '/api/instance/id%201/start', target: 'host-b' });
        expect(calls[2]).toMatchObject({ method: 'POST', path: '/api/instance/id-2/stop', target: 'local', body: { force: false } });
        expect(await api.remoteInstanceAction('host-b', 'x', 'delete')).toMatchObject({ success: false });
        expect(calls).toHaveLength(3);
    });

    it('고른 원격 데몬에 닿지 못한 오류는 어느 화면에서든 문구로 온다', async () => {
        const { api } = await install({
            daemon_request: daemon({
                'POST /api/instance/a/stop': { status: 502, data: { error: 'remote_unreachable', error_code: 'remote_unreachable' } },
            }),
        });
        expect(await api.instanceStop('a')).toMatchObject({
            error_code: 'remote_unreachable',
            error: 'Could not reach the remote daemon. Check that the PC is on and saba-chan is running',
        });
    });
});

describe('가져오기 (dataImport)', () => {
    it('파일 선택을 취소하면 {canceled:true}, 고르면 결과에 path를 붙인다', async () => {
        let picked = null;
        const requests = [];
        const { api } = await install({
            dialog_open_file: () => picked,
            daemon_request: (args) => {
                requests.push(args);
                return { status: 200, data: { success: true, preview: [] } };
            },
        });
        expect(await api.dataImport({ dryRun: true })).toEqual({ canceled: true });
        expect(requests).toHaveLength(0);

        picked = '/tmp/backup.zip';
        const result = await api.dataImport({ dryRun: true });
        expect(result).toEqual({ success: true, preview: [], path: '/tmp/backup.zip' });
        expect(requests[0]).toMatchObject({
            method: 'POST',
            path: '/api/migration/import',
            body: { path: '/tmp/backup.zip', dryRun: true, overwrite: false },
        });
    });
});

describe('이벤트', () => {
    it('onDaemonEvent가 돌려준 함수로 그 구독만 해제한다', async () => {
        const { api, tauri } = await install({});
        const a = vi.fn();
        const b = vi.fn();
        const offA = api.onDaemonEvent(a);
        api.onDaemonEvent(b);
        tauri.emit('events:message', { id: 1, type: 'resync', data: {} });
        offA();
        tauri.emit('events:message', { id: 2, type: 'resync', data: {} });
        expect(a).toHaveBeenCalledTimes(1);
        expect(a).toHaveBeenCalledWith({ id: 1, type: 'resync', data: {} });
        expect(b).toHaveBeenCalledTimes(2);
    });

    it('늦게 구독한 onStatusUpdate도 마지막 상태를 바로 받는다', async () => {
        const { api, tauri } = await install({});
        tauri.emit('status:update', { step: 'daemon', message: 'd', ts: 1 });
        tauri.emit('status:update', { step: 'ready', message: 'r', ts: 2 });
        const cb = vi.fn();
        api.onStatusUpdate(cb);
        await flush();
        expect(cb).toHaveBeenCalledTimes(1);
        expect(cb).toHaveBeenCalledWith({ step: 'ready', message: 'r', ts: 2 });

        // 돌려받은 함수는 자기 구독만 뗀다: 다른 화면(알림 등)의 구독은 남는다
        const other = vi.fn();
        const off = api.onStatusUpdate(cb);
        api.onStatusUpdate(other);
        await flush();
        off();
        cb.mockClear();
        other.mockClear();
        tauri.emit('status:update', { step: 'ready', message: 'again', ts: 3 });
        expect(cb).not.toHaveBeenCalled();
        expect(other).toHaveBeenCalledTimes(1);
    });

    it('닫기 요청·콘솔 창 구독도 돌려준 함수로 그 구독만 해제한다', async () => {
        const { api, tauri } = await install({});
        for (const [on, channel, payload] of [
            ['onCloseRequest', 'app:closeRequest', undefined],
            ['onConsolePopoutOpened', 'console:popoutOpened', { instanceId: 'a' }],
            ['onConsolePopoutClosed', 'console:popoutClosed', { instanceId: 'a' }],
            ['onConsolePopinRequest', 'console:popinRequest', { instanceId: 'a', serverName: 'A' }],
        ]) {
            const a = vi.fn();
            const b = vi.fn();
            const offA = api[on](a);
            api[on](b);
            offA();
            tauri.emit(channel, payload);
            expect(a, on).not.toHaveBeenCalled();
            expect(b, on).toHaveBeenCalledTimes(1);
        }
        expect(api.offStatusUpdate).toBeUndefined();
        expect(api.offCloseRequest).toBeUndefined();
    });

    it('closeResponse는 고른 값을 app_close_response로 넘긴다', async () => {
        const { api, tauri } = await install({});
        api.closeResponse('exit-interface');
        expect(tauri.invoke).toHaveBeenCalledWith('app_close_response', { choice: 'exit-interface' });
    });
});
