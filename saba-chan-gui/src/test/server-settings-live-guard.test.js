/**
 * 서버 설정 라이브 읽기 가드 + diff 저장 테스트
 *
 * 사고 배경: 설정 모달이 실제 설정 파일(live) 읽기에 실패하면 스키마 기본값을
 * 화면에 채우고, 저장 시 전 필드를 전송해 실제 게임 설정 파일을 기본값으로
 * 덮어쓰는 사고가 발생했다 (2026-07-10, 2026-07-26 Palworld INI 소실).
 *
 * 검증 목표:
 * 1. live 읽기 성공 → 화면값은 디스크 실제값, liveSettingsStatus === 'ok'
 * 2. 저장은 diff 기반: live로 확인된 필드 + 사용자가 변경한 필드만 전송
 * 3. live 읽기 실패/빈 결과 → 무음 폴백 금지:
 *    - 변경 필드가 없으면 저장 차단 (API 호출 없음)
 *    - 변경 필드가 있으면 확인 모달을 거쳐 변경 필드만 전송
 * 4. 구버전 프리로드(API 부재)에서도 예외 없이 'failed' 처리
 */
import { act, renderHook, } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

// ── Fixtures ─────────────────────────────────────────────────

const MODULE = {
    name: 'palworld',
    protocols: { supported: ['rest'], default: 'rest' },
    install: { method: 'steamcmd' },
    settings: {
        fields: [
            { name: 'exp_rate', field_type: 'number', default: 1, label: 'EXP Rate' },
            { name: 'server_name', field_type: 'text', default: 'Default Palworld Server', label: 'Server Name' },
            { name: 'is_pvp', field_type: 'boolean', default: false, label: 'PvP' },
        ],
    },
};

const SERVER = {
    id: 'srv-1',
    name: 'pal-test',
    module: 'palworld',
    protocol_mode: 'rest',
    module_settings: {},
};

function setup({ liveImpl } = {}) {
    window.api.serverList = vi.fn().mockResolvedValue({ servers: [SERVER] });
    window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true });
    if (liveImpl === 'missing') {
        // 구버전 프리로드: API 자체가 존재하지 않음
        window.api.instanceReadLiveSettings = undefined;
    } else {
        window.api.instanceReadLiveSettings = liveImpl || vi.fn().mockResolvedValue({ success: true, settings: {} });
    }

    const setModal = vi.fn();
    const setProgressBar = vi.fn();
    const fetchServers = vi.fn();

    const { result } = renderHook(() =>
        useServerSettings({
            servers: [SERVER],
            modules: [MODULE],
            setModal,
            setProgressBar,
            fetchServers,
        }),
    );
    return { result, setModal, setProgressBar, fetchServers };
}

beforeEach(() => {
    vi.clearAllMocks();
});

// ═════════════════════════════════════════════════════════════
// 1. live 읽기 성공 경로
// ═════════════════════════════════════════════════════════════

describe('live 읽기 성공', () => {
    it('화면값이 디스크 실제값으로 채워지고 상태가 ok가 된다', async () => {
        const { result } = setup({
            liveImpl: vi.fn().mockResolvedValue({
                success: true,
                settings: { exp_rate: 2, server_name: 'RealServer', is_pvp: true },
            }),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });

        expect(result.current.settingsValues.exp_rate).toBe('2');
        expect(result.current.settingsValues.server_name).toBe('RealServer');
        expect(result.current.liveSettingsStatus).toBe('ok');
    });

    it('저장 시 live로 확인된 필드만 전송한다 (미확인·미변경 필드 제외)', async () => {
        const { result } = setup({
            liveImpl: vi.fn().mockResolvedValue({
                success: true,
                // is_pvp는 live 결과에 없음 → 미변경 시 전송 금지
                settings: { exp_rate: 2, server_name: 'RealServer' },
            }),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        expect(window.api.instanceUpdateSettings).toHaveBeenCalledTimes(1);
        const payload = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(payload.exp_rate).toBe(2);
        expect(payload.server_name).toBe('RealServer');
        expect(payload).not.toHaveProperty('is_pvp');
    });

    it('사용자가 변경한 필드는 live 결과에 없어도 전송된다', async () => {
        const { result } = setup({
            liveImpl: vi.fn().mockResolvedValue({
                success: true,
                settings: { exp_rate: 2 },
            }),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        act(() => {
            result.current.handleSettingChange('is_pvp', true);
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        const payload = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(payload.is_pvp).toBe(true);
        expect(payload.exp_rate).toBe(2);
        expect(payload).not.toHaveProperty('server_name');
    });
});

// ═════════════════════════════════════════════════════════════
// 2. live 읽기 실패 / 빈 결과 — 무음 폴백 금지
// ═════════════════════════════════════════════════════════════

describe('live 읽기 실패', () => {
    it('읽기 실패 시 상태가 failed가 된다', async () => {
        const { result } = setup({
            liveImpl: vi.fn().mockRejectedValue(new Error('daemon unreachable')),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });

        expect(result.current.liveSettingsStatus).toBe('failed');
    });

    it('빈 결과(파싱 실패 등)는 empty 상태로 구분한다', async () => {
        const { result } = setup({
            liveImpl: vi.fn().mockResolvedValue({ success: true, settings: {} }),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });

        expect(result.current.liveSettingsStatus).toBe('empty');
    });

    it('구버전 프리로드(API 부재)에서도 예외 없이 failed 처리한다', async () => {
        const { result } = setup({ liveImpl: 'missing' });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });

        expect(result.current.liveSettingsStatus).toBe('failed');
    });

    it('변경 필드가 없으면 저장을 차단하고 API를 호출하지 않는다', async () => {
        const { result, setModal } = setup({
            liveImpl: vi.fn().mockRejectedValue(new Error('boom')),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        expect(window.api.instanceUpdateSettings).not.toHaveBeenCalled();
        const lastModal = setModal.mock.calls.at(-1)[0];
        expect(lastModal.type).toBe('failure');
    });

    it('변경 필드가 있으면 확인 모달을 거쳐 변경 필드만 전송한다', async () => {
        const { result, setModal } = setup({
            liveImpl: vi.fn().mockRejectedValue(new Error('boom')),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        act(() => {
            result.current.handleSettingChange('exp_rate', '3');
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        // 즉시 저장 금지 — 확인 모달이 먼저 떠야 한다
        expect(window.api.instanceUpdateSettings).not.toHaveBeenCalled();
        const confirmModal = setModal.mock.calls.at(-1)[0];
        expect(confirmModal.type).toBe('question');

        // 확인(첫 번째 버튼) → 변경한 필드만 전송
        await act(async () => {
            await confirmModal.buttons[0].action();
        });

        expect(window.api.instanceUpdateSettings).toHaveBeenCalledTimes(1);
        const payload = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(payload.exp_rate).toBe(3);
        expect(payload).not.toHaveProperty('server_name');
        expect(payload).not.toHaveProperty('is_pvp');
    });

    it('빈 결과(empty)에서도 미변경 저장은 차단된다', async () => {
        const { result, setModal } = setup({
            liveImpl: vi.fn().mockResolvedValue({ success: true, settings: {} }),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        expect(window.api.instanceUpdateSettings).not.toHaveBeenCalled();
        const lastModal = setModal.mock.calls.at(-1)[0];
        expect(lastModal.type).toBe('failure');
    });
});

// ═════════════════════════════════════════════════════════════
// 3. 저장 성공 후 touched 초기화 (연속 저장 시 중복 전송 방지)
// ═════════════════════════════════════════════════════════════

describe('저장 후 상태', () => {
    it('저장 성공 후 다시 저장하면 이전 변경분이 touched로 남지 않는다', async () => {
        const { result } = setup({
            liveImpl: vi.fn().mockResolvedValue({ success: true, settings: { exp_rate: 2 } }),
        });

        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        act(() => {
            result.current.handleSettingChange('is_pvp', true);
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        // 모달을 다시 열면 (live에 여전히 is_pvp 없음) → 저장 시 is_pvp 미전송
        await act(async () => {
            await result.current.handleOpenSettings(SERVER);
        });
        await act(async () => {
            await result.current.handleSaveSettings();
        });

        const secondPayload = window.api.instanceUpdateSettings.mock.calls.at(-1)[1];
        expect(secondPayload).not.toHaveProperty('is_pvp');
    });
});

// ═════════════════════════════════════════════════════════════
// 4. 모달 경고 배너 (ServerSettingsModal)
// ═════════════════════════════════════════════════════════════

import { render, screen } from '@testing-library/react';
import { ServerSettingsModal } from '../components/ServerSettingsModal';

describe('ServerSettingsModal 경고 배너', () => {
    const baseProps = {
        settingsServer: SERVER,
        settingsValues: {},
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules: [MODULE],
        advancedExpanded: false,
        setAdvancedExpanded: vi.fn(),
        communicationExpanded: false,
        setCommunicationExpanded: vi.fn(),
        availableVersions: [],
        versionsLoading: false,
        versionInstalling: false,
        handleSettingChange: vi.fn(),
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        resettingServer: false,
        isClosing: false,
        onClose: vi.fn(),
        servers: [SERVER],
    };

    it('liveSettingsStatus=failed면 경고 배너를 표시한다', () => {
        render(<ServerSettingsModal {...baseProps} liveSettingsStatus="failed" />);
        expect(screen.getByTestId('live-settings-warning')).toBeInTheDocument();
    });

    it('liveSettingsStatus=empty면 경고 배너를 표시한다', () => {
        render(<ServerSettingsModal {...baseProps} liveSettingsStatus="empty" />);
        expect(screen.getByTestId('live-settings-warning')).toBeInTheDocument();
    });

    it('liveSettingsStatus=ok면 배너가 없다', () => {
        render(<ServerSettingsModal {...baseProps} liveSettingsStatus="ok" />);
        expect(screen.queryByTestId('live-settings-warning')).toBeNull();
    });
});
