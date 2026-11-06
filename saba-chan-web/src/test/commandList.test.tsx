import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { CommandList } from '../components/CommandList';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

// 사용자 요청(2026-11-06): 권한 화면을 GUI처럼 명령 목록으로 — 명령마다 "누구부터 쓸 수 있나"를 그 줄에서 고른다
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
const board = () => ({
    guildId: 'g1',
    instances: [{ id: 'mc-1', name: 'Survival', module: 'minecraft' }],
    items: [
        item('mc-1', 'say', 'Say', 0, { risk: 'safe', preset: 0 }),
        item('mc-1', 'stop', 'Stop', 2, { overridden: true }),
        item('', 'update.apply', 'Update', 3, { kind: 'admin', fixed: true }),
    ],
});

function mount(api: object) {
    const write = vi.fn(async (fn) => fn());
    render(
        <CommandList
            api={mockApi(api)}
            guildId="g1"
            perm={perm}
            board={board()}
            instance="mc-1"
            busy={false}
            write={write}
        />,
    );
}

describe('웹 명령 목록', () => {
    beforeAll(() => initI18n('en'));
    afterEach(cleanup);

    it('명령 줄에서 레벨을 고르면 그 레벨부터 쓰게 저장한다', async () => {
        const api = { permissionsUpdate: vi.fn().mockResolvedValue({ ok: true }) };
        mount(api);
        fireEvent.click(screen.getByLabelText('Say: who can use it'));
        fireEvent.click(screen.getByRole('option', { name: /Lv 3 · Admin/ }));
        await act(async () => {});
        expect(api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/say': 2 } });
    });

    it('옮긴 명령은 프리셋으로 되돌리고, 고정 명령은 고를 수 없다', async () => {
        const api = { permissionsUpdate: vi.fn().mockResolvedValue({ ok: true }) };
        mount(api);
        const stop = screen.getByText('Stop').closest('li') as HTMLElement;
        fireEvent.click(within(stop).getByRole('button', { name: 'Use preset' }));
        await act(async () => {});
        expect(api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/stop': null } });
        const update = screen.getByText('Update').closest('li') as HTMLElement;
        expect(within(update).getByText(/whole PC/)).toBeTruthy();
    });

    it('검색어로 명령을 거르고, 되묻기 단계를 보여 준다', () => {
        mount({});
        fireEvent.change(screen.getByLabelText('Find a command'), { target: { value: 'sto' } });
        expect(screen.queryByText('Say')).toBeNull();
        const stop = screen.getByText('Stop').closest('li') as HTMLElement;
        expect(within(stop).getByText('Caution')).toBeTruthy();
    });
});
