import { fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it } from 'vitest';
import { LinkedText } from '../components/ScreenLink';
import { initI18n } from '../i18n';
import { useApp } from '../store';

// 사용자 11-06 #19: 안내 문구의 탭 이름 대신 그 화면을 여는 링크 (앱과 같은 화면 주소)
describe('화면 링크', () => {
    beforeAll(() => initI18n('en'));

    it('게임 채팅 안내의 <link/> 자리에 디스코드 탭 링크가 들어가고, 누르면 디스코드 탭을 연다', () => {
        useApp.setState({ tab: 'servers', serverId: 's-1' });
        render(
            <small>
                <LinkedText ns="gui" i18nKey="game_chat.channel_note" to="discord/*/moderation/channels" />
            </small>,
        );
        expect(document.querySelector('small')?.textContent).toBe(
            'Assign the channel as Game chat in Discord↗, or with /saba assign → Game chat.',
        );
        fireEvent.click(screen.getByRole('button', { name: 'Open Discord' }));
        expect(useApp.getState().tab).toBe('discord');
    });

    it('서버 주소는 서버 탭에서 그 서버를 연다', () => {
        useApp.setState({ tab: 'discord', serverId: null });
        render(<LinkedText ns="gui" i18nKey="game_chat.channel_note" to="server/s-9/console" />);
        fireEvent.click(screen.getByRole('button', { name: 'Open Servers' }));
        expect([useApp.getState().tab, useApp.getState().serverId]).toEqual(['servers', 's-9']);
    });
});
