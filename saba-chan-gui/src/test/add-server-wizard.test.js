import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';
import { next, submitWizard, toNameStep } from './addServerWizard';

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
    });
    const onAddServer = vi.fn().mockResolvedValue(undefined);
    render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={onAddServer} />);
    fireEvent.click(screen.getByText('Minecraft'));
    return onAddServer;
};

afterEach(() => {
    delete window.api.instanceCheck;
    delete window.api.moduleListVersions;
    check.mockClear();
});

describe('새 서버 만들기 마법사', () => {
    it('버전·폴더·실행 방식을 차례로 고르고, 고른 것만 보낸다', async () => {
        const onAddServer = open();

        // 서버 종류·버전: 릴리스만 고른다 (기본은 최신)
        expect(screen.getByRole('heading', { name: 'Server type and version' })).toBeTruthy();
        fireEvent.click(screen.getByText('Latest version (recommended)'));
        await waitFor(() => expect(screen.getByText('1.21.1')).toBeTruthy());
        expect(screen.queryByText('24w01a')).toBeNull();
        fireEvent.click(screen.getByText('1.21.1'));
        await next();

        // 파일 위치: 파일이 든 폴더면 넘어가지 않고 알린다
        fireEvent.click(screen.getByRole('tab', { name: 'Another folder' }));
        fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/full' } });
        await next();
        expect(screen.getByRole('alert').textContent).toMatch(/already has files/);
        fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/srv/mc' } });
        await next();

        // 실행 방식
        expect(screen.getByRole('heading', { name: 'Run mode' })).toBeTruthy();
        fireEvent.click(screen.getByRole('tab', { name: 'Manage it yourself' }));
        await next();

        // 이름·포트: 데몬이 고른 포트와 같은 포트를 쓰는 다른 서버를 보인다
        await waitFor(() => expect(screen.getByLabelText('Port').value).toBe('25575'));
        expect(screen.getByText(/lobby also uses port 25575/)).toBeTruthy();
        await next();

        // 확인 단계: 고른 내용을 보이고 만든다
        expect(screen.getByText('/srv/mc', { selector: 'dd' })).toBeTruthy();
        expect(screen.getByText('1.21.1', { selector: 'dd' })).toBeTruthy();
        await submitWizard();
        const payload = onAddServer.mock.calls[0][0];
        expect(payload).toEqual({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: {},
            version: '1.21.1',
            working_dir: '/srv/mc',
            managed_start: false,
        });
    });

    it('기본값만 고르면 버전·폴더·실행 방식·포트를 보내지 않는다 (데몬이 정한다)', async () => {
        const onAddServer = open();
        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).toEqual({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: {},
        });
        // 이름·포트 단계와 확인 단계에서 겹치는지 물었다
        expect(check).toHaveBeenCalledWith({ name: 'minecraft', module_name: 'minecraft', module_settings: {} });
    });

    it('포트를 고치면 그 포트로 만든다', async () => {
        const onAddServer = open();
        await toNameStep();
        await waitFor(() => expect(screen.getByLabelText('Port').value).toBe('25575'));
        await act(async () => fireEvent.change(screen.getByLabelText('Port'), { target: { value: '25600' } }));
        await submitWizard();
        expect(onAddServer.mock.calls[0][0].port).toBe(25600);
    });

    it('쓸 수 없는 포트면 알리고 넘어가지 않는다. 비우면 데몬이 고른 포트로 돌아간다', async () => {
        const onAddServer = open();
        await toNameStep();
        await waitFor(() => expect(screen.getByLabelText('Port').value).toBe('25575'));
        await act(async () => fireEvent.change(screen.getByLabelText('Port'), { target: { value: '70000' } }));
        expect(screen.getByRole('alert').textContent).toMatch(/1 to 65535/);
        expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
        // 지우는 중에는 빈 칸 그대로 (데몬 포트로 되돌아가 글자가 섞이지 않게)
        await act(async () => fireEvent.change(screen.getByLabelText('Port'), { target: { value: '' } }));
        expect(screen.getByLabelText('Port').value).toBe('');
        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).not.toHaveProperty('port');
    });

    it('확인을 기다리는 중에 이전 단계로 가도 다음 버튼이 막히지 않는다', async () => {
        open();
        await toNameStep();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Back' })));
        expect(screen.getByRole('heading', { name: 'Run mode' })).toBeTruthy();
        expect(screen.getByRole('button', { name: 'Next' })).not.toBeDisabled();
    });

    it('이전 단계로 돌아가도 고른 값이 남는다', async () => {
        open();
        await next();
        fireEvent.click(screen.getByRole('tab', { name: 'Another folder' }));
        fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/srv/mc' } });
        await next();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Back' })));
        expect(screen.getByLabelText('Folder path').value).toBe('/srv/mc');
    });
});
