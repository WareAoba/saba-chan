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

function setup(openSettingsToExtensions, agreements) {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped', agreements }],
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

describe('시작: 켜기 전 점검(preflight_failed)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });
    const eula = {
        success: false,
        error: 'Minecraft EULA not accepted',
        error_code: 'preflight_failed',
        issue_code: 'EULA_NOT_ACCEPTED',
        solution: 'Accept the EULA',
        issues: [
            { code: 'EULA_NOT_ACCEPTED', message: 'Minecraft EULA not accepted' },
            { code: 'X', message: 'Second' },
        ],
    };

    it('문제와 해결 방법을 보여 주고, 동의 기록이 있는데 EULA 파일이 없으면 약관 창으로 다시 동의받는다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValueOnce(eula).mockResolvedValueOnce({ success: true });
        window.api.instanceAgreements = vi.fn().mockResolvedValue({ success: true });
        window.api.serverStatus = vi.fn().mockResolvedValue({ status: 'running' });
        const { result, setModal } = setup(vi.fn(), [{ id: 'eula', url: 'https://aka.ms/MinecraftEULA', accepted: true }]);
        await act(async () => result.current.handleStart('A', 'game'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('agreements');
        expect(modal.message).toContain('Minecraft EULA not accepted');
        expect(modal.message).toContain('Accept the EULA');
        expect(modal.message).toContain('• Second');
        await act(async () => modal.onAccept(['eula']));
        expect(window.api.instanceAgreements).toHaveBeenCalledWith('a', ['eula']);
        expect(window.api.instanceStart).toHaveBeenCalledTimes(2);
    });

    it('동의 연결이 없거나 다른 문제면 실패 창에 해결 방법을 쓴다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue({ ...eula, issue_code: 'SERVER_JAR_MISSING' });
        const { result, setModal } = setup(vi.fn());
        await act(async () => result.current.handleStart('A', 'game'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('failure');
        expect(modal.message).toContain('Accept the EULA');
    });
});

describe('명령 답과 언어별 설명', () => {
    it('data.response를 줄로 나눈다', async () => {
        const { commandReplyLines } = await import('../utils/commandReply');
        expect(commandReplyLines({ data: { response: 'There are 2 players\r\nA, B\n' } })).toEqual([
            'There are 2 players',
            'A, B',
        ]);
        expect(commandReplyLines({ data: { response: { raw: '' } } })).toEqual([]);
        expect(commandReplyLines({ data: null, message: 'sent' })).toEqual(['sent']);
        const { isNoCommandPath } = await import('../utils/commandReply');
        expect(isNoCommandPath({ error: 'no_command_path', message: 'x' })).toBe(true);
        expect(isNoCommandPath({ error: 'other' })).toBe(false);
    });

    it('GUI 언어 → 기본 언어 → 영어 description', async () => {
        const { localizedDescription } = await import('../utils/localizedText');
        const ext = { description: 'English', descriptions: { ko: '한국어', 'zh-CN': '简体' } };
        expect(localizedDescription(ext, 'ko')).toBe('한국어');
        expect(localizedDescription(ext, 'ko-KR')).toBe('한국어');
        expect(localizedDescription(ext, 'zh-TW')).toBe('简体');
        expect(localizedDescription(ext, 'fr')).toBe('English');
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
