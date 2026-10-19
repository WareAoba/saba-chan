import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { cloudStateOf, DiscordPage } from '../components/discord/DiscordPage';
import { useDiscordStore } from '../stores/useDiscordStore';

const stateOf = () => {
    const s = useDiscordStore.getState();
    return cloudStateOf({
        hostId: s.discordCloudHostId,
        connected: s.relayConnected,
        connecting: s.relayConnecting,
        error: s.relayError,
        loaded: true,
    });
};

beforeEach(() => {
    useDiscordStore.getState()._resetForTest();
    useDiscordStore.setState({ _botConfigLoaded: true, discordCloudHostId: 'host' });
    window.api = {
        discordBotStatus: vi.fn().mockResolvedValue('running'),
        relayCheckHostStatus: vi.fn(),
    };
});

describe('릴레이에 닿지 않을 때 디스코드 연결 상태', () => {
    it('계속 닿지 않으면 "연결 중"에 머물지 않고 실패와 이유를 보여 준다', async () => {
        window.api.relayCheckHostStatus.mockResolvedValue({ error: 'connect ECONNREFUSED' });
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        expect(stateOf()).toBe('error');
        expect(useDiscordStore.getState().relayError).toMatch(/ECONNREFUSED/);
    });

    it('연결돼 있다가 한 번 놓친 것은 끊김으로 보지 않는다', async () => {
        window.api.relayCheckHostStatus.mockResolvedValueOnce({ connected: true });
        await useDiscordStore.getState().checkStatus();
        expect(stateOf()).toBe('connected');

        window.api.relayCheckHostStatus.mockRejectedValueOnce(new Error('timeout'));
        await useDiscordStore.getState().checkStatus();
        expect(stateOf()).toBe('connected');

        window.api.relayCheckHostStatus.mockRejectedValueOnce(new Error('timeout'));
        await useDiscordStore.getState().checkStatus();
        expect(stateOf()).toBe('error');
    });

    it('다시 닿으면 오류를 지운다', async () => {
        window.api.relayCheckHostStatus.mockResolvedValue({ error: 'down' });
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        window.api.relayCheckHostStatus.mockResolvedValue({ connected: true });
        await useDiscordStore.getState().checkStatus();
        expect(stateOf()).toBe('connected');
        expect(useDiscordStore.getState().relayError).toBe('');
    });
});

describe('디스코드 화면', () => {
    it('릴레이 확인이 실패로 정해지면 연결 화면에 실패와 이유를 보인다', async () => {
        useDiscordStore.setState({ _botConfigLoaded: true, relayError: 'connect ECONNREFUSED' });
        window.api.relayListHostNodes = vi.fn();
        window.showToast = vi.fn();
        Object.defineProperty(window, 'innerWidth', { value: 1280, writable: true, configurable: true });
        await act(async () =>
            render(
                <DiscordPage
                    discordBotStatus="connecting"
                    discordCloudHostId="host"
                    setDiscordCloudHostId={() => {}}
                    relayConnected={false}
                    relayConnecting={false}
                    setDiscordPrefixes={() => {}}
                    setNodeSettings={() => {}}
                    setCloudNodes={() => {}}
                    setCloudMembers={() => {}}
                />,
            ),
        );
        expect(screen.getByText(/connect ECONNREFUSED/)).toBeTruthy();
        // 연결되지 않았으니 노드 목록은 부르지 않는다
        expect(window.api.relayListHostNodes).not.toHaveBeenCalled();
    });
});
