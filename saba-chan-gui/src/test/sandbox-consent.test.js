import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import i18n from '../i18n';
import { blockedText, showTrustWarnings } from '../utils/trustNotice';

const t = i18n.getFixedT('en', 'gui');
const unavailable = {
    success: false,
    action_required: 'sandbox_unavailable',
    message: 'AppContainer failed',
    detail: 'CreateAppContainerProfile: 0x80070005',
    component: 'module-palworld',
};

function setup() {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: [{ id: 'p1', name: 'Pal', module: 'palworld', status: 'stopped' }],
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
    return { result, setModal };
}

describe('격리를 준비하지 못했을 때 (sandbox_unavailable)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('그 구성요소만 격리 없이 돌릴지 묻고, 동의하면 저장한 뒤 다시 시작한다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValueOnce(unavailable).mockResolvedValue({ success: false });
        window.api.sandboxAllowUnisolated = vi.fn().mockResolvedValue({ success: true, component: 'module-palworld' });
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('Pal', 'palworld'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('question');
        expect(modal.title).toBe('Run without isolation?');
        expect(modal.message).toContain('module-palworld');
        expect(modal.message).toContain('0x80070005');

        await act(async () => modal.buttons[0].action());
        expect(window.api.sandboxAllowUnisolated).toHaveBeenCalledWith('module-palworld');
        expect(window.api.instanceStart).toHaveBeenCalledTimes(2);
    });

    it('확인 창에서 취소하면 오류 없이 다시 시작하지 않는다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(unavailable);
        window.api.sandboxAllowUnisolated = vi.fn().mockResolvedValue({ success: false, error_code: 'canceled' });
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('Pal', 'palworld'));
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        expect(window.showToast.mock.calls.filter((c) => c[1] === 'error')).toEqual([]);
        expect(window.api.instanceStart).toHaveBeenCalledTimes(1);
    });

    it('취소하면 아무것도 저장하지 않는다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(unavailable);
        window.api.sandboxAllowUnisolated = vi.fn();
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('Pal', 'palworld'));
        act(() => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(window.api.sandboxAllowUnisolated).not.toHaveBeenCalled();
        expect(window.api.instanceStart).toHaveBeenCalledTimes(1);
    });

    it('동의 채널이 없으면 묻지 않고 이유만 알린다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(unavailable);
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('Pal', 'palworld'));
        expect(setModal).not.toHaveBeenCalled();
        expect(window.showToast.mock.calls.at(-1)[0]).toContain(
            "Couldn't prepare an isolated environment for module-palworld",
        );
    });
});

describe('격리·검사 문구', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
    });

    it('isolated=true인 경고에만 격리 안내 한 줄을 붙인다', () => {
        showTrustWarnings({ warnings: [{ code: 'module_unverified', module: 'm', isolated: true }] }, t);
        showTrustWarnings({ warnings: [{ code: 'module_unverified', module: 'm', isolated: false }] }, t);
        expect(window.showToast.mock.calls[0][0]).toMatch(/\nThis component runs in an isolated environment/);
        expect(window.showToast.mock.calls[1][0]).not.toMatch(/isolated environment/);
    });

    it('백신 검사를 뺀 뒤로 검사 관련 막힘 코드는 따로 알리지 않는다 (데몬이 보내지 않는다)', () => {
        for (const code of ['malware_detected', 'unscannable_file', 'scan_failed']) {
            expect(blockedText({ action_required: code, file: 'mod/x.pyc' }, t)).toBeNull();
        }
    });
});
