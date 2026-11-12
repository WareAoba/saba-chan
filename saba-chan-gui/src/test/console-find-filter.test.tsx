import { useRef } from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { PageFind } from '../components/servers/PageFind';
import { ServerConsole } from '../components/servers/ServerConsole';
import type { ConsoleEntry } from '../stores/useConsoleStore';
import type { Server } from '../stores/useServerStore';

// 콘솔 탭: 수준 칩(전체·경고·오류), 위로 올려 읽는 동안의 "맨 아래로", Ctrl+F 줄 찾기 (사용자 11-12 시안 A)
const LINES = [
    { key: 1, id: 1, content: 'Preparing spawn area', level: 'INFO' },
    { key: 2, id: 2, content: "Can't keep up! Running 2031ms behind", level: 'WARN' },
    { key: 3, id: 3, content: "Couldn't load chunk", level: 'ERROR' },
    { key: 4, id: 4, content: "Can't keep up! Running 5210ms behind", level: 'WARN' },
];
const more = (n: number) => Array.from({ length: n }, (_, i) => ({ key: 100 + i, id: 100 + i, content: `tick ${i}`, level: 'INFO' }));

const Console = ({ id = 'a', lines = LINES }: { id?: string; lines?: unknown[] }) => (
    <ServerConsole
        server={{ id, name: 'A', status: 'running' } as Server}
        state={{ lines, input: '' } as ConsoleEntry}
        quickCommands={[]}
        openConsole={() => {}}
        setConsoleInput={() => {}}
        sendConsoleCommand={() => {}}
    />
);
const shownLines = () => Array.from(document.querySelectorAll('.console-line')).map((l) => l.textContent);
const chips = () => within(screen.getByRole('group', { name: 'Show by level' }));

describe('콘솔 수준 필터', () => {
    it('칩에 경고·오류 줄 수를 보이고, 켠 수준의 줄만 남기며 가린 줄 수를 알린다', () => {
        render(<Console />);
        expect(chips().getByRole('button', { name: 'All' }).getAttribute('aria-pressed')).toBe('true');
        fireEvent.click(chips().getByRole('button', { name: 'Warnings 2' }));
        expect(shownLines()).toEqual([LINES[1].content, LINES[3].content]);
        expect(screen.getByText('Other lines hidden: 2')).toBeTruthy();
        fireEvent.click(chips().getByRole('button', { name: 'Errors 1' }));
        expect(shownLines()).toHaveLength(3);
        expect(screen.getByText('Other lines hidden: 1')).toBeTruthy();
        fireEvent.click(chips().getByRole('button', { name: 'All' }));
        expect(shownLines()).toHaveLength(4);
        expect(screen.queryByText(/hidden:/)).toBeNull();
    });

    it('탭을 떠났다 돌아와도 그 서버의 필터는 남고, 다른 서버는 전체다', () => {
        const first = render(<Console id="keep" />);
        fireEvent.click(chips().getByRole('button', { name: 'Errors 1' }));
        first.unmount();
        const again = render(<Console id="keep" />);
        expect(shownLines()).toEqual([LINES[2].content]);
        again.unmount();
        render(<Console id="other" />);
        expect(shownLines()).toHaveLength(4);
    });
});

describe('콘솔 맨 아래로', () => {
    it('위로 올려 읽는 동안 단추와 그사이 온 줄 수를 보이고, 누르면 맨 아래로 간다', () => {
        const { rerender } = render(<Console id="jump" />);
        const log = screen.getByRole('log');
        expect(screen.queryByRole('button', { name: /Jump to bottom/ })).toBeNull();
        Object.defineProperty(log, 'scrollHeight', { configurable: true, value: 1000 });
        Object.defineProperty(log, 'clientHeight', { configurable: true, value: 200 });
        log.scrollTop = 100;
        fireEvent.scroll(log);
        expect(screen.getByRole('button', { name: 'Jump to bottom' })).toBeTruthy();
        rerender(<Console id="jump" lines={[...LINES, ...more(3)]} />);
        // 올려 읽는 동안에는 새 줄을 따라 내려가지 않는다
        expect(log.scrollTop).toBe(100);
        const jump = screen.getByRole('button', { name: 'Jump to bottom 3 new' });
        fireEvent.click(jump);
        expect(log.scrollTop).toBe(1000);
        expect(screen.queryByRole('button', { name: /Jump to bottom/ })).toBeNull();
    });
});

describe('콘솔 줄 찾기 (Ctrl+F)', () => {
    function Harness() {
        const ref = useRef<HTMLDivElement>(null);
        return (
            <>
                <PageFind rootRef={ref} enabled pageKey="console" />
                <div ref={ref}>
                    <Console id="find" />
                </div>
            </>
        );
    }
    const settle = () => act(() => new Promise((r) => setTimeout(r, 30)));
    const count = () => screen.getByRole('search').querySelector('[aria-live]')?.textContent;

    it('일치하는 줄을 세고, 다음 결과의 줄을 콘솔 안에서 보이게 한다', async () => {
        render(<Harness />);
        fireEvent.keyDown(window, { key: 'f', ctrlKey: true });
        fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'behind' } });
        await settle();
        expect(count()).toBe('1/2');
        // 콘솔 안에서 가려진 줄: 창 전체로는 보이는 높이여도 콘솔 스크롤 영역 밖이면 그 줄로 스크롤한다
        const log = screen.getByRole('log');
        log.getBoundingClientRect = () => ({ top: 300, bottom: 500 }) as DOMRect;
        const target = document.querySelectorAll<HTMLElement>('.console-line')[3];
        target.getBoundingClientRect = () => ({ top: 520, bottom: 540 }) as DOMRect;
        target.scrollIntoView = vi.fn();
        fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Enter' });
        expect(count()).toBe('2/2');
        expect(target.scrollIntoView).toHaveBeenCalled();
    });
});
