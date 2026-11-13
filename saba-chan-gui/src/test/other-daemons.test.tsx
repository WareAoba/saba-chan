import { act, fireEvent, render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from 'vitest';
import { ServersPage } from '../components/servers/ServersPage';
import { OtherDaemons } from '../components/shell/OtherDaemons';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

const LIST = {
    daemons: [
        {
            hostId: null,
            name: '',
            ok: true,
            servers: [{ id: 'l1', name: 'test-pal', module: 'palworld', status: 'stopped' }],
        },
        {
            hostId: 'host-b',
            name: '게임 PC',
            ok: true,
            servers: [{ id: 'b1', name: 'test-pal', module: 'palworld', status: 'running', players: { online: 5, max: 20 } }],
        },
        { hostId: 'host-c', name: '클라우드', ok: false, error_code: 'remote_unreachable', servers: [] },
    ],
};

describe('합쳐 보기: 다른 데몬의 서버 (G1c)', () => {
    let switched: Mock;
    beforeEach(() => {
        switched = vi.fn();
        window.showToast = vi.fn();
        window.api = {
            remoteServers: vi.fn().mockResolvedValue(LIST),
            remoteInstanceAction: vi.fn().mockResolvedValue({ success: true }),
            daemonTargetSet: vi.fn().mockResolvedValue({ success: true }),
        } as unknown as typeof window.api;
    });
    afterEach(() => {
        vi.useRealTimers();
        delete window.showToast;
    });

    it('고른 데몬은 빼고 다른 데몬마다 절을 둔다. 닿지 않는 데몬은 이유를 보인다', async () => {
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        expect(screen.getByRole('region', { name: 'This PC' })).toHaveTextContent('test-pal');
        expect(screen.queryByRole('region', { name: '게임 PC' })).toBeNull();
        expect(screen.getByRole('region', { name: '클라우드' })).toHaveTextContent('Remote daemon connection failed');
    });

    it('켜기는 그 데몬에 바로 보내고, 끝나면 목록을 다시 읽는다', async () => {
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Start' })));
        expect(window.api.remoteInstanceAction).toHaveBeenCalledWith(null, 'l1', 'start', {});
        expect(window.api.remoteServers).toHaveBeenCalledTimes(2);
    });

    it('정지는 어느 PC인지와 접속자 수를 보이는 확인 창을 거친다', async () => {
        await act(async () => render(<OtherDaemons currentTarget={null} onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: '게임 PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Stop' })));
        expect(window.api.remoteInstanceAction).not.toHaveBeenCalled();
        const dialog = screen.getByRole('dialog', { name: 'Stop Server' });
        expect(dialog).toHaveTextContent('test-pal');
        expect(dialog).toHaveTextContent('Running on 게임 PC 5 player(s) online');
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Confirm' })));
        expect(window.api.remoteInstanceAction).toHaveBeenCalledWith('host-b', 'b1', 'stop', {});
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(window.api.remoteServers).toHaveBeenCalledTimes(2);
    });

    it('정지 확인을 취소하면 아무것도 보내지 않는다', async () => {
        await act(async () => render(<OtherDaemons currentTarget={null} onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: '게임 PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Stop' })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Cancel' })));
        expect(window.api.remoteInstanceAction).not.toHaveBeenCalled();
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('저장하고 정지할 길이 없으면 저장 없이 정지할지 묻고, 그러겠다면 force로 다시 보낸다', async () => {
        vi.mocked(window.api.remoteInstanceAction).mockImplementation(async (_h, _id, _action, opts) =>
            opts?.force ? { success: true } : { success: false, error_code: 'graceful_stop_unavailable', error: 'no console' },
        );
        await act(async () => render(<OtherDaemons currentTarget={null} onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: '게임 PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Stop' })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Confirm' })));
        const dialog = screen.getByRole('dialog', { name: 'Stop without saving?' });
        expect(window.showToast).not.toHaveBeenCalled();
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Stop without saving' })));
        expect(window.api.remoteInstanceAction).toHaveBeenLastCalledWith('host-b', 'b1', 'stop', { force: true });
    });

    it('약관 동의가 필요하면 같은 동의 창을 띄우고, 동의는 그 데몬에 기록한 뒤 다시 켠다', async () => {
        vi.mocked(window.api.remoteInstanceAction)
            .mockResolvedValueOnce({
                success: false,
                error_code: 'agreement_required',
                agreements: [{ id: 'eula', label: 'Game EULA', url: 'https://example.com/eula' }],
            })
            .mockResolvedValue({ success: true });
        window.api.remoteInstanceAgreements = vi.fn().mockResolvedValue({ success: true });
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Start' })));
        const dialog = screen.getByRole('dialog', { name: 'Agreement required' });
        expect(window.showToast).not.toHaveBeenCalled();
        expect(dialog).toHaveTextContent('Consent is recorded on the daemon of This PC.');
        fireEvent.click(within(dialog).getByRole('checkbox'));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Agree and start' })));
        expect(window.api.remoteInstanceAgreements).toHaveBeenCalledWith(null, 'l1', ['eula']);
        expect(window.api.remoteInstanceAction).toHaveBeenCalledTimes(2);
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('서버 종류는 모듈 id가 아니라 게임 이름으로 보인다', async () => {
        useServerStore.setState({ modules: [{ name: 'palworld', game_name: 'Palworld' }] as never });
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        expect(screen.getByRole('region', { name: 'This PC' })).toHaveTextContent('Palworld');
        useServerStore.setState({ modules: [] });
    });

    it('행 왼쪽에 서버 목록과 같은 아이콘 칸을 둔다: 게임 아이콘이 없으면 게임 이름 첫 글자', async () => {
        useServerStore.setState({ modules: [{ name: 'palworld', game_name: 'Palworld' }] as never });
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const icon = screen.getByRole('region', { name: 'This PC' }).querySelector('.server-row-icon');
        expect(icon).toHaveTextContent('P');
        expect(icon?.querySelector('.server-row-dot')).toHaveClass('tone-idle');
        useServerStore.setState({ modules: [] });
    });

    it('켜지 못하면 데몬이 준 문구를 알린다', async () => {
        vi.mocked(window.api.remoteInstanceAction).mockResolvedValue({ success: false, error: '동의가 필요해요' });
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Start' })));
        expect(window.api.remoteInstanceAction).toHaveBeenCalledWith(null, 'l1', 'start', {});
        expect(vi.mocked(window.showToast!).mock.calls[0].slice(0, 3)).toEqual(['동의가 필요해요', 'error', 5000]);
    });

    it('error_code만 실린 답도 실패로 알린다 (앱의 다른 켜기·끄기와 같은 판정)', async () => {
        vi.mocked(window.api.remoteInstanceAction).mockResolvedValue({ error_code: 'remote_unreachable' });
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Start' })));
        expect(vi.mocked(window.showToast!).mock.calls[0].slice(1, 3)).toEqual(['error', 5000]);
    });

    it('"이 데몬 열기"는 그 데몬으로 바꾸고 화면을 다시 읽는다', async () => {
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'Open this daemon' })));
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith(null);
        expect(switched).toHaveBeenCalled();
    });

    it('10초마다 다시 읽는다', async () => {
        vi.useFakeTimers();
        await act(async () => render(<OtherDaemons currentTarget={null} onSwitched={switched} />));
        expect(window.api.remoteServers).toHaveBeenCalledTimes(1);
        await act(async () => vi.advanceTimersByTime(10_000));
        expect(window.api.remoteServers).toHaveBeenCalledTimes(2);
    });

    it('고른 데몬에 서버가 없어도 다른 데몬의 서버를 보인다', async () => {
        useSettingsStore.setState({ daemonView: 'unified' });
        useUIStore.setState({ backgroundDaemonStatus: 'running' });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
        await act(async () =>
            render(
                <ServersPage
                    {...({
                        servers: [],
                        modules: [],
                        onAddServer: vi.fn(),
                        drag: { cardRefs: { current: {} } },
                    } as unknown as ComponentProps<typeof ServersPage>)}
                />,
            ),
        );
        expect(screen.getByText('No servers added')).toBeTruthy();
        expect(screen.getByRole('region', { name: '게임 PC' })).toHaveTextContent('test-pal');
        useSettingsStore.setState({ daemonView: 'switch' });
    });
});
