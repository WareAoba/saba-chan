import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null }));

import { ServerRow } from '../components/servers/ServerRow';
import { useServerStore } from '../stores/useServerStore';

const row = (server) =>
    render(
        <ServerRow
            server={server}
            index={0}
            moduleData={{ name: 'mc', game_name: 'Minecraft' }}
            onSelect={() => {}}
            onStart={() => {}}
            onStop={() => {}}
            onPointerDown={() => {}}
            skipNextClick={{ current: false }}
        />,
    );

describe('서버 목록 한 줄', () => {
    it('접속자 수가 있으면 상태 글자 대신 접속자 수를 보이고, 상태는 툴팁에 남긴다', () => {
        useServerStore.setState({ modulesLoaded: true });
        row({ id: 'a', name: 'A', module: 'mc', status: 'running', players: { online: 3, max: 20 } });
        const players = screen.getByText('3/20').closest('.server-row-players');
        expect(players.getAttribute('title')).toContain('3/20');
        expect(document.querySelector('.server-row-sub .status-chip')).toBeNull();
    });

    it('접속자 수가 없으면 상태 글자를 보인다', () => {
        useServerStore.setState({ modulesLoaded: true });
        row({ id: 'b', name: 'B', module: 'mc', status: 'stopped' });
        expect(document.querySelector('.server-row-sub .status-chip')).not.toBeNull();
        expect(document.querySelector('.server-row-players')).toBeNull();
    });
});
