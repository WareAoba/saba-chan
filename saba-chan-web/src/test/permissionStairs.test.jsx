import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { PermissionStairs } from '../components/PermissionStairs';
import { initI18n } from '../i18n';

// 사용자 요청(2026-10-28): 웹도 GUI처럼 칩을 끌어 레벨을 옮기고, 옮길 때 미끄러지는 애니메이션
const perm = { guildId: 'g1', preset: 'default', commandLevels: {} };
const item = (instance, name, title, level, extra = {}) => ({
    key: instance ? `${instance}/${name}` : name,
    keys: [instance ? `${instance}/${name}` : name],
    instance,
    name,
    title,
    kind: 'command',
    risk: 'caution',
    level,
    preset: level,
    overridden: false,
    fixed: false,
    ...extra,
});
const board = (sayLevel = 0) => ({
    guildId: 'g1',
    instances: [{ id: 'mc-1', name: 'Survival', module: 'minecraft' }],
    items: [item('mc-1', 'say', 'Say', sayLevel, { risk: 'safe', preset: 0 }), item('mc-1', 'stop', 'Stop', 1)],
});
const step = (name) => screen.getByRole('listitem', { name });

function mount(api) {
    const write = vi.fn(async (fn) => fn());
    const props = { api, guildId: 'g1', perm, instance: 'mc-1', busy: false, write };
    const view = render(<PermissionStairs {...props} board={board()} />);
    return (next) => view.rerender(<PermissionStairs {...props} board={next} />);
}

describe('웹 레벨별 명령 계단', () => {
    beforeAll(() => initI18n('en'));
    afterEach(cleanup);

    it('칩을 누르고 레벨을 고르면 그 레벨부터 쓰게 옮긴다', async () => {
        const api = { permissionsUpdate: vi.fn().mockResolvedValue({ ok: true }) };
        mount(api);
        fireEvent.click(within(step('Lv 1 · Guest')).getByRole('button', { name: /Say/ }));
        fireEvent.click(
            within(screen.getByRole('region', { name: 'Say' })).getByRole('button', { name: 'Lv 3 · Admin' }),
        );
        await act(async () => {});
        expect(api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/say': 2 } });
    });

    it('칩 편집 창은 이 서버의 디스코드 되묻기 단계를 보여 주고, 바꾸기는 PC 앱에서 한다고 알린다', () => {
        mount({});
        fireEvent.click(within(step('Lv 2 · User')).getByRole('button', { name: /Stop/ }));
        expect(within(screen.getByRole('region', { name: 'Stop' })).getByText(/Discord confirm: Caution/)).toBeTruthy();
    });

    it('손가락으로 칩을 다른 계단에 끌어 놓으면 옮기고, 새 자리로 미끄러진다', async () => {
        const api = { permissionsUpdate: vi.fn().mockResolvedValue({ ok: true }) };
        const rerender = mount(api);
        const order = ['Lv 0 · Banned', 'Lv 1 · Guest', 'Lv 2 · User', 'Lv 3 · Admin', 'Lv 4 · Owner'];
        const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function () {
            const top = (4 - order.indexOf(this.closest?.('li')?.getAttribute('aria-label'))) * 100;
            return { left: 0, top, width: 10, height: 10, right: 10, bottom: top + 10, x: 0, y: top };
        });
        const animate = vi.fn();
        HTMLElement.prototype.animate = animate;
        document.elementFromPoint = () => step('Lv 3 · Admin');
        try {
            const say = within(step('Lv 1 · Guest')).getByRole('button', { name: /Say/ });
            fireEvent.pointerDown(say, { clientX: 10, clientY: 300, button: 0 });
            fireEvent.pointerMove(window, { clientX: 10, clientY: 100 });
            expect(say.classList.contains('is-dragging')).toBe(true);
            expect(step('Lv 3 · Admin').classList.contains('is-over')).toBe(true);
            fireEvent.pointerUp(window, { clientX: 10, clientY: 100 });
            await act(async () => {});
            expect(api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/say': 2 } });
            expect(screen.queryByRole('region', { name: 'Say' })).toBeNull();
            rerender(board(2));
            expect(within(step('Lv 3 · Admin')).getByRole('button', { name: /Say/ })).toBeTruthy();
            expect(animate).toHaveBeenCalled();
            // 끈 뒤에도 다음 탭은 칩을 고른다 (브라우저는 끈 칩에 click 을 보내지 않는다)
            fireEvent.click(within(step('Lv 3 · Admin')).getByRole('button', { name: /Say/ }));
            expect(screen.getByRole('region', { name: 'Say' })).toBeTruthy();
        } finally {
            rect.mockRestore();
            delete HTMLElement.prototype.animate;
            delete document.elementFromPoint;
        }
    });
});
