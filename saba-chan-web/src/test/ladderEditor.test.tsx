import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { LadderEditor } from '../components/PermissionsTab';
import { initI18n } from '../i18n';

// 사용자 지정(2026-11-06): 모든 레벨 이름 바꾸기, 소유자·게스트·금지는 못 지움, 조작 가능 줄로 네 명령을 한꺼번에 옮기기
const perm = { guildId: 'g1', preset: 'default', commandLevels: {} };
const item = (instance: string, name: string, level: number, extra = {}) => ({
    key: `${instance}/${name}`,
    keys: [`${instance}/${name}`],
    instance,
    name,
    title: name,
    kind: 'command',
    risk: 'safe',
    level,
    preset: 2,
    overridden: false,
    fixed: false,
    controllers: true,
    ...extra,
});
const board = {
    items: [item('mc-1', 'start', 2), item('mc-1', 'stop', 2), item('mc-1', 'restart', 3, { overridden: true })],
};

function mount() {
    const onSave = vi.fn();
    render(<LadderEditor perm={perm} board={board} busy={false} onSave={onSave} />);
    return onSave;
}

describe('웹 레벨 사다리', () => {
    beforeAll(() => initI18n('en'));
    afterEach(cleanup);

    it('기본 레벨 이름을 바꾸고, 게스트는 지울 수 없다', () => {
        const onSave = mount();
        expect((screen.getByRole('button', { name: 'Delete Guest' }) as HTMLButtonElement).disabled).toBe(true);
        expect((screen.getByRole('button', { name: 'Delete User' }) as HTMLButtonElement).disabled).toBe(false);
        const names = screen.getAllByRole('textbox', { name: 'Level name' });
        expect(names[1].getAttribute('placeholder')).toBe('Admin');
        fireEvent.change(names[1], { target: { value: 'Staff' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith({
            ladder: [{ id: 3 }, { id: 2, name: 'Staff' }, { id: 1 }, { id: 0 }, { id: -1 }],
        });
    });

    it('조작 가능 줄을 옮기면 줄을 따르는 명령만 저장한다', () => {
        const onSave = mount();
        const line = screen.getByRole('listitem', { name: 'Operators line' });
        expect(line.textContent).toContain('Operators · Admin and up');
        fireEvent.click(screen.getByRole('button', { name: 'Move the operators line down' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move the operators line down' }));
        expect(screen.getByRole('listitem', { name: 'Operators line' }).textContent).toContain('Guest and up');
        expect(
            (screen.getByRole('button', { name: 'Move the operators line down' }) as HTMLButtonElement).disabled,
        ).toBe(true);
        fireEvent.click(screen.getByRole('button', { name: 'Save levels' }));
        expect(onSave).toHaveBeenCalledWith({ commandLevels: { 'mc-1/start': 0, 'mc-1/stop': 0 } });
    });
});
