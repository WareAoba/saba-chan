/**
 * 우클릭 메뉴 규칙 (2026-10-31 시안):
 * 대상이 있는 곳은 그 대상의 메뉴, 입력칸은 편집 메뉴, 글자를 고르면 복사, 그 밖은 아무것도 뜨지 않는다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ContextMenuProvider, computeMenuPosition, useContextMenu } from '../components/ui/ContextMenu';

function Row({ onRemove }) {
    const openMenu = useContextMenu();
    return (
        <div
            className="row"
            onContextMenu={(e) =>
                openMenu(e, [
                    { id: 'open', label: 'Open', onSelect: vi.fn() },
                    { id: 'locked', label: 'Locked', disabled: true, onSelect: vi.fn() },
                    'separator',
                    { id: 'remove', label: 'Remove', danger: true, onSelect: onRemove },
                ])
            }
        >
            row
        </div>
    );
}

const menu = () => document.querySelector('[role="menu"]');
const items = () =>
    [...document.querySelectorAll('[role="menuitem"]')].map((b) => b.querySelector('.context-menu-label').textContent);
const rightClick = (el) => fireEvent.contextMenu(el, { clientX: 40, clientY: 50 });

describe('computeMenuPosition', () => {
    it('커서 자리에 두고, 화면 밖으로 나가면 안쪽으로 민다', () => {
        expect(computeMenuPosition(40, 50, 180, 120, 1000, 800)).toEqual({ left: 40, top: 50 });
        expect(computeMenuPosition(950, 750, 180, 120, 1000, 800)).toEqual({ left: 816, top: 676 });
        expect(computeMenuPosition(-5, -5, 180, 120, 1000, 800)).toEqual({ left: 4, top: 4 });
    });
});

describe('ContextMenuProvider', () => {
    let writeText;
    beforeEach(() => {
        writeText = vi.fn(() => Promise.resolve());
        Object.defineProperty(navigator, 'clipboard', {
            configurable: true,
            value: { writeText, readText: vi.fn(() => Promise.resolve('붙일 글')) },
        });
    });
    afterEach(() => window.getSelection()?.removeAllRanges());

    it('메뉴가 없는 곳은 우클릭을 막고 아무것도 띄우지 않는다', () => {
        render(
            <ContextMenuProvider allowNative={false}>
                <p>바탕</p>
            </ContextMenuProvider>,
        );
        const notCancelled = fireEvent.contextMenu(screen.getByText('바탕'));
        expect(notCancelled).toBe(false);
        expect(menu()).toBeNull();
    });

    it('대상이 단 메뉴를 띄우고, 고르면 실행하고 닫는다', () => {
        const onRemove = vi.fn();
        render(
            <ContextMenuProvider allowNative={false}>
                <Row onRemove={onRemove} />
            </ContextMenuProvider>,
        );
        rightClick(screen.getByText('row'));
        expect(menu().parentElement).toBe(document.body);
        expect(items()).toEqual(['Open', 'Locked', 'Remove']);
        expect(screen.getByRole('menuitem', { name: 'Locked' }).disabled).toBe(true);
        fireEvent.click(screen.getByRole('menuitem', { name: 'Remove' }));
        expect(onRemove).toHaveBeenCalledTimes(1);
        expect(menu()).toBeNull();
    });

    it('Esc·바깥 누르기로 닫히고, 방향키와 Enter로 고를 수 있다', () => {
        const onRemove = vi.fn();
        render(
            <ContextMenuProvider allowNative={false}>
                <Row onRemove={onRemove} />
                <p>밖</p>
            </ContextMenuProvider>,
        );
        rightClick(screen.getByText('row'));
        fireEvent.keyDown(document, { key: 'Escape' });
        expect(menu()).toBeNull();

        rightClick(screen.getByText('row'));
        fireEvent.pointerDown(screen.getByText('밖'));
        expect(menu()).toBeNull();

        rightClick(screen.getByText('row'));
        // 비활성 항목은 건너뛴다: Open → Remove
        fireEvent.keyDown(document, { key: 'ArrowDown' });
        fireEvent.keyDown(document, { key: 'ArrowDown' });
        fireEvent.keyDown(document, { key: 'Enter' });
        expect(onRemove).toHaveBeenCalledTimes(1);
    });

    it('입력칸은 편집 메뉴를 띄우고, 고른 글자가 없으면 잘라내기·복사를 끈다', async () => {
        render(
            <ContextMenuProvider allowNative={false}>
                <input aria-label="name" defaultValue="hello world" />
            </ContextMenuProvider>,
        );
        const input = screen.getByLabelText('name');
        input.setSelectionRange(0, 0);
        rightClick(input);
        expect(items()).toEqual(['Undo', 'Cut', 'Copy', 'Paste', 'Select all']);
        expect(screen.getByRole('menuitem', { name: 'Copy' }).disabled).toBe(true);
        fireEvent.click(screen.getByRole('menuitem', { name: 'Select all' }));
        expect([input.selectionStart, input.selectionEnd]).toEqual([0, 11]);

        input.setSelectionRange(6, 11);
        rightClick(input);
        await act(async () => fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' })));
        expect(writeText).toHaveBeenCalledWith('world');
    });

    it('메뉴를 단 대상 안의 입력칸은 대상 메뉴 대신 편집 메뉴를 띄운다', () => {
        function Card() {
            const openMenu = useContextMenu();
            return (
                <div onContextMenu={(e) => openMenu(e, [{ id: 'remove', label: 'Remove', onSelect: vi.fn() }])}>
                    <input aria-label="field" defaultValue="x" />
                </div>
            );
        }
        render(
            <ContextMenuProvider allowNative={false}>
                <Card />
            </ContextMenuProvider>,
        );
        rightClick(screen.getByLabelText('field'));
        expect(items()).toContain('Paste');
        expect(items()).not.toContain('Remove');
    });

    it('읽기 전용 입력칸은 잘라내기·붙여넣기·실행 취소를 끈다', () => {
        render(
            <ContextMenuProvider allowNative={false}>
                <input aria-label="ro" readOnly defaultValue="abc" />
            </ContextMenuProvider>,
        );
        rightClick(screen.getByLabelText('ro'));
        for (const name of ['Undo', 'Cut', 'Paste']) {
            expect(screen.getByRole('menuitem', { name }).disabled).toBe(true);
        }
    });

    it('글자를 골라 두고 우클릭하면 복사 메뉴만 띄운다', async () => {
        render(
            <ContextMenuProvider allowNative={false}>
                <p>play.example.kr:25565</p>
            </ContextMenuProvider>,
        );
        const p = screen.getByText('play.example.kr:25565');
        p.textContent = '  play.example.kr:25565\n';
        const range = document.createRange();
        range.selectNodeContents(p);
        window.getSelection().addRange(range);
        rightClick(p);
        expect(items()).toEqual(['Copy']);
        await act(async () => fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' })));
        expect(writeText).toHaveBeenCalledWith('  play.example.kr:25565\n');
    });

    it('개발 빌드(allowNative)에서는 메뉴 없는 곳의 기본 메뉴를 막지 않는다', () => {
        render(
            <ContextMenuProvider allowNative>
                <p>바탕</p>
            </ContextMenuProvider>,
        );
        expect(fireEvent.contextMenu(screen.getByText('바탕'))).toBe(true);
    });
});

describe('콘솔 출력 우클릭', () => {
    it('복사·모두 선택·화면 지우기를 띄우고, 화면 지우기는 이 서버의 줄만 비운다', async () => {
        const { ServerConsole } = await import('../components/servers/ServerConsole');
        const { useConsoleStore } = await import('../stores/useConsoleStore');
        const line = (id, content) => ({ id, key: id, content, source: 'STDOUT', level: 'INFO' });
        useConsoleStore.getState().setConsoles({
            m: { lines: [line(1, 'Done (4.2s)!')] },
            other: { lines: [line(2, 'keep')] },
        });
        function Live() {
            const state = useConsoleStore((s) => s.consoles.m);
            return (
                <ServerConsole
                    server={{ id: 'm', name: 'MC', status: 'running' }}
                    state={state}
                    openConsole={() => {}}
                    setConsoleInput={() => {}}
                    sendConsoleCommand={() => {}}
                />
            );
        }
        render(
            <ContextMenuProvider allowNative={false}>
                <Live />
            </ContextMenuProvider>,
        );
        rightClick(document.querySelector('.term-output'));
        expect(items()).toEqual(['Copy', 'Select all', 'Clear screen']);
        expect(screen.getByRole('menuitem', { name: 'Copy' }).disabled).toBe(true);
        fireEvent.click(screen.getByRole('menuitem', { name: 'Clear screen' }));
        expect(useConsoleStore.getState().consoles.m.lines).toEqual([]);
        expect(useConsoleStore.getState().consoles.other.lines).toHaveLength(1);
    });
});

describe('ContextMenuProvider 키보드·숫자칸', () => {
    afterEach(() => {
        delete document.execCommand;
    });

    it('메뉴가 쓴 방향키·Enter·Esc는 입력칸까지 가지 않는다', () => {
        const onKeyDown = vi.fn();
        render(
            <ContextMenuProvider allowNative={false}>
                <input aria-label="cmd" defaultValue="say hi" onKeyDown={onKeyDown} />
            </ContextMenuProvider>,
        );
        const input = screen.getByLabelText('cmd');
        rightClick(input);
        fireEvent.keyDown(input, { key: 'ArrowDown' });
        fireEvent.keyDown(input, { key: 'ArrowUp' });
        fireEvent.keyDown(input, { key: 'Escape' });
        expect(onKeyDown).not.toHaveBeenCalled();
        fireEvent.keyDown(input, { key: 'ArrowUp' });
        expect(onKeyDown).toHaveBeenCalledTimes(1);
    });

    it('숫자 입력칸도 붙여넣기가 끝까지 간다 (선택 범위를 다룰 수 없는 칸)', async () => {
        Object.defineProperty(navigator, 'clipboard', {
            configurable: true,
            value: { writeText: vi.fn(), readText: vi.fn(() => Promise.resolve('25565')) },
        });
        document.execCommand = vi.fn(() => true);
        render(
            <ContextMenuProvider allowNative={false}>
                <input aria-label="port" type="number" defaultValue="" />
            </ContextMenuProvider>,
        );
        rightClick(screen.getByLabelText('port'));
        await act(async () => fireEvent.click(screen.getByRole('menuitem', { name: 'Paste' })));
        expect(document.execCommand).toHaveBeenCalledWith('insertText', false, '25565');
    });
});
