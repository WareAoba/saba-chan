import fs from 'node:fs';
import path from 'node:path';
import { act, render } from '@testing-library/react';
import { useRef } from 'react';
import { describe, expect, it } from 'vitest';
import { useStickyClip } from '../hooks/useStickyClip';

const SRC = path.resolve(__dirname, '..');

function cssFiles(dir: string): string[] {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((d) => {
        const p = path.join(dir, d.name);
        if (d.isDirectory()) return cssFiles(p);
        return d.name.endsWith('.css') ? [p] : [];
    });
}

/** 창 바탕색(--bg-0)을 칠하는 규칙의 선택자 */
function windowBgSelectors(): string[] {
    const out: string[] = [];
    for (const file of cssFiles(SRC)) {
        const css = fs.readFileSync(file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
        for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
            if (!/background(-color)?:\s*var\(--bg-0\)\s*;/.test(m[2])) continue;
            for (const sel of m[1].split(',')) out.push(sel.trim());
        }
    }
    return out;
}

// 작은 칩처럼 창 바탕 위의 작은 조각은 칠해도 된다.
// 로딩 화면은 링 안쪽 마스크와 같은 색이어야 하고, 오류 화면은 일부러 가린다
const SMALL = new Set(['html', 'body', '.dc-cmd-chip', '.loading-screen', '.glow-mask', '.error-fallback']);

describe('window backdrop coverage', () => {
    it('every large surface painted with the window colour is cleared when the backdrop is on', () => {
        const backdrop = fs.readFileSync(path.join(SRC, 'styles/backdrop.css'), 'utf8');
        const missing = windowBgSelectors().filter(
            (sel) => !SMALL.has(sel) && !backdrop.includes(`:root[data-backdrop="on"] ${sel}`),
        );
        expect(missing).toEqual([]);
    });
});

function Sticky({ onScroller }: { onScroller: (el: HTMLDivElement) => void }) {
    const topRef = useRef<HTMLDivElement>(null);
    useStickyClip(topRef);
    return (
        <div ref={onScroller} style={{ overflowY: 'auto' }} data-testid="scroller">
            <div>
                <div ref={topRef} data-testid="top" />
                <p data-testid="a">a</p>
                <p data-testid="b">b</p>
            </div>
        </div>
    );
}

const rect = (top: number, bottom: number) => () => ({
    top,
    bottom,
    left: 0,
    right: 0,
    width: 0,
    height: bottom - top,
    x: 0,
    y: top,
    toJSON() {},
});

describe('useStickyClip', () => {
    it('tells each block under the stuck head how far the head covers it', () => {
        let scroller!: HTMLDivElement;
        const { getByTestId } = render(
            <Sticky
                onScroller={(el) => {
                    if (el) scroller = el;
                }}
            />,
        );
        getByTestId('top').getBoundingClientRect = rect(0, 120);
        getByTestId('a').getBoundingClientRect = rect(80, 300);
        getByTestId('b').getBoundingClientRect = rect(316, 500);
        act(() => {
            scroller.dispatchEvent(new Event('scroll'));
        });
        expect(getByTestId('a').style.getPropertyValue('--sticky-clip')).toBe('40px');
        expect(getByTestId('b').style.getPropertyValue('--sticky-clip')).toBe('-196px');
        expect(getByTestId('top').style.getPropertyValue('--sticky-clip')).toBe('');
    });
});
