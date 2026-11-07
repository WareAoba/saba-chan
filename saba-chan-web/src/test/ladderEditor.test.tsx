import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { LadderEditor } from '../components/PermissionsTab';
import { initI18n } from '../i18n';

// 사용자 지정(2026-11-06·07, 앱 레벨 편집과 같다): 모든 레벨 이름 바꾸기, 소유자·게스트·금지는 못 지움,
// 카드는 끌어서 옮기고(화살표 없음) 칸에는 사바쨩 기능 칩만, 칩을 끌어 다른 레벨로, 기본값으로 초기화
const perm = { guildId: 'g1', preset: 'default', commandLevels: {} };
const item = (instance: string, name: string, level: number, extra = {}) => ({
    key: instance ? `${instance}/${name}` : name,
    keys: [instance ? `${instance}/${name}` : name],
    instance,
    name,
    title: name,
    kind: 'command',
    risk: 'safe',
    level,
    preset: 2,
    overridden: false,
    fixed: false,
    ...extra,
});
const board = {
    items: [
        item('mc-1', 'start', 2, { controllers: true }),
        item('mc-1', 'say', 1, { module: true, preset: 1 }),
        item('mc-1', 'notice', 2),
        item('mc-1', 'words', 3, { overridden: true }),
    ],
};
const card = (name: string) => screen.getByRole('listitem', { name });

function mount() {
    const onSave = vi.fn();
    render(<LadderEditor perm={perm} board={board} busy={false} onSave={onSave} />);
    return onSave;
}

/** 끄는 요소를 `to` 칸 위에 놓는다 (pointer 이벤트, @gui pointerDrag) */
function drag(el: HTMLElement, to: HTMLElement) {
    document.elementFromPoint = () => to;
    try {
        fireEvent.pointerDown(el, { clientX: 0, clientY: 0, button: 0 });
        fireEvent.pointerMove(window, { clientX: 0, clientY: 40 });
        fireEvent.pointerUp(window, { clientX: 0, clientY: 40 });
    } finally {
        delete (document as Partial<Document>).elementFromPoint;
    }
}

describe('웹 레벨 편집', () => {
    beforeAll(() => initI18n('en'));
    afterEach(cleanup);

    it('이름을 눌러 바꾸고, 게스트는 지울 수 없고, 위·아래 화살표는 없다', () => {
        const onSave = mount();
        expect(screen.getByText('Edit levels')).toBeTruthy();
        expect((screen.getByRole('button', { name: 'Delete Guest' }) as HTMLButtonElement).disabled).toBe(true);
        expect((screen.getByRole('button', { name: 'Delete User' }) as HTMLButtonElement).disabled).toBe(false);
        expect(screen.queryByRole('button', { name: 'Move Admin up' })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Rename Admin' }));
        const input = screen.getByRole('textbox', { name: 'Level name' });
        expect(input.getAttribute('placeholder')).toBe('Admin');
        fireEvent.change(input, { target: { value: 'Staff' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith({
            ladder: [{ id: 3 }, { id: 2, name: 'Staff' }, { id: 1 }, { id: 0 }, { id: -1 }],
        });
    });

    it('칸에는 사바쨩 기능 칩만 보인다 (게임 명령·조작 가능·조작 가능 줄은 인스턴스 권한에서)', () => {
        mount();
        expect(within(card('Admin')).getByRole('button', { name: 'notice' })).toBeTruthy();
        expect(screen.queryByRole('button', { name: 'say' })).toBeNull();
        expect(screen.queryByRole('button', { name: 'start' })).toBeNull();
        expect(screen.queryByRole('listitem', { name: 'Operators line' })).toBeNull();
    });

    it('칩을 다른 레벨 카드에 끌어 놓으면 노란 테두리, 그 명령만 저장한다', () => {
        const onSave = mount();
        drag(within(card('Admin')).getByRole('button', { name: 'notice' }), card('User'));
        const moved = within(card('User')).getByRole('button', { name: 'notice' });
        expect(moved.classList.contains('is-changed')).toBe(true);
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith({ commandLevels: { 'mc-1/notice': 1 } });
    });

    it('카드를 끌어 놓으면 순서가 바뀐다', () => {
        const onSave = mount();
        drag(card('User'), card('Admin'));
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith(
            expect.objectContaining({ ladder: [{ id: 3 }, { id: 1 }, { id: 2 }, { id: 0 }, { id: -1 }] }),
        );
    });

    it('권한 맞바꾸기: 두 레벨을 고르면 칩이 바뀌고, 바뀐 명령만 저장한다', () => {
        const onSave = mount();
        fireEvent.click(screen.getByRole('button', { name: /Swap permissions/ }));
        fireEvent.click(screen.getByRole('button', { name: 'Pick Owner' }));
        fireEvent.click(screen.getByRole('button', { name: 'Pick Admin' }));
        expect(within(card('Owner')).getByRole('button', { name: 'notice' })).toBeTruthy();
        expect(within(card('Admin')).getByRole('button', { name: 'words' })).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith({
            commandLevels: { 'mc-1/start': 3, 'mc-1/notice': 3, 'mc-1/words': null },
        });
    });

    it('기본값으로 초기화: 기본 사다리와 프리셋 레벨', () => {
        const onSave = mount();
        fireEvent.click(screen.getByRole('button', { name: 'Delete User' }));
        fireEvent.click(screen.getByRole('button', { name: /Reset to defaults/ }));
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith({ commandLevels: { 'mc-1/words': null } });
    });
});
