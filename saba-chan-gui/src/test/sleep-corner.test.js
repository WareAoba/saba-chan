import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SleepCorner } from '../components/discord/SleepCorner';

const flush = () => act(async () => {});

describe('코코낸내 구석 그림', () => {
    afterEach(() => {
        delete window.api.discordSettingsGet;
        vi.useRealTimers();
    });

    it('자는 중이면 잠자는 사바쨩, 깨어 있으면 아무것도 없다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ asleep: true });
        const { unmount } = render(<SleepCorner guildId="g1" />);
        await flush();
        expect(screen.getByRole('img', { name: 'Saba-chan is asleep' })).toBeInTheDocument();
        unmount();
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ asleep: false });
        render(<SleepCorner guildId="g1" />);
        await flush();
        expect(screen.queryByRole('img')).toBeNull();
    });

    it('보는 동안 깨면 일어나는 사바쨩을 잠깐 보인다', async () => {
        vi.useFakeTimers();
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ asleep: true });
        render(<SleepCorner guildId="g1" />);
        await flush();
        window.api.discordSettingsGet.mockResolvedValue({ asleep: false });
        await act(async () => {
            await vi.advanceTimersByTimeAsync(60_000);
        });
        expect(screen.getByRole('img', { name: 'Saba-chan woke up' })).toBeInTheDocument();
        await act(async () => {
            await vi.advanceTimersByTimeAsync(10_000);
        });
        expect(screen.queryByRole('img')).toBeNull();
    });
});
