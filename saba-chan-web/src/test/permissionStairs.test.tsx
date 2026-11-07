import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { PermissionStairs } from '../components/PermissionStairs';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

// 사용자 지정(2026-11-07): 웹 인스턴스 권한도 앱처럼 계단·칩으로. 금지 칸 없음, 인스턴스 화면은 그 인스턴스 명령만,
// 시작·정지·재시작·백업은 "서버 조작" 칩 하나, 게임 명령 칩은 명령 원문(번역 이름은 말풍선)
const perm = { guildId: 'g1', preset: 'default', commandLevels: {} };
const item = (instance: string, name: string, title: string, level: number, extra = {}) => ({
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
    items: [
        item('mc-1', 'say', 'Say', sayLevel, { risk: 'safe', preset: 0, module: true }),
        item('mc-1', 'server.start', 'Start', 1, { controllers: true }),
        item('mc-1', 'server.stop', 'Stop', 1, { controllers: true }),
        item('mc-1', 'backup.create', 'Make backup', 2, { controllers: true }),
        item('mc-1', 'server.players', 'Players', 0, { kind: 'query', risk: 'safe' }),
        item('', 'update.apply', 'Update', 3, { kind: 'admin', fixed: true }),
    ],
});
const step = (name: string) => screen.getByRole('listitem', { name });

function mount(api: object, instance = 'mc-1') {
    const write = vi.fn(async (fn) => fn());
    const props = { api: mockApi(api), guildId: 'g1', perm, instance, busy: false, write };
    const view = render(<PermissionStairs {...props} board={board()} />);
    return (next: ReturnType<typeof board>) => view.rerender(<PermissionStairs {...props} board={next} />);
}

describe('웹 레벨별 명령 계단', () => {
    beforeAll(() => initI18n('en'));
    afterEach(cleanup);

    it('금지 칸 없이 게스트부터 소유자까지, 인스턴스 화면은 그 인스턴스 명령만 보인다', () => {
        mount({});
        const steps = within(screen.getByRole('list', { name: 'Command steps' })).getAllByRole('listitem');
        expect(steps.map((s) => s.getAttribute('aria-label'))).toEqual([
            'Lv 0 · Guest',
            'Lv 1 · User',
            'Lv 2 · Admin',
            'Lv 3 · Owner',
        ]);
        expect(screen.queryByRole('button', { name: /Update/ })).toBeNull();
        expect(screen.queryByRole('button', { name: /Help/ })).toBeNull();
    });

    it('게임 명령 칩은 명령 원문, 번역 이름은 말풍선. 사바쨩 기능은 번역 이름', () => {
        mount({});
        const say = within(step('Lv 0 · Guest')).getByRole('button', { name: 'Say' });
        expect(say.textContent).toBe('say');
        expect(say.getAttribute('title')).toBe('Say');
        expect(within(step('Lv 0 · Guest')).getByRole('button', { name: 'Players' }).textContent).toBe('Players');
    });

    it('서버 조작 칩은 시작·정지·백업을 묶고, 옮기면 묶인 명령을 모두 그 레벨로', async () => {
        const api = { permissionsUpdate: vi.fn().mockResolvedValue({ ok: true }) };
        mount(api);
        expect(screen.queryByRole('button', { name: 'Start' })).toBeNull();
        fireEvent.click(within(step('Lv 1 · User')).getByRole('button', { name: 'Server control' }));
        fireEvent.click(
            within(screen.getByRole('region', { name: 'Server control' })).getByRole('button', {
                name: 'Lv 2 · Admin',
            }),
        );
        // 옮긴 것은 초안이다: 저장을 눌러야 데몬에 간다 (백업은 이미 관리자라 바뀌지 않는다)
        expect(api.permissionsUpdate).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await act(async () => {});
        expect(api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            commandLevels: { 'mc-1/server.start': 2, 'mc-1/server.stop': 2 },
        });
    });

    it('모든 인스턴스 보기는 도움말·목록과 서버 전체 명령도 보인다', () => {
        mount({}, '*');
        expect(within(step('Lv 0 · Guest')).getByRole('button', { name: /Help/ })).toBeTruthy();
        expect(within(step('Lv 3 · Owner')).getByRole('button', { name: 'Update' })).toBeTruthy();
    });

    it('제자리로 돌려 놓으면 노란 테두리와 저장 막대가 사라지고, 기본값으로 초기화는 프리셋 레벨로', () => {
        mount({});
        const pick = (chip: string, level: string) => {
            if (!screen.queryByRole('region', { name: chip }))
                fireEvent.click(screen.getByRole('button', { name: chip }));
            fireEvent.click(within(screen.getByRole('region', { name: chip })).getByRole('button', { name: level }));
        };
        pick('Say', 'Lv 1 · User');
        expect(within(step('Lv 1 · User')).getByRole('button', { name: 'Say' }).classList.contains('is-changed')).toBe(
            true,
        );
        expect(screen.getByRole('status').textContent).toContain('unsaved');
        pick('Say', 'Lv 0 · Guest');
        expect(within(step('Lv 0 · Guest')).getByRole('button', { name: 'Say' }).classList.contains('is-changed')).toBe(
            false,
        );
        expect(screen.queryByRole('status')).toBeNull();
        pick('Say', 'Lv 3 · Owner');
        fireEvent.click(screen.getByRole('button', { name: /Reset to defaults/ }));
        expect(within(step('Lv 0 · Guest')).getByRole('button', { name: 'Say' })).toBeTruthy();
    });

    it('손가락으로 칩을 다른 칸에 끌어 놓으면 옮기고, 새 자리로 미끄러진다', async () => {
        const api = { permissionsUpdate: vi.fn().mockResolvedValue({ ok: true }) };
        const rerender = mount(api);
        const order = ['Lv 0 · Guest', 'Lv 1 · User', 'Lv 2 · Admin', 'Lv 3 · Owner'];
        const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
            this: HTMLElement,
        ) {
            const left = order.indexOf(this.closest?.('li')?.getAttribute('aria-label') as string) * 100;
            return { left, top: 0, width: 10, height: 10, right: left + 10, bottom: 10, x: left, y: 0 } as DOMRect;
        });
        const animate = vi.fn();
        HTMLElement.prototype.animate = animate;
        document.elementFromPoint = () => step('Lv 2 · Admin');
        try {
            const say = within(step('Lv 0 · Guest')).getByRole('button', { name: 'Say' });
            fireEvent.pointerDown(say, { clientX: 5, clientY: 5, button: 0 });
            fireEvent.pointerMove(window, { clientX: 205, clientY: 5 });
            expect(say.classList.contains('is-dragging')).toBe(true);
            expect(step('Lv 2 · Admin').classList.contains('is-over')).toBe(true);
            fireEvent.pointerUp(window, { clientX: 205, clientY: 5 });
            await act(async () => {});
            // 초안에서 바로 옮겨 그리고, 저장하지 않은 칩은 노란 테두리
            const moved = within(step('Lv 2 · Admin')).getByRole('button', { name: 'Say' });
            expect(moved.classList.contains('is-changed')).toBe(true);
            expect(animate).toHaveBeenCalled();
            fireEvent.click(screen.getByRole('button', { name: 'Save' }));
            await act(async () => {});
            expect(api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/say': 2 } });
            rerender(board(2));
            expect(within(step('Lv 2 · Admin')).getByRole('button', { name: 'Say' })).toBeTruthy();
            expect(animate).toHaveBeenCalled();
        } finally {
            rect.mockRestore();
            delete (HTMLElement.prototype as Partial<HTMLElement>).animate;
            delete (document as Partial<Document>).elementFromPoint;
        }
    });
});
