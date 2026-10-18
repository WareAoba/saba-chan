import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BotSettingsTab, botSettingsAvailable } from '../components/discord/BotSettingsTab';

const flush = () => act(async () => {});
const node = { guildId: 'g1', guildName: 'Test' };
const base = {
    guildId: 'g1',
    locale: 'auto',
    sleeping: false,
    music: { enabled: true, ui: true, queueLines: 10, refreshInterval: 5000, normalize: true },
    sync: { ok: true },
};

describe('디스코드 봇 설정 탭', () => {
    afterEach(() => {
        delete window.api.discordSettingsGet;
        delete window.api.discordSettingsSet;
        window.showToast = undefined;
    });

    it('채널이 없으면 탭을 숨긴다', () => {
        window.api.discordSettingsGet = undefined;
        expect(botSettingsAvailable()).toBe(false);
    });

    it('토글은 바뀐 키만 보내고, 음악은 부분 객체로 보낸다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue(base);
        window.api.discordSettingsSet = vi.fn().mockResolvedValue({
            success: true,
            settings: base,
            sync: { ok: true },
        });
        render(<BotSettingsTab node={node} />);
        await flush();
        // 잠자기는 연결 화면의 재우기와 겹쳐서 여기 두지 않는다
        expect(screen.queryByRole('checkbox', { name: 'Sleep' })).toBeNull();

        fireEvent.click(screen.getByRole('checkbox', { name: 'Volume normalization' }));
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { music: { normalize: false } });
    });

    it('범위 밖 숫자는 보내지 않고, 새로고침 간격은 ms로 보낸다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue(base);
        window.api.discordSettingsSet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: base, sync: { ok: true } });
        render(<BotSettingsTab node={node} />);
        await flush();
        const [lines, refresh] = screen.getAllByRole('spinbutton');
        fireEvent.change(lines, { target: { value: '30' } });
        fireEvent.blur(lines);
        expect(screen.getByText('Enter a whole number from 1 to 20.')).toBeInTheDocument();
        expect(window.api.discordSettingsSet).not.toHaveBeenCalled();

        fireEvent.change(refresh, { target: { value: '3' } });
        fireEvent.blur(refresh);
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { music: { refreshInterval: 3000 } });
    });

    it('relay에 못 보냈으면 알리고, 거부되면 토스트로 알린다', async () => {
        window.showToast = vi.fn();
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, sync: { ok: false } });
        window.api.discordSettingsSet = vi.fn().mockResolvedValue({ success: false, error_code: 'not_paired' });
        render(<BotSettingsTab node={node} />);
        await flush();
        expect(screen.getByText(/not applied to Discord yet/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Music features' }));
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("This server isn't paired.", 'error', 6000, undefined);
    });

    it('릴레이에서 설정을 못 읽었으면(loaded:false) 알리고 바꾸지 못하게 한다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, loaded: false });
        window.api.discordSettingsSet = vi.fn();
        render(<BotSettingsTab node={node} />);
        await flush();
        expect(screen.getByText(/Couldn't read this server's settings from the relay/)).toBeTruthy();
        expect(screen.getByRole('checkbox', { name: 'Volume normalization' })).toBeDisabled();

        window.api.discordSettingsGet.mockResolvedValue(base);
        fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
        await flush();
        expect(screen.getByRole('checkbox', { name: 'Volume normalization' })).not.toBeDisabled();
    });
});
