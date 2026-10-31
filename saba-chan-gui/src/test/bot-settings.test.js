import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BotSettingsTab, botSettingsAvailable } from '../components/discord/BotSettingsTab';
import { pickOption } from './dropdown';

const flush = () => act(async () => {});
const node = { guildId: 'g1', guildName: 'Test' };
const base = {
    guildId: 'g1',
    locale: 'auto',
    sleeping: false,
    music: { enabled: true, ui: true, queueLines: 10, refreshInterval: 5000, normalize: true },
    tts: { volume: 100, speed: 100, pitch: 0 },
    sync: { ok: true },
};

describe('디스코드 봇 설정 탭', () => {
    afterEach(() => {
        delete window.api.discordSettingsGet;
        delete window.api.discordSettingsSet;
        delete window.api.chatbotGet;
        delete window.api.chatbotSave;
        window.showToast = undefined;
    });

    it('채널이 없으면 탭을 숨긴다', () => {
        window.api.discordSettingsGet = undefined;
        expect(botSettingsAvailable()).toBe(false);
    });

    it('토글은 바뀐 키만 보내고, 음악은 부분 객체로 보낸다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue(base);
        window.api.discordSettingsSet = vi.fn().mockResolvedValue({
            success: true,
            settings: base,
            sync: { ok: true },
        });
        render(<BotSettingsTab node={node} section="music" />);
        await flush();
        // 잠자기는 연결 화면의 재우기와 겹쳐서 여기 두지 않는다
        expect(screen.queryByRole('checkbox', { name: 'Sleep' })).toBeNull();

        fireEvent.click(screen.getByRole('checkbox', { name: 'Volume normalization' }));
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { music: { normalize: false } });
    });

    it('범위 밖 숫자는 보내지 않고, 새로고침 간격은 ms로 보낸다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue(base);
        window.api.discordSettingsSet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: base, sync: { ok: true } });
        render(<BotSettingsTab node={node} section="music" />);
        await flush();
        const [lines, refresh] = screen.getAllByRole('spinbutton');
        fireEvent.change(lines, { target: { value: '30' } });
        fireEvent.blur(lines);
        expect(screen.getByText('Enter a whole number from 1 to 20.')).toBeInTheDocument();
        expect(window.api.discordSettingsSet).not.toHaveBeenCalled();

        fireEvent.change(refresh, { target: { value: '3' } });
        fireEvent.blur(refresh);
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { music: { refreshInterval: 3000 } });
    });

    it('읽어주는 사바쨩: 볼륨·피치는 정수로, 속도는 배로 받아 %로 보내고, 범위 밖은 보내지 않는다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue(base);
        window.api.discordSettingsSet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: base, sync: { ok: true } });
        render(<BotSettingsTab node={node} section="tts" />);
        await flush();
        const [volume, speed, pitch] = screen.getAllByRole('spinbutton');
        expect(speed).toHaveValue(1);
        fireEvent.change(speed, { target: { value: '1.25' } });
        fireEvent.blur(speed);
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenLastCalledWith('g1', { tts: { speed: 125 } });

        fireEvent.change(pitch, { target: { value: '7' } });
        fireEvent.blur(pitch);
        fireEvent.change(volume, { target: { value: '9' } });
        fireEvent.blur(volume);
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledTimes(1);

        fireEvent.change(pitch, { target: { value: '-3' } });
        fireEvent.blur(pitch);
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenLastCalledWith('g1', { tts: { pitch: -3 } });
        // 고정 제한은 보여 주기만
        expect(screen.getByText(/400/)).toBeInTheDocument();
    });

    it('읽어주는 사바쨩 이름 부르기: 기본은 여러 명일 때, 고르면 tts.names 만 보낸다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue(base);
        window.api.discordSettingsSet = vi.fn().mockResolvedValue({ success: true, settings: base, sync: { ok: true } });
        render(<BotSettingsTab node={node} section="tts" />);
        await flush();
        expect(screen.getByRole('tab', { name: 'With several people' })).toHaveAttribute('aria-selected', 'true');
        fireEvent.click(screen.getByRole('tab', { name: 'Never' }));
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenLastCalledWith('g1', { tts: { names: 'never' } });
    });

    it('relay에 못 보냈으면 알리고, 거부되면 토스트로 알린다', async () => {
        window.showToast = vi.fn();
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, sync: { ok: false } });
        window.api.discordSettingsSet = vi.fn().mockResolvedValue({ success: false, error_code: 'not_paired' });
        render(<BotSettingsTab node={node} section="music" />);
        await flush();
        expect(screen.getByText(/not applied to Discord yet/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Turn on Singing Saba-chan' }));
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("This server isn't paired.", 'error', 6000, undefined);
    });

    it('릴레이에서 설정을 못 읽었으면(loaded:false) 알리고 바꾸지 못하게 한다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, loaded: false });
        window.api.discordSettingsSet = vi.fn();
        render(<BotSettingsTab node={node} section="music" />);
        await flush();
        expect(screen.getByText(/Couldn't read this server's settings from Server-chan/)).toBeTruthy();
        expect(screen.getByRole('checkbox', { name: 'Volume normalization' })).toBeDisabled();

        window.api.discordSettingsGet.mockResolvedValue(base);
        fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
        await flush();
        expect(screen.getByRole('checkbox', { name: 'Volume normalization' })).not.toBeDisabled();
    });

    it('되묻기: 직접 요청은 디스코드 설정에, 말하는 사바쨩은 이 서버의 챗봇 설정에 저장한다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, confirmLevel: 'danger' });
        window.api.discordSettingsSet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: base, sync: { ok: true } });
        window.api.chatbotGet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: { confirmLevel: 'all' }, guilds: {}, hasApiKey: true });
        window.api.chatbotSave = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: true });
        render(<BotSettingsTab node={node} />);
        await flush();
        pickOption(screen.getByLabelText('Requests to the bot'), "Don't ask");
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { confirmLevel: 'none' });
        pickOption(screen.getByLabelText('Talking Saba-chan'), 'Caution and above');
        await flush();
        expect(window.api.chatbotSave).toHaveBeenCalledWith({ guilds: { g1: { confirmLevel: 'caution' } } });
    });

    it('호출어와 명령 이름 번역은 이 서버 값으로 저장하고, 비어 있으면 기본 호출어를 안내한다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, prefixes: [], translateCommands: true });
        window.api.discordSettingsSet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: { ...base, prefixes: ['!pal'] }, sync: { ok: true } });
        render(<BotSettingsTab node={{ ...node, prefixDefault: true, prefixes: ['사바쨩'] }} />);
        await flush();
        expect(screen.getByText(/사바쨩/, { selector: '.dc-hint' })).toBeTruthy();
        const input = screen.getByLabelText('Command prefix');
        fireEvent.change(input, { target: { value: '!pal' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { prefixes: ['!pal'] });
        expect(screen.queryByRole('checkbox', { name: 'Volume normalization' })).toBeNull();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Translated command names' }));
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { translateCommands: false });
    });

    it('슬래시 명령 모양(사바쨩 접두어·서버 명령 이름·조회 결과 나만 보기)은 이 서버 값으로 저장한다', async () => {
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base });
        window.api.discordSettingsSet = vi.fn().mockResolvedValue({ success: true, settings: base, sync: { ok: true } });
        render(<BotSettingsTab node={node} />);
        await flush();
        // 값이 없으면(옛 데몬) 접두어 켜짐·인스턴스 이름·모두에게 보임
        expect(screen.getByRole('checkbox', { name: 'Saba-chan prefix on commands' }).checked).toBe(true);
        expect(screen.getByRole('checkbox', { name: 'Show query results only to the caller' }).checked).toBe(false);
        fireEvent.click(screen.getByRole('checkbox', { name: 'Saba-chan prefix on commands' }));
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { slashPrefix: false });
        pickOption(screen.getByLabelText('Server command names'), 'Server call word');
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { slashRootName: 'word' });
        fireEvent.click(screen.getByRole('checkbox', { name: 'Show query results only to the caller' }));
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { privateQueries: true });
    });

    it('호출어를 빠르게 여러 개 넣어도 앞 저장의 답이 화면의 칩을 지우지 않고, 설정을 못 읽었으면 바꾸지 않는다', async () => {
        const answers = [];
        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, prefixes: [] });
        window.api.discordSettingsSet = vi.fn(
            (_g, p) =>
                new Promise((resolve) =>
                    answers.push(() => resolve({ success: true, settings: { ...base, ...p }, sync: { ok: true } })),
                ),
        );
        const { unmount } = render(<BotSettingsTab node={node} />);
        await flush();
        const input = screen.getByLabelText('Command prefix');
        fireEvent.change(input, { target: { value: 'a' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        fireEvent.change(input, { target: { value: 'b' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        await act(async () => answers[0]());
        expect(screen.getByRole('button', { name: 'b ×' })).toBeTruthy();
        await act(async () => answers[1]?.());
        expect(screen.getByRole('button', { name: 'a ×' })).toBeTruthy();
        expect(screen.getByRole('button', { name: 'b ×' })).toBeTruthy();
        unmount();

        window.api.discordSettingsGet = vi.fn().mockResolvedValue({ ...base, prefixes: [], loaded: false });
        window.api.discordSettingsSet = vi.fn();
        render(<BotSettingsTab node={node} />);
        await flush();
        const locked = screen.getByLabelText('Command prefix');
        fireEvent.change(locked, { target: { value: 'c' } });
        fireEvent.keyDown(locked, { key: 'Enter' });
        expect(window.api.discordSettingsSet).not.toHaveBeenCalled();
    });
});
