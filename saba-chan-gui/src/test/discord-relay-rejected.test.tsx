import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DiscordPage as DiscordPageImpl } from '../components/discord/DiscordPage';
import { relayFailureKind, useDiscordStore } from '../stores/useDiscordStore';
import type { ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const DiscordPage = DiscordPageImpl as unknown as ComponentType<Record<string, unknown>>;

const props = {
    discordBotStatus: 'connecting',
    discordCloudHostId: 'host',
    setDiscordCloudHostId: () => {},
    relayConnected: false,
    relayConnecting: false,
    setDiscordPrefixes: () => {},
    setNodeSettings: () => {},
    setCloudNodes: () => {},
    setCloudMembers: () => {},
};

beforeEach(() => {
    useDiscordStore.getState()._resetForTest();
    useDiscordStore.setState({ _botConfigLoaded: true, discordCloudHostId: 'host' });
    window.api = {
        discordBotStatus: vi.fn().mockResolvedValue('running'),
        relayCheckHostStatus: vi.fn(),
        relayListHostNodes: vi.fn(),
    } as unknown as typeof window.api;
    window.showToast = vi.fn();
    Object.defineProperty(window, 'innerWidth', { value: 1280, writable: true, configurable: true });
});

describe('릴레이 실패 종류', () => {
    it('토큰 거절은 다시 연결해야 하는 실패다', () => {
        for (const error of ['INVALID_TOKEN', 'TOKEN_REPLACED', 'MISSING_TOKEN', 'NODE_NOT_FOUND', 'FORBIDDEN']) {
            expect(relayFailureKind({ error })).toBe('rejected');
        }
    });

    it('요청 한도 초과는 잠깐 쉬는 것이다 (릴레이의 한도 오류는 코드 없이 ERROR로 온다)', () => {
        expect(relayFailureKind({ error: 'ERROR', message: 'Rate limit exceeded, retry in 1 minute' })).toBe('rate');
        expect(relayFailureKind({ error: 'RATE_LIMITED' })).toBe('rate');
    });

    it('그 밖의 실패는 따로 가르지 않는다', () => {
        expect(relayFailureKind({ error: 'connect ECONNREFUSED' })).toBe('other');
        expect(relayFailureKind(new Error('timeout'))).toBe('other');
    });

    it('두 번 연달아 거절되면 거절로 정해진다', async () => {
        vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'INVALID_TOKEN' });
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().relayErrorKind).toBe('rejected');
    });

    it('거절된 동안은 5초마다 묻지 않고 30초에 한 번만 묻는다', async () => {
        vi.useFakeTimers();
        try {
            vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'INVALID_TOKEN' });
            await useDiscordStore.getState().checkStatus();
            await useDiscordStore.getState().checkStatus();
            vi.mocked(window.api.relayCheckHostStatus).mockClear();
            vi.advanceTimersByTime(5_000);
            await useDiscordStore.getState().checkStatus();
            expect(window.api.relayCheckHostStatus).not.toHaveBeenCalled();
            vi.advanceTimersByTime(30_000);
            await useDiscordStore.getState().checkStatus();
            expect(window.api.relayCheckHostStatus).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    it('릴레이를 덜 묻는 동안에도 에이전트 상태는 5초마다 반영한다', async () => {
        vi.useFakeTimers();
        try {
            vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'INVALID_TOKEN' });
            await useDiscordStore.getState().checkStatus();
            await useDiscordStore.getState().checkStatus();
            vi.mocked(window.api.discordBotStatus).mockResolvedValue('stopped');
            vi.advanceTimersByTime(5_000);
            await useDiscordStore.getState().checkStatus();
            await useDiscordStore.getState().checkStatus();
            expect(useDiscordStore.getState().discordBotStatus).toBe('stopped');
            expect(useDiscordStore.getState().relayErrorKind).toBe('rejected');
        } finally {
            vi.useRealTimers();
        }
    });
});

describe('연결 카드', () => {
    it('거절되면 이유와 함께 이 PC를 다시 연결하는 버튼을 보이고, 누르면 그 자리에서 코드를 받는다', async () => {
        useDiscordStore.setState({
            relayError: 'INVALID_TOKEN',
            relayErrorKind: 'rejected',
            saveConfig: vi.fn().mockResolvedValue(true),
        });
        window.api.relayInitiatePairing = vi
            .fn()
            .mockResolvedValue({ code: 'CODE-123', pollSecret: 's', expiresAt: Date.now() + 300_000 }) as never;
        window.api.relayPollPairingStatus = vi.fn().mockResolvedValue({ status: 'pending' }) as never;
        await act(async () => render(<DiscordPage {...props} />));
        expect(screen.getByText(/isn't accepting this PC's connection key/)).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Reconnect this PC/ })));
        expect(window.api.relayInitiatePairing).toHaveBeenCalled();
        // 코드는 아래 "새 서버 추가"가 아니라 연결 카드 안에 나온다
        const card = screen.getByText(/isn't accepting this PC's connection key/).closest('section');
        expect(card?.textContent).toContain('CODE-123');
    });

    it('요청 한도로 끊긴 것은 다시 연결 버튼 없이 곧 다시 붙는다고 알린다', async () => {
        useDiscordStore.setState({ relayError: 'ERROR', relayErrorKind: 'rate' });
        await act(async () => render(<DiscordPage {...props} />));
        expect(screen.getByText(/slow down for a moment/)).toBeTruthy();
        expect(screen.queryByRole('button', { name: /Reconnect this PC/ })).toBeNull();
    });

    it('연결이 끊겨도 디스코드 서버 목록의 + 버튼은 남는다', async () => {
        useDiscordStore.setState({ relayError: 'INVALID_TOKEN', relayErrorKind: 'rejected' });
        await act(async () => render(<DiscordPage {...props} />));
        expect(screen.getByRole('button', { name: 'Connect this PC' })).toBeTruthy();
    });
});
