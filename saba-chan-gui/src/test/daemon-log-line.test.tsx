import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { DaemonLogLine } from '../components/DaemonLogLine';

afterEach(cleanup);

// 시각 칸 너비가 줄마다 달라지면 본문 세로줄이 어긋난다 (사용자 2026-11-12 화면 확인)
describe('데몬 로그 한 줄', () => {
    it('시각은 언어와 관계없이 두 자리씩이라 줄마다 너비가 같다', () => {
        const at = (s: number) => new Date(2026, 10, 12, 9, 5, s).toISOString();
        for (const locale of ['ko', 'en', 'ja']) {
            const widths = [0, 37].map((s) => {
                const { container } = render(
                    <DaemonLogLine line={{ id: s, level: 'info', timestamp: at(s), message: 'x' }} locale={locale} />,
                );
                const text = container.querySelector('.daemon-log-time')?.textContent ?? '';
                cleanup();
                return text.length;
            });
            expect(widths[0]).toBe(widths[1]);
        }
    });
});
