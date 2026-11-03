import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ServerLogs } from '../components/servers/ServerLogs';
import { pickOption } from './dropdown';

const flush = () => act(async () => {});
const SOURCES = [
    { id: 'r2', label: '11-03 20:12' },
    { id: 'r1', label: '11-02 19:00' },
];

describe('ServerLogs (지난 로그)', () => {
    afterEach(() => {
        delete window.api.instanceLogs;
        delete window.api.instanceLogsSave;
    });

    it('연결 함수가 없으면 그리지 않는다', () => {
        render(<ServerLogs server={{ id: 'a' }} />);
        expect(document.querySelector('.server-logs')).toBeNull();
    });

    it('끝에서 읽고, 더 보기는 before 로 앞을 붙이고, 내보내기는 저장 창으로 데몬이 쓴다', async () => {
        window.api.instanceLogs = vi
            .fn()
            .mockResolvedValueOnce({ success: true, sources: SOURCES, source: 'r2', lines: ['c'], before: 5 })
            .mockResolvedValueOnce({ success: true, sources: SOURCES, source: 'r2', lines: ['a', 'b'] });
        window.api.instanceLogsSave = vi.fn().mockResolvedValue({ success: true, path: '/x.log', truncated: false });
        render(<ServerLogs server={{ id: 'a', name: 'Mc' }} />);
        await flush();
        expect(window.api.instanceLogs).toHaveBeenCalledWith('a', { source: undefined, lines: 200 });
        expect(document.querySelector('.server-logs pre').textContent).toBe('c');
        fireEvent.click(document.querySelector('.server-logs-more'));
        await flush();
        expect(window.api.instanceLogs).toHaveBeenLastCalledWith('a', { source: 'r2', lines: 200, before: 5 });
        expect(document.querySelector('.server-logs pre').textContent).toBe('a\nb\nc');
        expect(document.querySelector('.server-logs-more')).toBeNull();
        fireEvent.click(document.querySelector('.server-logs-export'));
        await flush();
        expect(window.api.instanceLogsSave).toHaveBeenCalledWith('a', { source: 'r2', defaultName: 'Mc-r2.log' });
    });

    it('실행분을 고르면 그 실행분을 읽는다', async () => {
        window.api.instanceLogs = vi.fn(async (_id, { source }) => ({
            success: true,
            sources: SOURCES,
            source: source ?? 'r2',
            lines: [source ?? 'r2'],
        }));
        render(<ServerLogs server={{ id: 'a', name: 'Mc' }} />);
        await flush();
        pickOption(document.querySelector('.server-logs [role="combobox"]'), '11-02 19:00');
        await flush();
        expect(document.querySelector('.server-logs pre').textContent).toBe('r1');
    });
});
