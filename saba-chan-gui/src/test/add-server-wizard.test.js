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

    it('버전 목록은 고른 서버 종류로 다시 묻고, 그 종류에 없는 버전은 고른 것을 푼다', async () => {
        open();
        await waitFor(() => expect(window.api.moduleListVersions).toHaveBeenCalledWith('minecraft', { per_page: 50, settings: {} }));
        fireEvent.click(screen.getByText('Latest version (recommended)'));
        await waitFor(() => expect(screen.getByText('1.21.1')).toBeTruthy());
        fireEvent.click(screen.getByText('1.21.1'));
        window.api.moduleListVersions.mockResolvedValue({ versions: [{ id: '1.20.6', type: 'release' }] });
        fireEvent.click(screen.getByText('vanilla'));
        fireEvent.click(screen.getByText('paper'));
        await waitFor(() =>
            expect(window.api.moduleListVersions).toHaveBeenLastCalledWith('minecraft', {
                per_page: 50,
                settings: { server_type: 'paper' },
            }),
        );
        await waitFor(() => expect(screen.getByText('Latest version (recommended)')).toBeTruthy());
        fireEvent.click(screen.getByText('Latest version (recommended)'));
        expect(screen.getByText('1.20.6')).toBeTruthy();
        expect(screen.queryByText('1.21.1')).toBeNull();
    });

    it('서버 종류를 바꾸고 새 목록이 오기 전에 넘어가도 앞 종류의 버전은 보내지 않는다', async () => {
        const onAddServer = open();
        fireEvent.click(screen.getByText('Latest version (recommended)'));
        await waitFor(() => expect(screen.getByText('1.21.1')).toBeTruthy());
        fireEvent.click(screen.getByText('1.21.1'));
        window.api.moduleListVersions.mockReturnValue(new Promise(() => {}));
        fireEvent.click(screen.getByText('vanilla'));
        fireEvent.click(screen.getByText('paper'));
        // 앞 종류의 목록에서 다시 고를 수도 없다
        fireEvent.click(screen.getByText('Latest version (recommended)', { selector: '.custom-dropdown-label' }));
        expect(screen.queryByText('1.21.1')).toBeNull();
        await submitWizard();
        expect(onAddServer.mock.calls[0][0]).toEqual({
            name: 'minecraft',
            module_name: 'minecraft',
            module_settings: { server_type: 'paper' },
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

    it('폴더 확인을 기다리는 사이 단계나 폴더가 바뀌면 늦게 온 결과로 넘어가지 않는다', async () => {
        open();
        await next();
        fireEvent.click(screen.getByRole('tab', { name: 'Another folder' }));
        fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/srv/mc' } });
        const hold = () => {
            let answer;
            check.mockImplementationOnce((body) => new Promise((resolve) => (answer = () => resolve(check(body)))));
            return () => answer();
        };
        // 폴더를 고치면 예전 폴더의 결과로 넘어가지 않는다
        let answer = hold();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Next' })));
        fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/full' } });
        await act(async () => answer());
        expect(screen.getByLabelText('Folder path').value).toBe('/full');
        // 이전 단계로 갔으면 그 단계에 머문다
        fireEvent.change(screen.getByLabelText('Folder path'), { target: { value: '/srv/mc' } });
        answer = hold();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Next' })));
        await act(async () => fireEvent.click(screen.getAllByRole('button', { name: 'Back' })[0]));
        await act(async () => answer());
        expect(screen.getByRole('heading', { name: 'Server type and version' })).toBeTruthy();
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

describe('진행 막대', () => {
    it('단계를 옮길 때 몇 번째 단계인지 비율로 알린다', async () => {
        window.api.moduleListVersions = vi.fn().mockResolvedValue({ versions: [] });
        const onStepChange = vi.fn();
        render(
            <AddInstanceNewServer
                extensions={[minecraft]}
                servers={[]}
                onAddServer={vi.fn()}
                onStepChange={onStepChange}
            />,
        );
        fireEvent.click(screen.getByText('Minecraft'));
        // 게임 → 서버 종류·버전 → 파일 위치 → 실행 방식 → 이름·포트 → 확인: 첫 단계는 1/5
        expect(onStepChange).toHaveBeenLastCalledWith('server', 1 / 5);
    });
});
