import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';

const minecraft = {
    name: 'minecraft',
    version: '0.1.0',
    game_name: 'Minecraft',
    settings: {
        fields: [
            { name: 'server_type', field_type: 'select', label: 'Server Type', options: ['vanilla', 'paper'], default: 'vanilla', install: true },
            { name: 'motd', field_type: 'text', label: 'MOTD' },
        ],
    },
};

describe('새 서버 만들기 — 만들 때 고르는 모듈 설정', () => {
    it('install 설정만 묻고, 고른 값을 module_settings로 보낸다', async () => {
        const onAddServer = vi.fn().mockResolvedValue(undefined);
        render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={onAddServer} />);
        fireEvent.click(screen.getByText('Minecraft'));

        expect(screen.getByText('Server Type')).toBeTruthy();
        expect(screen.queryByText('MOTD')).toBeNull();

        fireEvent.click(screen.getByText('vanilla'));
        fireEvent.click(screen.getByText('paper'));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: /add_server|Add/i }));
        });

        expect(onAddServer).toHaveBeenCalledWith(
            expect.objectContaining({ module_name: 'minecraft', module_settings: { server_type: 'paper' } }),
        );
    });
});

describe('새 서버 만들기 — 만들지 못했을 때', () => {
    it('만들기가 실패하면 입력한 이름과 고른 게임을 그대로 둔다 (다시 입력하지 않게)', async () => {
        const onAddServer = vi.fn().mockResolvedValue(false);
        render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={onAddServer} />);
        fireEvent.click(screen.getByText('Minecraft'));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: /add_server|Add/i }));
        });
        expect(onAddServer).toHaveBeenCalled();
        expect(screen.getByText('Server Type')).toBeTruthy();
    });
});
