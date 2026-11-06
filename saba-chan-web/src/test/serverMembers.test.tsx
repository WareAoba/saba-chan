import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { GuildPermissions } from '../components/PermissionsTab';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

// 멤버 레벨은 디스코드 서버 단위 (사용자 지정 2026-11-04, 앱과 같다): 서버의 멤버 탭에서 정하고, 인스턴스 메뉴에는 레벨별 명령만
const flush = () => act(async () => {});

function setup() {
    const api = new Proxy(
        {
            permissionsGet: vi.fn().mockResolvedValue({ guildId: 'g1', defaultLevel: 1, members: {} }),
            permissionsCommands: vi
                .fn()
                .mockResolvedValue({ instances: [{ id: 'mc-1', name: 'Survival', module: 'minecraft' }] }),
            relayListNodeMembers: vi
                .fn()
                .mockResolvedValue({ members: [{ id: '1', username: 'acorn', displayName: 'Acorn' }] }),
            guildStartNoticeGet: vi.fn().mockResolvedValue({ guildId: 'g1', instances: [], allowedInstances: null }),
        },
        { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
    );
    render(<GuildPermissions api={mockApi(api)} guildId="g1" />);
}

describe('서버 멤버 레벨', () => {
    beforeAll(() => initI18n('en'));

    it('멤버 탭은 서버에 하나, 인스턴스 메뉴에는 멤버 레벨이 없다', async () => {
        setup();
        await flush();
        await flush();
        fireEvent.click(screen.getByRole('tab', { name: 'Member levels' }));
        expect(screen.getByText('Acorn')).toBeTruthy();
        fireEvent.click(screen.getByRole('tab', { name: 'Instances' }));
        fireEvent.click(screen.getByRole('button', { name: /Survival/ }));
        await flush();
        expect(screen.queryByRole('tab', { name: 'Member levels' })).toBeNull();
        expect(screen.queryByText('Acorn')).toBeNull();
    });
});
