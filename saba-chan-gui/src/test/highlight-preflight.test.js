import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HighlightedContent } from '../components/ConsoleView';
import { RuntimePreflight } from '../components/shell/RuntimePreflight';
import { getCachedRules, highlightLine, spanSegments } from '../utils/syntaxHighlight';

let n = 0;
const compile = (rules) => getCachedRules(`test-${n++}`, rules);
const styleAt = (segs, text) => segs.find((s) => s.text === text)?.style;
const tokenAt = (segs, text) => segs.find((s) => s.text === text)?.token;

describe('콘솔 하이라이트 규칙 (C12)', () => {
    it('겹치면 priority 큰 규칙이 이기고, 같으면 먼저 선언된 규칙', () => {
        const rules = compile([
            { name: 'word', pattern: 'ERROR', color: 'info' },
            { name: 'line', pattern: 'ERROR here', color: 'error', priority: 5 },
            { name: 'later', pattern: 'here', color: 'success', priority: 5 },
        ]);
        const segs = highlightLine('ERROR here', rules);
        expect(segs).toHaveLength(1);
        // 색은 테마가 정한다: 토큰만 넘기고 인라인 색은 없다
        expect(segs[0].token).toBe('error');
        expect(segs[0].style).toBeNull();
    });

    it('배경·밑줄을 스타일로 낸다 (토큰 배경은 옅게)', () => {
        const rules = compile([
            { name: 'a', pattern: 'warn', color: 'warn', background: 'warn', underline: true },
            { name: 'b', pattern: 'hex', color: '#fff', background: '#123456' },
        ]);
        const segs = highlightLine('warn hex', rules);
        expect(tokenAt(segs, 'warn')).toBe('warn');
        expect(styleAt(segs, 'warn')).toMatchObject({
            backgroundColor: 'color-mix(in srgb, var(--tok-warn) 20%, transparent)',
            textDecoration: 'underline',
        });
        // 옛 모듈의 #hex는 그대로 칠한다
        expect(styleAt(segs, 'hex')).toMatchObject({ color: '#fff', backgroundColor: '#123456' });
        expect(styleAt(segs, 'hex').textDecoration).toBeUndefined();
    });

    it('맨 앞 (?i) 단독 플래그는 JS 플래그로 바꿔 대소문자를 무시한다', () => {
        const rules = compile([{ name: 'i', pattern: '(?i)error', color: 'error' }]);
        expect(tokenAt(highlightLine('Error', rules), 'Error')).toBe('error');
    });
});

describe('콘솔 줄 그리기', () => {
    it('한 줄 전체가 한 규칙에 걸려도 스타일을 입힌다', () => {
        const rules = compile([{ name: 'all', pattern: 'ERROR here', color: 'error', underline: true }]);
        render(<HighlightedContent text="ERROR here" compiledRules={rules} />);
        const span = [...document.querySelectorAll('span')].find((e) => e.textContent === 'ERROR here');
        expect(span.style.textDecoration).toBe('underline');
        expect(span.className).toBe('tok-error');
    });

    it('데몬 spans가 있으면 규칙 대신 그것으로 칠한다 (UTF-16 오프셋, 모르는 토큰은 건너뜀)', () => {
        const rules = compile([{ name: 'all', pattern: '.+', color: 'error' }]);
        const text = '[12:00] 스티브 joined';
        render(
            <HighlightedContent
                text={text}
                spans={[
                    [0, 7, 'timestamp'],
                    [8, 11, 'player'],
                    [12, 18, 'nope'],
                ]}
                compiledRules={rules}
            />,
        );
        const spans = [...document.querySelectorAll('span')];
        expect(spans.map((e) => [e.className, e.textContent])).toEqual([
            ['tok-timestamp', '[12:00]'],
            ['tok-player', '스티브'],
        ]);
        expect(document.body.textContent).toContain(text);
    });

    it('빈 spans면 칠하지 않고, 범위를 넘는 구간은 잘라 낸다', () => {
        expect(spanSegments('abc', [])).toEqual([{ text: 'abc', token: null, style: null }]);
        expect(spanSegments('abc', [[1, 99, 'warning']]).map((s) => [s.text, s.token])).toEqual([
            ['a', null],
            ['bc', 'warn'],
        ]);
    });
});

describe('실행 환경 준비 표시 (C5)', () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it('준비 중엔 진행 고리, 끝나면 사라지고 더 묻지 않는다', async () => {
        const steps = [
            { phase: 'preflight-python', percent: 40, complete: false },
            { phase: 'preflight-done', percent: 100, complete: true },
        ];
        window.api = { runtimePreflight: vi.fn(() => Promise.resolve(steps.shift())) };
        await act(async () => render(<RuntimePreflight daemonStatus="running" />));
        const chip = document.querySelector('.appbar-preflight');
        expect(chip.getAttribute('data-tooltip')).toContain('40%');
        expect(chip.textContent).toContain('40%');
        await act(async () => vi.advanceTimersByTimeAsync(1000));
        expect(document.querySelector('.appbar-preflight')).toBeNull();
        await act(async () => vi.advanceTimersByTimeAsync(10000));
        expect(window.api.runtimePreflight).toHaveBeenCalledTimes(2);
    });

    it('실패하면 경고로 남고, 다시 시도해 성공하면 사라진다', async () => {
        const steps = [
            { phase: 'preflight-failed', percent: 0, complete: true, error: 'pip timeout' },
            { phase: 'preflight-done', percent: 100, complete: true },
        ];
        window.api = { runtimePreflight: vi.fn(() => Promise.resolve(steps.shift())) };
        await act(async () => render(<RuntimePreflight daemonStatus="running" />));
        const chip = document.querySelector('.appbar-preflight.failed');
        expect(chip.getAttribute('data-tooltip')).toContain('pip timeout');
        // 키보드로 닿고, 누르면 이유 전문을 알림으로 보여 준다
        expect(chip.tagName).toBe('BUTTON');
        window.showToast = vi.fn();
        act(() => chip.click());
        expect(window.showToast).toHaveBeenCalledWith(expect.stringContaining('pip timeout'), 'error', 8000, undefined);
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(document.querySelector('.appbar-preflight')).toBeNull();
    });

    it('데몬이 다시 켜지면 처음부터 다시 묻는다', async () => {
        const steps = [
            { phase: 'preflight-done', percent: 100, complete: true },
            { phase: 'preflight-python', percent: 10, complete: false },
        ];
        window.api = { runtimePreflight: vi.fn(() => Promise.resolve(steps.shift())) };
        const view = await act(async () => render(<RuntimePreflight daemonStatus="running" />));
        expect(document.querySelector('.appbar-preflight')).toBeNull();
        await act(async () => view.rerender(<RuntimePreflight daemonStatus="stopped" />));
        await act(async () => view.rerender(<RuntimePreflight daemonStatus="running" />));
        expect(document.querySelector('.appbar-preflight').getAttribute('data-tooltip')).toContain('10%');
    });

    it('IPC가 없으면 아무것도 그리지 않는다', async () => {
        window.api = {};
        await act(async () => render(<RuntimePreflight daemonStatus="running" />));
        expect(document.querySelector('.appbar-preflight')).toBeNull();
    });
});

describe('콘솔 바탕 밝기 → 토큰 색 묶음', () => {
    it('밝은 바탕을 고르면 data-console-tone=light, 어두우면 지운다', async () => {
        const { applyCustomTheme, THEME_DEFAULTS } = await import('../utils/themeManager');
        applyCustomTheme({ ...THEME_DEFAULTS, consoleBgColor: '#f5f5f5' });
        expect(document.documentElement.getAttribute('data-console-tone')).toBe('light');
        applyCustomTheme({ ...THEME_DEFAULTS, consoleBgColor: '#1e1e2e' });
        expect(document.documentElement.hasAttribute('data-console-tone')).toBe(false);
    });
});
