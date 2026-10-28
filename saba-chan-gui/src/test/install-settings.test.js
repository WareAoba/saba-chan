import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';

const minecraft = {
    name: 'minecraft',
    version: '0.1.0',
    game_name: 'Minecraft',
    settings: {
        fields: [
            {
                name: 'server_type',
                field_type: 'select',
                label: 'Server Type',
                options: ['vanilla', 'paper'],
                default: 'vanilla',
                install: true,
            },
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

const withInstallOptions = {
    ...minecraft,
    settings: {
        fields: [
            {
                name: 'server_type',
                field_type: 'select',
                label: 'Server Type',
                options: ['vanilla', 'paper', 'spigot'],
                install_options: ['vanilla', 'paper'],
                default: 'vanilla',
                install: true,
            },
            { name: 'hardcore_seed', field_type: 'boolean', label: 'Hardcore Seed', install: true },
            { name: 'world_seed', field_type: 'text', label: 'World Seed', install: true },
        ],
    },
};

const openConfigure = (servers = [], onAddServer = vi.fn().mockResolvedValue(undefined), ext = withInstallOptions) => {
    render(<AddInstanceNewServer extensions={[ext]} servers={servers} onAddServer={onAddServer} />);
    fireEvent.click(screen.getByText('Minecraft'));
    return onAddServer;
};

const submit = async () => {
    await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /add_server|Add/i }));
    });
};

describe('새 서버 만들기 — 설치할 수 있는 값과 모든 칸', () => {
    it('설치할 수 있는 값(install_options)만 고르게 하고, 선택지 이름은 모듈 로케일을 쓴다', () => {
        i18n.addResourceBundle(
            'en',
            'mod_minecraft',
            { settings: { server_type: { options: { paper: 'Paper' } } } },
            true,
            true,
        );
        openConfigure();
        fireEvent.click(screen.getByText('vanilla'));
        expect(screen.getByText('Paper')).toBeTruthy();
        expect(screen.queryByText('spigot')).toBeNull();
        i18n.removeResourceBundle('en', 'mod_minecraft');
    });

    it('켜고 끄는 칸과 글자 칸도 묻고, 답을 module_settings로 보낸다', async () => {
        const onAddServer = openConfigure();
        expect(screen.getByText('Hardcore Seed')).toBeTruthy();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Hardcore Seed' }));
        fireEvent.change(screen.getByLabelText('World Seed'), { target: { value: 'abc' } });
        await submit();
        expect(onAddServer).toHaveBeenCalledWith(
            expect.objectContaining({ module_settings: { hardcore_seed: true, world_seed: 'abc' } }),
        );
    });
});

describe('새 서버 만들기 — 이름', () => {
    const nameInput = () => screen.getByPlaceholderText(/./, { selector: 'input.as-input' });

    it('기본 이름은 모듈 이름 그대로, 겹치면 2부터 붙인다 (CLI와 같다)', () => {
        openConfigure();
        expect(nameInput().value).toBe('minecraft');
        cleanup();
        openConfigure([{ name: 'minecraft', module: 'minecraft' }]);
        expect(nameInput().value).toBe('minecraft-2');
        cleanup();
        openConfigure([
            { name: 'Minecraft', module: 'minecraft' },
            { name: 'minecraft-2', module: 'other' },
        ]);
        expect(nameInput().value).toBe('minecraft-3');
    });

    it('이미 있는 이름이면 바로 알려 주고 만들기를 막는다', () => {
        openConfigure([{ name: 'lobby', module: 'minecraft' }]);
        fireEvent.change(nameInput(), { target: { value: 'Lobby ' } });
        expect(screen.getByRole('alert')).toBeTruthy();
        expect(screen.getByRole('button', { name: /add_server|Add/i })).toBeDisabled();
    });
});

describe('새 서버 만들기 — 비운 칸', () => {
    it('고쳤다가 비운 칸은 보내지 않는다 (데몬이 모듈 기본값을 쓴다)', async () => {
        const onAddServer = openConfigure();
        fireEvent.change(screen.getByLabelText('World Seed'), { target: { value: 'abc' } });
        fireEvent.change(screen.getByLabelText('World Seed'), { target: { value: '' } });
        await submit();
        expect(onAddServer.mock.calls[0][0].module_settings).toEqual({});
    });
});
