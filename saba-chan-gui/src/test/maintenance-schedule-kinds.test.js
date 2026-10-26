/**
 * 예약 작업: 매일·N일마다·요일·특정 날짜와 시작·정지·저장·게임 명령.
 * 게임 명령은 따로 띄우는 목록에서 고르고 입력을 채운다. 새로 넣은 위험 명령은 한 번 더 눌러 저장한다.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { draftProblems, fromDraft, MaintenanceTab, toDraft } from '../components/servers/MaintenanceTab';

const t = (k) => i18n.t(`gui:${k}`);
const server = { id: 'srv1', status: 'running' };
const commands = [
    {
        name: 'say',
        label: 'Say',
        risk: 'safe',
        inputs: [{ name: 'message', label: 'Message', type: 'text', required: true }],
    },
    { name: 'wipe', label: 'Wipe world', risk: 'danger', inputs: [] },
];

afterEach(() => {
    delete window.api.instanceScheduleGet;
    delete window.api.instanceScheduleSave;
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
        expect(fromDraft(toDraft(cli), { announce: true, inputs: [{ name: 'radius', type: 'number' }] })).toEqual(cli);
        const d = { ...toDraft(tasks[0]), weekdays: ['sat', 'mon'] };
        expect(fromDraft(d, { announce: true }).weekdays).toEqual(['mon', 'sat']);
        // 공지 명령이 없으면 예고를 보내지 않는다
        expect(fromDraft(toDraft(tasks[0]), { announce: false }).warn_minutes).toBeUndefined();
    });

    it('빈 요일·날짜·명령과 필수 입력을 잡는다', () => {
        const tr = (k) => k;
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
        expect(window.api.instanceScheduleSave.mock.calls[0][1].schedule[1]).toEqual({
            action: 'command',
            enabled: true,
            every_minutes: 60,
            command: 'say',
            args: { message: 'hello' },
        });

        // 위험 명령: 첫 누름은 확인만 한다
        await waitFor(() => expect(window.api.instanceScheduleGet).toHaveBeenCalledTimes(2));
        fireEvent.click(await screen.findByRole('button', { name: /^\s*Say$/ }));
        const again = await screen.findByRole('dialog', { name: t('maintenance.command_pick') });
        fireEvent.click(within(again).getByRole('button', { name: /Wipe world/ }));
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        await screen.findByText(t('maintenance.danger_confirm'));
        expect(window.api.instanceScheduleSave).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByRole('button', { name: t('maintenance.danger_save') }));
        await waitFor(() => expect(window.api.instanceScheduleSave).toHaveBeenCalledTimes(2));
        expect(window.api.instanceScheduleSave.mock.calls[1][1].schedule[1].command).toBe('wipe');
    });
});
