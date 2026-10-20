/**
 * G-U3: 서버 초기화 뒤 설정 폼을 module_settings만으로 다시 채워 램 상한·정지 대기 칸이 비어 보였다.
 */
import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [{ name: 'dl', settings: { fields: [{ name: 'motd', field_type: 'text' }] } }];

describe('초기화 뒤 설정 폼', () => {
    it('데몬이 따로 두는 칸(램 상한·정지 대기·자동 재시작)을 그대로 보인다', async () => {
        const server = {
            id: 's1',
            name: 'S',
            module: 'dl',
            status: 'stopped',
            memory_limit_mb: 2048,
            module_settings: { motd: 'hi', stop_timeout: 90, auto_restart: false },
        };
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: [server] }),
            instanceResetServer: vi.fn().mockResolvedValue({ success: true, deleted: ['world'] }),
        };
        const setModal = vi.fn();
        const { result } = renderHook(() =>
            useServerSettings({ servers: [server], modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
        );
        await act(async () => result.current.handleOpenSettings(server));
        await act(async () => result.current.handleResetServer());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(result.current.settingsValues).toMatchObject({
            motd: 'hi',
            memory_limit_mb: '2048',
            stop_timeout: '90',
            auto_restart: false,
        });
    });
});
