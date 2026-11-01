/**
 * 연결 카드 흐름: 릴레이에 닿지 못하면 끊긴 자리만 빨간 X로, 재우기·연결 중이면 이음줄 없이 보인다.
 */
import { fireEvent, render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
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
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
};

const links = (container) => [...container.querySelectorAll('.dc-chain-link')].map((el) => el.className);

describe('연결 카드 흐름', () => {
    it('모두 연결되면 두 이음줄이 켜진다', () => {
        const { container } = render(<ConnectionView {...viewProps} />);
        expect(links(container)).toEqual(['dc-chain-link on', 'dc-chain-link on']);
        expect(container.querySelector('.dc-chain-x')).toBeNull();
    });

    it('릴레이에 닿지 못하면 사바쨩-릴레이 사이만 빨간 X로 끊긴다', () => {
        const { container } = render(<ConnectionView {...viewProps} cloudState="error" cloudError="timeout" />);
        expect(links(container)).toEqual(['dc-chain-link fail', 'dc-chain-link off']);
        expect(container.querySelectorAll('.dc-chain-x')).toHaveLength(1);
        expect(container.querySelector('.dc-chain-node.fail')).not.toBeNull();
    });

    it('연결 중이면 사바쨩 상자도 켜지지 않고 끊김 표시도 없다', () => {
        const { container } = render(<ConnectionView {...viewProps} cloudState="connecting" />);
        expect(links(container)).toEqual(['dc-chain-link off', 'dc-chain-link off']);
        expect(container.querySelector('.dc-chain-node.on')).toBeNull();
        expect(container.querySelector('.dc-chain-x')).toBeNull();
    });

    it('가운데 상자는 서버쨩 얼굴로 보인다', () => {
        const { container } = render(<ConnectionView {...viewProps} />);
        const relayNode = container.querySelectorAll('.dc-chain-node')[1];
        expect(relayNode.querySelector('img.dc-chain-icon').getAttribute('src')).toBe('./server-chan.webp');
    });

    it('릴레이에 닿지 못했다는 알림에서 바로 다시 받을 수 있다', () => {
        const onReload = vi.fn();
        const { container } = render(
            <ConnectionView {...viewProps} cloudState="error" cloudError="timeout" onReload={onReload} />,
        );
        fireEvent.click(container.querySelector('.notice.tone-stop button'));
        expect(onReload).toHaveBeenCalledTimes(1);
    });

    it('모든 서버를 재우면 사바쨩 상자와 이음줄이 꺼진다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ sleeping: true });
        const { container } = render(<ConnectionView {...viewProps} nodes={[{ guildId: 'g1' }]} />);
        await waitFor(() => expect(links(container)).toEqual(['dc-chain-link off', 'dc-chain-link on']));
        expect(container.querySelectorAll('.dc-chain-node')[0].classList.contains('on')).toBe(false);
        delete window.api.discordSettingsGet;
    });
});
