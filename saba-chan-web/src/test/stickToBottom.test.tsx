import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useStickToBottom } from '../components/useStickToBottom';

function Box({ lines }: { lines: string[] }) {
    const box = useStickToBottom<HTMLPreElement>(lines);
    return (
        <pre data-testid="box" ref={box.ref} onScroll={box.onScroll}>
            {lines.join('\n')}
        </pre>
    );
}

/** jsdom 은 크기를 재지 않는다 — 상자 높이 100, 내용 높이는 줄 수 × 20 */
function sized(el: HTMLElement, lines: number) {
    Object.defineProperty(el, 'clientHeight', { configurable: true, value: 100 });
    Object.defineProperty(el, 'scrollHeight', { configurable: true, value: lines * 20 });
}

describe('콘솔 따라가기', () => {
    it('맨 아래에 있으면 새 줄을 따라 내려가고, 위로 올려 읽는 중이면 내리지 않는다', () => {
        const lines = Array.from({ length: 20 }, (_, i) => `l${i}`);
        const { getByTestId, rerender } = render(<Box lines={lines} />);
        const el = getByTestId('box');
        sized(el, 21);
        rerender(<Box lines={[...lines, 'a']} />);
        expect(el.scrollTop).toBe(420);

        // 위로 올려 읽는다
        el.scrollTop = 100;
        fireEvent.scroll(el);
        sized(el, 22);
        rerender(<Box lines={[...lines, 'a', 'b']} />);
        expect(el.scrollTop).toBe(100);

        // 다시 맨 아래까지 내리면 따라간다
        el.scrollTop = 440 - 100;
        fireEvent.scroll(el);
        sized(el, 23);
        rerender(<Box lines={[...lines, 'a', 'b', 'c']} />);
        expect(el.scrollTop).toBe(460);
    });
});
