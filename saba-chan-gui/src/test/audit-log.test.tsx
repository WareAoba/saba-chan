import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ActivityPage, AuditLog, clearAuditCache } from '../components/servers/AuditTab';
import i18n from '../i18n';
import { type Server, useServerStore } from '../stores/useServerStore';
import { pickDate } from './picker';

const entry = (over?: Record<string, unknown>) => ({
    ts: 1760000000,
    source: 'discord',
    user: 'aoba',
    guild: 'Village',
    action: 'command',
    instanceId: 'mc-1',
    instance: 'Minecraft',
    command: 'say',
    args: '["hi"]',
    ok: true,
    status: 200,
    ...over,
});

describe('AuditLog', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).auditList;
        delete (window.api as Partial<typeof window.api>).auditExport;
        vi.useRealTimers();
        clearAuditCache();
    });

    it('다시 들어오면 받아 둔 기록을 바로 보이고, 새 기록은 뒤에서 받아 바꾼다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] }) as never;
        const first = render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Command · say hi')).toBeTruthy();
        first.unmount();

        let answer: (v: unknown) => void = () => {};
        window.api.auditList = vi.fn(() => new Promise((r) => (answer = r))) as never;
        render(<AuditLog instanceId="mc-1" />);
        // 응답 전에도 지난번 목록이 그대로, 도는 표시 없이 보인다
        expect(screen.getByText('Command · say hi')).toBeTruthy();
        expect(document.querySelector('.audit-scroll > .saba-spinner, .audit-scroll > [role="status"]')).toBeNull();
        expect(window.api.auditList).toHaveBeenCalledTimes(1);
        await act(async () => answer({ entries: [entry({ command: 'list', args: undefined })] }));
        expect(await screen.findByText('Command · list')).toBeTruthy();
    });

    it('다른 서버로 바꾸면 응답이 오기 전까지 앞 서버의 기록을 보이지 않는다', async () => {
        window.api.auditList = vi.fn(async ({ instance }) =>
            instance === 'mc-1' ? { entries: [entry()] } : new Promise(() => {}),
        ) as never;
        const view = render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Command · say hi')).toBeTruthy();
        view.rerender(<AuditLog instanceId="pal-1" />);
        expect(screen.queryByText('Command · say hi')).toBeNull();
    });

    it('서버별로 누가·무엇을·어디서·성공 여부를 보이고, 목록 끝까지 내리면 더 불러온다', async () => {
        // 목록 끝 표시가 보인다고 알리는 IntersectionObserver
        const seen: Array<() => void> = [];
        vi.stubGlobal(
            'IntersectionObserver',
            class {
                constructor(private cb: (e: Array<{ isIntersecting: boolean }>) => void) {}
                observe() {
                    seen.push(() => this.cb([{ isIntersecting: true }]));
                }
                disconnect() {}
            },
        );
        window.api.auditList = vi.fn(async ({ limit }) => ({
            entries: Array.from({ length: limit }, (_, i) =>
                entry(
                    i === 0
                        ? {}
                        : {
                              action: 'stop',
                              source: 'schedule',
                              user: undefined,
                              guild: undefined,
                              command: undefined,
                              args: undefined,
                              ok: false,
                              status: 500,
                          },
                ),
            ),
        })) as never;
        render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Command · say hi')).toBeTruthy();
        expect(screen.getByText('Discord · aoba · Village')).toBeTruthy();
        expect(within(document.querySelector('.audit-list') as HTMLElement).getAllByText('Failed').length).toBe(99);
        expect(screen.queryByText('Minecraft')).toBeNull(); // 서버별 화면에는 서버 이름을 적지 않는다
        expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100, instance: 'mc-1' });
        expect(screen.queryByRole('button', { name: 'Show more' })).toBeNull(); // 더 보기 단추는 없다
        await waitFor(() => expect(seen.length).toBeGreaterThan(0));
        act(() => seen.at(-1)?.());
        await waitFor(() => expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 200, instance: 'mc-1' }));
        vi.unstubAllGlobals();
    });

    it('원격 데몬에 중계한 조작은 데몬 이름과 요청을 적는다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({
            entries: [
                entry({
                    source: 'cli',
                    user: undefined,
                    guild: undefined,
                    action: 'remote',
                    daemon: 'Game PC',
                    instanceId: '',
                    instance: undefined,
                    command: 'POST /api/instance/x/start',
                    args: undefined,
                }),
            ],
        }) as never;
        render(<AuditLog />);
        expect(await screen.findByText('Remote daemon · Game PC · POST /api/instance/x/start')).toBeTruthy();
    });

    it('전체 기록은 서버 이름을 적고, 적게 오면 더 불러오지 않는다', async () => {
        window.api.auditList = vi
            .fn()
            .mockResolvedValue({ entries: [entry({ source: 'gui', user: undefined, guild: undefined })] });
        render(<AuditLog />);
        expect(await screen.findByText(/Minecraft/)).toBeTruthy();
        expect(screen.getByText('App')).toBeTruthy();
        expect(window.api.auditList).toHaveBeenCalledWith({ limit: 100 });
        expect(document.querySelector('.audit-end')).toBeNull();
    });

    it('데몬이 이름을 주면 디스코드 id 대신 이름을 보인다', async () => {
        window.api.auditList = vi
            .fn()
            .mockResolvedValue({ entries: [entry({ user: '2840', userName: 'Aoba', guildName: 'Saba Village' })] });
        render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Discord · Aoba · Saba Village')).toBeTruthy();
    });

    it('서버가 꺼져 있어 건너뛴 예약 백업은 실패가 아니라 건너뜀으로 보인다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({
            entries: [
                entry({
                    action: 'backup',
                    source: 'schedule',
                    command: undefined,
                    args: undefined,
                    ok: false,
                    status: 409,
                }),
            ],
        }) as never;
        render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Backup')).toBeTruthy();
        expect(screen.getByText('Skipped (server off)')).toBeTruthy();
        expect(within(document.querySelector('.audit-list') as HTMLElement).queryByText('Failed')).toBeNull();
    });

    it('스크롤해도 새로고침·기간·거르기는 목록 밖에 있어 늘 누를 수 있다', async () => {
        window.api.auditList = vi.fn(async ({ limit }) => ({
            entries: Array.from({ length: limit }, () => entry()),
        })) as never;
        render(<AuditLog />);
        await screen.findAllByText('Command · say hi');
        const scroll = document.querySelector('.audit-scroll') as HTMLElement;
        expect(scroll.querySelector('.audit-list')).not.toBeNull();
        expect(scroll.contains(screen.getByRole('button', { name: 'Refresh' }))).toBe(false);
        expect(scroll.contains(screen.getByRole('tab', { name: 'All' }))).toBe(false);
        expect(scroll.contains(screen.getByRole('button', { name: 'Filter' }))).toBe(false);
    });

    it('기간을 고르면 그 기간만 처음부터 다시 불러온다', async () => {
        vi.useFakeTimers({ toFake: ['Date'] });
        vi.setSystemTime(new Date(2026, 9, 26, 15, 30));
        const day = (d: number) => new Date(2026, 9, d).getTime() / 1000;
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] }) as never;
        render(<AuditLog instanceId="mc-1" />);
        await screen.findByText('Command · say hi');
        fireEvent.click(screen.getByRole('tab', { name: 'Today' }));
        await waitFor(() =>
            expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100, instance: 'mc-1', since: day(26) }),
        );
        fireEvent.click(screen.getByRole('tab', { name: 'Last 7 days' }));
        await waitFor(() =>
            expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100, instance: 'mc-1', since: day(20) }),
        );
        fireEvent.click(screen.getByRole('tab', { name: 'Custom' }));
        pickDate(screen.getByLabelText('From'), '2026-10-01');
        pickDate(screen.getByLabelText('To'), '2026-10-03');
        await waitFor(() =>
            expect(window.api.auditList).toHaveBeenLastCalledWith({
                limit: 100,
                instance: 'mc-1',
                since: day(1),
                until: day(4), // 끝 날짜는 그날 끝까지
            }),
        );
    });

    it('보고 있는 기간과 서버 그대로 내보내고 건수를 알린다', async () => {
        vi.useFakeTimers({ toFake: ['Date'] });
        vi.setSystemTime(new Date(2026, 9, 26, 15, 30));
        window.showToast = vi.fn();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] }) as never;
        window.api.auditExport = vi.fn().mockResolvedValue({ success: true, count: 42 }) as never;
        render(<AuditLog instanceId="mc-1" />);
        await screen.findByText('Command · say hi');
        fireEvent.click(screen.getByRole('tab', { name: 'Today' }));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Export' })));
        expect(window.api.auditExport).toHaveBeenCalledWith({
            instance: 'mc-1',
            since: new Date(2026, 9, 26).getTime() / 1000,
        });
        expect(vi.mocked(window.showToast).mock.calls[0].slice(0, 2)).toEqual(['42 records exported', 'success']);
    });

    it('저장 창을 닫으면 아무것도 알리지 않는다', async () => {
        window.showToast = vi.fn();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] }) as never;
        window.api.auditExport = vi.fn().mockResolvedValue({ canceled: true }) as never;
        render(<AuditLog />);
        await screen.findByText(/Minecraft/);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Export' })));
        expect(window.api.auditExport).toHaveBeenCalledWith({});
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('내보내기 실패는 데몬의 영문 오류 대신 번역된 문구로 알린다', async () => {
        window.showToast = vi.fn();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] }) as never;
        window.api.auditExport = vi.fn().mockResolvedValue({
            success: false,
            error: 'Could not write the audit export: denied',
            error_code: 'WRITE_FAILED',
        }) as never;
        render(<AuditLog />);
        await screen.findByText(/Minecraft/);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Export' })));
        const [message, type] = vi.mocked(window.showToast).mock.calls[0];
        expect(type).toBe('error');
        expect(message).toContain(i18n.t('gui:data_transfer.err_write_failed'));
        expect(message).not.toContain('Could not write');
    });
    it('거르기: 시작·정지·재시작, 크래시, 그 외 명령, 실패', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({
            entries: [
                entry({ action: 'start', command: undefined, args: undefined }),
                entry({
                    action: 'crash',
                    source: 'daemon',
                    command: undefined,
                    args: undefined,
                    ok: false,
                    code: 'crash_file',
                }),
                entry({ command: 'kick', args: undefined, ok: false, status: 400 }),
                entry(),
            ],
        }) as never;
        render(<AuditLog instanceId="mc-1" />);
        await screen.findByText('Command · say hi');
        const rows = () => document.querySelectorAll('.audit-list li').length;
        expect(rows()).toBe(4);
        expect(screen.getByText('Crash report file created')).toBeTruthy();
        // 거르기 단추를 열고 체크하면 고른 것 중 하나라도 드는 기록만 (여럿 고를 수 있다)
        fireEvent.click(screen.getByRole('button', { name: 'Filter' }));
        const check = async (name: string, n: number) => {
            fireEvent.click(screen.getByRole('checkbox', { name }));
            await waitFor(() => expect(rows()).toBe(n));
        };
        await check('Start · Stop · Restart', 1);
        await check('Crashes', 2);
        expect(screen.getByText('Crashed')).toBeTruthy();
        await check('Start · Stop · Restart', 1); // 다시 누르면 뺀다
        await check('Failed', 2);
        expect(screen.getByText('Command · kick')).toBeTruthy();
        expect(screen.getByRole('button', { name: /Filters*2/ })).toBeTruthy(); // 고른 수
        fireEvent.click(screen.getByRole('button', { name: 'Show all' }));
        await waitFor(() => expect(rows()).toBe(4));
        await check('Other commands', 2);
        expect(window.api.auditList).toHaveBeenCalledTimes(1); // 거르기는 다시 부르지 않는다
    });

    it('한 줄을 누르면 자세한 칸을 펼치고 다시 누르면 접는다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({
            entries: [entry({ command: 'give', args: '{"player":"Steve","item":{"id":"diamond"}}', userName: 'Aoba' })],
        }) as never;
        render(<AuditLog instanceId="mc-1" />);
        const row = await screen.findByRole('button', { name: /Command · give/ });
        expect(row.getAttribute('aria-expanded')).toBe('false');
        fireEvent.click(row);
        expect(row.getAttribute('aria-expanded')).toBe('true');
        const detail = document.querySelector('.audit-detail') as HTMLElement;
        expect(within(detail).getByText('player: Steve')).toBeTruthy();
        expect(within(detail).getByText('item: diamond')).toBeTruthy();
        expect(within(detail).getByText('Minecraft · mc-1')).toBeTruthy();
        expect(within(detail).getByText('Aoba · aoba')).toBeTruthy();
        expect(detail.textContent).not.toMatch(/[{}"]/); // JSON 기호 없이
        fireEvent.click(row);
        expect(document.querySelector('.audit-detail')).toBeNull();
    });

    it('작업 기록 페이지: 왼쪽 서버를 누르면 그 서버 기록만, 서버 밖 설정 기록은 숨긴다', async () => {
        useServerStore.setState({ servers: [{ id: 'mc-1', name: 'Minecraft', module: 'minecraft' }] as Server[] });
        window.api.auditList = vi.fn().mockResolvedValue({
            entries: [
                entry(),
                entry({
                    action: 'discord.settings.set',
                    instanceId: '',
                    instance: undefined,
                    command: undefined,
                    args: undefined,
                }),
            ],
        }) as never;
        render(<ActivityPage />);
        expect(await screen.findByText('Command · say hi')).toBeTruthy();
        expect(screen.queryByText(/discord.settings.set/)).toBeNull();
        expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100 });
        fireEvent.click(screen.getByRole('button', { name: 'Minecraft' }));
        await waitFor(() => expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100, instance: 'mc-1' }));
        expect(screen.getByRole('button', { name: 'Minecraft' }).getAttribute('aria-current')).toBe('page');
        fireEvent.click(screen.getByRole('button', { name: 'All servers' }));
        await waitFor(() => expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100 }));
    });
});
