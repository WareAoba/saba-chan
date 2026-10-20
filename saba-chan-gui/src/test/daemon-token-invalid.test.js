import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import BackgroundModal from '../components/Modals/BackgroundModal';
import { daemonStateOf } from '../components/servers/DaemonDown';
import { ServersPage } from '../components/servers/ServersPage';
import { ConnectionRail } from '../components/shell/ConnectionRail';
import { useUIStore } from '../stores/useUIStore';

// 데몬은 떠 있지만 이 앱의 토큰을 받지 않을 때(token_invalid): 다시 시작해도 안 되므로 "꺼짐"·재시작 대신 따로 알린다 (cloud-review-333 P3)
const TOKEN_TEXT = /doesn't accept this app's connection key/;

describe('데몬이 이 앱의 토큰을 받지 않을 때', () => {
    afterEach(() => useUIStore.setState({ backgroundDaemonStatus: 'checking' }));

    it('daemonStatus 답을 token_invalid 상태로 읽는다', () => {
        expect(daemonStateOf({ running: true })).toBe('running');
        expect(daemonStateOf({ running: false, error_code: 'token_invalid' })).toBe('token_invalid');
        expect(daemonStateOf({ running: false })).toBe('stopped');
    });

    it('상단 연결 표시가 꺼짐이 아니라 토큰 문제라고 보인다', () => {
        render(<ConnectionRail daemonStatus="token_invalid" botStatus="running" />);
        const node = document.querySelector('.conn-node');
        expect(node.classList.contains('tone-stop')).toBe(true);
        expect(node.textContent).not.toMatch(/Daemon stopped/);
        expect(node.getAttribute('title')).toMatch(TOKEN_TEXT);
    });

    it('서버 화면은 깨우기 버튼 없이 이유를 알린다', () => {
        useUIStore.setState({ backgroundDaemonStatus: 'token_invalid' });
        render(<ServersPage servers={[]} modules={[]} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />);
        expect(screen.getByText(TOKEN_TEXT)).toBeTruthy();
        expect(screen.queryByRole('button', { name: /Wake up/ })).toBeNull();
    });

    it('데몬 창은 다시 시작 버튼 없이 이유를 알린다', async () => {
        window.api.daemonStatus = vi.fn().mockResolvedValue({ running: false, error_code: 'token_invalid' });
        await act(async () => {
            render(<BackgroundModal isOpen onClose={vi.fn()} />);
        });
        expect(screen.getByText(TOKEN_TEXT)).toBeTruthy();
        expect(screen.queryByRole('button', { name: /Restart/ })).toBeNull();
    });
});
