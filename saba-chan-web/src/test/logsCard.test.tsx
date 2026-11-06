import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { LogsCard } from '../components/LogsCard';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

const flush = () => act(async () => {});
const SOURCES = [
    { id: '20261103-111200', label: '11-03 20:12', startedAt: 1 },
    { id: '20261102-100000', label: '11-02 19:00', startedAt: 0 },
];

describe('지난 로그', () => {
    beforeAll(() => initI18n('en'));

    it('가장 최근 실행분의 끝을 보이고, [더 보기]는 before 커서로 앞을 붙인다', async () => {
        const api = {
            instanceLogs: vi
                .fn()
                .mockResolvedValueOnce({
                    success: true,
                    sources: SOURCES,
                    source: SOURCES[0].id,
                    lines: ['c', 'd'],
                    before: 10,
                })
                .mockResolvedValueOnce({ success: true, sources: SOURCES, source: SOURCES[0].id, lines: ['a', 'b'] }),
        };
        render(<LogsCard api={mockApi(api)} server={{ id: 's1', name: 'Mc' }} />);
        await flush();
        expect(api.instanceLogs).toHaveBeenCalledWith('s1', { source: undefined, lines: 200 });
        expect(screen.getByText(/c\s*d/)).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: 'Load more' }));
        await flush();
        expect(api.instanceLogs).toHaveBeenLastCalledWith('s1', { source: SOURCES[0].id, lines: 200, before: 10 });
        expect(screen.getByText(/a\s*b\s*c\s*d/)).toBeTruthy();
        // 더 앞이 없으면 버튼이 사라진다
        expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull();
    });

    it('다른 실행분을 고르면 그 실행분을 읽고, 내보내기는 받은 내용을 파일로 내려받는다', async () => {
        const api = {
            instanceLogs: vi.fn(async (_id, { source }) => ({
                success: true,
                sources: SOURCES,
                source: source ?? SOURCES[0].id,
                lines: ['x'],
            })),
            instanceLogsExport: vi
                .fn()
                .mockResolvedValue({ success: true, fileName: 'Mc-1.log', content: 'x\n', truncated: false }),
        };
        const created: Blob[] = [];
        URL.createObjectURL = vi.fn((blob) => {
            created.push(blob);
            return 'blob:1';
        });
        URL.revokeObjectURL = vi.fn();
        const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
        render(<LogsCard api={mockApi(api)} server={{ id: 's1', name: 'Mc' }} />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: /11-02 19:00/ }));
        await flush();
        expect(api.instanceLogs).toHaveBeenLastCalledWith('s1', { source: SOURCES[1].id, lines: 200 });
        fireEvent.click(screen.getByRole('button', { name: 'Export' }));
        await flush();
        expect(api.instanceLogsExport).toHaveBeenCalledWith('s1', { source: SOURCES[1].id });
        expect(click).toHaveBeenCalled();
        const text = await new Promise((resolve) => {
            const r = new FileReader();
            r.onload = () => resolve(r.result);
            r.readAsText(created[0]);
        });
        expect(text).toBe('x\n');
        click.mockRestore();
    });

    it('한 번도 켜지 않은 서버는 빈 안내만', async () => {
        const api = {
            instanceLogs: vi.fn().mockResolvedValue({ success: true, sources: [], source: null, lines: [] }),
        };
        render(<LogsCard api={mockApi(api)} server={{ id: 's1', name: 'Mc' }} />);
        await flush();
        expect(screen.getByText('No logs yet. They appear after the server runs once.')).toBeTruthy();
        expect(screen.queryByRole('button', { name: 'Export' })).toBeNull();
    });
});
