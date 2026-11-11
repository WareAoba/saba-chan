import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AgentLink } from '../components/discord/AgentLink';
import i18n from '../i18n';

describe('AI 에이전트 연결', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            relayMcpGrant: vi.fn().mockResolvedValue({
                grants: [{ id: 'aaaa', clientName: 'Claude', createdAt: '2026-10-28T00:00:00Z', lastUsedAt: '2026-10-28T01:00:00Z' }],
                url: 'https://relay.test/mcp',
            }),
            relayMcpGrantRevoke: vi.fn().mockResolvedValue({ ok: true }),
        } as unknown as typeof window.api;
    });

    it('연결된 에이전트를 보이고, 끊으면 연결 없음으로 바뀐다', async () => {
        await act(async () => render(<AgentLink />));
        expect(screen.getByText(/Claude/)).toBeInTheDocument();
        // 끊기 버튼은 줄을 눌러야 나온다 (사용자 요청 2026-11-08)
        expect(screen.queryByText(i18n.t('gui:discord.agent_revoke'))).not.toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Claude/ })));
        await act(async () => fireEvent.click(screen.getByText(i18n.t('gui:discord.agent_revoke'))));
        expect(window.api.relayMcpGrantRevoke).toHaveBeenCalledWith('aaaa');
        expect(screen.queryByRole('button')).not.toBeInTheDocument();
        expect(screen.getByText(/relay\.test\/mcp/)).toBeInTheDocument();
    });

    it('끊기가 실패하면 알리고 연결을 그대로 보인다', async () => {
        window.api.relayMcpGrantRevoke = vi.fn().mockResolvedValue({ error: 'Relay connection failed' }) as never;
        await act(async () => render(<AgentLink />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Claude/ })));
        await act(async () => fireEvent.click(screen.getByText(i18n.t('gui:discord.agent_revoke'))));
        expect(window.showToast).toHaveBeenCalled();
        expect(screen.getByText(i18n.t('gui:discord.agent_revoke')).closest('button')).toBeEnabled();
    });

    // 사용자 요청(2026-11-08): 여러 에이전트를 목록으로 보이고 하나씩 끊는다
    it('여러 에이전트를 목록으로 보이고, 고른 에이전트만 그 id로 끊는다', async () => {
        window.api.relayMcpGrant = vi.fn().mockResolvedValue({
            grants: [
                { id: 'aaaa', clientName: 'Claude', createdAt: '2026-10-28T00:00:00Z', lastUsedAt: '2026-10-28T01:00:00Z' },
                { id: 'bbbb', clientName: 'ChatGPT', createdAt: '2026-10-29T00:00:00Z', lastUsedAt: '2026-10-29T01:00:00Z' },
            ],
            url: 'https://relay.test/mcp',
        }) as never;
        await act(async () => render(<AgentLink />));
        expect(screen.getByText('Claude')).toBeInTheDocument();
        expect(screen.getByText('ChatGPT')).toBeInTheDocument();
        // PC 카드처럼 에이전트마다 카드 하나 (사용자 요청 2026-11-08)
        expect(screen.getByRole('list', { name: i18n.t('gui:discord.agent_title') }).children).toHaveLength(2);
        expect(screen.getAllByText(i18n.t('gui:discord.agent_created'))).toHaveLength(2);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /ChatGPT/ })));
        await act(async () => fireEvent.click(screen.getByText(i18n.t('gui:discord.agent_revoke'))));
        expect(window.api.relayMcpGrantRevoke).toHaveBeenCalledWith('bbbb');
        expect(screen.getByText('Claude')).toBeInTheDocument();
        expect(screen.queryByText('ChatGPT')).not.toBeInTheDocument();
    });

    it('연결 상태를 받지 못하면 줄을 그리지 않는다', async () => {
        window.api.relayMcpGrant = vi.fn().mockResolvedValue({ error: 'Relay connection failed' }) as never;
        const { container } = await act(async () => render(<AgentLink />));
        expect(container).toBeEmptyDOMElement();
    });
});
