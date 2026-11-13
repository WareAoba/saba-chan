import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer as AddInstanceNewServerImpl } from '../components/Modals/AddInstanceNewServer';
import { createButton, submitWizard, toNameStep } from './addServerWizard';
import type { ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const AddInstanceNewServer = AddInstanceNewServerImpl as unknown as ComponentType<Record<string, unknown>>;

const minecraft = {
    name: 'minecraft',
    version: '0.1.0',
    game_name: 'Minecraft',
    install: { method: 'download' },
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
        ],
    },
};

// 데몬 확인: /full은 파일이 든 폴더, 포트 25575는 lobby도 쓴다
const check = vi.fn(async (body) => ({
    success: true,
    name: { value: body.name, taken: false, suggested: body.name },
    folder: {
        custom: !!body.working_dir,
        path: body.working_dir ?? null,
        in_use_by: null,
        not_empty: body.working_dir === '/full',
    },
    ports: [{ field: 'port', port: body.port ?? 25575, in_use_by: body.port ? [] : ['lobby'], reserved: false }],
}));

const open = () => {
    window.api.instanceCheck = check;
    window.api.moduleListVersions = vi.fn().mockResolvedValue({
        versions: [
            { id: '1.21.1', type: 'release' },
            { id: '24w01a', type: 'snapshot' },
        ],
    }) as never;
    const onAddServer = vi.fn().mockResolvedValue(undefined);
    render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={onAddServer} />);
    fireEvent.click(screen.getByText('Minecraft'));
    return onAddServer;
};

afterEach(() => {
    delete (window.api as Partial<typeof window.api>).instanceCheck;
    delete (window.api as Partial<typeof window.api>).moduleListVersions;
    check.mockClear();
});

describe('새 서버 만들기 — 요약 한 장 (사용자 11-11)', () => {
    it('게임을 고르면 이름·버전·위치·실행 방식·포트가 한 장에 있고, 고른 것만 보낸다', async () => {
        const onAddServer = open();
        await toNameStep();
        expect((screen.getByLabelText('Server Name') as HTMLInputElement).value).toBe('minecraft');

        // 서버 버전: 릴리스만 고른다 (기본은 최신)
        fireEvent.click(screen.getByText('Latest version'));
        await waitFor(() => expect(screen.getByText('1.21.1')).toBeTruthy());
        expect(screen.queryByText('24w01a')).toBeNull();
        fireEvent.click(screen.getByText('1.21.1'));

        // 파일 위치: 파일이 든 폴더면 알리고 만들기를 막는다
        fireEvent.click(screen.getByRole('tab', { name: 'Another folder' }));
        await act(async () => fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/full' } }));
        await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/already has files/));
        expect(createButton()).toBeDisabled();
        await act(async () => fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/srv/mc' } }));

        // 실행 방식
        fireEvent.click(screen.getByRole('tab', { name: 'Manage it yourself' }));

        // 포트: 데몬이 고른 포트와 같은 포트를 쓰는 다른 서버를 보인다
        await waitFor(() => expect((screen.getByLabelText('Port') as HTMLInputElement).value).toBe('25575'));
        await waitFor(() => expect(screen.getByText(/lobby also uses port 25575/)).toBeTruthy());

        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).toEqual({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: {},
            version: '1.21.1',
            working_dir: '/srv/mc',
            managed_start: false,
        });
    });

    it('버전 목록은 고른 서버 종류로 다시 묻고, 그 종류에 없는 버전은 고른 것을 푼다', async () => {
        open();
        await waitFor(() => expect(window.api.moduleListVersions).toHaveBeenCalledWith('minecraft', { per_page: 50, settings: {} }));
        fireEvent.click(screen.getByText('Latest version'));
        await waitFor(() => expect(screen.getByText('1.21.1')).toBeTruthy());
        fireEvent.click(screen.getByText('1.21.1'));
        vi.mocked(window.api.moduleListVersions).mockResolvedValue({ versions: [{ id: '1.20.6', type: 'release' }] });
        fireEvent.click(screen.getByText('vanilla'));
        fireEvent.click(screen.getByText('paper'));
        await waitFor(() =>
            expect(window.api.moduleListVersions).toHaveBeenLastCalledWith('minecraft', {
                per_page: 50,
                settings: { server_type: 'paper' },
            }),
        );
        await waitFor(() => expect(screen.getByText('Latest version')).toBeTruthy());
        fireEvent.click(screen.getByText('Latest version'));
        expect(screen.getByText('1.20.6')).toBeTruthy();
        expect(screen.queryByText('1.21.1')).toBeNull();
    });

    it('서버 종류를 바꾸고 새 목록이 오기 전에 만들어도 앞 종류의 버전은 보내지 않는다', async () => {
        const onAddServer = open();
        fireEvent.click(screen.getByText('Latest version'));
        await waitFor(() => expect(screen.getByText('1.21.1')).toBeTruthy());
        fireEvent.click(screen.getByText('1.21.1'));
        vi.mocked(window.api.moduleListVersions).mockReturnValue(new Promise(() => {}));
        fireEvent.click(screen.getByText('vanilla'));
        fireEvent.click(screen.getByText('paper'));
        fireEvent.click(screen.getByText('Latest version', { selector: '.custom-dropdown-label' }));
        expect(screen.queryByText('1.21.1')).toBeNull();
        fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' });
        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).toEqual({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: { server_type: 'paper' },
        });
    });

    it('기본값 그대로 만들면 버전·폴더·실행 방식·포트를 보내지 않는다 (데몬이 정한다)', async () => {
        const onAddServer = open();
        // 겹치는지 데몬에 묻는다
        await waitFor(() =>
            expect(check).toHaveBeenCalledWith({ name: 'minecraft', module_name: 'minecraft', module_settings: {} }),
        );
        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).toEqual({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: {},
        });
    });

    it('포트를 고치면 그 포트로 만든다', async () => {
        const onAddServer = open();
        await waitFor(() => expect((screen.getByLabelText('Port') as HTMLInputElement).value).toBe('25575'));
        await act(async () => fireEvent.change(screen.getByLabelText('Port'), { target: { value: '25600' } }));
        await submitWizard();
        expect(onAddServer.mock.calls[0][0].port).toBe(25600);
    });

    it('쓸 수 없는 포트면 알리고 만들기를 막는다. 비우면 데몬이 고른 포트로 돌아간다', async () => {
        const onAddServer = open();
        await waitFor(() => expect((screen.getByLabelText('Port') as HTMLInputElement).value).toBe('25575'));
        await act(async () => fireEvent.change(screen.getByLabelText('Port'), { target: { value: '70000' } }));
        expect(screen.getByRole('alert').textContent).toMatch(/1 to 65535/);
        expect(createButton()).toBeDisabled();
        await act(async () => fireEvent.change(screen.getByLabelText('Port'), { target: { value: '' } }));
        expect((screen.getByLabelText('Port') as HTMLInputElement).value).toBe('');
        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).not.toHaveProperty('port');
    });

    // 사용자 11-06 #2: 디스코드에서 부르는 말을 쉼표로 나누므로 서버 이름에 쉼표를 받지 않는다 (띄어쓰기는 된다)
    it('쉼표가 든 이름은 알리고 만들기를 막는다', async () => {
        open();
        const name = screen.getByLabelText(/Server Name/i);
        await act(async () => fireEvent.change(name, { target: { value: '우리, 마크' } }));
        expect(screen.getByRole('alert').textContent).toMatch(/comma/);
        expect(createButton()).toBeDisabled();
        await act(async () => fireEvent.change(name, { target: { value: '우리 마크' } }));
        expect(screen.queryByText(/comma/)).toBeNull();
    });

    // 사용자 11-06 #13: 버전은 "숫자 (최신)"으로 보인다
    it('모듈이 알려 준 최신 버전 숫자를 "(latest)"와 함께 보인다', async () => {
        window.api.instanceCheck = check;
        window.api.moduleListVersions = vi.fn().mockResolvedValue({
            versions: [
                { id: '26.3', type: 'release' },
                { id: '26.2', type: 'release' },
            ],
            latest: { release: '26.3' },
        }) as never;
        render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={vi.fn()} />);
        fireEvent.click(screen.getByText('Minecraft'));
        await waitFor(() => expect(screen.getByText('26.3 (latest)', { selector: '.custom-dropdown-label' })).toBeTruthy());
        fireEvent.click(screen.getByText('26.3 (latest)', { selector: '.custom-dropdown-label' }));
        expect(screen.queryByText('26.3')).toBeNull();
        expect(screen.getByText('26.2')).toBeTruthy();
    });

    it('"게임 다시 고르기"로 게임 목록에 돌아간다', async () => {
        open();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Choose another game' })));
        expect(screen.getByRole('button', { name: /Minecraft/ })).toBeTruthy();
        expect(document.getElementById('as-server-name')).toBeNull();
    });
});

/* 상세 설정: 서버 설정 화면과 같은 칸으로 서버 설정을 미리 정한다 (사용자 11-11) */
const withSettings = {
    ...minecraft,
    presets: [{ id: 'friends', label: 'Friends', settings: { max_players: 10 } }],
    settings: {
        fields: [
            ...minecraft.settings.fields,
            { name: 'difficulty', field_type: 'select', label: 'Difficulty', options: ['easy', 'normal', 'hard'], default: 'easy', group: 'basic' },
            { name: 'max_players', field_type: 'number', label: 'Max Players', default: 20, group: 'basic' },
            { name: 'pvp', field_type: 'boolean', label: 'PVP', default: true, group: 'basic' },
            { name: 'view_distance', field_type: 'number', label: 'View Distance', default: 10, group: 'advanced' },
            { name: 'ram', field_type: 'text', label: 'Memory', default: '4G', group: 'saba-chan' },
            { name: 'rcon_password', field_type: 'password', label: 'RCON Password', default: '', group: 'advanced' },
            { name: 'world_dir', field_type: 'file', label: 'World Folder', default: '{instance_dir}/world', group: 'advanced' },
            { name: 'rcon_port', field_type: 'number', label: 'RCON Port', default: 25575, group: 'advanced', port: true },
            { name: 'managed_start', field_type: 'boolean', label: 'Managed Start', default: true, group: 'saba-chan' },
        ],
    },
};

const openDetail = async () => {
    window.api.instanceCheck = check;
    window.api.moduleListVersions = vi.fn().mockResolvedValue({ versions: [] }) as never;
    const onAddServer = vi.fn().mockResolvedValue(undefined);
    render(<AddInstanceNewServer extensions={[withSettings]} servers={[]} onAddServer={onAddServer} />);
    fireEvent.click(screen.getByText('Minecraft'));
    await toNameStep();
    return onAddServer;
};

describe('새 서버 만들기 — 상세 설정', () => {
    it('서버 설정 화면과 같은 묶음(사바쨩·기본·고급 접힘)으로 그리고, 설치·포트·비밀번호·폴더 칸은 뺀다', async () => {
        await openDetail();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        expect(screen.getByLabelText(/Difficulty/)).toBeTruthy();
        expect(screen.getByLabelText(/Memory/)).toBeTruthy();
        // 고급은 접혀 있지만 그려 둔다 (Ctrl+F가 안을 찾는다)
        const fold = document.querySelector('[data-find-fold]') as HTMLElement;
        expect(fold.hidden).toBe(true);
        expect(fold.textContent).toMatch(/View Distance/);
        for (const gone of ['Server Type', 'RCON Password', 'World Folder', 'RCON Port', 'Managed Start']) {
            expect(screen.queryByText(gone)).toBeNull();
        }
    });

    it('바꾼 칸만 module_settings로 보내고, 요약 단추에 바꾼 수를 보인다', async () => {
        const onAddServer = await openDetail();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        await act(async () => fireEvent.change(screen.getByLabelText(/Max Players/), { target: { value: '12' } }));
        fireEvent.click(screen.getByRole('checkbox', { name: /PVP/ }));
        expect(screen.getByText('2 changed')).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Summary' })));
        expect(screen.getByRole('button', { name: /Detailed settings · 2 changed/ })).toBeTruthy();
        await submitWizard();
        expect(onAddServer.mock.calls[0][0].module_settings).toEqual({ max_players: 12, pvp: false });
    });

    it('처음 값은 고른 추천 설정 위의 값이고, 추천 설정과 같은 값은 보내지 않는다', async () => {
        const onAddServer = await openDetail();
        fireEvent.click(screen.getByRole('tab', { name: 'Friends' }));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        expect((screen.getByLabelText(/Max Players/) as HTMLInputElement).value).toBe('10');
        await act(async () => fireEvent.change(screen.getByLabelText(/Max Players/), { target: { value: '10' } }));
        expect(screen.queryByText(/changed/)).toBeNull();
        // 상세 설정 화면에서도 바로 만든다
        await waitFor(() => expect(createButton()).not.toBeDisabled());
        await act(async () => fireEvent.click(createButton()));
        expect(onAddServer.mock.calls[0][0]).toMatchObject({ preset: 'friends', module_settings: {} });
    });

    // 리뷰 #944: 데몬 확인이 설정 값을 거절하면 그 줄에 알리고 만들기를 막는다
    it('데몬이 받지 않는 설정 값은 그 줄에 알리고 만들기를 막는다', async () => {
        await openDetail();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        window.api.instanceCheck = vi.fn(async (body) =>
            body.module_settings?.max_players === 99999
                ? ({ success: false, error_code: 'invalid_format', fields: [{ field: 'max_players' }] } as never)
                : ({ success: true, name: { value: body.name, taken: false }, folder: {}, ports: [] } as never),
        );
        await act(async () => fireEvent.change(screen.getByLabelText(/Max Players/), { target: { value: '99999' } }));
        await waitFor(() => expect(screen.getByText(/Max Players.*unusable value/i)).toBeTruthy());
        expect(createButton()).toBeDisabled();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Summary' })));
        expect(createButton()).toBeDisabled();
        expect(document.querySelector('.as-detail-invalid')).toBeTruthy();
    });

    it('되돌리기는 바꾼 칸을 모두 처음 값으로 돌린다', async () => {
        await openDetail();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        await act(async () => fireEvent.change(screen.getByLabelText(/Max Players/), { target: { value: '12' } }));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Revert' })));
        expect((screen.getByLabelText(/Max Players/) as HTMLInputElement).value).toBe('20');
        expect(screen.queryByText(/changed/)).toBeNull();
    });

    it('상세 설정에서 Ctrl+F로 찾기 창을 연다', async () => {
        await openDetail();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        await act(async () => fireEvent.keyDown(window, { key: 'f', ctrlKey: true }));
        expect(screen.getByRole('search')).toBeTruthy();
        // 칠하는 층은 body에 그려진다: 창(Dialog) 위에 오도록 창 안 찾기 표시를 단다 (Codex 리뷰 #13, 공통 .modal 스타일과 겹치지 않는 이름)
        expect(document.querySelector('.page-find-layer--modal')).toBeTruthy();
        expect(document.querySelector('.page-find-layer.modal')).toBeNull();
    });

    it('미리 정할 칸이 없는 모듈은 상세 설정 단추가 없다', async () => {
        open();
        await toNameStep();
        expect(screen.queryByRole('button', { name: /Detailed settings/ })).toBeNull();
    });
});
