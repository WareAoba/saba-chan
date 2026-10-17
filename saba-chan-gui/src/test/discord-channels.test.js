import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChannelsTab, channelsPatch } from '../components/discord/ChannelsTab';

const node = { guildId: 'g1', guildName: 'My Guild' };
const base = {
    guildId: 'g1',
    chatbot: ['111'],
    music: null,
    botAvailable: true,
    channels: [
        { id: '111', name: 'chat', category: 'Text' },
        { id: '222', name: 'music', category: 'Text' },
    ],
};

describe('채널 배정 변환', () => {
    it('바뀐 키만 보낸다 (음악 해제는 null, 챗봇 전부 해제는 [])', () => {
        const saved = { chatbot: ['1', '2'], music: '3' };
        expect(channelsPatch(saved, { chatbot: ['2', '1'], music: '3' })).toEqual({});
        expect(channelsPatch(saved, { chatbot: [], music: null })).toEqual({ chatbot: [], music: null });
    });
});

describe('ChannelsTab', () => {
    afterEach(() => {
        delete window.api.relayNodeChannels;
        delete window.api.relayNodeChannelsSave;
    });

    it('음악 채널을 고르면 삭제 경고 후 두 번 눌러 저장하고, 그 채널은 챗봇에서 빠진다', async () => {
        window.api.relayNodeChannels = vi.fn().mockResolvedValue({ ...base, chatbot: ['111', '222'] });
        window.api.relayNodeChannelsSave = vi.fn().mockResolvedValue({ ...base, chatbot: ['111'], music: '222' });
        render(<ChannelsTab node={node} />);
        const select = await screen.findByLabelText('Music channel');
        fireEvent.change(select, { target: { value: '222' } });
        expect(screen.getByText(/All regular messages in #music will be deleted/)).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
        expect(window.api.relayNodeChannelsSave).not.toHaveBeenCalled();
        // 확인 상태에서 채널을 바꾸면 다시 확인받는다
        fireEvent.change(select, { target: { value: '' } });
        fireEvent.change(select, { target: { value: '222' } });
        expect(screen.getByRole('button', { name: /^Save$/ })).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
        expect(window.api.relayNodeChannelsSave).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole('button', { name: /Save and delete messages/ }));
        await waitFor(() =>
            expect(window.api.relayNodeChannelsSave).toHaveBeenCalledWith('g1', { chatbot: ['111'], music: '222' }),
        );
    });

    it('봇이 없으면 알리고, 챗봇 해제와 릴레이 오류를 보여 준다', async () => {
        window.api.relayNodeChannels = vi.fn().mockResolvedValue({
            ...base,
            botAvailable: false,
            channels: [{ id: '222', name: 'music', category: '' }],
        });
        window.api.relayNodeChannelsSave = vi
            .fn()
            .mockResolvedValue({ error: 'UNKNOWN_CHANNEL', error_code: 'UNKNOWN_CHANNEL', channelId: '999' });
        render(<ChannelsTab node={node} />);
        expect(await screen.findByText(/You can only unassign/)).toBeTruthy();
        expect(screen.getByLabelText('Use #music as a chatbot channel').disabled).toBe(true);
        expect(screen.getByRole('option', { name: '#music' }).disabled).toBe(true);
        fireEvent.click(screen.getByLabelText('Use #111 as a chatbot channel'));
        fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
        await waitFor(() => expect(window.api.relayNodeChannelsSave).toHaveBeenCalledWith('g1', { chatbot: [] }));
        expect(await screen.findByText(/999 isn't a text channel/)).toBeTruthy();
    });
});
