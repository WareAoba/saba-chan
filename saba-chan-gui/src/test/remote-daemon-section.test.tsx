import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { olderVersion, RemoteDaemonSection } from '../components/Modals/RemoteDaemonSection';
import { useSettingsStore } from '../stores/useSettingsStore';
import { pickOption } from './dropdown';
import { SaveBarHost } from '../components/ui/SaveBar';

const settings = {
    enabled: true,
    port: 57480,
    address: '',
    transport: 'direct',
    status: { listening: true, published: true },
    hostId: 'host-me',
    thisPc: { platform: { os: 'windows x86_64' }, agentVersion: '0.1.13', publicIp: '125.180.44.17' },
};

const vps = {
    hostId: 'host-b',
    name: 'vps',
    label: '김마무(vps)',
    status: 'online',
    hasEndpoint: true,
    saved: true,
    platform: { os: 'linux x86_64', distro: { id: 'ubuntu', version: '24.04', name: 'Ubuntu 24.04 LTS' } },
    agentVersion: '0.1.13',
    lastSeenAt: new Date().toISOString(),
    publicIp: '203.0.113.24',
    route: 'direct',
};
const vpsDaemon = { hostId: 'host-b', name: 'vps', label: '김마무(vps)', address: '203.0.113.24', port: 57480, pairedAt: '2026-11-08T09:12:00Z' };

const cards = () => screen.getByRole('list', { name: 'My PCs' });
const card = (name: string) => within(cards()).getByText(name).closest('li')!;
// 카드마다 수정 버튼 하나: 업데이트·지우기는 그 창 안에 있다 (사용자 결정 2026-11-11)
const openEdit = async (name: string) => {
    await act(async () => fireEvent.click(within(card(name)).getByRole('button', { name: 'Edit' })));
    return screen.getByRole('dialog');
};

describe('설정 > 연결: 원격 데몬 (/api/remote/*)', () => {
    beforeEach(() => {
        useSettingsStore.setState({ mainDaemon: null, daemonView: 'switch' });
        window.api = {
            remoteSettings: vi.fn().mockResolvedValue(settings),
            remoteSettingsSet: vi.fn(),
            remotePeers: vi.fn().mockResolvedValue({
                peers: [
                    { hostId: 'host-c', name: '게임 PC', label: '게임 PC', status: 'online', hasEndpoint: true, saved: false, platform: { os: 'windows x86_64' } },
                    { hostId: 'host-d', name: '사무실', label: '사무실', status: 'offline', hasEndpoint: false, saved: false },
                ],
            }),
            remoteDaemons: vi.fn().mockResolvedValue({ daemons: [] }),
            remoteConnect: vi.fn(),
            remoteRemove: vi.fn().mockResolvedValue({ uninstalled: false, removedFromRelay: false }),
            remoteDaemonUpdate: vi.fn().mockResolvedValue({ success: true }),
            remoteInstallStart: vi.fn(),
            remoteInstall: vi.fn(),
            remoteInstallInput: vi.fn().mockResolvedValue({ success: true }),
            remoteInstallCancel: vi.fn().mockResolvedValue({ success: true }),
            remoteClaim: vi.fn(),
        } as unknown as typeof window.api;
    });

    it('이 PC가 맨 앞 카드로 강조되고, 공인 IP와 원격 포트를 보인다', async () => {
        await act(async () => render(<RemoteDaemonSection />));
        const first = within(cards()).getAllByRole('listitem')[0];
        expect(first).toHaveClass('me');
        expect(within(first).getByText('This PC', { selector: '.remote-pc-name' })).toBeInTheDocument();
        expect(within(first).getByText('125.180.44.17')).toBeInTheDocument();
        expect(within(first).getByText('Listening on port 57480')).toBeInTheDocument();
        // OS 줄은 OS 이름만 (x86_64는 적지 않는다)
        expect(within(first).getByText('Windows', { selector: '.remote-pc-sub' })).toBeInTheDocument();
        // 이름 줄은 이름이 혼자 쓴다: 상태·OS·판은 그 아래 한 줄 (이름이 잘리지 않게, 사용자 11-13)
        const meta = first.querySelector('.remote-pc-meta') as HTMLElement;
        expect(within(meta).getByText('Online')).toBeInTheDocument();
        expect(within(meta).getByText('v0.1.13')).toBeInTheDocument();
        expect(first.querySelector('.remote-pc-namerow')?.textContent).not.toMatch(/v0\.1\.13|Online/);
    });

    it('연결한 PC 카드는 배포판 로고, 연결한 날, 공인 IP, 직접 연결을 보인다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ peers: [vps] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        await act(async () => render(<RemoteDaemonSection />));
        const c = card('김마무(vps)');
        expect(within(c).getByRole('img', { name: 'Ubuntu' })).toBeInTheDocument();
        expect(within(c).getByText('Ubuntu 24.04 LTS', { selector: '.remote-pc-sub' })).toBeInTheDocument();
        expect(within(c).getByText('v0.1.13')).toBeInTheDocument();
        expect(within(c).getByText('203.0.113.24')).toBeInTheDocument();
        expect(within(c).getByText('Direct (port 57480)')).toBeInTheDocument();
        expect(within(c).getByText('Online')).toBeInTheDocument();
        expect(within(c).queryByText('Not connected yet')).toBeNull();
    });

    it('CPU는 ARM일 때만 OS 이름 뒤에 (ARM)으로 붙인다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({
            peers: [
                { ...vps, platform: { ...vps.platform, os: 'linux aarch64' } },
                { hostId: 'host-m', name: '맥북', label: '맥북', status: 'online', hasEndpoint: false, saved: false, platform: { os: 'macos aarch64' } },
            ],
        });
        await act(async () => render(<RemoteDaemonSection />));
        expect(within(card('김마무(vps)')).getByText('Ubuntu 24.04 LTS (ARM)')).toBeInTheDocument();
        expect(within(card('맥북')).getByText('macOS (ARM)')).toBeInTheDocument();
    });

    it('별을 누른 PC가 메인 데몬이 되고, 이 PC의 별을 누르면 이 PC로 돌아간다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ peers: [vps] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        await act(async () => render(<RemoteDaemonSection />));
        const me = within(cards()).getAllByRole('listitem')[0];
        expect(within(me).getByRole('button', { name: 'Main daemon' })).toHaveAttribute('aria-pressed', 'true');
        fireEvent.click(within(card('김마무(vps)')).getByRole('button', { name: 'Make main daemon' }));
        expect(useSettingsStore.getState().mainDaemon).toBe('host-b');
        fireEvent.click(within(me).getByRole('button', { name: 'Make main daemon' }));
        expect(useSettingsStore.getState().mainDaemon).toBeNull();
        // 메인 데몬 드롭다운은 별로 바뀌었다
        expect(screen.queryByRole('combobox', { name: 'Main daemon' })).toBeNull();
    });

    it('원격 포트를 연 내 다른 PC에만 연결 버튼이 있고, 붙은 뒤 목록을 다시 읽는다', async () => {
        vi.mocked(window.api.remoteConnect).mockResolvedValue({ hostId: 'host-c', name: '게임 PC', label: '게임 PC' });
        await act(async () => render(<RemoteDaemonSection />));
        expect(within(card('사무실')).queryByRole('button', { name: 'Connect' })).toBeNull();
        expect(within(card('사무실')).getByText('Not open for remote connections')).toBeInTheDocument();
        expect(within(card('사무실')).getByText('Not connected yet')).toBeInTheDocument();
        await act(async () => fireEvent.click(within(card('게임 PC')).getByRole('button', { name: 'Connect' })));
        expect(window.api.remoteConnect).toHaveBeenCalledWith('host-c');
        expect(window.api.remoteDaemons).toHaveBeenCalledTimes(2);
    });

    it('붙지 못한 이유를 번역해 보인다', async () => {
        vi.mocked(window.api.remoteConnect).mockResolvedValue({ error: 'remote_unreachable', error_code: 'remote_unreachable' });
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(within(card('게임 PC')).getByRole('button', { name: 'Connect' })));
        expect(screen.getByRole('alert')).toHaveTextContent('Remote daemon connection failed');
    });

    it('삭제는 한 창에서 고른다: 온라인 리눅스면 그 PC에서도 지우고 서버쨩 목록에서도 뺀다', async () => {
        useSettingsStore.setState({ mainDaemon: 'host-b' });
        vi.mocked(window.api.remotePeers).mockResolvedValue({ peers: [vps] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(within(await openEdit('김마무(vps)')).getByRole('button', { name: 'Delete' })));
        const dialog = screen.getByRole('dialog', { name: 'Remove 김마무(vps)?' });
        const uninstall = within(dialog).getByRole('checkbox', { name: 'Also uninstall Saba-chan on that PC' });
        expect(uninstall).not.toBeDisabled();
        fireEvent.click(uninstall);
        fireEvent.click(within(dialog).getByRole('checkbox', { name: 'Keep settings' }));
        fireEvent.click(within(dialog).getByRole('checkbox', { name: 'Also remove from My PCs' }));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Remove' })));
        expect(window.api.remoteRemove).toHaveBeenCalledWith('host-b', {
            uninstall: true,
            keepSettings: false,
            keepInstances: true,
            removePc: true,
        });
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(useSettingsStore.getState().mainDaemon).toBeNull();
    });

    it('Windows나 꺼진 PC는 그 PC에서 지우기를 고를 수 없다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ peers: [{ ...vps, platform: { os: 'windows x86_64' } }] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(within(await openEdit('김마무(vps)')).getByRole('button', { name: 'Delete' })));
        expect(screen.getByRole('checkbox', { name: 'Also uninstall Saba-chan on that PC' })).toBeDisabled();
    });

    it('지금 다루는 원격 데몬을 지우면 이 PC로 돌아가고 화면을 다시 읽는다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ peers: [vps] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        window.api.daemonTargetSet = vi.fn().mockResolvedValue({ success: true, target: null });
        const reload = vi.fn();
        await act(async () => render(<RemoteDaemonSection onTargetGone={reload} />));
        await act(async () => fireEvent.click(within(await openEdit('김마무(vps)')).getByRole('button', { name: 'Delete' })));
        await act(async () => fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove' })));
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith(null);
        expect(reload).toHaveBeenCalled();
    });

    it('이 PC보다 옛 판인 원격 데몬은 업데이트 버튼으로 그 PC를 업데이트한다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ peers: [{ ...vps, agentVersion: '0.1.12' }] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        await act(async () => render(<RemoteDaemonSection />));
        // 카드에는 판 옆 표시만, 업데이트는 수정 창에서
        expect(within(card('김마무(vps)')).getByText('v0.1.12')).toHaveAttribute('data-tooltip', 'Needs update to 0.1.13');
        expect(within(card('김마무(vps)')).queryByRole('button', { name: 'Update' })).toBeNull();
        const dialog = await openEdit('김마무(vps)');
        expect(within(dialog).getByText('Needs update to 0.1.13')).toBeInTheDocument();
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Update' })));
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(window.api.remoteDaemonUpdate).toHaveBeenCalledWith('host-b');
        expect(screen.getByText(/Update started/)).toBeInTheDocument();
        expect(olderVersion('0.1.9', '0.1.13')).toBe(true);
        expect(olderVersion('0.1.13', '0.1.13')).toBe(false);
        expect(olderVersion(undefined, '0.1.13')).toBe(false);
    });

    it('그 PC의 데몬이 지워졌으면 까닭을 보이고, 짝을 잊었으면 다시 연결한다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({
            peers: [
                { hostId: 'host-b', name: '게임 PC', label: '게임 PC', hasEndpoint: true, saved: true },
                { hostId: 'host-c', name: '사무실', label: '사무실', hasEndpoint: false, saved: true },
            ],
        });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({
            daemons: [
                { hostId: 'host-b', name: '게임 PC', label: '게임 PC', address: '', port: 57480 },
                { hostId: 'host-c', name: '사무실', label: '사무실', address: '', port: 57480 },
                { hostId: 'host-d', name: '옛 PC', label: '옛 PC', address: '', port: 57480 },
            ],
        });
        window.api.remoteServers = vi.fn().mockResolvedValue({
            daemons: [
                { hostId: null, ok: true, servers: [] },
                { hostId: 'host-b', ok: false, error_code: 'remote_unauthorized', servers: [] },
                { hostId: 'host-c', ok: false, error_code: 'remote_unreachable', servers: [] },
                { hostId: 'host-d', ok: false, error_code: 'remote_unreachable', servers: [] },
            ],
        });
        vi.mocked(window.api.remoteConnect).mockResolvedValue({ hostId: 'host-b', name: '게임 PC', label: '게임 PC' });
        await act(async () => render(<RemoteDaemonSection />));
        expect(within(card('사무실')).getByText('Not reachable')).toBeInTheDocument();
        expect(within(card('옛 PC')).getByText('Removed from Server-chan')).toBeInTheDocument();
        expect(within(card('게임 PC')).getByText('That PC forgot this one')).toBeInTheDocument();
        expect(within(card('게임 PC')).getByText('Through Server-chan')).toBeInTheDocument();
        await act(async () => fireEvent.click(within(card('게임 PC')).getByRole('button', { name: 'Reconnect' })));
        expect(window.api.remoteConnect).toHaveBeenCalledWith('host-b');
    });

    it('연결한 PC는 그 데몬을 여는 버튼이 있고, 지금 보는 데몬이면 보는 중이라고 보인다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({
            peers: [
                { hostId: 'host-b', name: '게임 PC', label: '게임 PC', hasEndpoint: true, saved: true },
                { hostId: 'host-c', name: '사무실', label: '사무실', hasEndpoint: true, saved: true },
            ],
        });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({
            daemons: [
                { hostId: 'host-b', name: '게임 PC', label: '게임 PC', address: 'g', port: 57480 },
                { hostId: 'host-c', name: '사무실', label: '사무실', address: 'o', port: 57480 },
            ],
        });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-c' });
        window.api.daemonTargetSet = vi.fn().mockResolvedValue({ success: true, target: 'host-b' });
        const switched = vi.fn();
        await act(async () => render(<RemoteDaemonSection onSwitched={switched} />));
        expect(within(card('사무실')).queryByRole('button', { name: 'Open' })).toBeNull();
        expect(within(card('사무실')).getByText('Viewing')).toBeInTheDocument();
        await act(async () => fireEvent.click(within(card('게임 PC')).getByRole('button', { name: 'Open' })));
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith('host-b');
        expect(switched).toHaveBeenCalled();
    });

    it('리눅스에 설치: ssh 물음은 카드로 묻고 답만 보내며, 끝나면 목록을 다시 읽는다', async () => {
        vi.mocked(window.api.remoteInstallStart).mockResolvedValue({ id: 'i1' });
        vi.mocked(window.api.remoteInstall)
            .mockResolvedValueOnce({ phase: 'installing', output: 'saba@nas password: ', offset: 19, prompt: { kind: 'password', text: "saba@nas's password:" } })
            .mockResolvedValue({ phase: 'done', output: '\nok\n', offset: 23, prompt: null, hostId: 'host-n', name: '김마무(nas)', label: '김마무(nas)' });
        vi.useFakeTimers();
        try {
            await act(async () => render(<RemoteDaemonSection />));
            await act(async () => fireEvent.click(within(cards()).getByRole('button', { name: /Install on Linux/ })));
            const dialog = screen.getByRole('dialog', { name: 'Install on Linux' });
            fireEvent.change(within(dialog).getByRole('textbox', { name: 'Address' }), { target: { value: 'nas.local' } });
            fireEvent.change(within(dialog).getByRole('textbox', { name: 'User' }), { target: { value: 'saba' } });
            await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Install' })));
            expect(window.api.remoteInstallStart).toHaveBeenCalledWith({ host: 'nas.local', user: 'saba', port: 22, linger: true, steamLibs: false });
            await act(async () => vi.advanceTimersByTime(10));
            const prompt = within(dialog).getByRole('group', { name: 'Question' });
            fireEvent.change(within(prompt).getByLabelText('Password'), { target: { value: 's3cret' } });
            await act(async () => fireEvent.click(within(prompt).getByRole('button', { name: 'Send' })));
            expect(window.api.remoteInstallInput).toHaveBeenCalledWith('i1', 's3cret');
            await act(async () => vi.advanceTimersByTime(800));
            expect(within(dialog).getByText('김마무(nas) connected')).toBeInTheDocument();
            expect(window.api.remotePeers).toHaveBeenCalledTimes(2);
        } finally {
            vi.useRealTimers();
        }
    });

    it('SSH가 안 되면 한 줄 설치 뒤 나온 코드를 넣어 붙인다', async () => {
        vi.mocked(window.api.remoteClaim).mockResolvedValue({ hostId: 'host-n', name: 'nas', label: '김마무(nas)' });
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(within(cards()).getByRole('button', { name: /Install on Linux/ })));
        const dialog = screen.getByRole('dialog', { name: 'Install on Linux' });
        fireEvent.click(within(dialog).getByRole('button', { name: 'No SSH?' }));
        expect(within(dialog).getByText(/install\.sh \| sh -s -- --pair/)).toBeInTheDocument();
        fireEvent.change(within(dialog).getByRole('textbox', { name: 'Pairing code' }), { target: { value: ' Abc123_xyz-0 ' } });
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Add' })));
        expect(window.api.remoteClaim).toHaveBeenCalledWith('Abc123_xyz-0');
        expect(within(dialog).getByText('김마무(nas) connected')).toBeInTheDocument();
    });

    it('원격 데몬을 연결했으면 서버 목록 보기(고른 데몬만 / 합쳐 보기)를 고른다', async () => {
        await act(async () => render(<RemoteDaemonSection />));
        expect(screen.queryByRole('combobox', { name: 'Server list' })).toBeNull();
        cleanup();
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [{ hostId: 'host-b', name: '게임 PC', label: '게임 PC', address: 'g', port: 57480 }] });
        await act(async () => render(<RemoteDaemonSection />));
        const view = screen.getByRole('combobox', { name: 'Server list' });
        pickOption(view, 'All daemons combined');
        await act(async () => {});
        expect(useSettingsStore.getState().daemonView).toBe('unified');
    });

    // 주소는 내 PC 카드의 게임 서버 주소 줄 한 칸에서 정한다 (PublicAddress, 사용자 결정 2026-11-10): 여기서는 켬·포트만
    // 켬은 바로 저장한다. 포트는 다 친 뒤 저장 막대로 묻는다 (사용자 11-11: 즉시 저장 + 어쩔 수 없는 것만 아래 막대)
    it('켜면 바로 저장하고 데몬이 준 상태를 보인다', async () => {
        vi.mocked(window.api.remoteSettings).mockResolvedValue({ ...settings, enabled: false, status: { listening: false, published: false } });
        vi.mocked(window.api.remoteSettingsSet).mockResolvedValue({ ...settings });
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(screen.getByRole('checkbox', { name: 'Allow remote connections' })));
        expect(window.api.remoteSettingsSet).toHaveBeenCalledWith({ enabled: true, port: 57480 });
        expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
        expect(screen.getByText('Your other PCs notified of this PC')).toBeInTheDocument();
    });

    it('포트를 바꾸면 저장 막대가 올라오고, 저장하면 켬 상태 그대로 새 포트를 보낸다', async () => {
        vi.mocked(window.api.remoteSettingsSet).mockResolvedValue({ ...settings, port: 57490 });
        await act(async () =>
            render(
                <SaveBarHost>
                    <RemoteDaemonSection />
                </SaveBarHost>,
            ),
        );
        expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
        fireEvent.change(screen.getByRole('spinbutton', { name: 'Port' }), { target: { value: '57490' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Save' })));
        expect(window.api.remoteSettingsSet).toHaveBeenCalledWith({ enabled: true, port: 57490 });
    });

    it('포트를 친 채로 연결 탭을 떠났다 와도 친 포트와 저장 막대가 남는다', async () => {
        const Page = ({ show }: { show: boolean }) => <SaveBarHost>{show && <RemoteDaemonSection />}</SaveBarHost>;
        const { rerender } = await act(async () => render(<Page show />));
        fireEvent.change(screen.getByRole('spinbutton', { name: 'Port' }), { target: { value: '57490' } });
        rerender(<Page show={false} />);
        expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
        await act(async () => rerender(<Page show />));
        expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Port' }).value).toBe('57490');
    });

    // 리뷰 재검토 #3: 다른 탭에서 저장했다 실패하면 막대를 내리지 않고 실패를 알린다
    it('연결 탭을 떠난 뒤 저장이 실패하면 막대가 남고 이유를 알린다', async () => {
        window.showToast = vi.fn();
        vi.mocked(window.api.remoteSettingsSet).mockResolvedValue({ success: false, error_code: 'port_in_use' });
        const Page = ({ show }: { show: boolean }) => <SaveBarHost>{show && <RemoteDaemonSection />}</SaveBarHost>;
        const { rerender } = await act(async () => render(<Page show />));
        fireEvent.change(screen.getByRole('spinbutton', { name: 'Port' }), { target: { value: '57490' } });
        rerender(<Page show={false} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Save' })));
        expect(window.showToast).toHaveBeenCalledWith('Port in use', 'error');
        expect(document.querySelector('.saba-savebar.dirty')).not.toBeNull();
    });

    it('포트를 친 채로 켬을 바꿔도 친 포트는 남아 저장 막대에서 저장한다', async () => {
        vi.mocked(window.api.remoteSettingsSet).mockResolvedValue({ ...settings, enabled: false });
        await act(async () =>
            render(
                <SaveBarHost>
                    <RemoteDaemonSection />
                </SaveBarHost>,
            ),
        );
        fireEvent.change(screen.getByRole('spinbutton', { name: 'Port' }), { target: { value: '57490' } });
        await act(async () => fireEvent.click(screen.getByRole('checkbox', { name: 'Allow remote connections' })));
        expect(window.api.remoteSettingsSet).toHaveBeenCalledWith({ enabled: false, port: 57480 });
        expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Port' }).value).toBe('57490');
        expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
    });

    it('포트를 열지 못하면 이유를 번역해 보인다', async () => {
        vi.mocked(window.api.remoteSettings).mockResolvedValue({ ...settings, status: { listening: false, published: false, error: 'port_in_use' } });
        await act(async () => render(<RemoteDaemonSection />));
        expect(screen.getByRole('alert')).toHaveTextContent('Port in use');
    });

    // 내 PC 카드 개편 뒤 쓸모가 없어진 "이 PC에 붙은 PC" 목록은 뺐다 (사용자 11-12)
    it('릴레이에 연결하지 않았으면 안내를 보이고, 이 PC에 붙은 PC 목록은 없다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ error: 'relay_not_paired', error_code: 'relay_not_paired' });
        await act(async () => render(<RemoteDaemonSection />));
        expect(screen.getByText('This PC is not connected to Discord. Connect it on the Discord page first.')).toBeInTheDocument();
        expect(screen.queryByRole('list', { name: 'PCs connected to this one' })).toBeNull();
    });

    // 사용자 11-12: "연결"을 누르면 결과가 뜰 때까지 화면이 멈췄다 (칸의 모든 버튼이 잠겼다)
    it('한 PC에 연결하는 동안 그 카드만 진행 중이고 다른 PC·켬 스위치는 그대로 쓸 수 있다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({
            peers: [
                { hostId: 'host-c', name: '게임 PC', label: '게임 PC', status: 'online', hasEndpoint: true, saved: false },
                { hostId: 'host-e', name: '거실', label: '거실', status: 'online', hasEndpoint: true, saved: false },
            ],
        });
        let finish: (v: unknown) => void = () => {};
        vi.mocked(window.api.remoteConnect).mockImplementation(
            () =>
                new Promise((r) => {
                    finish = r as (v: unknown) => void;
                }),
        );
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(within(card('게임 PC')).getByRole('button', { name: 'Connect' })));
        expect(within(card('게임 PC')).getByRole('button', { name: /Connect/ })).toBeDisabled();
        expect(within(card('거실')).getByRole('button', { name: 'Connect' })).toBeEnabled();
        expect(within(card('거실')).getByRole('button', { name: 'Edit' })).toBeEnabled();
        expect(screen.getByRole('checkbox', { name: 'Allow remote connections' })).toBeEnabled();
        await act(async () => fireEvent.click(within(card('거실')).getByRole('button', { name: 'Connect' })));
        expect(window.api.remoteConnect).toHaveBeenCalledWith('host-e');
        await act(async () => finish({ hostId: 'host-c', name: '게임 PC', label: '게임 PC' }));
    });

    // 어느 PC에서든 내 PC 이름을 바꾼다. 이름은 서버쨩 한 곳에 있다 (사용자 11-12)
    it('이 PC 카드는 PC 이름에 이 PC 표시를 붙이고, 수정 창에서 바꾼 이름을 데몬에 보낸다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ self: { hostId: 'host-me', name: '작업용', label: '작업용' }, peers: [vps] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon], thisName: '작업용' });
        window.api.remoteRenamePc = vi.fn().mockResolvedValue({ hostId: 'host-b', name: '맥북', label: '맥북' });
        await act(async () => render(<RemoteDaemonSection />));
        const first = within(cards()).getAllByRole('listitem')[0];
        expect(within(first).getByText('작업용', { selector: '.remote-pc-name' })).toBeInTheDocument();
        expect(within(first).getByText('This PC', { selector: '.remote-pc-tag' })).toBeInTheDocument();

        const dialog = await openEdit('김마무(vps)');
        const input = within(dialog).getByRole('textbox', { name: 'PC name' });
        expect(input).toHaveValue('vps');
        await act(async () => fireEvent.change(input, { target: { value: '  맥북 ' } }));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Save' })));
        expect(window.api.remoteRenamePc).toHaveBeenCalledWith('host-b', '맥북');
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('이름을 바꾸지 못하면 창을 닫지 않고 까닭을 보인다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ self: { hostId: 'host-me', name: '작업용', label: '작업용' }, peers: [vps] });
        window.api.remoteRenamePc = vi.fn().mockResolvedValue({ error: 'x', error_code: 'relay_unreachable' });
        await act(async () => render(<RemoteDaemonSection />));
        await act(async () => fireEvent.click(within(within(cards()).getAllByRole('listitem')[0]).getByRole('button', { name: 'Edit' })));
        const dialog = screen.getByRole('dialog');
        await act(async () => fireEvent.change(within(dialog).getByRole('textbox', { name: 'PC name' }), { target: { value: '새 이름' } }));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Save' })));
        expect(window.api.remoteRenamePc).toHaveBeenCalledWith('host-me', '새 이름');
        expect(within(screen.getByRole('dialog')).getByRole('alert')).toBeInTheDocument();
    });

    it('수정 창 제목 자리가 PC 이름 칸이고, 바로 오른쪽 별로 메인 데몬을 고른다', async () => {
        vi.mocked(window.api.remotePeers).mockResolvedValue({ self: { hostId: 'host-me', name: '작업용', label: '작업용' }, peers: [vps] });
        vi.mocked(window.api.remoteDaemons).mockResolvedValue({ daemons: [vpsDaemon] });
        await act(async () => render(<RemoteDaemonSection />));
        const dialog = await openEdit('김마무(vps)');
        const title = dialog.querySelector('.saba-dialog-title')!;
        expect(within(title as HTMLElement).getByRole('textbox', { name: 'PC name' })).toHaveValue('vps');
        await act(async () => fireEvent.click(within(title as HTMLElement).getByRole('button', { pressed: false })));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Save' })));
        expect(useSettingsStore.getState().mainDaemon).toBe('host-b');
    });
});
