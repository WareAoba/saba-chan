import { act, fireEvent, render, screen } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CommandModal from '../components/Modals/CommandModal';
import { useUIStore } from '../stores/useUIStore';

// 서버 상세의 "명령" 탭: 지금 디자인 부품(목록 항목·입력 칸·기록 줄)을 쓰고 키보드로 다 쓸 수 있다
const server = { id: 'd1', name: 'D', module: 'mc', status: 'running' };
const modules = [
    {
        name: 'mc',
        commands: {
            fields: [
                { name: 'say', label: '말하기', description: '전체 메시지' },
                { name: 'stop', label: '정지', risk: 'danger' },
            ],
        },
    },
];

afterEach(() => {
    useUIStore.setState({ commandHistoryMap: {} });
    delete window.api.executeCommand;
});

describe('명령 탭 디자인', () => {
    it('명령 목록은 고를 수 있는 항목이고 하이라이트가 aria-selected로 드러난다', () => {
        render(<CommandModal server={server} modules={modules} />);
        const input = screen.getByRole('textbox');
        fireEvent.change(input, { target: { value: 's' } });
        const options = screen.getAllByRole('option');
        expect(options.map((o) => o.getAttribute('aria-selected'))).toEqual(['true', 'false']);
        fireEvent.click(options[1]);
        expect(input.value).toBe('stop ');
    });

    it('기록 줄은 버튼이라 키보드로 펼친다', async () => {
        window.api.executeCommand = vi.fn().mockResolvedValue({ data: { response: 'hi there' } });
        render(<CommandModal server={server} modules={modules} />);
        const input = screen.getByRole('textbox');
        fireEvent.change(input, { target: { value: 'say hi' } });
        await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
        const row = screen.getByRole('button', { name: /say hi/ });
        expect(row.getAttribute('aria-expanded')).toBe('false');
        fireEvent.click(row);
        expect(row.getAttribute('aria-expanded')).toBe('true');
    });

    it('명령 탭 CSS는 옛 글꼴·그라데이션 없이 토큰만 쓴다', () => {
        const css = readFileSync(join(__dirname, '../styles/shell.css'), 'utf8');
        const block = css.slice(css.indexOf('/* ── 명령 탭'), css.indexOf('/* ── 명령 탭 끝'));
        expect(block.length).toBeGreaterThan(100);
        expect(block).not.toMatch(/Courier New|-gradient\(|--brand-gradient|#[0-9a-f]{3,6}\b/i);
    });
});
