import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    CHATBOT_TOOLS,
    ChatbotView,
    relayErrorLines,
    settingsDraft,
    settingsPatch,
} from '../components/discord/ChatbotView';
import { draftProblems, formatBytes, fromDraft, MaintenanceTab, toDraft } from '../components/servers/MaintenanceTab';
import { pickOption } from './dropdown';

const t = (k) => k;
const API_KEYS = [
    'instanceScheduleGet',
    'instanceScheduleSave',
    'instanceBackups',
    'instanceBackupCreate',
    'instanceBackupDelete',
    'instanceBackupRestore',
    'chatbotGet',
    'chatbotSave',
    'chatbotClearMemory',
];

afterEach(() => {
    for (const k of API_KEYS) delete window.api[k];
});

describe('예약 작업 초안 ↔ 데몬 형식', () => {
    it('주기·시각·예고·공지를 데몬 ScheduledTask로 바꾼다', () => {
        const restart = toDraft({ action: 'restart', at: '04:00', warn_minutes: [10, 1] });
        expect(fromDraft(restart, { announce: true })).toEqual({
            action: 'restart',
            enabled: true,
            at: '04:00',
            warn_minutes: [10, 1],
        });
        // 공지 명령이 없는 모듈이면 예고를 보내지 않는다 (데몬이 거절한다)
        expect(fromDraft(restart, { announce: false })).not.toHaveProperty('warn_minutes');
        const backup = toDraft({ action: 'backup', every_minutes: 30, enabled: false });
        expect(fromDraft(backup, { announce: true })).toEqual({ action: 'backup', enabled: false, every_minutes: 30 });
    });

    it('데몬 problems()와 같은 규칙으로 막는다', () => {
        expect(draftProblems({ ...toDraft({ action: 'backup' }), every: '0' }, t)).toEqual(['maintenance.err_every']);
        expect(draftProblems({ ...toDraft({ action: 'restart', at: '4pm' }) }, t)).toEqual(['maintenance.err_at']);
        expect(draftProblems(toDraft({ action: 'announce', every_minutes: 60 }), t)).toEqual([
            'maintenance.err_message',
        ]);
        expect(formatBytes(1536)).toBe('1.5 KB');
    });
});

describe('MaintenanceTab', () => {
    const stopped = { id: 'srv1', status: 'stopped' };

    it('백업을 지원하지 않으면 백업 칸과 백업·공지 선택지를 숨긴다', async () => {
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({
            success: true,
            schedule: [{ action: 'restart', every_minutes: 60 }],
            backup_keep: 10,
            backup_supported: false,
            announce_supported: false,
        });
        window.api.instanceBackups = vi.fn();
        render(<MaintenanceTab server={stopped} />);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        expect(screen.queryByText('Back up now')).toBeNull();
        expect(window.api.instanceBackups).not.toHaveBeenCalled();
        const tabs = screen.getAllByRole('tab').map((b) => b.textContent);
        expect(tabs).toEqual(['Restart', 'Update']);
        expect(screen.queryByText(/Warn players/)).toBeNull();
    });

    it('백업 목록·복원(두 번 누름)·저장 오류 표시', async () => {
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({
            success: true,
            schedule: [],
            backup_keep: 5,
            backup_supported: true,
            announce_supported: true,
        });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            success: true,
            backups: [{ name: '20261017-0400-scheduled.zip', kind: 'scheduled', created: 1760670000, size: 2048 }],
        });
        window.api.instanceBackupRestore = vi.fn().mockResolvedValue({ success: true, restored: true });
        window.api.instanceScheduleSave = vi.fn().mockResolvedValue({
            success: false,
            error_code: 'validation_failed',
            details: ['schedule[0]: announce needs a message'],
        });
        render(<MaintenanceTab server={stopped} />);
        const restore = await screen.findByRole('button', { name: 'Restore' });
        fireEvent.click(restore);
        expect(window.api.instanceBackupRestore).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole('button', { name: 'Click to restore' }));
        await waitFor(() =>
            expect(window.api.instanceBackupRestore).toHaveBeenCalledWith('srv1', '20261017-0400-scheduled.zip'),
        );

        fireEvent.click(screen.getByRole('button', { name: /Add task/ }));
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        await screen.findByText('schedule[0]: announce needs a message');
        expect(window.api.instanceScheduleSave).toHaveBeenCalledWith('srv1', {
            schedule: [{ action: 'backup', enabled: true, every_minutes: 60 }],
            backup_keep: 5,
        });
    });

    // 증분 백업(MK16): size는 담은 내용 크기, stored는 그 백업이 새로 디스크에 쓴 크기. 다르면 둘 다 보인다
    it('백업이 새로 저장한 크기가 담은 크기와 다르면 함께 보인다', async () => {
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValue({ success: true, schedule: [], backup_supported: true });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            success: true,
            backups: [
                { name: 'b-manual', kind: 'manual', created: 2, size: 120 * 1024 * 1024, stored: 2 * 1024 * 1024 },
                { name: 'a-manual.zip', kind: 'manual', created: 1, size: 2048, stored: 2048 },
            ],
        });
        render(<MaintenanceTab server={stopped} />);
        expect(await screen.findByText(/120(\.0)? MB · 2(\.0)? MB new/)).toBeTruthy();
        expect(screen.getByText(/2(\.0)? KB · a-manual\.zip/).textContent).not.toMatch(/new/);
    });

    // 데몬 #533: zip으로 열리지 않는 백업은 표시하고 지우기를 권한다. 복원은 막는다 (옛 데몬은 readable이 없다)
    it('열 수 없는 백업은 표시하고 복원을 막는다', async () => {
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValue({ success: true, schedule: [], backup_supported: true });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            success: true,
            backups: [
                { name: 'b-manual.zip', kind: 'manual', created: 2, size: 9, stored: 9, readable: false },
                { name: 'a-manual.zip', kind: 'manual', created: 1, size: 2048, stored: 2048 },
            ],
        });
        render(<MaintenanceTab server={stopped} />);
        const bad = await screen.findByText(/cannot be opened/);
        expect(bad.closest('li').textContent).toMatch(/b-manual\.zip/);
        const restores = screen.getAllByRole('button', { name: 'Restore' });
        expect(restores[0].disabled).toBe(true);
        expect(restores[0].title).toMatch(/delete/i);
        expect(restores[1].disabled).toBe(false);
    });

    it('복원·백업 거절은 데몬의 영어 원문 대신 이유를 알린다', async () => {
        window.showToast = vi.fn();
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValue({ success: true, schedule: [], backup_supported: true });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            backups: [{ name: 'a-manual.zip', kind: 'manual', created: 1, size: 1 }],
        });
        window.api.instanceBackupRestore = vi.fn().mockResolvedValue({
            success: false,
            error: 'This backup points outside the server folders',
            error_code: 'restore_outside',
        });
        window.api.instanceBackupCreate = vi.fn().mockResolvedValue({
            success: false,
            error: 'A backup or restore is already running for this instance',
            error_code: 'backup_busy',
        });
        render(<MaintenanceTab server={stopped} />);
        fireEvent.click(await screen.findByRole('button', { name: 'Restore' }));
        fireEvent.click(screen.getByRole('button', { name: 'Click to restore' }));
        await waitFor(() => expect(window.showToast).toHaveBeenCalled());
        expect(window.showToast.mock.calls[0][0]).toMatch(/server folder/);
        expect(window.showToast.mock.calls[0][0]).not.toMatch(/points outside/);

        fireEvent.click(await screen.findByRole('button', { name: /Back up now/ }));
        await waitFor(() => expect(window.showToast).toHaveBeenCalledTimes(2));
        expect(window.showToast.mock.calls[1][0]).not.toMatch(/already running for this instance/);
        delete window.showToast;
    });

    it('되살리지 못한 링크가 있으면 복원 뒤 그 수를 경고로 알린다 (D-B14)', async () => {
        window.showToast = vi.fn();
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValue({ success: true, schedule: [], backup_supported: true });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            backups: [{ name: 'a-manual.zip', kind: 'manual', created: 1, size: 1 }],
        });
        window.api.instanceBackupRestore = vi
            .fn()
            .mockResolvedValue({ success: true, restored: ['/w'], pre_restore_backup: 'p.zip', links_not_restored: 2 });
        render(<MaintenanceTab server={stopped} />);
        fireEvent.click(await screen.findByRole('button', { name: 'Restore' }));
        fireEvent.click(screen.getByRole('button', { name: 'Click to restore' }));
        await waitFor(() => expect(window.showToast).toHaveBeenCalled());
        const [text, tone] = window.showToast.mock.calls[0];
        expect(tone).toBe('warning');
        expect(text).toMatch(/2/);
        expect(text).toMatch(/link/i);
        delete window.showToast;
    });

    it('켜져 있으면 복원 버튼이 잠긴다', async () => {
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValue({ success: true, schedule: [], backup_supported: true });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            backups: [{ name: 'a-manual.zip', kind: 'manual', created: 1, size: 1 }],
        });
        render(<MaintenanceTab server={{ id: 's', status: 'running' }} />);
        expect(await screen.findByRole('button', { name: 'Restore' })).toBeDisabled();
    });

    it('게임 자동 백업: 보관 버튼은 스냅샷 원본으로 만들고, 목록의 게임 백업은 되돌리기만 된다', async () => {
        window.api.instanceScheduleGet = vi
            .fn()
            .mockResolvedValue({ success: true, schedule: [], backup_supported: true });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            snapshots_supported: true,
            backups: [{ name: 'game-snapshot:S1', kind: 'game-snapshot', created: 1, readable: true }],
        });
        window.api.instanceBackupCreate = vi
            .fn()
            .mockResolvedValue({ success: true, backup: { name: 'x-snapshot.zip' } });
        render(<MaintenanceTab server={stopped} />);
        expect(await screen.findByText(/Made by the game/)).toBeInTheDocument();
        expect(screen.getByRole('button', { name: 'Restore' })).toBeEnabled();
        expect(screen.queryByRole('button', { name: 'Delete' })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /Keep game backup/ }));
        await waitFor(() =>
            expect(window.api.instanceBackupCreate).toHaveBeenCalledWith('srv1', { source: 'snapshot' }),
        );
    });
});

describe('챗봇 설정 변환', () => {
    it('기본값과 같은 값은 null로 보내 릴레이 기본을 따른다; 계약에서 빠진 키는 보내지 않는다', () => {
        const d = settingsDraft({ model: 'gpt-x', ambientSeconds: 30, sessionSeconds: 60, triggerWords: ['a'] });
        expect(d).not.toHaveProperty('ambientSeconds');
        expect(d).not.toHaveProperty('triggerWords');
        const p = settingsPatch({ ...d, enabled: true, disabledTools: ['images', 'lol'] });
        expect(p).toMatchObject({ model: 'gpt-x', enabled: true, persona: null, memory: null });
        expect(Object.keys(p)).not.toContain('sessionSeconds');
        // 계약 순서로 정리, 비면 null
        expect(p.disabledTools).toEqual(['lol', 'images']);
        expect(settingsPatch(d).disabledTools).toBeNull();
    });

    it('눈치 메모는 끌 수 없는 기본 사양이라 도구 목록에 없고, 예전에 꺼 둔 값은 다음 저장 때 지운다', () => {
        expect(CHATBOT_TOOLS).not.toContain('social_notes');
        expect(settingsPatch({ ...settingsDraft(), disabledTools: ['social_notes'] }).disabledTools).toBeNull();
    });

    it('릴레이 400을 읽을 수 있는 줄로', () => {
        const lines = relayErrorLines(
            { guildId: 'g1', status: 400, error: 'invalid_settings', details: [{ path: 'model', message: 'bad' }] },
            () => 'My Guild',
        );
        expect(lines).toEqual(['My Guild: invalid_settings', 'model: bad']);
    });
});

describe('ChatbotView', () => {
    const nodes = [{ guildId: 'g1', guildName: 'My Guild' }];

    it('릴레이 삭제가 남아 있으면 알린다', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({
            success: true,
            settings: {},
            guilds: {},
            hasApiKey: false,
            pendingRelayDelete: true,
        });
        render(<ChatbotView nodes={nodes} />);
        expect(await screen.findByText(/retry on the next connection/)).toBeTruthy();
    });

    it('노드 목록이 설정보다 늦게 와도 깨지지 않고, 그 전에 바꾼 값은 남는다', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        const { rerender } = render(<ChatbotView nodes={[]} />);
        await screen.findByText('No Discord servers connected.');
        fireEvent.click(screen.getByLabelText('Image generation'));
        rerender(<ChatbotView nodes={nodes} />);
        const sel = await screen.findByLabelText('Talking Saba-chan in My Guild');
        expect(screen.getByLabelText('Image generation').checked).toBe(false);
        pickOption(sel, 'Off');
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await waitFor(() => expect(window.api.chatbotSave).toHaveBeenCalled());
        const patch = window.api.chatbotSave.mock.calls[0][0];
        expect(patch.guilds).toEqual({ g1: { enabled: false, memo: null } });
        expect(patch.settings.disabledTools).toEqual(['images']);
    });

    it('노드 메모는 길드별로, 글자 수를 세고 넘으면 저장을 막는다; 도구 끄기는 disabledTools', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({
            success: true,
            settings: { persona: 'old' },
            guilds: { g1: { memo: '안녕' } },
            hasApiKey: true,
        });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        render(<ChatbotView nodes={nodes} />);
        const memo = await screen.findByLabelText('Server prompt for My Guild');
        expect(memo.value).toBe('안녕');
        expect(screen.getByText('2 / 8,000')).toBeTruthy();
        fireEvent.change(memo, { target: { value: 'x'.repeat(8001) } });
        expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
        fireEvent.change(memo, { target: { value: '반말로 대답해' } });
        fireEvent.click(screen.getByLabelText('Image generation'));
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await waitFor(() => expect(window.api.chatbotSave).toHaveBeenCalled());
        const patch = window.api.chatbotSave.mock.calls[0][0];
        expect(patch.guilds.g1.memo).toBe('반말로 대답해');
        expect(patch.settings.disabledTools).toEqual(['images']);
        expect(patch.settings.persona).toBeNull();
    });

    it('API 키는 쓰기만 하고 등록 여부만 보인다, 삭제는 null', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: false });
        render(<ChatbotView nodes={nodes} />);
        expect(await screen.findByText('Set')).toBeTruthy();
        expect(screen.getByLabelText('OpenAI API key').value).toBe('');
        fireEvent.click(screen.getByRole('button', { name: 'Remove key' }));
        fireEvent.click(screen.getByRole('button', { name: 'Click to remove' }));
        await waitFor(() => expect(window.api.chatbotSave).toHaveBeenCalledWith({ apiKey: null }));
        expect(await screen.findByText('None')).toBeTruthy();
    });

    it('길드 설정을 바꿔 저장하고, 릴레이 400을 보여준다', async () => {
        window.api.chatbotGet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: false });
        window.api.chatbotSave = vi.fn().mockResolvedValue({
            success: false,
            guildId: 'g1',
            status: 400,
            error: 'invalid_settings',
            details: ['channelIds too long'],
        });
        window.api.chatbotClearMemory = vi.fn().mockResolvedValue({ success: true });
        render(<ChatbotView nodes={nodes} />);
        const sel = await screen.findByLabelText('Talking Saba-chan in My Guild');
        pickOption(sel, 'On');
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await screen.findByText(/channelIds too long/);
        expect(window.api.chatbotSave).toHaveBeenCalledWith({
            guilds: { g1: { enabled: true, memo: null } },
        });

        fireEvent.click(screen.getByRole('button', { name: 'Clear memory' }));
        fireEvent.click(screen.getByRole('button', { name: 'Click to clear' }));
        await waitFor(() => expect(window.api.chatbotClearMemory).toHaveBeenCalledWith('g1'));
    });
});
