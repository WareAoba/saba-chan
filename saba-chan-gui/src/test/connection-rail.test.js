import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ConnectionRail } from '../components/shell/ConnectionRail';

const nodes = () => document.querySelectorAll('.conn-node');

describe('ConnectionRail', () => {
    it('데몬이 꺼지면 봇 구간을 숨기고 데몬 상태만 보여준다', () => {
        render(<ConnectionRail daemonStatus="stopped" botStatus="running" />);
        expect(nodes()).toHaveLength(1);
        expect(nodes()[0].classList.contains('tone-stop')).toBe(true);
        expect(document.querySelector('.conn-link')).toBeNull();
    });

    it('데몬이 켜져 있으면 봇 상태와 연결선을 보여준다', () => {
        render(<ConnectionRail daemonStatus="running" botStatus="connecting" />);
        expect(nodes()).toHaveLength(2);
        expect(nodes()[0].classList.contains('tone-ok')).toBe(true);
        expect(nodes()[1].classList.contains('tone-warn')).toBe(true);
        expect(document.querySelector('.conn-link.link-connecting')).not.toBeNull();
    });

    it('각 구간을 누르면 해당 동작을 부른다', () => {
        const onDaemonClick = vi.fn();
        const onBotClick = vi.fn();
        render(
            <ConnectionRail
                daemonStatus="running"
                botStatus="stopped"
                onDaemonClick={onDaemonClick}
                onBotClick={onBotClick}
            />,
        );
        fireEvent.click(nodes()[0]);
        fireEvent.click(nodes()[1]);
        expect(onDaemonClick).toHaveBeenCalledTimes(1);
        expect(onBotClick).toHaveBeenCalledTimes(1);
    });
});
