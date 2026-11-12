/**
 * 예약 작업: 매일·N일마다·요일·특정 날짜와 시작·정지·저장·게임 명령.
 * 게임 명령은 따로 띄우는 목록에서 고르고 입력을 채운다. 새로 넣은 위험 명령은 한 번 더 눌러 저장한다.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { TFunction } from 'i18next';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { draftProblems, fromDraft, MaintenanceTab, toDraft } from '../components/servers/MaintenanceTab';
import type { Server } from '../stores/useServerStore';
import type { CommandField } from '../types/daemon';
import { pickTime } from './picker';

const t = ((k: string) => i18n.t(`gui:${k}`)) as unknown as TFunction;
const server = { id: 'srv1', status: 'running' } as unknown as Server;
const commands = [
    {
        name: 'say',
        label: 'Say',
        risk: 'safe',
        inputs: [{ name: 'message', label: 'Message', type: 'text', required: true }],
    },
    { name: 'wipe', label: 'Wipe world', risk: 'danger', aliases: ['erase'], inputs: [] },
] as unknown as CommandField[];

afterEach(() => {
    delete (window.api as Partial<typeof window.api>).instanceScheduleGet;
    delete (window.api as Partial<typeof window.api>).instanceScheduleSave;
});

describe('예약 작업 초안', () => {
    it('날 규칙을 데몬 형식으로 오가고, 요일은 월요일부터 정렬한다', () => {
        const tasks = [
            { action: 'stop', at: '23:00', weekdays: ['mon', 'sat'], warn_minutes: [5], enabled: true },
            { action: 'start', at: '08:00', every_days: 3, start_date: '2026-10-30', enabled: true },
            { action: 'save', at: '04:00', date: '2026-12-24', enabled: false },
            { action: 'backup', at: '04:00', enabled: true },
            { action: 'command', every_minutes: 30, command: 'say', args: { message: 'hi' }, enabled: true },
        ];
        for (const task of tasks) {
            expect(fromDraft(toDraft(task), { announce: true })).toEqual(task);
        }
        // 다른 곳에서 글자로 저장한 숫자 입력은 고치지 않으면 그대로 (다음 실행·위험 확인이 같은 작업으로 본다)
        const cli = { action: 'command', at: '04:00', command: 'tp', args: { radius: '5' }, enabled: true };
        expect(
            fromDraft(toDraft(cli), { announce: true, inputs: [{ name: 'radius', type: 'number' }] } as never),
        ).toEqual(cli);
        const d = { ...toDraft(tasks[0]), weekdays: ['sat', 'mon'] };
        expect(fromDraft(d, { announce: true }).weekdays).toEqual(['mon', 'sat']);
        // 공지 명령이 없으면 예고를 보내지 않는다
        expect(fromDraft(toDraft(tasks[0]), { announce: false }).warn_minutes).toBeUndefined();
    });

    it('빈 요일·날짜·명령과 필수 입력을 잡는다', () => {
        const tr = ((k: string) => k) as unknown as TFunction;
        expect(draftProblems({ ...toDraft({ action: 'stop', at: '04:00' }), mode: 'weekdays' }, tr)).toContain(
            'maintenance.err_weekdays',
        );
        expect(draftProblems({ ...toDraft({ action: 'stop', at: '04:00' }), mode: 'date' }, tr)).toContain(
            'maintenance.err_date',
        );
        expect(
            draftProblems({ ...toDraft({ action: 'stop', at: '04:00' }), mode: 'days', everyDays: '366' }, tr),
        ).toContain('maintenance.err_every_days');
        expect(draftProblems(toDraft({ action: 'command', at: '04:00' }), tr)).toContain('maintenance.err_command');
        expect(draftProblems(toDraft({ action: 'command', at: '04:00', command: 'say' }), tr, commands[0])).toContain(
            'maintenance.err_command_input',
        );
        // 기본값이 있는 필수 인자는 비워도 데몬이 기본값을 쓴다
        const withDefault = { ...commands[0], inputs: [{ ...commands[0].inputs![0], default: 'hi' }] };
        expect(draftProblems(toDraft({ action: 'command', at: '04:00', command: 'say' }), tr, withDefault)).toEqual([]);
    });
});

describe('예약 작업 화면', () => {
    it('게임 명령을 목록에서 고르고 입력을 채워 저장하며, 위험 명령은 한 번 더 눌러야 저장한다', async () => {
        window.showToast = vi.fn();
        let current = [{ action: 'restart', at: '04:00', enabled: true }];
        window.api.instanceScheduleGet = vi.fn(async () => ({
            success: true,
            schedule: current,
            next_runs: [1_800_000_000],
            utc_offset_minutes: -new Date().getTimezoneOffset(),
            backup_keep: 5,
            revision: 'r1',
        }));
        window.api.instanceScheduleSave = vi.fn(async (_id, body) => {
            current = body.schedule;
            return { success: true };
        });
        render(<MaintenanceTab server={server} moduleCommands={commands} />);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        await act(async () => {});
        // 저장된 작업은 다음 실행 시각을 보인다
        expect(screen.getByText((text) => text.startsWith(t('maintenance.next_run').split('{{')[0]))).toBeTruthy();

        fireEvent.click(screen.getByRole('button', { name: /Add task/ }));
        const pickers = await screen.findAllByRole('button', { name: new RegExp(t('maintenance.action_command')) });
        fireEvent.click(pickers[1]);
        const dialog = await screen.findByRole('dialog', { name: t('maintenance.command_pick') });
        fireEvent.click(within(dialog).getByRole('button', { name: /Say/ }));
        await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
        fireEvent.change(screen.getByRole('textbox', { name: /Message/ }), { target: { value: 'hello' } });
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        await waitFor(() => expect(window.api.instanceScheduleSave).toHaveBeenCalledTimes(1));
        expect(vi.mocked(window.api.instanceScheduleSave).mock.calls[0][1].schedule[1]).toEqual({
            action: 'command',
            enabled: true,
            every_minutes: 60,
            command: 'say',
            args: { message: 'hello' },
        });

        // 위험 명령: 첫 누름은 확인만 한다
        await waitFor(() => expect(window.api.instanceScheduleGet).toHaveBeenCalledTimes(2));
        // 다시 읽은 결과가 작업 목록을 새로 그릴 때까지 기다린다: 그 전에 누른 버튼은 사라진다
        await act(async () => {});
        await waitFor(() => expect(screen.queryByText(t('discord.unsaved'))).toBeNull());
        fireEvent.click(await screen.findByRole('button', { name: /^\s*Say$/ }));
        const again = await screen.findByRole('dialog', { name: t('maintenance.command_pick') });
        fireEvent.click(within(again).getByRole('button', { name: /Wipe world/ }));
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        await screen.findByText(t('maintenance.danger_confirm'));
        expect(window.api.instanceScheduleSave).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByRole('button', { name: t('maintenance.danger_save') }));
        await waitFor(() => expect(window.api.instanceScheduleSave).toHaveBeenCalledTimes(2));
        expect(vi.mocked(window.api.instanceScheduleSave).mock.calls[1][1].schedule[1].command).toBe('wipe');
    });

    it('한 번 작업은 돌고 나면 끝났다고 보이고, 돈 뒤 다음 실행 시각만 다시 읽는다', async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            const now = Math.floor(Date.now() / 1000);
            const schedule = [
                { action: 'start', at: '04:00', date: '2026-10-27', enabled: true },
                { action: 'restart', at: '05:00', enabled: true },
            ];
            const reply = (runs: (number | null)[]) => ({
                success: true,
                schedule,
                next_runs: runs,
                backup_keep: 5,
                revision: 'r1',
            });
            window.api.instanceScheduleGet = vi
                .fn()
                .mockResolvedValueOnce(reply([now + 60, now + 3600]))
                .mockResolvedValue(reply([null, now + 3600]));
            render(<MaintenanceTab server={server} moduleCommands={commands} />);
            await screen.findByRole('tablist', { name: 'Task 1 type' });
            await act(async () => {});
            const prefix = t('maintenance.next_run').split('{{')[0];
            expect(screen.getAllByText((text) => text.startsWith(prefix))).toHaveLength(2);
            // 작업 바로 뒤 시각을 지나면 다음 실행만 다시 읽는다 (고치던 내용은 그대로)
            pickTime(screen.getAllByLabelText(t('maintenance.at_time'))[1], '06:00');
            await act(async () => {
                await vi.advanceTimersByTimeAsync(2 * 60 * 1000);
            });
            expect(window.api.instanceScheduleGet).toHaveBeenCalledTimes(2);
            expect(screen.getByText(t('maintenance.date_done'))).toBeTruthy();
            expect(screen.getAllByLabelText(t('maintenance.at_time'))[1].textContent).toBe('06:00');
        } finally {
            vi.useRealTimers();
        }
    });

    it('작업마다 마지막 실행 결과를 보이고, 펼치면 최근 실행 기록이 나온다', async () => {
        const now = Math.floor(Date.now() / 1000);
        window.api.instanceScheduleGet = vi.fn(async () => ({
            success: true,
            schedule: [
                { action: 'backup', at: '04:00', enabled: true },
                { action: 'restart', at: '00:00', enabled: true },
                { action: 'update', at: '06:00', weekdays: ['mon'], enabled: true },
            ],
            next_runs: [now + 3600, now + 7200, now + 9000],
            history: [
                [
                    { at: now - 3600, result: 'ok', seconds: 12 },
                    { at: now - 90000, result: 'failed', code: 'save_failed', seconds: 3 },
                    { at: now - 180000, result: 'skipped', code: 'daemon_off', seconds: 0 },
                ],
                [{ at: now - 7200, result: 'failed', code: 'not_running', seconds: 0 }],
                [],
            ],
            backup_keep: 5,
            revision: 'r1',
        }));
        render(<MaintenanceTab server={server} moduleCommands={commands} />);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        await act(async () => {});
        const tasks = screen.getAllByRole('listitem').filter((li) => li.classList.contains('mt-task'));
        const lastRun = t('maintenance.last_run').split('{{')[0];
        const head = within(tasks[0]).getByRole('button', { name: new RegExp(`^${lastRun}`) });
        expect(head.textContent).toContain(t('maintenance.run_ok'));
        expect(within(tasks[1]).getByRole('button', { name: new RegExp(`^${lastRun}`) }).textContent).toContain(
            `${t('maintenance.run_failed')} · ${t('maintenance.run_code_not_running')}`,
        );
        expect(within(tasks[2]).getByText(t('maintenance.last_run_none'))).toBeTruthy();

        fireEvent.click(head);
        const rows = within(tasks[0])
            .getAllByRole('listitem')
            .map((li) => li.textContent);
        expect(rows).toHaveLength(3);
        expect(rows[0]).toContain(`${t('maintenance.run_ok')} · 12s`);
        expect(rows[1]).toContain(t('maintenance.run_failed'));
        expect(rows[2]).toContain(`${t('maintenance.run_skipped')} · ${t('maintenance.run_code_daemon_off')}`);
    });

    it('열어 둔 탭은 예약이 끝났다는 이벤트를 받으면 마지막 실행을 다시 읽는다', async () => {
        const now = Math.floor(Date.now() / 1000);
        const reply = (history: unknown[]) => ({
            success: true,
            schedule: [{ action: 'backup', at: '04:00', enabled: true }],
            next_runs: [now + 3600],
            history: [history],
            backup_keep: 5,
            revision: 'r1',
        });
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValueOnce(reply([]))
            .mockResolvedValue(reply([{ at: now, result: 'failed', code: 'save_failed', seconds: 95 }]));
        let emit: (ev: unknown) => void = () => {};
        window.api.onDaemonEvent = vi.fn((cb) => {
            emit = cb;
            return () => {};
        });
        try {
            render(<MaintenanceTab server={server} moduleCommands={commands} />);
            await screen.findByText(t('maintenance.last_run_none'));
            await act(async () => emit({ type: 'schedule.ran', data: { instanceId: 'other', action: 'backup' } }));
            expect(window.api.instanceScheduleGet).toHaveBeenCalledTimes(1);
            await act(async () => emit({ type: 'schedule.failed', data: { instanceId: 'srv1', action: 'backup' } }));
            const lastRun = t('maintenance.last_run').split('{{')[0];
            expect((await screen.findByRole('button', { name: new RegExp(`^${lastRun}`) })).textContent).toContain(
                t('maintenance.run_failed'),
            );
        } finally {
            delete (window.api as Partial<typeof window.api>).onDaemonEvent;
        }
    });

    it('명령 목록과 입력은 모듈 locales의 문구를 쓴다', async () => {
        i18n.addResourceBundle(
            'en',
            'mod_mcl',
            {
                commands: {
                    say: { label: 'Broadcast', description: 'Tell everyone', inputs: { message: { label: 'Text' } } },
                },
            },
            true,
            true,
        );
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({
            success: true,
            schedule: [{ action: 'command', at: '04:00', command: 'say', args: { message: 'hi' }, enabled: true }],
            backup_keep: 5,
        });
        render(<MaintenanceTab server={{ ...server, module: 'mcl' }} moduleCommands={commands} />);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        await act(async () => {});
        expect(screen.getByRole('textbox', { name: /Text/ })).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: /Broadcast/ }));
        const dialog = await screen.findByRole('dialog', { name: t('maintenance.command_pick') });
        expect(within(dialog).getByText('Tell everyone')).toBeTruthy();
    });

    it('별명·대문자로 저장된 꺼진 위험 명령을 켜도 한 번 더 확인한다', async () => {
        window.showToast = vi.fn();
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({
            success: true,
            schedule: [{ action: 'command', at: '04:00', command: 'ERASE', enabled: false }],
            backup_keep: 5,
        });
        window.api.instanceScheduleSave = vi.fn().mockResolvedValue({ success: true });
        render(<MaintenanceTab server={server} moduleCommands={commands} />);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        await act(async () => {});
        expect(screen.getByText(t('maintenance.command_danger'))).toBeTruthy();
        fireEvent.click(screen.getByRole('checkbox', { name: t('maintenance.task_enabled') }));
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        await screen.findByText(t('maintenance.danger_confirm'));
        expect(window.api.instanceScheduleSave).not.toHaveBeenCalled();
    });
});
