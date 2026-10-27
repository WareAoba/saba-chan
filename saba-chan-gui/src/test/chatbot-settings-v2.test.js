import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChatbotView, ModelField } from '../components/discord/ChatbotView';
import { DiscordPage } from '../components/discord/DiscordPage';
import { defaultPrefix, prefixProblems, readPrefixes } from '../components/discord/prefixes';
import { openDropdown } from './dropdown';

const t = (k, o) => (o ? `${k}:${JSON.stringify(o)}` : k);

describe('접두사 (chatbot-settings-v2 §2·§3)', () => {
    it('옛 기본값 !saba와 빈 값은 기본값(빈 목록)으로, 그 밖의 옛 값은 하나짜리 목록으로 읽는다', () => {
        expect(readPrefixes({ prefix: '!saba' })).toEqual([]);
        expect(readPrefixes({ prefix: '' })).toEqual([]);
        expect(readPrefixes({ prefix: '!pal' })).toEqual(['!pal']);
        expect(readPrefixes({ prefix: '!pal', prefixes: ['사바', '!s'] })).toEqual(['사바', '!s']);
    });

    it('11개 이상이거나 32자를 넘으면 저장할 수 없다', () => {
        expect(prefixProblems(['a', 'b'], t)).toEqual([]);
        expect(
            prefixProblems(
                Array.from({ length: 11 }, (_, i) => `p${i}`),
                t,
            ),
        ).toHaveLength(1);
        expect(prefixProblems(['x'.repeat(33)], t)[0]).toContain('discord.prefix_err_length');
    });

    it('기본 접두사는 릴레이가 알려 준 값을 먼저 쓴다', () => {
        expect(defaultPrefix(t, [{ prefixDefault: true, prefixes: ['サバちゃん'] }])).toBe('サバちゃん');
        expect(defaultPrefix(t, [{ prefixDefault: false, prefixes: ['!x'] }])).toBe('discord.default_prefix');
    });

    it('봇 설정 화면: 비어 있으면 기본값을 안내하고, 칩으로 여러 개 넣는다', async () => {
        window.api = {
            relayListHostNodes: vi.fn().mockResolvedValue([]),
            botConfigLoad: vi.fn().mockResolvedValue({}),
        };
        const setPrefixes = vi.fn();
        await act(async () =>
            render(
                <DiscordPage
                    discordPrefixes={[]}
                    setDiscordPrefixes={setPrefixes}
                    cloudNodes={[{ guildId: 'g1', prefixDefault: true, prefixes: ['사바쨩'] }]}
                    setCloudNodes={() => {}}
                    cloudMembers={{}}
                    setCloudMembers={() => {}}
                    initialView={{ kind: 'bot' }}
                />,
            ),
        );
        expect(screen.getByText(/사바쨩/, { selector: '.dc-hint' })).toBeTruthy();
        const input = screen.getByLabelText('Command prefix');
        fireEvent.change(input, { target: { value: '!pal' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        expect(setPrefixes).toHaveBeenLastCalledWith(['!pal']);
    });
});

describe('모델 고르기 (chatbot-settings-v2 §1)', () => {
    afterEach(() => {
        delete window.api.chatbotModels;
        delete window.api.chatbotGet;
    });

    it('릴레이가 준 목록으로 드롭다운을 채운다', async () => {
        window.api.chatbotModels = vi.fn().mockResolvedValue({
            provider: 'openai',
            models: ['gpt-a', 'gpt-b'],
            selected: 'gpt-a',
            effectiveModel: 'gpt-a',
        });
        const onChange = vi.fn();
        render(<ModelField value="gpt-a" saved="gpt-a" onChange={onChange} guildId="g1" hasApiKey />);
        await waitFor(() => expect(screen.getByLabelText('Model')).not.toBeDisabled());
        expect(window.api.chatbotModels).toHaveBeenCalledWith('g1');
        const list = openDropdown(screen.getByLabelText('Model'));
        expect(
            within(list)
                .getAllByRole('option')
                .map((o) => o.textContent),
        ).toEqual(['gpt-a', 'gpt-b']);
        fireEvent.click(within(list).getByRole('option', { name: 'gpt-b' }));
        expect(onChange).toHaveBeenLastCalledWith('gpt-b');
    });

    it('키로 연결된 제공사를 로고와 이름으로 보이고, 중국판 주소는 지역을 붙인다', async () => {
        window.api.chatbotModels = vi.fn().mockResolvedValue({
            provider: 'anthropic',
            models: ['claude-haiku-5-5'],
            selected: 'claude-haiku-5-5',
            effectiveModel: 'claude-haiku-5-5',
        });
        const { container, unmount } = render(
            <ModelField value="claude-haiku-5-5" saved="claude-haiku-5-5" onChange={() => {}} guildId="g1" hasApiKey />,
        );
        expect(await screen.findByText('Connected with a Claude key.')).toBeTruthy();
        expect(container.querySelector('.cb-provider-logo svg')).toBeTruthy();
        unmount();

        window.api.chatbotModels = vi.fn().mockResolvedValue({
            provider: 'qwen-cn',
            models: ['qwen-flash'],
            selected: 'qwen-flash',
            effectiveModel: 'qwen-flash',
        });
        render(<ModelField value="qwen-flash" saved="qwen-flash" onChange={() => {}} guildId="g1" hasApiKey />);
        expect(await screen.findByText('Connected with a Qwen China key.')).toBeTruthy();
    });

    it('고른 모델을 대신 쓰고 있으면 알리고 대신 쓰는 모델을 고른다', async () => {
        window.api.chatbotModels = vi
            .fn()
            .mockResolvedValue({ models: ['gpt-6-mini'], selected: 'gpt-5-mini', effectiveModel: 'gpt-6-mini' });
        const onChange = vi.fn();
        render(<ModelField value="gpt-5-mini" saved="gpt-5-mini" onChange={onChange} guildId="g1" hasApiKey />);
        await waitFor(() => expect(onChange).toHaveBeenCalledWith('gpt-6-mini'));
        expect(screen.getByText(/gpt-5-mini.*gpt-6-mini/)).toBeTruthy();
    });

    it('키가 없으면 묻지 않고, 릴레이 오류는 이유를 보인다', async () => {
        window.api.chatbotModels = vi.fn().mockResolvedValue({ error: 'INVALID_API_KEY' });
        const { rerender } = render(
            <ModelField value="m" saved="m" onChange={() => {}} guildId="g1" hasApiKey={false} />,
        );
        expect(window.api.chatbotModels).not.toHaveBeenCalled();
        rerender(<ModelField value="m" saved="m" onChange={() => {}} guildId="g1" hasApiKey />);
        expect(await screen.findByText(/API key was rejected/)).toBeTruthy();
        expect(screen.getByLabelText('Model')).toBeDisabled();
    });

    it('권한이 모자란 키는 틀린 키와 다르게 알린다', async () => {
        window.api.chatbotModels = vi.fn().mockResolvedValue({ error: 'KEY_PERMISSION' });
        render(<ModelField value="m" saved="m" onChange={() => {}} guildId="g1" hasApiKey />);
        expect(await screen.findByText(/lacks the permissions/)).toBeTruthy();
    });

    it('챗봇 화면에 대화 유지시간·끼어들기·호출어 칸이 없다', async () => {
        window.api.chatbotGet = vi
            .fn()
            .mockResolvedValue({ success: true, settings: {}, guilds: {}, hasApiKey: false });
        render(<ChatbotView nodes={[]} />);
        await screen.findByLabelText('Model');
        expect(document.querySelector('input[type="number"]')).toBeNull();
        expect(document.querySelector('.word-input')).toBeNull();
    });
});
