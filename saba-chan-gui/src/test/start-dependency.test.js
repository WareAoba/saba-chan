import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

const blocked = (missing) => ({
    success: false,
    error: 'module_dependency_unsatisfied',
    error_code: 'module_dependency_unsatisfied',
    message: "Cannot start 'A': module 'game' needs ...",
    missing_dependencies: missing,
});

function setup(openSettingsToExtensions) {
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
            openSettingsToExtensions,
        }),
    );
    return { result, setModal };
}

describe('시작: 모듈 의존성이 안 맞을 때', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('익스텐션이 빠졌으면 무엇이 빠졌는지 보여 주고 확장 기능 설정으로 안내한다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(blocked(['ext-steamcmd >=1.0.0 (not installed)']));
        const open = vi.fn();
        const { result, setModal } = setup(open);
        await act(async () => result.current.handleStart('A', 'game'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('question');
        expect(modal.message).toContain('• ext-steamcmd >=1.0.0 (not installed)');
        act(() => modal.buttons[0].action());
        expect(open).toHaveBeenCalled();
    });

    it('사바쨩 버전만 모자라면 실패 창으로 알린다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(blocked(['saba-core >=9.0.0 (installed 0.1.1)']));
        const { result, setModal } = setup(vi.fn());
        await act(async () => result.current.handleStart('A', 'game'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('failure');
        expect(modal.message).toContain('saba-core >=9.0.0');
    });
});

describe('불러오지 못한 모듈의 이유', () => {
    it('의존성 버전이 안 맞으면 필요한 것을 나열하고, 아니면 데몬 문구를 쓴다', async () => {
        const { failedModuleReason } = await import('../utils/failedModules');
        const t = (k, o) => `${k}:${o?.deps}`;
        expect(
            failedModuleReason(
                {
                    path: 'modules/x',
                    error: 'raw',
                    error_code: 'module_dependency_unsatisfied',
                    missing_dependencies: ['saba-core >=9.0.0 (installed 0.1.1)'],
                },
                t,
            ),
        ).toBe('shell.module_needs:saba-core >=9.0.0 (installed 0.1.1)');
        expect(failedModuleReason({ path: 'modules/x', error: 'bad toml' }, t)).toBe('bad toml');
    });
});
