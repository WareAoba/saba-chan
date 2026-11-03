import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { VisibleServers } from '../components/PermissionsTab';
import { initI18n } from '../i18n';
import { useData } from '../store';

const flush = () => act(async () => {});

describe('이 디스코드 서버에서 보일 서버', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() =>
        useData.setState({
            servers: [
                { id: 'a', name: 'Alpha' },
                { id: 'b', name: 'Beta' },
                { id: 'c', name: 'Gamma' },
            ],
            serversLoaded: true,
        }),
    );

    it('모두 보이기를 끄면 다른 디스코드 서버가 고른 것을 뺀 지금 목록으로 고르기 시작한다', async () => {
        const api = {
            guildAllowedInstancesSave: vi.fn(async (_g, list) => ({ guildId: 'g1', allowedInstances: list })),
        };
        const onSaved = vi.fn();
        render(<VisibleServers api={api} guildId="g1" allowed={null} others={{ g2: ['c'] }} onSaved={onSaved} />);
        // 모두 보이기면 서버마다의 토글은 잠긴다
        expect(screen.getByRole('checkbox', { name: 'Alpha' }).disabled).toBe(true);
        fireEvent.click(screen.getByRole('checkbox', { name: 'All instances' }));
        await flush();
        expect(api.guildAllowedInstancesSave).toHaveBeenCalledWith('g1', ['a', 'b']);
        expect(onSaved).toHaveBeenCalledWith(['a', 'b']);
    });

    it('고르는 중에는 하나씩 끄고 켜며, 다른 디스코드 서버가 쓰는 서버는 켤 수 없다', async () => {
        const api = { guildAllowedInstancesSave: vi.fn(async (_g, list) => ({ allowedInstances: list })) };
        render(<VisibleServers api={api} guildId="g1" allowed={['a', 'b']} others={{ g2: ['c'] }} onSaved={vi.fn()} />);
        expect(screen.getByText('2/3')).toBeTruthy();
        expect(screen.getByRole('checkbox', { name: 'Gamma' }).disabled).toBe(true);
        fireEvent.click(screen.getByRole('checkbox', { name: 'Beta' }));
        await flush();
        expect(api.guildAllowedInstancesSave).toHaveBeenLastCalledWith('g1', ['a']);
    });

    it('모두 보이기를 다시 켜면 제한을 지운다 (null)', async () => {
        const api = { guildAllowedInstancesSave: vi.fn(async () => ({ allowedInstances: null })) };
        render(<VisibleServers api={api} guildId="g1" allowed={['a']} others={{}} onSaved={vi.fn()} />);
        fireEvent.click(screen.getByRole('checkbox', { name: 'All instances' }));
        await flush();
        expect(api.guildAllowedInstancesSave).toHaveBeenCalledWith('g1', null);
    });
});
