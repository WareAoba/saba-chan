/**
 * 서버쨩(릴레이)이 이 PC의 상태 보고(heartbeat)를 검증에서 거절하면 데몬 상태에 `heartbeatRejected`
 * {code, field, max?, actual?}가 실린다. 연결 화면은 "연결 실패"와 어떤 값 때문인지를 짧게 보인다.
 */
import { render, screen } from '@testing-library/react';
import i18n from 'i18next';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { ConnectionView } from '../components/discord/ConnectionView';
import { useDiscordStore } from '../stores/useDiscordStore';

const tr = (key: string, vars?: Record<string, unknown>) => i18n.t(`gui:discord.${key}`, vars ?? {});
const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'http://relay.example',
    nodeCount: 1,
    botStatus: 'running',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
} as unknown as ComponentProps<typeof ConnectionView>;
const TOO_MANY = { code: 'TOO_BIG', field: 'metadata.servers', max: 200, actual: 201 };

describe('릴레이가 거절한 상태 보고', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
    });

    it('브리지가 거절 이유를 넘기고, 스토어는 다시 받아 주면 지운다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { state: 'rejected', running: true, connected: false, heartbeatRejected: TOO_MANY } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        expect(await api.discordBotStatus()).toEqual({ status: 'rejected', running: true, reason: TOO_MANY, link: { connected: false, error: '', heard: false } });

        window.api = {
            discordBotStatus: vi.fn().mockResolvedValue({ status: 'rejected', running: true, reason: TOO_MANY }),
            relayCheckHostStatus: vi.fn().mockResolvedValue({ online: true }),
        } as unknown as typeof window.api;
        useDiscordStore.setState({ discordCloudHostId: 'host-1' });
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordHeartbeatReject).toEqual(TOO_MANY);
        expect(useDiscordStore.getState().discordAgentMinVersion).toBeNull();

        vi.mocked(window.api.discordBotStatus).mockResolvedValue('running');
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordHeartbeatReject).toBeNull();
    });

    it('연결 화면이 연결 실패와 어떤 값 때문인지를 짧게 보인다', () => {
        const { rerender } = render(<ConnectionView {...viewProps} />);
        expect(screen.queryByText(tr('heartbeat_rejected_title'))).toBeNull();
        expect(screen.getByText(tr('relay_connected'))).toBeInTheDocument();

        useDiscordStore.setState({ discordHeartbeatReject: TOO_MANY });
        rerender(<ConnectionView {...viewProps} />);
        expect(screen.getByText(tr('relay_failed'))).toBeInTheDocument();
        expect(screen.queryByText(tr('relay_connected'))).toBeNull();
        expect(screen.getByText(tr('heartbeat_rejected_title'))).toBeInTheDocument();
        expect(screen.getByText(tr('heartbeat_rejected_too_big', { what: tr('heartbeat_what_servers'), actual: 201, max: 200 }))).toBeInTheDocument();

        // 이름을 모르는 값은 칸 이름 그대로 알린다
        useDiscordStore.setState({ discordHeartbeatReject: { code: 'INVALID', field: 'metadataUnchanged' } });
        rerender(<ConnectionView {...viewProps} />);
        expect(screen.getByText(tr('heartbeat_rejected_invalid', { what: 'metadataUnchanged' }))).toBeInTheDocument();
    });
});
