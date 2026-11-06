import type { ComponentProps, ComponentType } from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null }));

import { ServerRow as ServerRowFull } from '../components/servers/ServerRow';
import { useServerStore } from '../stores/useServerStore';

// 이 시험이 보는 props만 넘기므로 나머지는 선택으로 느슨하게 받는다
const ServerRow = ServerRowFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerRowFull>>>;

const row = (server: Record<string, unknown>) =>
    render(
        <ServerRow
            server={server as never}
            index={0}
            moduleData={{ name: 'mc', game_name: 'Minecraft' } as never}
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
        expect(players!.getAttribute('data-tooltip')).toContain('3/20');
        expect(document.querySelector('.server-row-sub .status-chip')).toBeNull();
    });

    it('접속자 수가 없으면 상태 글자를 보인다', () => {
        useServerStore.setState({ modulesLoaded: true });
        row({ id: 'b', name: 'B', module: 'mc', status: 'stopped' });
        expect(document.querySelector('.server-row-sub .status-chip')).not.toBeNull();
        expect(document.querySelector('.server-row-players')).toBeNull();
    });

    // 사용자 11-06 #26: 정지된 서버는 오류처럼 빨갛지 않고 회색이다
    it('정지된 서버는 회색(idle)으로 보인다', () => {
        useServerStore.setState({ modulesLoaded: true });
        row({ id: 'c', name: 'C', module: 'mc', status: 'stopped' });
        expect(document.querySelector('.server-row-dot')!.classList.contains('tone-idle')).toBe(true);
        expect(document.querySelector('.server-row-sub .status-chip')!.classList.contains('tone-idle')).toBe(true);
    });
});
