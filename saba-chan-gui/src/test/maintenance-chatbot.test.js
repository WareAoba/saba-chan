import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChatbotGuildTab } from '../components/discord/ChatbotGuildTab';
import { ChatbotView, relayErrorLines, settingsDraft, settingsPatch } from '../components/discord/ChatbotView';
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
    'chatbotCredit',
    'chatbotMemoTemplate',
    'chatbotTools',
];

/** 릴레이가 주는 도구 목록 (GET /api/relay/node/:guild/chatbot/tools) */
const CATALOG = {
    categories: ['game_servers', 'games', 'info', 'create'],
    tools: [
        { name: 'game_servers', category: 'game_servers', available: true },
        { name: 'lol', category: 'games', available: true },
        { name: 'pubg', category: 'games', available: false, unavailable: 'operator_key' },
        { name: 'maplestory', category: 'games', available: true },
        { name: 'web_search', category: 'info', available: true },
        { name: 'research', category: 'info', available: true },
        { name: 'images', category: 'create', available: true },
    ],
};

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
        // 저장 명령이 없는 모듈은 저장도 숨긴다
        expect(tabs).toEqual(['Start', 'Stop', 'Restart', 'Update']);
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
        const p = settingsPatch({ ...d, allowedTools: ['lol'] });
        expect(p).toMatchObject({ model: 'gpt-x', persona: null, memory: null, allowedTools: ['lol'] });
        expect(Object.keys(p)).not.toContain('sessionSeconds');
        // 켜기·도구는 서버마다 정하므로 공통으로 보내지 않는다
        expect(Object.keys(p)).not.toContain('enabled');
        expect(Object.keys(p)).not.toContain('disabledTools');
        expect(settingsPatch(d).allowedTools).toBeNull();
    });

    it('추론 강도·끼어들기·확인 단계도 기본값이면 null, 게임 서버 도구도 끌 수 있다', () => {
        const d = settingsDraft();
        expect(d).toMatchObject({
            effort: 'medium',
            ambient: true,
            ambientMinutes: 3,
            ambientEffort: 'low',
            confirmLevel: 'all',
        });
        const p = settingsPatch({ ...d, effort: 'high', ambient: false, confirmLevel: 'caution' });
        expect(p).toMatchObject({ effort: 'high', ambient: false, confirmLevel: 'caution', ambientMinutes: null });
        expect(p).not.toHaveProperty('confirmCommands');
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

    it('전체 허용 도구는 카테고리별로, 막으면 목록을 보내고 모두 허용하면 다시 null', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotTools = vi.fn().mockResolvedValue(CATALOG);
        render(<ChatbotView nodes={nodes} />);
        const images = await screen.findByLabelText('Image generation');
        expect(window.api.chatbotTools).toHaveBeenCalledWith('g1');
        expect(screen.getByText('Games')).toBeTruthy();
        expect(screen.getByText('The Saba-chan server cannot use this tool right now.')).toBeTruthy();
        expect(images.checked).toBe(true);
        fireEvent.click(images);
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await waitFor(() => expect(window.api.chatbotSave).toHaveBeenCalled());
        expect(window.api.chatbotSave.mock.calls[0][0]).toEqual({
            settings: expect.objectContaining({
                allowedTools: ['game_servers', 'lol', 'pubg', 'maplestory', 'web_search', 'research'],
            }),
        });
        // 다시 허용하면 기본(모두 허용)으로
        fireEvent.click(screen.getByLabelText('Image generation'));
        expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    });

    it('도구 목록을 못 불러오면 알리고 다시 묻는다; 서버가 없으면 연결을 안내한다', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotTools = vi.fn().mockResolvedValueOnce({ error: 'NODE_NOT_FOUND' }).mockResolvedValue(CATALOG);
        const { unmount } = render(<ChatbotView nodes={nodes} />);
        expect(await screen.findByText('Could not load the tool list.')).toBeTruthy();
        fireEvent.click(screen.getAllByRole('button', { name: 'Retry' }).at(-1));
        expect(await screen.findByLabelText('Image generation')).toBeTruthy();
        unmount();
        render(<ChatbotView nodes={[]} />);
        expect(await screen.findByText('Connect a Discord server to load the tool list.')).toBeTruthy();
    });

    it('추론 강도·끼어들기·실행 전 확인을 고르고, 끼어들기를 끄면 주기와 판단 강도는 잠긴다', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        render(<ChatbotView nodes={nodes} />);
        pickOption(await screen.findByLabelText('Reasoning effort'), 'High');
        pickOption(screen.getByLabelText('Check interval'), 'Every 10 min');
        pickOption(screen.getByLabelText('Ask before running'), 'Dangerous only');
        fireEvent.click(screen.getByLabelText('Join in uninvited'));
        expect(screen.getByLabelText('Check interval')).toBeDisabled();
        expect(screen.getByLabelText('Join-in reasoning effort')).toBeDisabled();
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await waitFor(() => expect(window.api.chatbotSave).toHaveBeenCalled());
        expect(window.api.chatbotSave.mock.calls[0][0].settings).toMatchObject({
            effort: 'high',
            ambient: false,
            ambientMinutes: 10,
            ambientEffort: null,
            confirmLevel: 'danger',
        });
    });

    it('비어 있는 서버 프롬프트는 기본 틀을 앱 언어로 받아 채울 수 있다', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotMemoTemplate = vi
            .fn()
            .mockResolvedValue({ success: true, locale: 'en', template: '## Room vibe\n<!-- guide -->\n' });
        render(<ChatbotGuildTab node={nodes[0]} />);
        const memo = await screen.findByLabelText('Server prompt for My Guild');
        fireEvent.click(screen.getByRole('button', { name: 'Insert template' }));
        await waitFor(() => expect(memo.value).toBe('## Room vibe\n<!-- guide -->\n'));
        expect(window.api.chatbotMemoTemplate).toHaveBeenCalledWith('en');
        // 채운 뒤에는 버튼이 사라진다 (쓴 글을 덮지 않게)
        expect(screen.queryByRole('button', { name: 'Insert template' })).toBeNull();
    });

    it('남은 크레딧: 잔액을 주는 제공사는 잔액, 아니면 안내와 이번 달 사용량, 소진을 알렸으면 경고', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotCredit = vi.fn().mockResolvedValue({
            provider: 'deepseek',
            balanceSupported: true,
            balance: { amount: 12.5, currency: 'USD' },
            usage: { month: '2026-10', calls: 3, input: 12000, cached: 4000, output: 800 },
            exhaustedAt: null,
        });
        const { unmount } = render(<ChatbotView nodes={nodes} />);
        expect(await screen.findByText('$12.50')).toBeTruthy();
        expect(screen.getByText(/12,000/)).toBeTruthy();
        expect(window.api.chatbotCredit).toHaveBeenCalledWith('g1');
        unmount();

        window.api.chatbotCredit = vi.fn().mockResolvedValue({
            provider: 'openai',
            balanceSupported: false,
            balance: null,
            usage: { month: '2026-10', calls: 0, input: 0, cached: 0, output: 0 },
            exhaustedAt: '2026-10-28T01:00:00.000Z',
        });
        render(<ChatbotView nodes={nodes} />);
        expect(await screen.findByText(/OpenAI does not report the balance/)).toBeTruthy();
        expect(screen.getByText(/Credit ran out/)).toBeTruthy();
    });

    it('남은 크레딧: 못 불러온 서버도 줄로 보이고, 키를 지우면 예전 잔액을 치운다', async () => {
        window.api.chatbotGet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: { g2: { memo: 'x' } }, hasApiKey: true });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: false });
        window.api.chatbotCredit = vi.fn((id) =>
            id === 'g1'
                ? Promise.resolve({
                      provider: 'deepseek',
                      balanceSupported: true,
                      balance: { amount: 12.5, currency: 'USD' },
                      usage: { month: '2026-10', calls: 1, input: 10, cached: 0, output: 1 },
                      exhaustedAt: null,
                  })
                : Promise.resolve({ success: false, error: 'RATE_LIMITED' }),
        );
        render(<ChatbotView nodes={nodes} />);
        expect(await screen.findByText('$12.50')).toBeTruthy();
        expect(screen.getByText('Could not load this server.')).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: 'Remove key' }));
        fireEvent.click(screen.getByRole('button', { name: 'Click to remove' }));
        await waitFor(() => expect(screen.queryByText('$12.50')).toBeNull());
    });

    it('목록에 없는 끼어들기 주기(CLI로 저장한 값)도 고른 값으로 보인다', async () => {
        window.api.chatbotGet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: { ambientMinutes: 15 }, guilds: {}, hasApiKey: true });
        render(<ChatbotView nodes={nodes} />);
        expect((await screen.findByLabelText('Check interval')).textContent).toContain('Every 15 min');
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

    it('서버 탭: 새 서버는 꺼져 있고, 켜기·도구·프롬프트를 그 서버에만 저장한다; 공통에서 막힌 도구는 잠근다', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({
            success: true,
            settings: { allowedTools: ['lol', 'web_search'] },
            guilds: {},
            guildDefault: { enabled: false, tools: [] },
            hasApiKey: true,
        });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        window.api.chatbotTools = vi.fn().mockResolvedValue(CATALOG);
        render(<ChatbotGuildTab node={nodes[0]} />);
        const on = await screen.findByLabelText('Use Talking Saba-chan in this server');
        expect(on.checked).toBe(false);
        const lol = await screen.findByLabelText('LoL stats');
        expect(lol.checked).toBe(false);
        expect(screen.getByLabelText('Image generation')).toBeDisabled();
        expect(screen.getAllByText('This tool is blocked in the shared settings.').length).toBeGreaterThan(0);
        fireEvent.click(on);
        fireEvent.click(lol);
        const memo = screen.getByLabelText('Server prompt for My Guild');
        fireEvent.change(memo, { target: { value: 'x'.repeat(8001) } });
        expect(screen.getByText('8,001 / 8,000')).toBeTruthy();
        expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
        fireEvent.change(memo, { target: { value: '반말로 대답해' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await waitFor(() =>
            expect(window.api.chatbotSave).toHaveBeenCalledWith({
                guilds: { g1: { enabled: true, tools: ['lol'], memo: '반말로 대답해' } },
            }),
        );
    });

    it('서버 탭: 옛 공통 설정이 남아 있으면 그 값(guildDefault)으로 보이고, 릴레이 400과 기억 지우기', async () => {
        window.api.chatbotGet = vi.fn().mockResolvedValue({
            success: true,
            settings: {},
            guilds: {},
            guildDefault: { enabled: true, tools: ['game_servers', 'web_search'] },
            hasApiKey: false,
        });
        window.api.chatbotSave = vi.fn().mockResolvedValue({
            success: false,
            guildId: 'g1',
            status: 400,
            error: 'invalid_settings',
            details: ['tools too long'],
        });
        window.api.chatbotClearMemory = vi.fn().mockResolvedValue({ success: true });
        window.api.chatbotTools = vi.fn().mockResolvedValue(CATALOG);
        render(<ChatbotGuildTab node={nodes[0]} />);
        expect((await screen.findByLabelText('Use Talking Saba-chan in this server')).checked).toBe(true);
        expect((await screen.findByLabelText('Web search')).checked).toBe(true);
        expect(screen.getByText(/Add a key/)).toBeTruthy();
        fireEvent.click(screen.getByLabelText('Web search'));
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await screen.findByText(/tools too long/);
        expect(window.api.chatbotSave).toHaveBeenCalledWith({
            guilds: { g1: { enabled: true, tools: ['game_servers'], memo: null } },
        });

        fireEvent.click(screen.getByRole('button', { name: 'Clear memory' }));
        fireEvent.click(screen.getByRole('button', { name: 'Click to clear' }));
        await waitFor(() => expect(window.api.chatbotClearMemory).toHaveBeenCalledWith('g1'));
    });
});
