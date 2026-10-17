import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

function deferred() {
    let resolve;
    const promise = new Promise((done) => {
        resolve = done;
    });
    return { promise, resolve };
}

const modules = [
    {
        name: 'game',
        settings: {
            fields: [
                { name: 'port', field_type: 'number', default: 1000 },
                { name: 'title', field_type: 'text', default: 'Default' },
                { name: 'password', field_type: 'password' },
            ],
        },
    },
];

const setup = () =>
    renderHook(() =>
        useServerSettings({ servers: [], modules, setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );

beforeEach(() => {
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
    };
});

describe('서버 설정 창: 실제 설정', () => {
    it('열면 디스크의 실제 설정으로 채운다', async () => {
        window.api.instanceReadLiveSettings = vi
            .fn()
            .mockResolvedValue({ success: true, settings: { title: '진짜 이름' } });
        const { result } = setup();
        const server = { id: 's', name: 'S', module: 'game', port: 2222, module_settings: { title: '캐시' } };
        await act(async () => result.current.handleOpenSettings(server));
        expect(window.api.instanceReadLiveSettings).toHaveBeenCalledWith('s');
        expect(result.current.settingsValues).toMatchObject({ port: '2222', title: '진짜 이름' });
    });

    it('실제 설정을 못 읽으면 port는 데몬이 저장하는 최상위 값을 쓴다 (module_settings는 낡을 수 있다)', async () => {
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: false });
        const { result } = setup();
        const server = { id: 's', name: 'S', module: 'game', port: 2222, module_settings: { port: 1 } };
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues.port).toBe('2222');
    });

    it('문자열 칸에 숫자로 바뀐 실제 값이 오면 저장된 문자열을 지킨다', async () => {
        const settings = { password: 123, title: 7 };
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: true, settings });
        const { result } = setup();
        const server = { id: 's', name: 'S', module: 'game', module_settings: { password: '000123' } };
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues).toMatchObject({ password: '000123', title: '7' });
    });

    it('실제 설정을 못 읽으면 기본값으로 채운 칸은 저장하지 않는다 (고친 칸은 저장)', async () => {
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: false, error: 'no file' });
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game', port: 2222 }));
        expect(result.current.settingsValues.title).toBe('Default');
        act(() => result.current.handleSettingChange('password', 'pw'));
        await act(async () => result.current.handleSaveSettings());
        expect(result.current.settingsSource).toBe('stored');
        const sent = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(sent).toMatchObject({ password: 'pw' });
        expect(sent).not.toHaveProperty('title');
        expect(sent).not.toHaveProperty('port'); // 저장된 값도 확인 못 했으니 고치지 않으면 보내지 않는다
    });

    it('실제 설정을 읽었으면 그 값과 (데몬이 맞춘) 캐시 값은 저장하고, 기본값만 있던 칸은 뺀다', async () => {
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: true, settings: { title: '진짜' } });
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game', port: 2222 }));
        expect(result.current.settingsSource).toBe('live');
        await act(async () => result.current.handleSaveSettings());
        const sent = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(sent).toMatchObject({ port: 2222, title: '진짜' });
        expect(sent).not.toHaveProperty('password');
    });

    it('늦게 온 실제 설정은 사용자가 이미 고친 칸과 나중에 연 다른 서버를 덮지 않는다', async () => {
        const firstLive = deferred();
        window.api.instanceReadLiveSettings = vi
            .fn()
            .mockReturnValueOnce(firstLive.promise)
            .mockResolvedValue({ success: true, settings: { title: '두 번째', port: 3000 } });
        const { result } = setup();
        let firstOpen;
        await act(async () => {
            firstOpen = result.current.handleOpenSettings({ id: 'a', name: 'A', module: 'game' });
            await Promise.resolve();
        });
        await act(async () => result.current.handleOpenSettings({ id: 'b', name: 'B', module: 'game' }));
        act(() => result.current.handleSettingChange('port', '4000'));
        await act(async () => {
            firstLive.resolve({ success: true, settings: { title: '첫 번째' } });
            await firstOpen;
        });
        expect(result.current.settingsServer.id).toBe('b');
        expect(result.current.settingsValues).toMatchObject({ title: '두 번째', port: '4000' });
    });

    it('저장하는 사이 다른 서버 설정을 열었으면 그 창은 열어 둔다', async () => {
        const saved = deferred();
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: true, settings: {} });
        window.api.instanceUpdateSettings = vi.fn().mockReturnValue(saved.promise);
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 'a', name: 'A', module: 'game' }));
        let save;
        act(() => {
            save = result.current.handleSaveSettings();
        });
        await act(async () => result.current.handleOpenSettings({ id: 'b', name: 'B', module: 'game' }));
        await act(async () => {
            saved.resolve({ success: true });
            await save;
        });
        expect(result.current.showSettingsModal).toBe(true);
        expect(result.current.settingsServer.id).toBe('b');
    });
});
