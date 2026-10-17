import { describe, expect, it } from 'vitest';
import { interactionOf, isManaged } from '../utils/serverView';

const console = { protocols: { supported: ['stdin'] }, interaction_mode: 'commands' };

describe('managed 판단 (데몬 Supervisor::uses_managed_start와 같다)', () => {
    it('데몬이 알려 준 managed가 먼저, 없으면 managed_start, 그것도 없으면 managed', () => {
        expect(isManaged({ managed: false, module_settings: { managed_start: true } })).toBe(false);
        expect(isManaged({ module_settings: { managed_start: false } })).toBe(false);
        expect(isManaged({ module_settings: { managed_start: 'false' } })).toBe(false);
        expect(isManaged({ module_settings: {} })).toBe(true);
        expect(isManaged({})).toBe(true);
    });

    it("모듈이 interaction_mode='commands'여도 기본은 managed라 stdin이 있으면 콘솔을 보여 준다", () => {
        expect(interactionOf({ module_settings: {} }, console).console).toBe(true);
        expect(interactionOf({ module_settings: { managed_start: false } }, console)).toEqual({
            console: false,
            commands: true,
        });
        expect(interactionOf({}, { protocols: { supported: ['rest'] } }).console).toBe(false);
    });
});

describe('콘솔 빠른 명령', () => {
    it('console(예전 rcon) 명령 중 템플릿이 이름으로 시작하는 것만', async () => {
        const { consoleQuickCommands } = await import('../utils/serverView');
        const fields = [
            { name: 'say', method: 'console', console_template: 'say {message}' },
            { name: 'save-all', method: 'rcon', rcon_template: 'save-all' },
            { name: 'kickall', method: 'console', console_template: 'kick @a' },
            { name: 'players', method: 'rest' },
            { name: 'backup' },
        ];
        expect(consoleQuickCommands(fields).map((c) => c.name)).toEqual(['say', 'save-all']);
        expect(consoleQuickCommands(undefined)).toEqual([]);
    });
});
