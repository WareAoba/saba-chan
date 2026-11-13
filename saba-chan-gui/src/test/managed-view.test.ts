import { describe, expect, it } from 'vitest';
import type { CommandField, ModuleInfo } from '../types/daemon';
import { defaultServerTab, interactionOf, isManaged } from '../utils/serverView';

const console: Partial<ModuleInfo> = {
    protocols: { supported: ['stdin'], default: null },
    interaction_mode: 'commands',
};

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
            input: false,
            commands: true,
        });
    });

    it('REST 전용 서버도 managed면 러너가 받은 출력을 읽기 전용 콘솔로 보여 준다', () => {
        const rest: Partial<ModuleInfo> = { protocols: { supported: ['rest'], default: null } };
        expect(interactionOf({}, rest)).toEqual({ console: true, input: false, commands: true });
        expect(interactionOf({ module_settings: { managed_start: false } }, rest).console).toBe(false);
    });
});

describe('서버를 처음 열 때의 탭', () => {
    const rest: Partial<ModuleInfo> = { protocols: { supported: ['rest'], default: null } };
    it('콘솔이 있으면 입력을 받지 못해도(REST 전용, 팰월드) 꺼져 있을 때 콘솔이다', () => {
        expect(defaultServerTab(interactionOf({}, rest), 'stopped')).toBe('console');
        expect(defaultServerTab(interactionOf({}, console), 'stopped')).toBe('console');
    });
    it('콘솔이 없으면 켜져 있을 때 명령, 꺼져 있을 때 정보', () => {
        const unmanaged = interactionOf({ module_settings: { managed_start: false } }, rest);
        expect(defaultServerTab(unmanaged, 'running')).toBe('commands');
        expect(defaultServerTab(unmanaged, 'stopped')).toBe('info');
    });
});

describe('콘솔 빠른 명령', () => {
    // 예전 이름 rcon·rcon_template은 데몬이 console·console_template으로 읽어 준다 (module_loader)
    it('console 명령 중 템플릿이 이름으로 시작하는 것만', async () => {
        const { consoleQuickCommands } = await import('../utils/serverView');
        const fields = [
            { name: 'say', method: 'console', console_template: 'say {message}' },
            { name: 'save-all', method: 'console', console_template: 'save-all' },
            { name: 'kickall', method: 'console', console_template: 'kick @a' },
            { name: 'players', method: 'rest' },
            { name: 'backup' },
        ] as CommandField[];
        expect(consoleQuickCommands(fields).map((c) => c.name)).toEqual(['say', 'save-all']);
        expect(consoleQuickCommands(undefined)).toEqual([]);
    });
});
