import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';
import { useServerActions } from '../hooks/useServerActions';

// 콘솔도 REST도 없어 저장 후 종료를 요청할 수 없으면 데몬은 서버를 그대로 두고 graceful_stop_unavailable을 준다 (MK10).
// GUI는 저장 없이 끝난다는 것을 알리고, 사용자가 고를 때만 force로 다시 요청한다
const refused = {
    success: false,
    server: 'A',
    error_code: 'graceful_stop_unavailable',
    error: 'no console',
};

const render = () => {
    const list = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 10 }];
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: list,
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
        }),
    );
    window.api.serverList = vi.fn(async () => ({ servers: list }));
    window.api.serverStatus = vi.fn(async () => ({ status: 'stopped' }));
    return { result, setModal };
};

describe('저장 후 종료를 할 수 없는 서버', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
    });

    it('정지: 강제 종료를 묻고, 고르면 force로 다시 끈다', async () => {
        window.api = { instanceStop: vi.fn().mockResolvedValueOnce(refused).mockResolvedValueOnce({ success: true }) };
        const { result, setModal } = render();
        await act(async () => result.current.handleStop('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());

        const ask = setModal.mock.calls.at(-1)[0];
        expect(ask.title).toBe(i18n.t('servers.force_stop_title'));
        expect(ask.message).toBe(i18n.t('servers.force_stop_message', { name: 'A' }));
        expect(window.showToast).not.toHaveBeenCalledWith(expect.anything(), 'error', expect.anything());
        expect(window.api.instanceStop).toHaveBeenCalledTimes(1);
        expect(window.api.instanceStop).toHaveBeenLastCalledWith('a', { force: false });

        await act(async () => ask.buttons[0].action());
        expect(window.api.instanceStop).toHaveBeenCalledTimes(2);
        expect(window.api.instanceStop).toHaveBeenLastCalledWith('a', { force: true });
    });

    it('정지: 취소하면 다시 요청하지 않는다', async () => {
        window.api = { instanceStop: vi.fn().mockResolvedValue(refused) };
        const { result, setModal } = render();
        await act(async () => result.current.handleStop('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(window.api.instanceStop).toHaveBeenCalledTimes(1);
    });

    it('재시작: 강제 종료를 묻고, 고르면 force로 다시 재시작한다', async () => {
        window.api = {
            instanceRestart: vi.fn().mockResolvedValueOnce(refused).mockResolvedValueOnce({ success: true }),
        };
        const { result, setModal } = render();
        await act(async () => result.current.handleRestart('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());

        const ask = setModal.mock.calls.at(-1)[0];
        expect(ask.title).toBe(i18n.t('servers.force_stop_title'));
        expect(window.showToast).not.toHaveBeenCalledWith(expect.anything(), 'error', expect.anything());

        await act(async () => ask.buttons[0].action());
        expect(window.api.instanceRestart).toHaveBeenCalledTimes(2);
        expect(window.api.instanceRestart).toHaveBeenLastCalledWith('a', { force: true });
    });
});
