import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }];

function setup() {
    const setModal = vi.fn();
    const hook = renderHook(() =>
        useServerSettings({ servers: [], modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );
    return { ...hook, setModal };
}

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { title: 'x' } }),
    };
});

describe('설정 저장 결과 (PATCH /api/instance/:id)', () => {
    it('저장은 됐지만 게임 파일에 못 쓰면(synced:false) sync_error로 경고 토스트를 띄운다', async () => {
        window.api.instanceUpdateSettings = vi
            .fn()
            .mockResolvedValue({ success: true, synced: false, sync_error: 'server.properties is read-only' });
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        await act(async () => result.current.handleSaveSettings());
        expect(window.showToast).toHaveBeenCalledWith(
            expect.stringContaining('server.properties is read-only'),
            'warning',
            6000,
            undefined,
        );
    });

    it('데몬이 받고 쓰지 않은 키(ignored)는 그 키 이름으로 알린다', async () => {
        window.api.instanceUpdateSettings = vi
            .fn()
            .mockResolvedValue({ success: true, synced: true, ignored: ['rcon_port', 'enable_rcon'] });
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        await act(async () => result.current.handleSaveSettings());
        expect(window.showToast).toHaveBeenCalledWith(
            expect.stringContaining('rcon_port, enable_rcon'),
            'warning',
            6000,
            undefined,
        );
    });

    it('모듈이 모르는 키(unknown_setting)는 그 키 이름으로 알린다', async () => {
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({
            error: 'unknown setting',
            error_code: 'unknown_setting',
            fields: [{ field: 'bogus' }],
        });
        const { result, setModal } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        await act(async () => result.current.handleSaveSettings());
        expect(setModal.mock.calls.at(-1)[0].message).toContain('bogus');
    });

    it('저장 응답을 기다리는 동안 다시 누르면 한 번만 보내고, 저장 중임을 알린다', async () => {
        let finish;
        window.api.instanceUpdateSettings = vi.fn(
            () =>
                new Promise((resolve) => {
                    finish = resolve;
                }),
        );
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        let first;
        await act(async () => {
            first = result.current.handleSaveSettings();
        });
        expect(result.current.savingSettings).toBe(true);
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings).toHaveBeenCalledTimes(1);
        await act(async () => {
            finish({ success: true, synced: true });
            await first;
        });
        expect(result.current.savingSettings).toBe(false);
    });
});
