import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AgreementDialog } from '../components/AgreementList';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';
import { useServerActions } from '../hooks/useServerActions';

const eula = { id: 'eula', label: 'Minecraft EULA', url: 'https://aka.ms/MinecraftEULA' };

describe('약관 동의: 서버 만들기', () => {
    it('모듈이 약관을 선언하면 모두 체크해야 만들 수 있고, 체크한 id를 accept_agreements로 보낸다', async () => {
        const onAddServer = vi.fn().mockResolvedValue(undefined);
        render(
            <AddInstanceNewServer
                extensions={[{ name: 'game', version: '1.0.0', agreements: [eula] }]}
                servers={[]}
                onAddServer={onAddServer}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: /game/ }));
        const create = screen.getByRole('button', { name: /Add Server/i });
        expect(create).toBeDisabled();

        fireEvent.click(screen.getByRole('checkbox'));
        expect(create).not.toBeDisabled();
        await act(async () => fireEvent.click(create));
        const payload = onAddServer.mock.calls[0][0];
        expect(payload.accept_agreements).toEqual(['eula']);
        expect(payload).not.toHaveProperty('accept_eula');
    });

    it('약관이 없는 모듈은 체크 없이 만들고 accept_agreements를 싣지 않는다', async () => {
        const onAddServer = vi.fn().mockResolvedValue(undefined);
        render(
            <AddInstanceNewServer
                extensions={[{ name: 'game', version: '1.0.0' }]}
                servers={[]}
                onAddServer={onAddServer}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: /game/ }));
        expect(screen.queryByRole('checkbox')).toBeNull();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Add Server/i })));
        expect(onAddServer.mock.calls[0][0]).not.toHaveProperty('accept_agreements');
    });
});

describe('약관 동의: 확인 창', () => {
    it('원문은 외부 브라우저로 열고, 모두 체크해야 동의 버튼이 켜진다', () => {
        window.electron = { shell: { openExternal: vi.fn() } };
        const onAccept = vi.fn();
        render(
            <AgreementDialog
                title="T"
                message="M"
                module="game"
                agreements={[eula]}
                onAccept={onAccept}
                onCancel={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: /Read/i }));
        expect(window.electron.shell.openExternal).toHaveBeenCalledWith(eula.url);

        const accept = screen.getByRole('button', { name: /Agree and start/i });
        expect(accept).toBeDisabled();
        fireEvent.click(screen.getByRole('checkbox'));
        fireEvent.click(accept);
        expect(onAccept).toHaveBeenCalledWith(['eula']);
    });
});

describe('약관 동의: 시작', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    function setup() {
        const setModal = vi.fn();
        const { result } = renderHook(() =>
            useServerActions({
                servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }],
                setServers: vi.fn(),
                modules: [],
                loading: false,
                setLoading: vi.fn(),
                setModal,
                setProgressBar: vi.fn(),
                openConsole: vi.fn(),
                closeConsole: vi.fn(),
                setShowModuleManager: vi.fn(),
                formatUptime: vi.fn(),
            }),
        );
        return { result, setModal };
    }

    const required = {
        success: false,
        error_code: 'agreement_required',
        error: 'Minecraft EULA에 동의해야 서버를 켤 수 있습니다',
        agreements: [{ ...eula, accepted: false }],
    };

    it('agreement_required면 확인 창을 띄우고, 동의하면 /agreements를 부른 뒤 다시 켠다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValueOnce(required).mockResolvedValueOnce({ success: true });
        window.api.instanceAgreements = vi.fn().mockResolvedValue({ success: true });
        window.api.serverStatus = vi.fn().mockResolvedValue({ status: 'running' });
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('agreements');
        expect(modal.agreements).toEqual(required.agreements);
        await act(async () => modal.onAccept(['eula']));
        expect(window.api.instanceAgreements).toHaveBeenCalledWith('a', ['eula']);
        expect(window.api.instanceStart).toHaveBeenCalledTimes(2);
    });

    it('동의 채널이 없으면 데몬 문구를 실패 창으로 보인다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(required);
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('failure');
        expect(modal.message).toContain('Minecraft EULA');
    });
});
