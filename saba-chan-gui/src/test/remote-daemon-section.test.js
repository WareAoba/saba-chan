import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { RemoteDaemonSection } from '../components/Modals/RemoteDaemonSection';
import { useSettingsStore } from '../stores/useSettingsStore';
import { pickOption } from './dropdown';

const settings = { enabled: false, port: 57480, address: '', transport: 'direct', status: { listening: false, published: false } };

describe('설정 > 고급: 원격 데몬 (/api/remote/*)', () => {
    beforeEach(() => {
        window.api = {
            remoteSettings: vi.fn().mockResolvedValue(settings),
            remoteSettingsSet: vi.fn(),
            remoteClients: vi.fn().mockResolvedValue({ clients: [{ hostId: 'host-a', name: '노트북', pairedAt: '' }] }),
            remoteRevoke: vi.fn().mockResolvedValue({ ok: true }),
            remotePeers: vi.fn().mockResolvedValue({
                peers: [
                    { hostId: 'host-b', name: '게임 PC', status: 'online', hasEndpoint: true, saved: false },
                    { hostId: 'host-c', name: '사무실', status: 'offline', hasEndpoint: false, saved: false },
                ],
            }),
            remoteDaemons: vi.fn().mockResolvedValue({ daemons: [] }),
            remoteConnect: vi.fn(),
            remoteForget: vi.fn().mockResolvedValue({ ok: true }),
        };
    });

    it('원격 데몬을 연결했으면 서버 목록 보기(고른 데몬만 / 합쳐 보기)를 고른다', async () => {
        useSettingsStore.setState({ daemonView: 'switch' });
        await act(async () => render(<RemoteDaemonSection />));
        expect(screen.queryByRole('combobox', { name: 'Server list' })).toBeNull();
        cleanup();
        window.api.remoteDaemons.mockResolvedValue({ daemons: [{ hostId: 'host-b', name: '게임 PC', address: 'g', port: 57480 }] });
        await act(async () => render(<RemoteDaemonSection />));
        const view = screen.getByRole('combobox', { name: 'Server list' });
        expect(view).toHaveTextContent('Chosen daemon only');
        pickOption(view, 'All daemons combined');
        await act(async () => {});
        expect(useSettingsStore.getState().daemonView).toBe('unified');
    });

    it('이 PC를 열면 켬·포트·주소를 함께 저장하고 데몬이 준 상태를 보인다', async () => {
        window.api.remoteSettingsSet.mockResolvedValue({
            ...settings,
            enabled: true,
            address: 'saba.example.com',
            status: { listening: true, published: true },
        });
        await act(async () => render(<RemoteDaemonSection />));
        fireEvent.click(screen.getByRole('checkbox', { name: 'Allow remote connections' }));
        fireEvent.change(screen.getByRole('textbox', { name: 'Address' }), { target: { value: ' saba.example.com ' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Save' })));
        expect(window.api.remoteSettingsSet).toHaveBeenCalledWith({ enabled: true, port: 57480, address: 'saba.example.com' });
        expect(screen.getByText('Listening on port 57480')).toBeInTheDocument();
        expect(screen.getByText('Your other PCs know about this PC')).toBeInTheDocument();
    });

    it('포트를 열지 못하면 이유를 번역해 보인다', async () => {
        window.api.remoteSettings.mockResolvedValue({ ...settings, enabled: true, status: { listening: false, published: false, error: 'port_in_use' } });
        await act(async () => render(<RemoteDaemonSection />));
        expect(screen.getByRole('alert')).toHaveTextContent('The port is already in use');
    });

    it('주소를 알린 내 다른 PC에만 붙고, 붙은 뒤 목록을 다시 읽는다', async () => {
        window.api.remoteConnect.mockResolvedValue({ hostId: 'host-b', name: '게임 PC' });
        await act(async () => render(<RemoteDaemonSection />));
        const peers = screen.getByRole('list', { name: 'My other PCs' });
        const office = within(peers).getByText('사무실').closest('li');
        expect(within(office).queryByRole('button', { name: 'Connect' })).toBeNull();
        expect(within(office).getByText('Not open for remote connections')).toBeInTheDocument();
        const game = within(peers).getByText('게임 PC').closest('li');
        await act(async () => fireEvent.click(within(game).getByRole('button', { name: 'Connect' })));
        expect(window.api.remoteConnect).toHaveBeenCalledWith('host-b');
        expect(window.api.remoteDaemons).toHaveBeenCalledTimes(2);
    });

    it('붙지 못한 이유를 번역해 보인다', async () => {
        window.api.remoteConnect.mockResolvedValue({ error: 'remote_unreachable', error_code: 'remote_unreachable' });
        await act(async () => render(<RemoteDaemonSection />));
        const game = within(screen.getByRole('list', { name: 'My other PCs' })).getByText('게임 PC').closest('li');
        await act(async () => fireEvent.click(within(game).getByRole('button', { name: 'Connect' })));
        expect(screen.getByRole('alert')).toHaveTextContent('Could not reach the remote daemon');
    });

    it('짝 맺은 PC를 풀고 저장한 원격 데몬을 잊는다', async () => {
        window.api.remoteDaemons.mockResolvedValue({ daemons: [{ hostId: 'host-b', name: '게임 PC', address: 'g.example', port: 57480 }] });
        await act(async () => render(<RemoteDaemonSection />));
        const clients = screen.getByRole('list', { name: 'PCs connected to this one' });
        await act(async () => fireEvent.click(within(clients).getByRole('button', { name: 'Disconnect' })));
        expect(window.api.remoteRevoke).toHaveBeenCalledWith('host-a');
        const daemons = screen.getByRole('list', { name: 'Remote daemons this PC uses' });
        await act(async () => fireEvent.click(within(daemons).getByRole('button', { name: 'Forget' })));
        expect(window.api.remoteForget).toHaveBeenCalledWith('host-b');
    });

    it('공인 주소가 없는 원격 데몬은 릴레이를 거친다고 보인다 (CLI와 같다)', async () => {
        window.api.remoteDaemons.mockResolvedValue({
            daemons: [
                { hostId: 'host-b', name: '게임 PC', address: '', port: 57480 },
                { hostId: 'host-d', name: '집 서버', address: 'g.example', port: 57480 },
            ],
        });
        await act(async () => render(<RemoteDaemonSection />));
        const daemons = screen.getByRole('list', { name: 'Remote daemons this PC uses' });
        expect(within(daemons).getByText('Through the relay')).toBeInTheDocument();
        expect(within(daemons).getByText('g.example:57480')).toBeInTheDocument();
        expect(within(daemons).queryByText(':57480')).toBeNull();
    });

    it('릴레이에 연결하지 않았으면 내 다른 PC 대신 안내를 보인다', async () => {
        window.api.remotePeers.mockResolvedValue({ error: 'relay_not_paired', error_code: 'relay_not_paired' });
        await act(async () => render(<RemoteDaemonSection />));
        expect(screen.getByText('This PC is not connected to Discord yet. Connect it on the Discord page first.')).toBeInTheDocument();
    });

    it('지금 다루는 원격 데몬을 잊으면 이 PC로 돌아가고 화면을 다시 읽는다', async () => {
        window.api.remoteDaemons.mockResolvedValue({ daemons: [{ hostId: 'host-b', name: '게임 PC', address: 'g.example', port: 57480 }] });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        window.api.daemonTargetSet = vi.fn().mockResolvedValue({ success: true, target: null });
        const reload = vi.fn();
        await act(async () => render(<RemoteDaemonSection onTargetGone={reload} />));
        const daemons = screen.getByRole('list', { name: 'Remote daemons this PC uses' });
        await act(async () => fireEvent.click(within(daemons).getByRole('button', { name: 'Forget' })));
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith(null);
        expect(reload).toHaveBeenCalled();
    });
});
