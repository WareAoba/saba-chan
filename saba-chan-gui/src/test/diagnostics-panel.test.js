import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DiagnosticsPanel } from '../components/DiagnosticsPanel';

const diag = {
    ok: true,
    version: '0.1.5',
    pid: 4242,
    uptime_secs: 3725,
    listen_addr: '127.0.0.1:57474',
    data_dir: 'C:/Users/me/AppData/Roaming/saba-chan',
    token_file: { path: 'x/.ipc_token', exists: true },
    extensions: { enabled: ['steamcmd'], enabled_but_missing: ['gone'] },
    modules: { count: 3 },
    recent_problems: [{ id: 1, timestamp: 1760767200000, level: 'warn', target: 'saba', message: 'disk slow' }],
};

describe('진단 정보 (/api/diagnostics)', () => {
    beforeEach(() => {
        window.api = {};
    });

    it('채널이 없으면 그리지 않는다', () => {
        const { container } = render(<DiagnosticsPanel />);
        expect(container).toBeEmptyDOMElement();
    });

    it('불러오면 버전·가동 시간·빠진 익스텐션·최근 문제를 보이고 복사할 수 있다', async () => {
        window.api.daemonDiagnostics = vi.fn().mockResolvedValue(diag);
        const writeText = vi.fn().mockResolvedValue();
        Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
        render(<DiagnosticsPanel />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Show diagnostics/i })));
        expect(screen.getByText('0.1.5')).toBeInTheDocument();
        expect(screen.getByText('1h 2m')).toBeInTheDocument();
        expect(screen.getByText(/gone/)).toBeInTheDocument();
        expect(screen.getByText(/disk slow/)).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Copy/i })));
        expect(writeText.mock.calls[0][0]).toContain('"version": "0.1.5"');
    });
});
