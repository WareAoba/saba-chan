import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ServersPage } from '../components/servers/ServersPage';
import { useSabaBubble } from '../stores/useSabaBubble';
import { useUIStore } from '../stores/useUIStore';

const page = (servers) => (
    <ServersPage servers={servers} modules={[]} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />
);

describe('데몬이 꺼져 있을 때 서버 화면', () => {
    afterEach(() => useUIStore.setState({ backgroundDaemonStatus: 'checking' }));

    it('서버가 비어 있으면 "서버가 없어요" 대신 데몬 연결 안내를 보인다', () => {
        useUIStore.setState({ backgroundDaemonStatus: 'stopped' });
        render(page([]));
        expect(screen.queryByText(/No servers yet|아직 서버가/)).toBeNull();
        // 잠든 사바쨩 + 깨우기
        expect(screen.getByText("Shh! She's still asleep…!")).toBeTruthy();
    });

    it('다시 시작 버튼이 데몬을 다시 띄운다', async () => {
        useUIStore.setState({ backgroundDaemonStatus: 'error' });
        window.api.daemonRestart = vi.fn().mockResolvedValue({ success: true });
        render(page([]));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: /Wake up/ }));
        });
        expect(window.api.daemonRestart).toHaveBeenCalled();
    });

    it('깨우기에 실패하면 깨운 표시를 남기지 않는다 (뒤늦은 졸린 사바쨩이 뜨지 않게)', async () => {
        useUIStore.setState({ backgroundDaemonStatus: 'stopped' });
        window.api.daemonRestart = vi.fn().mockResolvedValue({ success: false, error: 'no' });
        render(page([]));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: /Wake up/ }));
        });
        expect(useSabaBubble.getState().wokeAt).toBe(0);
    });

    it('데몬이 살아 있으면 원래 빈 화면이다', () => {
        useUIStore.setState({ backgroundDaemonStatus: 'running' });
        render(page([]));
        expect(screen.getByText('Nothing added yet')).toBeTruthy();
    });
});
