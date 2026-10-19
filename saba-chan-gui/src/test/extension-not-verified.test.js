import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

const notVerified = {
    success: false,
    error: 'extension_not_verified',
    error_code: 'extension_not_verified',
    unverified_extensions: ['ue4-ini', 'steamcmd'],
};

function setup() {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }],
            setServers: vi.fn(),
            modules: [],
            loading: false,
            setLoading: vi.fn(),
            setModal,
            setProgressBar: vi.fn(),
            openConsole: vi.fn(),
            closeConsole: vi.fn(),
            setShowModuleManager: vi.fn(),
            formatUptime: vi.fn(),
            openSettingsToExtensions: vi.fn(),
        }),
    );
    return { result, setModal };
}

/** 사용자에게 보인 글 (모달 메시지와 토스트) */
const shown = (setModal) =>
    [...setModal.mock.calls.map(([m]) => m?.message || ''), ...window.showToast.mock.calls.map(([msg]) => msg)].join(
        '\n',
    );

describe('익스텐션 파일이 바뀌어 확인되지 않았을 때', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('시작: 바뀐 익스텐션 이름과 다시 설치·되돌리기 안내를 보여 준다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(notVerified);
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        const text = shown(setModal);
        expect(text).toMatch(/ue4-ini, steamcmd/);
        expect(text).toMatch(/changed/i);
        expect(text).toMatch(/reinstall/i);
        // "설치하고 켜라"는 안내로 빠지지 않는다
        expect(text).not.toMatch(/not enabled|Enable them/);
    });

    it('action_required로 와도 같은 안내를 보여 준다', async () => {
        window.api.instanceStart = vi
            .fn()
            .mockResolvedValue({ ...notVerified, error_code: undefined, action_required: 'extension_not_verified' });
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        expect(shown(setModal)).toMatch(/ue4-ini, steamcmd/);
    });

    it('서버 만들기: extension_required 안내 대신 확인되지 않았다고 알린다', async () => {
        window.api.instanceCreate = vi.fn().mockResolvedValue(notVerified);
        const { result, setModal } = setup();
        await act(async () => result.current.handleAddServer({ name: 'B', module_name: 'game' }));
        const text = shown(setModal);
        expect(text).toMatch(/ue4-ini, steamcmd/);
        expect(text).not.toMatch(/requires the following extension/);
    });
});
