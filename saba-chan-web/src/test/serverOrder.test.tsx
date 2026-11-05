import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { moveItem } from '../components/ServerOrder';
import { ServersTab } from '../components/ServersTab';
import { initI18n } from '../i18n';
import { useApp, useData } from '../store';
import type { Loose } from '../types';

const flush = () => act(async () => {});

describe('서버 순서 바꾸기', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => {
        useApp.setState({ serverId: null });
        useData.setState({
            servers: [
                { id: 'a', name: 'Alpha', module: 'm', status: 'stopped' },
                { id: 'b', name: 'Beta', module: 'm', status: 'stopped' },
                { id: 'c', name: 'Gamma', module: 'm', status: 'stopped' },
            ],
            serversLoaded: true,
            modules: [],
            modulesLoaded: true,
        });
    });

    it('moveItem 은 한 칸을 옮기고, 범위 밖이면 그대로 둔다', () => {
        expect(moveItem(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
        expect(moveItem(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
        const same = ['a', 'b'];
        expect(moveItem(same, 0, 5)).toBe(same);
    });

    it('[순서] → ▼ 로 옮기고 저장하면 새 순서를 데몬에 보내고 목록도 그 순서가 된다', async () => {
        const api = new Proxy(
            { instanceReorder: vi.fn().mockResolvedValue({ success: true }) },
            { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
        );
        render(<ServersTab api={api} />);
        fireEvent.click(screen.getByRole('button', { name: 'Order' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move Alpha down' }));
        fireEvent.click(screen.getByRole('button', { name: 'Save order' }));
        await flush();
        expect(api.instanceReorder).toHaveBeenCalledWith(['b', 'a', 'c']);
        expect(useData.getState().servers.map((s: Loose) => s.id)).toEqual(['b', 'a', 'c']);
        // 저장하면 편집 화면이 닫힌다
        expect(screen.queryByRole('button', { name: 'Save order' })).toBeNull();
    });

    it('맨 위 줄의 ▲ 와 맨 아래 줄의 ▼ 는 누를 수 없다', () => {
        render(<ServersTab api={{}} />);
        fireEvent.click(screen.getByRole('button', { name: 'Order' }));
        expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Move Alpha up' }).disabled).toBe(true);
        expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Move Gamma down' }).disabled).toBe(true);
    });
});
