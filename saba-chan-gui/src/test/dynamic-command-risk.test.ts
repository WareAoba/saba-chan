import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useInstanceCommands } from '../hooks/useInstanceCommands';
import type { Server } from '../stores/useServerStore';

describe('데몬 명령 목록의 위험도', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).instanceCommands;
    });

    it('module.toml에 없는 동적 명령이 danger면 확인을 유지한다', async () => {
        window.api.instanceCommands = vi.fn().mockResolvedValue({
            commands: [
                { name: 'say', source: 'module', risk: 'caution' },
                { name: 'chunky-cancel', source: 'module', risk: 'danger' },
            ],
        }) as never;
        const { result } = renderHook(() => useInstanceCommands({ id: 'a' } as Server, [{ name: 'say', risk: 'safe' }]));
        await waitFor(() => expect(result.current).toHaveLength(2));
        expect(result.current.find((c) => c.name === 'chunky-cancel').risk).toBe('danger');
        // module.toml에 적힌 명령은 module.toml 값을 따른다
        expect(result.current.find((c) => c.name === 'say').risk).toBe('safe');
    });
});
