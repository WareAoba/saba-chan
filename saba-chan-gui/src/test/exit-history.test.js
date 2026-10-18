import { render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ExitHistory } from '../components/servers/ExitHistory';

describe('ExitHistory', () => {
    afterEach(() => {
        delete window.api.instanceEvents;
    });

    it('연결 함수가 없으면 아무것도 그리지 않는다', () => {
        render(<ExitHistory server={{ id: 'a', status: 'stopped' }} />);
        expect(document.querySelector('.exit-history')).toBeNull();
    });

    it('최신 기록이 위로 오고, 크래시는 빨간 칩·재시작 예정이 보인다', async () => {
        window.api.instanceEvents = vi.fn().mockResolvedValue({
            events: [
                { at: '2026-10-16T10:00:00Z', kind: 'user_stop', exit_code: 0 },
                { at: '2026-10-16T11:00:00Z', kind: 'crash', exit_code: 1, restart_in_secs: 10 },
            ],
        });
        render(<ExitHistory server={{ id: 'a', status: 'stopped' }} />);
        await waitFor(() => expect(document.querySelectorAll('.exit-history li')).toHaveLength(2));
        const first = document.querySelector('.exit-history li');
        expect(first.querySelector('.status-chip.tone-stop')).not.toBeNull();
        expect(first.querySelector('.exit-restart')).not.toBeNull();
        expect(window.api.instanceEvents).toHaveBeenCalledWith('a');
    });

    it('데몬이 남긴 재시작 안 함 이유를 현지화해서 보인다', async () => {
        window.api.instanceEvents = vi.fn().mockResolvedValue({
            events: [
                { at: '2026-10-16T09:00:00Z', kind: 'crash', note: 'auto_restart is off' },
                {
                    at: '2026-10-16T10:00:00Z',
                    kind: 'crash',
                    note: 'auto restart gave up after 5 restarts in 10 minutes',
                },
                { at: '2026-10-16T11:00:00Z', kind: 'crash', note: 'restart failed: port busy' },
            ],
        });
        render(<ExitHistory server={{ id: 'a', status: 'stopped' }} />);
        await waitFor(() => expect(document.querySelectorAll('.exit-history li')).toHaveLength(3));
        const text = document.querySelector('.exit-history').textContent;
        expect(text).toContain('Auto restart is off');
        expect(text).toContain('Gave up after 5 restarts in 10 minutes');
        expect(text).toContain("Couldn't restart: port busy");
    });

    it('note_code가 있으면 영어 문장 대신 코드로 문구를 고른다', async () => {
        window.api.instanceEvents = vi.fn().mockResolvedValue({
            events: [
                { at: '2026-10-16T09:00:00Z', kind: 'crash', note: 'x', note_code: 'auto_restart_off' },
                { at: '2026-10-16T10:00:00Z', kind: 'crash', note: 'y', note_code: 'gave_up', note_args: { count: 3 } },
                {
                    at: '2026-10-16T11:00:00Z',
                    kind: 'crash',
                    note: 'z',
                    note_code: 'restart_failed',
                    note_args: { reason: 'no java' },
                },
            ],
        });
        render(<ExitHistory server={{ id: 'a', status: 'stopped' }} />);
        await waitFor(() => expect(document.querySelectorAll('.exit-history li')).toHaveLength(3));
        const text = document.querySelector('.exit-history').textContent;
        expect(text).toContain('Auto restart is off');
        expect(text).toContain('Gave up after 3 restarts in 10 minutes');
        expect(text).toContain("Couldn't restart: no java");
    });

    it('모르는 note_code에 문장이 없으면 근거나 코드를 보인다', async () => {
        window.api.instanceEvents = vi.fn().mockResolvedValue({
            events: [{ at: '2026-10-16T09:00:00Z', kind: 'crash', note_code: 'future_code' }],
        });
        render(<ExitHistory server={{ id: 'a', status: 'stopped' }} />);
        await waitFor(() => expect(document.querySelector('.exit-history').textContent).toContain('future_code'));
    });
});
