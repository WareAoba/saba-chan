import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { PortCheck, PortCheckNotice } from '../components/servers/PortCheck';
import { ServerAddress } from '../components/servers/ServerAddress';
import type { Server } from '../stores/useServerStore';
import { portHelpUrl } from '../utils/portCheck';

const server = { id: 'a', name: 'A', module: 'game', status: 'stopped' } as unknown as Server;

const closedReport = {
    instanceId: 'a',
    verdict: 'closed',
    address: '8.8.4.4',
    ports: [
        { field: 'port', port: 7777, protocol: 'tcp', status: 'closed' },
        { field: 'port', port: 7777, protocol: 'udp', status: 'open' },
    ],
};

describe('포트포워딩 확인', () => {
    beforeEach(() => {
        window.api = {} as unknown as typeof window.api;
    });

    it('브리지는 인스턴스의 port-check를 POST로 확인하고 GET으로 마지막 결과를 읽는다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { verdict: 'open' } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        await expect(api.instancePortCheck('a b')).resolves.toEqual({ verdict: 'open' });
        expect(invoke.mock.calls.at(-1)![1]).toMatchObject({ method: 'POST', path: '/api/instance/a%20b/port-check' });
        await api.instancePortCheckLast('a b');
        expect(invoke.mock.calls.at(-1)![1]).toMatchObject({ method: 'GET', path: '/api/instance/a%20b/port-check' });
    });

    it('예전 데몬(브리지 없음)이면 그리지 않는다', () => {
        const { container } = render(<PortCheck server={server} />);
        expect(container).toBeEmptyDOMElement();
    });

    it('확인하면 판정, 공인 주소, 포트마다 상태와 닫힘 안내를 보인다', async () => {
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue({ instanceId: 'a', verdict: 'not_checked' });
        window.api.instancePortCheck = vi.fn().mockResolvedValue(closedReport);
        render(<PortCheck server={server} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Check port forwarding/i })));
        expect(window.api.instancePortCheck).toHaveBeenCalledWith('a');
        expect(screen.getByText(/Not reachable from the internet/)).toBeInTheDocument();
        expect(screen.getByText(/8\.8\.4\.4/)).toBeInTheDocument();
        const items = screen.getAllByRole('listitem').map((li) => li.textContent);
        expect(items).toEqual(['7777/TCP · Closed', '7777/UDP · Open']);
        expect(screen.getByRole('link', { name: /Port forwarding guide/ })).toHaveAttribute('href', portHelpUrl('en'));
    });

    it('공개 주소가 이 PC를 가리키지 않으면 그 주소가 가리키는 곳과 고칠 곳을 보이고 상세 위에도 알린다', async () => {
        const report = {
            instanceId: 'a',
            verdict: 'host_mismatch',
            address: '8.8.4.4',
            host: 'mc.example.com',
            hostResult: 'mismatch',
            resolved: ['104.21.0.1'],
            ports: [{ field: 'port', port: 25565, protocol: 'tcp', status: 'unknown' }],
        };
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue(report);
        window.api.instancePortCheck = vi.fn();
        await act(async () => {
            render(<PortCheck server={server} />);
        });
        expect(screen.getByText(/does not point to this PC/)).toBeInTheDocument();
        expect(screen.getByText(/mc\.example\.com → 104\.21\.0\.1/)).toBeInTheDocument();
        expect(screen.getByText(/8\.8\.4\.4/)).toBeInTheDocument();
        expect(screen.getByText(/A record/)).toBeInTheDocument();
        const { portCheckNeedsAttention } = await import('../utils/portCheck');
        expect(portCheckNeedsAttention(report)).toBe(true);
    });

    it('실패하면 오류 코드의 안내를 보인다', async () => {
        window.api.instancePortCheck = vi.fn().mockResolvedValue({ success: false, error_code: 'node_token_missing' });
        render(<PortCheck server={server} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Check port forwarding/i })));
        expect(screen.getByRole('alert').textContent).toMatch(/Connect to Discord/);
    });

    it('자동 확인 결과가 닫힘이면 상세 위에 알리고, 열림이면 알리지 않는다', async () => {
        window.api.instancePortCheck = vi.fn();
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue(closedReport);
        const { unmount } = render(<PortCheckNotice server={server} />);
        expect(await screen.findByRole('status')).toHaveTextContent(/Not reachable from the internet/);
        unmount();
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'open' });
        const { container } = render(<PortCheckNotice server={server} />);
        await act(async () => {});
        expect(container).toBeEmptyDOMElement();
    });

    it('방화벽이 막은 결과도 상세 위에 알린다', async () => {
        window.api.instancePortCheck = vi.fn();
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'firewall' });
        render(<PortCheckNotice server={server} />);
        expect(await screen.findByRole('status')).toHaveTextContent(/Blocked by the PC firewall/);
    });

    it('결과가 나오면 성공이든 실패든 다시 확인할 수 있다', async () => {
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'open' });
        window.api.instancePortCheck = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'router' });
        render(<PortCheck server={server} />);
        const retry = await screen.findByRole('button', { name: /Check again/i });
        await act(async () => fireEvent.click(retry));
        expect(window.api.instancePortCheck).toHaveBeenCalledWith('a');
        expect(screen.getByText(/Port forwarding is not set up on the router/)).toBeInTheDocument();
        expect(screen.getByRole('button', { name: /Check again/i })).toBeInTheDocument();
    });

    it('상단 알림에서 고친 뒤 바로 다시 확인하고, 열리면 알림이 사라진다', async () => {
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'firewall' });
        window.api.instancePortCheck = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'open' });
        const { container } = render(<PortCheckNotice server={server} />);
        await screen.findByRole('status');
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Check again/i })));
        expect(window.api.instancePortCheck).toHaveBeenCalledWith('a');
        expect(container).toBeEmptyDOMElement();
    });

    it('도움말 주소는 웹사이트 언어 경로를 따른다 (영어는 앞에 언어가 없다)', () => {
        expect(portHelpUrl('en')).toBe('https://saba-chan.online/docs/server-management');
        expect(portHelpUrl('ko')).toBe('https://saba-chan.online/ko/docs/server-management');
        expect(portHelpUrl('zh-cn')).toBe('https://saba-chan.online/zh-CN/docs/server-management');
        expect(portHelpUrl('pt')).toBe('https://saba-chan.online/docs/server-management');
    });
});

// 포트포워딩 안내는 닫을 수 있고, 같은 결과면 다시 그려도 닫힌 채다. 결과가 바뀌면 다시 뜬다 (사용자 11-07)
describe('포트포워딩 안내 닫기', () => {
    it('닫으면 사라지고, 다른 결과가 오면 다시 뜬다', async () => {
        sessionStorage.clear();
        window.api = { instancePortCheck: vi.fn(), instancePortCheckLast: vi.fn().mockResolvedValue(closedReport) } as never;
        const first = render(<PortCheckNotice server={server} />);
        await screen.findByRole('status');
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Close' })));
        expect(screen.queryByRole('status')).toBeNull();
        first.unmount();
        const again = render(<PortCheckNotice server={server} />);
        await act(async () => {});
        expect(screen.queryByRole('status')).toBeNull();
        again.unmount();
        window.api.instancePortCheckLast = vi.fn().mockResolvedValue({ ...closedReport, verdict: 'firewall' });
        render(<PortCheckNotice server={server} />);
        expect(await screen.findByRole('status')).toBeInTheDocument();
        sessionStorage.clear();
    });
});

// 접속 주소 상자와 상단 안내가 같은 판정을 두 번 말하지 않는다 (사용자 11-12). 안내를 닫으면 상자에 판정이 남는다
describe('접속 주소 상자의 판정 표시', () => {
    it('상단 안내가 떠 있으면 상자에는 판정을 다시 쓰지 않고, 닫으면 상자에 남긴다', async () => {
        sessionStorage.clear();
        window.api = {
            instancePortCheck: vi.fn(),
            instancePortCheckLast: vi.fn().mockResolvedValue({ ...closedReport, verdict: 'router', checkedAt: '2026-11-12T00:00:00Z' }),
            instanceAddress: vi.fn().mockResolvedValue({ address: '8.8.4.4', running: false, ports: [{ field: 'port', port: 7777 }] }),
        } as never;
        render(
            <>
                <ServerAddress server={server} />
                <PortCheckNotice server={server} />
            </>,
        );
        await screen.findByRole('status');
        const verdict = /Port forwarding is not set up on the router/;
        expect(screen.getAllByText(verdict)).toHaveLength(1);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Close' })));
        expect(screen.queryByRole('status')).toBeNull();
        expect(screen.getAllByText(verdict)).toHaveLength(1);
        sessionStorage.clear();
    });
});
