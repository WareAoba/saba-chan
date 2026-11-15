/**
 * DM 알림 카드 (사바쨩 설정 맨 아래) — PC 주인에게 가는 디스코드 DM을 전체·종류별로 켜고 끈다 (데몬 봇 설정 `dmNotice`·`backupNotice`).
 * 사용자 11-15: 연결 화면에서 사바쨩 설정으로 옮겼고, 말하는 사바쨩을 쓸 수 없는 PC에서도 보인다
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatbotView } from '../components/discord/ChatbotView';
import { useDiscordStore } from '../stores/useDiscordStore';

describe('DM 알림 카드', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
        // 말하는 사바쨩 브리지가 없는 PC: API 키·모델·크레딧 카드는 없고 DM 알림 카드만 보인다
        window.api = {
            ...window.api,
            chatbotGet: undefined,
            botConfigLoad: vi.fn().mockResolvedValue({}),
            botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        } as unknown as typeof window.api;
    });

    it('카드 이름은 DM 알림이다', async () => {
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ChatbotView nodes={[]} />);
        expect(screen.getByRole('heading', { name: 'DM alerts' })).toBeTruthy();
    });

    // 사용자 11-15: DM 알림 전체 스위치, 기본 꺼짐. 꺼져 있으면 종류별 칸을 보이지 않는다
    it('DM 알림은 기본 꺼짐이라 종류가 보이지 않고, 켜면 enabled 만 저장한다', async () => {
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ChatbotView nodes={[]} />);
        const all = screen.getByRole('checkbox', { name: 'Receive DM alerts' });
        expect(all).not.toBeChecked();
        expect(screen.queryByRole('checkbox', { name: 'Crashes' })).toBeNull();
        fireEvent.click(all);
        expect(screen.getByRole('checkbox', { name: 'Crashes' })).toBeChecked();
        await waitFor(() => expect(window.api.botConfigSave).toHaveBeenCalledWith({ dmNotice: { enabled: true } }));
    });

    // 사용자 11-08: DM 알림 종류별 켜고 끄기 (PC마다, 데몬 봇 설정 backupNotice·dmNotice)
    it('백업 DM 범위를 고르면 backupNotice 만 저장한다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({ dmNotice: { enabled: true } });
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ChatbotView nodes={[]} />);
        // 기본은 실패만 (사용자 11-15: 예약 백업마다 DM 이 가지 않게)
        expect(screen.getByRole('tab', { name: 'Failures' })).toHaveAttribute('aria-selected', 'true');
        fireEvent.click(screen.getByRole('tab', { name: 'Off' }));
        await waitFor(() => expect(window.api.botConfigSave).toHaveBeenCalledWith({ backupNotice: 'off' }));
    });

    it('DM 종류를 끄면 dmNotice 의 그 칸만 저장하고, 크래시를 끄면 경고를 보인다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({ dmNotice: { enabled: true } });
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ChatbotView nodes={[]} />);
        const crash = screen.getByRole('checkbox', { name: 'Crashes' });
        expect(crash).toBeChecked();
        expect(screen.queryByText(/crashes are not reported on Discord/)).toBeNull();
        fireEvent.click(crash);
        expect(screen.getByText(/crashes are not reported on Discord/)).toBeInTheDocument();
        await waitFor(() =>
            // 바뀐 칸만 (JSON Merge Patch)
            expect(window.api.botConfigSave).toHaveBeenCalledWith({ dmNotice: { crash: false } }),
        );
    });

    it('데몬이 끈 종류는 꺼진 채로 보인다 (빠진 칸은 켜짐)', async () => {
        window.api.botConfigLoad = vi
            .fn()
            .mockResolvedValue({ backupNotice: 'failures', dmNotice: { enabled: true, moderation: false } });
        await act(() => useDiscordStore.getState().loadConfig());
        render(<ChatbotView nodes={[]} />);
        expect(screen.getByRole('checkbox', { name: 'AI moderation reports' })).not.toBeChecked();
        expect(screen.getByRole('checkbox', { name: 'Low disk space' })).toBeChecked();
        expect(screen.getByRole('checkbox', { name: 'Not responding' })).toBeChecked();
        expect(screen.getByRole('tab', { name: 'Failures' })).toHaveAttribute('aria-selected', 'true');
    });

    // 사용자 11-12: PC 연결 끊김 알림은 옵션이고 기본은 꺼짐
    it('PC 연결 끊김 알림은 꺼진 채로 보이고, 켜면 dmNotice 의 그 칸만 저장한다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({ dmNotice: { enabled: true } });
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ChatbotView nodes={[]} />);
        const offline = screen.getByRole('checkbox', { name: 'PC went offline' });
        expect(offline).not.toBeChecked();
        fireEvent.click(offline);
        await waitFor(() =>
            expect(window.api.botConfigSave).toHaveBeenCalledWith({ dmNotice: { offline: true } }),
        );
    });
});
