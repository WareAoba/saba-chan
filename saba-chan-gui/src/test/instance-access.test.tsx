import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AccessMode, KickButton, MembersKick } from '../components/discord/InstanceAccess';
import { type Server, useServerStore } from '../stores/useServerStore';

// 사용자 지정(2026-11-08): 인스턴스 사용 모드 — 서버 공용(레벨 권한 + 자동 강퇴 닉네임) / 전용 멤버(명단만, 자동 강퇴 스위치)
const server = (over: Partial<Server> = {}) =>
    ({ id: 'mc-1', name: 'Survival', module: 'minecraft', controllers_only_supported: true, ...over }) as Server;

describe('인스턴스 사용 모드', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).instanceUpdateSettings;
    });

    it('모드를 누르면 바로 저장하고 화면의 서버 값도 바꾼다', async () => {
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true }) as never;
        useServerStore.setState({ servers: [server()] });
        render(<AccessMode server={server()} />);
        expect(screen.getByRole('tab', { name: 'Shared' }).getAttribute('aria-selected')).toBe('true');
        fireEvent.click(screen.getByRole('tab', { name: 'Members only' }));
        await waitFor(() => expect(window.api.instanceUpdateSettings).toHaveBeenCalledWith('mc-1', { access: 'members' }));
        await waitFor(() => expect(useServerStore.getState().servers[0].access).toBe('members'));
    });

    it('서버 공용의 자동 강퇴: 작은 단추(적은 수)를 눌러 팝업에서 닉네임을 넣고 빼며, 대소문자만 다른 이름은 넣지 않는다', async () => {
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true }) as never;
        render(<KickButton server={server({ kick_players: ['Griefer'] })} />);
        const trigger = screen.getByRole('button', { name: /Auto kick\s*1/ });
        expect(screen.queryByRole('dialog')).toBeNull();
        fireEvent.click(trigger);
        expect(screen.getByRole('dialog', { name: 'Auto kick' })).toBeTruthy();
        const input = screen.getByRole('textbox', { name: 'Game nickname to kick automatically' });
        fireEvent.change(input, { target: { value: 'griefer' } });
        fireEvent.click(screen.getByRole('button', { name: 'Add' }));
        expect(window.api.instanceUpdateSettings).not.toHaveBeenCalled();
        fireEvent.change(input, { target: { value: ' Troll ' } });
        fireEvent.click(screen.getByRole('button', { name: 'Add' }));
        await waitFor(() =>
            expect(window.api.instanceUpdateSettings).toHaveBeenCalledWith('mc-1', { kick_players: ['Griefer', 'Troll'] }),
        );
        fireEvent.click(screen.getByRole('button', { name: /Griefer/ }));
        await waitFor(() => expect(window.api.instanceUpdateSettings).toHaveBeenLastCalledWith('mc-1', { kick_players: ['Troll'] }));
    });

    it('전용 멤버의 자동 강퇴는 켜고 끌 수 있고, 명단이 비면 켤 수 없다', async () => {
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true }) as never;
        const { rerender } = render(<MembersKick server={server()} />);
        expect((screen.getByRole('checkbox') as HTMLInputElement).disabled).toBe(true);
        rerender(<MembersKick server={server({ controllers: [{ name: 'Steve' }] })} />);
        fireEvent.click(screen.getByRole('checkbox'));
        await waitFor(() => expect(window.api.instanceUpdateSettings).toHaveBeenCalledWith('mc-1', { controllers_only: true }));
    });

    it('접속을 읽지 못하는 서버는 자동 강퇴를 쓸 수 없다고 알린다', () => {
        render(<MembersKick server={server({ controllers_only_supported: false })} />);
        expect(screen.getByText(/auto kick isn’t available/)).toBeTruthy();
        expect(screen.queryByRole('checkbox')).toBeNull();
    });
});
