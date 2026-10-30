import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AuditLog } from '../components/servers/AuditTab';
import i18n from '../i18n';
import { pickDate } from './picker';

const entry = (over) => ({
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
        delete window.api.auditList;
        delete window.api.auditExport;
        vi.useRealTimers();
    });

    it('서버별로 누가·무엇을·어디서·성공 여부를 보이고, 꽉 차면 더 불러온다', async () => {
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
        }));
        render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Command · say hi')).toBeTruthy();
        expect(screen.getByText('Discord · aoba · Village')).toBeTruthy();
        expect(screen.getAllByText('Failed (500)').length).toBe(99);
        expect(screen.queryByText('Minecraft')).toBeNull(); // 서버별 화면에는 서버 이름을 적지 않는다
        expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 100, instance: 'mc-1' });
        fireEvent.click(screen.getByRole('button', { name: 'Show more' }));
        await waitFor(() => expect(window.api.auditList).toHaveBeenLastCalledWith({ limit: 200, instance: 'mc-1' }));
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
        });
        render(<AuditLog />);
        expect(await screen.findByText('Remote daemon · Game PC · POST /api/instance/x/start')).toBeTruthy();
    });

    it('전체 기록은 서버 이름을 적고, 적게 오면 더 보기가 없다', async () => {
        window.api.auditList = vi
            .fn()
            .mockResolvedValue({ entries: [entry({ source: 'gui', user: undefined, guild: undefined })] });
        render(<AuditLog />);
        expect(await screen.findByText(/Minecraft/)).toBeTruthy();
        expect(screen.getByText('App')).toBeTruthy();
        expect(window.api.auditList).toHaveBeenCalledWith({ limit: 100 });
        expect(screen.queryByRole('button', { name: 'Show more' })).toBeNull();
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
        });
        render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Backup')).toBeTruthy();
        expect(screen.getByText('Skipped (server off)')).toBeTruthy();
        expect(screen.queryByText('Failed (409)')).toBeNull();
    });

    it('스크롤해도 새로고침·기간·더 보기는 목록 밖에 있어 늘 누를 수 있다', async () => {
        window.api.auditList = vi.fn(async ({ limit }) => ({ entries: Array.from({ length: limit }, () => entry()) }));
        render(<AuditLog />);
        await screen.findAllByText('Command · say hi');
        const scroll = document.querySelector('.audit-scroll');
        expect(scroll.querySelector('.audit-list')).not.toBeNull();
        for (const name of ['Refresh', 'Show more', 'All']) {
            expect(scroll.contains(screen.getByRole(name === 'All' ? 'tab' : 'button', { name }))).toBe(false);
        }
    });

    it('기간을 고르면 그 기간만 처음부터 다시 불러온다', async () => {
        vi.useFakeTimers({ toFake: ['Date'] });
        vi.setSystemTime(new Date(2026, 9, 26, 15, 30));
        const day = (d) => new Date(2026, 9, d).getTime() / 1000;
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] });
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
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] });
        window.api.auditExport = vi.fn().mockResolvedValue({ success: true, count: 42 });
        render(<AuditLog instanceId="mc-1" />);
        await screen.findByText('Command · say hi');
        fireEvent.click(screen.getByRole('tab', { name: 'Today' }));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Export' })));
        expect(window.api.auditExport).toHaveBeenCalledWith({
            instance: 'mc-1',
            since: new Date(2026, 9, 26).getTime() / 1000,
        });
        expect(window.showToast.mock.calls[0].slice(0, 2)).toEqual(['Exported 42 records.', 'success']);
    });

    it('저장 창을 닫으면 아무것도 알리지 않는다', async () => {
        window.showToast = vi.fn();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] });
        window.api.auditExport = vi.fn().mockResolvedValue({ canceled: true });
        render(<AuditLog />);
        await screen.findByText(/Minecraft/);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Export' })));
        expect(window.api.auditExport).toHaveBeenCalledWith({});
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('내보내기 실패는 데몬의 영문 오류 대신 번역된 문구로 알린다', async () => {
        window.showToast = vi.fn();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [entry()] });
        window.api.auditExport = vi.fn().mockResolvedValue({
            success: false,
            error: 'Could not write the audit export: denied',
            error_code: 'WRITE_FAILED',
        });
        render(<AuditLog />);
        await screen.findByText(/Minecraft/);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Export' })));
        const [message, type] = window.showToast.mock.calls[0];
        expect(type).toBe('error');
        expect(message).toContain(i18n.t('gui:data_transfer.err_write_failed'));
        expect(message).not.toContain('Could not write');
    });
});
