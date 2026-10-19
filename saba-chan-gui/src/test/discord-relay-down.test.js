import { act, fireEvent, render, screen } from '@testing-library/react';
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

describe('실패로 정해진 뒤의 확인', () => {
    it('다시 확인하는 동안 실패를 "연결 중"으로 바꾸지 않는다', async () => {
        window.api.relayCheckHostStatus.mockResolvedValue({ error: 'down' });
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        let during;
        window.api.relayCheckHostStatus.mockImplementation(async () => {
            during = stateOf();
            return { error: 'down' };
        });
        await useDiscordStore.getState().checkStatus();
        expect(during).toBe('error');
    });

    it('겹친 확인 두 개가 한 번의 실패를 두 번으로 세지 않는다', async () => {
        window.api.relayCheckHostStatus.mockResolvedValueOnce({ connected: true });
        await useDiscordStore.getState().checkStatus();
        window.api.relayCheckHostStatus.mockResolvedValue({ error: 'blip' });
        await Promise.all([useDiscordStore.getState().checkStatus(), useDiscordStore.getState().checkStatus()]);
        expect(stateOf()).toBe('connected');
    });

    it('봇을 멈추면 남은 릴레이 실패를 지운다', async () => {
        window.api.relayCheckHostStatus.mockResolvedValue({ error: 'down' });
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        window.api.discordBotStop = vi.fn().mockResolvedValue({});
        window.showToast = vi.fn();
        await useDiscordStore.getState().stopBot();
        expect(useDiscordStore.getState().relayError).toBe('');
        expect(useDiscordStore.getState()._relayFailures).toBe(0);
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

    it('옛 노드 목록 오류가 남아 있어도 릴레이 실패를 보이고, 다시 시도하면 상태 확인을 다시 한다', async () => {
        window.api.relayListHostNodes = vi.fn().mockResolvedValue({ error: 'old nodes error' });
        window.showToast = vi.fn();
        Object.defineProperty(window, 'innerWidth', { value: 1280, writable: true, configurable: true });
        const props = {
            discordBotStatus: 'connecting',
            discordCloudHostId: 'host',
            setDiscordCloudHostId: () => {},
            relayConnecting: false,
            setDiscordPrefixes: () => {},
            setNodeSettings: () => {},
            setCloudNodes: () => {},
            setCloudMembers: () => {},
        };
        const view = await act(async () => render(<DiscordPage {...props} relayConnected={true} />));
        // 연결된 동안 노드 목록이 실패해 그 오류가 남은 채로 릴레이가 끊겼다
        useDiscordStore.setState({ relayError: 'connect ECONNREFUSED' });
        await act(async () => view.rerender(<DiscordPage {...props} relayConnected={false} />));
        expect(screen.getByText(/connect ECONNREFUSED/)).toBeTruthy();
        expect(screen.queryByText(/old nodes error/)).toBeNull();

        window.api.relayCheckHostStatus.mockResolvedValue({ connected: true });
        window.api.relayListHostNodes.mockClear();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /refresh/i })));
        expect(window.api.relayCheckHostStatus).toHaveBeenCalled();
    });
});
