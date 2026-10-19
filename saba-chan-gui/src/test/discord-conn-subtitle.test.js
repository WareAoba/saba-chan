/**
 * 연결 카드 부제는 지금 상태를 그대로 말한다:
 * 명령 전달이 꺼져 있으면 "연결됐어요"라고 하지 않고, 사바쨩 서버 오류는 아래 알림과 같은 말을 한다.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ConnectionView } from '../components/discord/ConnectionView';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'https://relay',
    nodeCount: 1,
    botStatus: 'running',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onReload: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
};

describe('연결 카드 부제', () => {
    it('명령 전달이 켜져 있으면 연결됐다고 한다', () => {
        render(<ConnectionView {...viewProps} />);
        expect(screen.getByText('Successfully connected to Discord')).toBeInTheDocument();
    });

    it('명령 전달이 꺼져 있으면 연결됐다고만 하지 않는다', () => {
        render(<ConnectionView {...viewProps} botStatus="stopped" />);
        expect(screen.queryByText('Successfully connected to Discord')).toBeNull();
        expect(screen.getByText(/command relay is off/i)).toBeInTheDocument();
    });

    it('사바쨩 서버에 닿지 못하면 디스코드 탓으로 말하지 않는다', () => {
        render(<ConnectionView {...viewProps} cloudState="error" cloudError="timeout" />);
        expect(screen.queryByText("Couldn't reach Discord")).toBeNull();
        expect(screen.getByText("Couldn't reach the Saba-chan server")).toBeInTheDocument();
    });
});
