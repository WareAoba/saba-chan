import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { AddServerSheet } from '../components/ServersTab';
import { initI18n } from '../i18n';
import { useApp, useData } from '../store';

// 웹 원격의 서버 만들기는 GUI 새 서버 화면과 같은 것을 묻는다 (사용자 지정 2026-10-28: GUI 기준)
const minecraft = {
    name: 'minecraft',
    game_name: 'Minecraft',
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
            { name: 'motd', field_type: 'text', label: 'MOTD' },
        ],
    },
    presets: [{ id: 'friends', label: 'Friends' }],
    agreements: [{ id: 'eula', label: 'Minecraft EULA', url: 'https://aka.ms/MinecraftEULA' }],
};
const windowsOnly = { name: 'palworld', game_name: 'Palworld', platform_supported: false, platforms: ['windows'] };
const flush = () => act(async () => {});

describe('웹 서버 만들기 — GUI와 같은 칸', () => {
    beforeAll(() => initI18n('en'));
    afterEach(() => {
        cleanup();
        useApp.setState({ toasts: [] });
    });

    const mount = (servers = []) => {
        useData.setState({ modules: [minecraft, windowsOnly], servers });
        const api = { instanceCreate: vi.fn().mockResolvedValue({ success: true, id: 'x' }) };
        render(<AddServerSheet api={api} onClose={() => {}} />);
        return api;
    };
    const create = () => screen.getByRole('button', { name: 'Create' });

    it('기본 이름은 모듈 이름, 겹치면 -2', () => {
        mount([{ name: 'minecraft', module: 'minecraft' }]);
        expect(screen.getByRole('textbox', { name: 'Name' }).value).toBe('minecraft-2');
    });

    it('이미 있는 이름이면 바로 알리고 만들기를 막는다', () => {
        mount([{ name: 'lobby', module: 'minecraft' }]);
        fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), { target: { value: 'Lobby' } });
        expect(screen.getByRole('alert')).toBeTruthy();
        expect(create().disabled).toBe(true);
    });

    it('설치할 수 있는 서버 종류만 보이고, 약관에 동의해야 만들며, 고른 것을 보낸다', async () => {
        const api = mount();
        expect(screen.queryByText('MOTD')).toBeNull();
        fireEvent.click(screen.getByText('vanilla'));
        expect(screen.queryByText('spigot')).toBeNull();
        fireEvent.click(screen.getByText('paper'));
        expect(create().disabled).toBe(true);
        fireEvent.click(screen.getByRole('checkbox', { name: /Minecraft EULA/ }));
        fireEvent.click(create());
        await flush();
        expect(api.instanceCreate).toHaveBeenCalledWith({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: { server_type: 'paper' },
            accept_agreements: ['eula'],
        });
    });

    it('이 OS에서 돌지 않는 게임은 고를 수 없다', () => {
        mount();
        fireEvent.click(screen.getByText('Minecraft'));
        const option = screen.getByRole('option', { name: /Palworld/ });
        expect(option.getAttribute('aria-disabled')).toBe('true');
    });
});

describe('웹 모듈 locales', () => {
    beforeAll(() => initI18n('en'));

    it('모듈 목록을 받으면 모듈 locales를 붙여 게임 이름·선택지를 번역한다 (판마다 한 번)', async () => {
        const { registerModuleLocales } = await import('../i18n');
        const i18n = (await import('i18next')).default;
        const api = { moduleGetLocales: vi.fn().mockResolvedValue({ en: { module: { display_name: 'Terraria!' } } }) };
        await registerModuleLocales(api, [{ name: 'terraria', version: '0.1.0' }]);
        await registerModuleLocales(api, [{ name: 'terraria', version: '0.1.0' }]);
        expect(api.moduleGetLocales).toHaveBeenCalledTimes(1);
        expect(i18n.t('mod_terraria:module.display_name')).toBe('Terraria!');
    });
});
