import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { PcSettingsDialog } from '../components/Modals/PcSettingsDialog';
import { PublicAddressRow } from '../components/Modals/PublicAddress';
import { ServerAddress } from '../components/servers/ServerAddress';
import type { Server } from '../stores/useServerStore';

const server = { id: 'a', name: 'A', module: 'game', status: 'running' } as unknown as Server;
const port = (p: number, label = 'Game port') => ({ field: 'port', label, port: p, protocols: ['tcp'] });

describe('PC 공개 주소 (사용자 결정 2026-11-10)', () => {
    beforeEach(() => {
        window.api = {} as unknown as typeof window.api;
        try {
            localStorage.clear();
        } catch {
            // 없어도 된다
        }
    });

    it('브리지는 이 PC든 원격 데몬이든 그 데몬의 /api/public-address를 부른다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { address: '' } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        await api.publicAddress(null);
        expect(invoke.mock.calls.at(-1)![1]).toMatchObject({ method: 'GET', path: '/api/public-address', target: 'local' });
        await api.publicAddressSet('host-b', { address: 'mc.example.com' });
        expect(invoke.mock.calls.at(-1)![1]).toMatchObject({
            method: 'PUT',
            path: '/api/public-address',
            target: 'host-b',
            body: { address: 'mc.example.com' },
        });
        await api.instanceAddress('a b');
        expect(invoke.mock.calls.at(-1)![1]).toMatchObject({ method: 'GET', path: '/api/instance/a%20b/address' });
    });

    it('내 PC 카드에 게임 서버 주소를 보인다', async () => {
        window.api.publicAddress = vi.fn().mockResolvedValue({ address: 'mc.example.com', shareAddress: false, publicIp: '203.0.113.7' });
        await act(async () => {
            render(
                <dl>
                    <PublicAddressRow hostId="host-b" />
                </dl>,
            );
        });
        expect(window.api.publicAddress).toHaveBeenCalledWith('host-b');
        expect(screen.getByText('mc.example.com')).toBeInTheDocument();
        expect(screen.getByText('Not shared on Discord')).toBeInTheDocument();
        // 바꾸기 버튼은 카드의 수정 창으로 옮겼다
        expect(screen.queryByRole('button')).toBeNull();
    });

    const dialog = (hostId: string | null, onClose = vi.fn()) =>
        render(
            <PcSettingsDialog
                hostId={hostId}
                name="게임 PC"
                sub="Windows"
                hasAddress
                canRemove={false}
                onMain={vi.fn()}
                onUpdate={vi.fn()}
                onRemove={vi.fn()}
                onClose={onClose}
            />,
        );

    it('수정 창에서 공개 주소와 디스코드 알려 주기를 저장한다', async () => {
        window.api.publicAddress = vi.fn().mockResolvedValue({ address: '', shareAddress: true, publicIp: '203.0.113.7' });
        window.api.publicAddressSet = vi.fn().mockResolvedValue({ address: 'mc.example.com', shareAddress: false, publicIp: '203.0.113.7' });
        const onClose = vi.fn();
        await act(async () => {
            dialog('host-b', onClose);
        });
        expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled();
        fireEvent.click(screen.getByRole('tab', { name: /Set a public address/ }));
        fireEvent.change(screen.getByRole('textbox'), { target: { value: 'https://mc.example.com:25565' } });
        fireEvent.click(screen.getByRole('switch'));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /^Save/ })));
        expect(window.api.publicAddressSet).toHaveBeenCalledWith('host-b', { address: 'https://mc.example.com:25565', shareAddress: false });
        expect(onClose).toHaveBeenCalledWith(true);
    });

    it('공인 IP 그대로를 고르면 주소를 비워 저장한다', async () => {
        window.api.publicAddress = vi.fn().mockResolvedValue({ address: 'mc.example.com', shareAddress: true, publicIp: null });
        window.api.publicAddressSet = vi.fn().mockResolvedValue({ address: '', shareAddress: true, publicIp: null });
        await act(async () => {
            dialog(null);
        });
        fireEvent.click(screen.getByRole('tab', { name: /Use the public IP/ }));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /^Save/ })));
        expect(window.api.publicAddressSet).toHaveBeenCalledWith(null, { address: '', shareAddress: true });
    });

    it('예전 데몬이라 공개 주소를 읽지 못하면 줄을 그리지 않는다', async () => {
        window.api.publicAddress = vi.fn().mockResolvedValue({ success: false, error_code: 'not_found' });
        let container: HTMLElement | undefined;
        await act(async () => {
            ({ container } = render(
                <dl>
                    <PublicAddressRow hostId="old" />
                </dl>,
            ));
        });
        expect(container!.querySelector('dt')).toBeNull();
    });
});

describe('인스턴스 접속 주소', () => {
    beforeEach(() => {
        window.api = {} as unknown as typeof window.api;
        try {
            localStorage.clear();
        } catch {
            // 없어도 된다
        }
    });

    it('공개 주소와 포트를 크게 보이고 복사한다', async () => {
        window.api.instanceAddress = vi.fn().mockResolvedValue({ address: 'mc.example.com', custom: true, running: true, ports: [port(25565)] });
        const writeText = vi.fn().mockResolvedValue(undefined);
        Object.assign(navigator, { clipboard: { writeText } });
        await act(async () => {
            render(<ServerAddress server={server} />);
        });
        expect(screen.getByText('mc.example.com:25565')).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /Hide/ })).toBeNull();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Copy/ })));
        expect(writeText).toHaveBeenCalledWith('mc.example.com:25565');
    });

    it('공인 IP도 가리기 없이 그대로 보인다 (사용자 11-12). 포트가 여럿이면 줄마다', async () => {
        localStorage.setItem('saba.hideServerIp', '1');
        window.api.instanceAddress = vi
            .fn()
            .mockResolvedValue({ address: '203.0.113.7', custom: false, running: false, ports: [port(16261), port(16262, 'UDP port')] });
        await act(async () => {
            render(<ServerAddress server={server} />);
        });
        expect(screen.getByText('203.0.113.7:16261')).toBeInTheDocument();
        expect(screen.getByText('UDP port')).toBeInTheDocument();
        expect(screen.getByText(/Start the server/)).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /Hide|Show/ })).toBeNull();
    });

    it('포트가 없거나 예전 데몬이면 그리지 않는다', async () => {
        window.api.instanceAddress = vi.fn().mockResolvedValue({ address: '203.0.113.7', ports: [] });
        let container: HTMLElement | undefined;
        await act(async () => {
            ({ container } = render(<ServerAddress server={server} />));
        });
        expect(container).toBeEmptyDOMElement();
    });
});
