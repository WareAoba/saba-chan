import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RuntimePreflight } from '../components/shell/RuntimePreflight';
import { getCachedRules, highlightLine } from '../utils/syntaxHighlight';

let n = 0;
const compile = (rules) => getCachedRules(`test-${n++}`, rules);
const styleAt = (segs, text) => segs.find((s) => s.text === text)?.style;

describe('콘솔 하이라이트 규칙 (C12)', () => {
    it('겹치면 priority 큰 규칙이 이기고, 같으면 먼저 선언된 규칙', () => {
        const rules = compile([
            { name: 'word', pattern: 'ERROR', color: 'info' },
            { name: 'line', pattern: 'ERROR here', color: 'error', priority: 5 },
            { name: 'later', pattern: 'here', color: 'success', priority: 5 },
        ]);
        const segs = highlightLine('ERROR here', rules);
        expect(segs).toHaveLength(1);
        expect(segs[0].style.color).toBe('#f38ba8');
    });

    it('배경·밑줄을 스타일로 낸다 (토큰 배경은 옅게)', () => {
        const rules = compile([
            { name: 'a', pattern: 'warn', color: 'warn', background: 'warn', underline: true },
            { name: 'b', pattern: 'hex', color: '#fff', background: '#123456' },
        ]);
        const segs = highlightLine('warn hex', rules);
        expect(styleAt(segs, 'warn')).toMatchObject({ backgroundColor: '#f9e2af33', textDecoration: 'underline' });
        expect(styleAt(segs, 'hex')).toMatchObject({ backgroundColor: '#123456' });
        expect(styleAt(segs, 'hex').textDecoration).toBeUndefined();
    });

    it('맨 앞 (?i) 단독 플래그는 JS 플래그로 바꿔 대소문자를 무시한다', () => {
        const rules = compile([{ name: 'i', pattern: '(?i)error', color: 'error' }]);
        expect(styleAt(highlightLine('Error', rules), 'Error')).toBeTruthy();
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
        await act(async () => render(<RuntimePreflight />));
        const chip = document.querySelector('.appbar-preflight');
        expect(chip.getAttribute('data-tooltip')).toContain('40%');
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
        await act(async () => render(<RuntimePreflight />));
        const chip = document.querySelector('.appbar-preflight.failed');
        expect(chip.getAttribute('data-tooltip')).toContain('pip timeout');
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(document.querySelector('.appbar-preflight')).toBeNull();
    });

    it('IPC가 없으면 아무것도 그리지 않는다', async () => {
        window.api = {};
        await act(async () => render(<RuntimePreflight />));
        expect(document.querySelector('.appbar-preflight')).toBeNull();
    });
});
