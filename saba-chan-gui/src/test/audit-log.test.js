import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AuditLog } from '../components/servers/AuditTab';

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
            entries: [entry({ action: 'backup', source: 'schedule', command: undefined, args: undefined, ok: false, status: 409 })],
        });
        render(<AuditLog instanceId="mc-1" />);
        expect(await screen.findByText('Backup')).toBeTruthy();
        expect(screen.getByText('Skipped (server off)')).toBeTruthy();
        expect(screen.queryByText('Failed (409)')).toBeNull();
    });
});
