import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { AgentLogo, agentLogo } from '../components/AgentLogo';

// 사용자 요청(2026-11-08): MCP 에이전트 카드에 에이전트 로고를 넣는다
describe('에이전트 로고', () => {
    it('OAuth client_name으로 로고를 고른다', () => {
        expect(agentLogo('Claude')).toBe('claude');
        expect(agentLogo('Claude Code (saba-chan)')).toBe('claude');
        expect(agentLogo('ChatGPT')).toBe('openai');
        expect(agentLogo('OpenAI Codex')).toBe('codex');
        expect(agentLogo('Gemini CLI')).toBe('gemini');
        expect(agentLogo('Cursor')).toBe('cursor');
        expect(agentLogo('GitHub Copilot')).toBe('githubcopilot');
        expect(agentLogo('MCP client')).toBeNull();
        expect(agentLogo('')).toBeNull();
    });

    it('로고가 있으면 이름표를 단 svg, 모르면 플러그 아이콘이다', () => {
        const known = render(<AgentLogo name="Claude" />);
        expect(known.getByRole('img', { name: 'Claude' })).toBeInTheDocument();
        const unknown = render(<AgentLogo name="MCP client" />);
        expect(unknown.container.querySelector('[aria-label="Claude"]')).toBeNull();
        expect(unknown.container.querySelector('svg')).not.toBeNull();
    });
});
