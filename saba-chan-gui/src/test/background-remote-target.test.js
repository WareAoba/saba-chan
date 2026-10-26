/**
 * 원격 데몬을 고른 동안 배경 팝오버는 이 PC의 서비스가 어느 데몬에 중계하는지도 보인다 (T1 PC 확인 후속).
 */
import { render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import BackgroundModal from '../components/Modals/BackgroundModal';

beforeEach(() => {
    window.api.daemonStatus = vi.fn().mockResolvedValue({ running: true });
    window.api.remoteDaemons = vi.fn().mockResolvedValue({
        daemons: [
            { hostId: 'host-b', name: 'saba-cloud-test', address: '', port: 57480 },
            { hostId: 'host-c', name: '집 서버', address: 'g.example', port: 57480 },
        ],
    });
});

describe('배경 팝오버의 다루는 데몬', () => {
    it('원격 데몬을 고르면 그 이름과 릴레이를 거친다는 것을 보인다', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        render(<BackgroundModal isOpen onClose={vi.fn()} />);
        expect(await screen.findByText(i18n.t('gui:shell.daemon_target'))).toBeInTheDocument();
        expect(screen.getByText(`saba-cloud-test · ${i18n.t('gui:remote_daemon.via_relay')}`)).toBeInTheDocument();
    });

    it('주소가 있는 원격 데몬은 주소를 보인다', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-c' });
        render(<BackgroundModal isOpen onClose={vi.fn()} />);
        expect(await screen.findByText('집 서버 · g.example:57480')).toBeInTheDocument();
    });

    it('이 PC를 다루면 줄을 더하지 않는다', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
        render(<BackgroundModal isOpen onClose={vi.fn()} />);
        await screen.findByText(i18n.t('gui:shell.daemon_address'));
        expect(screen.queryByText(i18n.t('gui:shell.daemon_target'))).toBeNull();
    });
});
