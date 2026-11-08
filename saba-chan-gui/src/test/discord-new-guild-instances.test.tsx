/**
 * 페어링 코드로 새로 묶인 디스코드 서버에 이 PC의 서버를 바로 열지(auto) 나중에 고를지(manual) — 데몬 봇 설정 `newGuildInstances`.
 * 코드 없이 /사바쨩 연결로 묶인 서버는 이 설정과 상관없이 데몬이 늘 나중에 고르게 둔다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ConnectionView } from '../components/discord/ConnectionView';
import { useDiscordStore } from '../stores/useDiscordStore';
import type { ComponentProps } from 'react';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'https://relay',
    nodeCount: 1,
    botStatus: 'running',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
} as unknown as ComponentProps<typeof ConnectionView>;

describe('새 디스코드 서버에 서버 열기', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
        window.api = {
            ...window.api,
            botConfigLoad: vi.fn().mockResolvedValue({ newGuildInstances: 'manual' }),
            botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        } as unknown as typeof window.api;
    });

    it('데몬 값을 읽어 보이고, 바꾸면 그 키만 저장한다', async () => {
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ConnectionView {...viewProps} />);
        const toggle = screen.getByRole('checkbox', { name: 'Open my servers in new Discord servers' });
        expect(toggle).not.toBeChecked();
        fireEvent.click(toggle);
        expect(useDiscordStore.getState().discordNewGuildInstances).toBe('auto');
        await waitFor(() => expect(window.api.botConfigSave).toHaveBeenCalledWith({ newGuildInstances: 'auto' }));
    });

    // 사용자 11-08: DM 알림 종류별 켜고 끄기 (PC마다, 데몬 봇 설정 backupNotice·dmNotice)
    it('백업 DM 범위를 고르면 backupNotice 만 저장한다', async () => {
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ConnectionView {...viewProps} />);
        expect(screen.getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
        fireEvent.click(screen.getByRole('tab', { name: 'Off' }));
        await waitFor(() => expect(window.api.botConfigSave).toHaveBeenCalledWith({ backupNotice: 'off' }));
    });

    it('DM 종류를 끄면 dmNotice 의 그 칸만 저장하고, 크래시를 끄면 경고를 보인다', async () => {
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ConnectionView {...viewProps} />);
        const crash = screen.getByRole('checkbox', { name: 'Crashes' });
        expect(crash).toBeChecked();
        expect(screen.queryByText(/you won't hear about crashes/)).toBeNull();
        fireEvent.click(crash);
        expect(screen.getByText(/you won't hear about crashes/)).toBeInTheDocument();
        await waitFor(() =>
            // 바뀐 칸만 (JSON Merge Patch)
            expect(window.api.botConfigSave).toHaveBeenCalledWith({ dmNotice: { crash: false } }),
        );
    });

    it('데몬이 끈 종류는 꺼진 채로 보인다 (빠진 칸은 켜짐)', async () => {
        window.api.botConfigLoad = vi
            .fn()
            .mockResolvedValue({ backupNotice: 'failures', dmNotice: { moderation: false } });
        await act(() => useDiscordStore.getState().loadConfig());
        render(<ConnectionView {...viewProps} />);
        expect(screen.getByRole('checkbox', { name: 'AI moderation reports' })).not.toBeChecked();
        expect(screen.getByRole('checkbox', { name: 'Low disk space' })).toBeChecked();
        expect(screen.getByRole('tab', { name: 'Failures' })).toHaveAttribute('aria-selected', 'true');
    });
});
