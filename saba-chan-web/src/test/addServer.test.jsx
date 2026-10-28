import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
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

    it('고쳤다가 비운 설치 칸은 보내지 않는다 (데몬이 모듈 기본값을 쓴다)', async () => {
        const seeded = {
            ...minecraft,
            agreements: [],
            settings: { fields: [{ name: 'seed', field_type: 'text', label: 'Seed', default: 'abc', install: true }] },
        };
        useData.setState({ modules: [seeded], servers: [] });
        const api = { instanceCreate: vi.fn().mockResolvedValue({ success: true }) };
        render(<AddServerSheet api={api} onClose={() => {}} />);
        fireEvent.change(screen.getByRole('textbox', { name: 'Seed' }), { target: { value: '' } });
        fireEvent.click(create());
        await flush();
        expect(api.instanceCreate.mock.calls[0][0].module_settings).toBeUndefined();
    });

    // GUI 마법사와 같은 선택: 버전(서버 파일을 받는 모듈), 실행 방식. 파일 위치는 PC에서만 (CREATE_PC_ONLY)
    const wizard = (check = { name: { taken: false }, folder: { custom: false }, ports: [] }) => {
        const downloads = { ...minecraft, agreements: [], install: { method: 'download' } };
        useData.setState({ modules: [downloads], servers: [] });
        const api = {
            instanceCreate: vi.fn().mockResolvedValue({ success: true }),
            moduleListVersions: vi.fn().mockResolvedValue({
                versions: [{ id: '1.21.4', type: 'release' }, { id: '24w10a', type: 'snapshot' }, { id: '1.20.6' }],
            }),
            instanceCheck: vi.fn().mockResolvedValue({ success: true, ...check }),
        };
        render(<AddServerSheet api={api} onClose={() => {}} />);
        return api;
    };

    it('고른 버전과 직접 관리를 보내고, 고르지 않으면 보내지 않는다', async () => {
        const api = wizard();
        await waitFor(() =>
            expect(api.moduleListVersions).toHaveBeenCalledWith('minecraft', { per_page: 50, settings: {} }),
        );
        await waitFor(() => expect(screen.getByText('Latest version (recommended)')).toBeTruthy());
        fireEvent.click(screen.getByText('Latest version (recommended)'));
        expect(screen.queryByText('24w10a')).toBeNull();
        fireEvent.click(screen.getByText('1.20.6'));
        fireEvent.click(screen.getByText('Managed by saba-chan (recommended)'));
        fireEvent.click(screen.getByText('Manage it yourself'));
        fireEvent.click(create());
        await flush();
        expect(api.instanceCreate).toHaveBeenCalledWith({
            name: 'minecraft',
            module_name: 'minecraft',
            version: '1.20.6',
            managed_start: false,
        });
    });

    it('만들기 전에 겹치는 포트를 데몬에 물어 알린다', async () => {
        vi.useFakeTimers();
        try {
            const api = wizard({
                name: { taken: false },
                folder: { custom: false },
                ports: [{ field: 'port', port: 25565, in_use_by: ['lobby'], reserved: false }],
            });
            await act(async () => {
                await vi.advanceTimersByTimeAsync(300);
            });
            expect(api.instanceCheck).toHaveBeenCalledWith({ name: 'minecraft', module_name: 'minecraft' });
            expect(screen.getByText(/lobby.*25565/)).toBeTruthy();
            expect(create().disabled).toBe(false);
        } finally {
            vi.useRealTimers();
        }
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
        // 받지 못한 모듈은 다음 목록 때 다시 받는다
        const flaky = {
            moduleGetLocales: vi
                .fn()
                .mockResolvedValueOnce({})
                .mockResolvedValue({ en: { module: { display_name: 'XNA' } } }),
        };
        await registerModuleLocales(flaky, [{ name: 'xna', version: '0.1.0' }]);
        await registerModuleLocales(flaky, [{ name: 'xna', version: '0.1.0' }]);
        expect(i18n.t('mod_xna:module.display_name')).toBe('XNA');
    });
});
