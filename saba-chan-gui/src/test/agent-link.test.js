import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AgentLink } from '../components/discord/AgentLink';

describe('AI 에이전트 연결', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            relayMcpGrant: vi.fn().mockResolvedValue({
                grant: { clientName: 'Claude', createdAt: '2026-10-28T00:00:00Z', lastUsedAt: '2026-10-28T01:00:00Z' },
            }),
            relayMcpGrantRevoke: vi.fn().mockResolvedValue(''),
        };
    });

    it('연결된 에이전트를 보이고, 끊으면 연결 없음으로 바뀐다', async () => {
        await act(async () => render(<AgentLink />));
        expect(screen.getByText(/Claude/)).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button')));
        expect(window.api.relayMcpGrantRevoke).toHaveBeenCalledTimes(1);
        expect(screen.queryByRole('button')).not.toBeInTheDocument();
        expect(screen.getByText(/saba-chan\.online\/mcp/)).toBeInTheDocument();
    });

    it('끊기가 실패하면 알리고 연결을 그대로 보인다', async () => {
        window.api.relayMcpGrantRevoke = vi.fn().mockResolvedValue({ error: 'Relay connection failed' });
        await act(async () => render(<AgentLink />));
        await act(async () => fireEvent.click(screen.getByRole('button')));
        expect(window.showToast).toHaveBeenCalled();
        expect(screen.getByRole('button')).toBeEnabled();
    });

    it('연결 상태를 받지 못하면 줄을 그리지 않는다', async () => {
        window.api.relayMcpGrant = vi.fn().mockResolvedValue({ error: 'Relay connection failed' });
        const { container } = await act(async () => render(<AgentLink />));
        expect(container).toBeEmptyDOMElement();
    });
});
