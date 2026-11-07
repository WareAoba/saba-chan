import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { InfoPanel } from '../components/InfoPanel';
import { ServerDetail } from '../components/ServerDetail';
import { initI18n } from '../i18n';
import { useData } from '../store';
import { asModules, asServer, mockApi } from './mocks';

/** 부른 함수마다 빈 답을 주는 가짜 (따로 정한 것만 그 답) */
const apiWith = (own: Record<string, unknown> = {}) =>
    mockApi(
        new Proxy(own, {
            get: (target, prop: string) => {
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        }),
    );

const minecraft = {
    name: 'minecraft',
    game_name: 'Minecraft',
    protocols: { supported: ['stdin'] },
    commands: { fields: [{ name: 'say', inputs: [{ name: 'message' }] }] },
};

const tabNames = () =>
    within(screen.getByRole('tablist', { name: 'Server menu' }))
        .getAllByRole('tab')
        .map((b) => b.textContent);

// 사용자 11-06 #25: 웹 서버 탭을 앱과 같은 이름·순서로, 모듈이 지원하는 것만
describe('웹 서버 탭', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => useData.setState({ modules: asModules([minecraft]), modulesLoaded: true }));

    it('앱과 같은 이름·순서 (기록 탭은 PC에만 있어 빼고, 플레이어·점검 탭은 없다)', async () => {
        const server = asServer({ id: 's1', name: 'MC', module: 'minecraft', status: 'stopped' });
        await act(async () => render(<ServerDetail api={apiWith()} server={server} />));
        expect(tabNames()).toEqual(['Console', 'Commands', 'Settings', 'Saba-chan', 'Backups', 'Discord', 'Info']);
    });

    it('사바쨩이 출력을 쥐지 않는 서버는 콘솔 탭이 없다 (모듈·실행 방식이 지원할 때만)', async () => {
        const server = asServer({ id: 's1', name: 'MC', module: 'minecraft', status: 'stopped', managed: false });
        await act(async () => render(<ServerDetail api={apiWith()} server={server} />));
        expect(tabNames()[0]).toBe('Commands');
        expect(tabNames()).not.toContain('Console');
    });

    it('접속자를 알려 주는 서버만 머리에 접속 칩을 두고, 누르면 접속자 목록을 연다', async () => {
        const instancePlayers = vi.fn().mockResolvedValue({ online: 2, max: 10, players: [{ name: 'Steve' }, { name: 'Alex' }] });
        const server = asServer({
            id: 's1',
            name: 'MC',
            module: 'minecraft',
            status: 'running',
            players: { online: 2, max: 10 },
        });
        await act(async () => render(<ServerDetail api={apiWith({ instancePlayers })} server={server} />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Players 2/10' })));
        expect(instancePlayers).toHaveBeenCalledWith('s1');
        expect(screen.getByText('Steve')).toBeTruthy();
    });

    it('접속자를 모르는 서버는 접속 칩이 없다', async () => {
        const server = asServer({ id: 's1', name: 'MC', module: 'minecraft', status: 'running' });
        await act(async () => render(<ServerDetail api={apiWith()} server={server} />));
        expect(screen.queryByRole('button', { name: /^Players/ })).toBeNull();
    });
});

// 사용자 11-06 #24: 정보 탭에 날것 값(exit 0, stdin)을 보이지 않는다. 인스턴스 ID는 그대로 보인다
describe('웹 정보 탭', () => {
    beforeAll(() => initI18n('en'));

    it('통신 방식은 자세히 안에, 모르는 값은 줄째 숨기고, 최근 종료 기록은 없다', async () => {
        const api = apiWith({
            instanceEvents: vi.fn().mockResolvedValue({
                events: [
                    { at: 1_700_000_000, kind: 'user_stop', exit_code: 0 },
                    { at: 1_699_000_000, kind: 'crash', exit_code: 3 },
                ],
            }),
        });
        const server = asServer({ id: 'abc-123', name: 'MC', module: 'minecraft', status: 'stopped', protocol_mode: 'stdin' });
        await act(async () => render(<InfoPanel api={api} server={server} moduleData={minecraft as never} />));
        const text = document.body.textContent || '';
        expect(text).toContain('abc-123');
        expect(text).not.toContain('Exit code 3'); // 최근 종료 기록은 뺐다 (사용자 지정 2026-11-08)
        expect(text).not.toMatch(/exit 0|exit 3/);
        expect(text).not.toContain('—');
        const more = screen.getByText('Details').closest('details') as HTMLElement;
        expect(within(more).getByText('STDIN')).toBeTruthy();
    });
});
