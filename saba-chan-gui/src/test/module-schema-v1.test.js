import { describe, expect, it } from 'vitest';
import { needsConfirm, riskOf } from '../utils/commandRisk';
import { failedModuleName, failureForModule } from '../utils/failedModules';

const commands = [
    { name: 'list', risk: 'safe' },
    { name: 'kick', risk: 'caution' },
    { name: 'stop', risk: 'danger' },
    { name: 'say' },
];

describe('명령 위험도', () => {
    it('첫 단어로 명령을 찾고, 대소문자는 가리지 않는다', () => {
        expect(riskOf(commands, 'STOP now')).toBe('danger');
        expect(riskOf(commands, '  kick Steve')).toBe('caution');
    });
    it('risk가 없거나 모르는 명령은 safe', () => {
        expect(riskOf(commands, 'say hi')).toBe('safe');
        expect(riskOf(commands, 'unknown')).toBe('safe');
        expect(riskOf(commands, '')).toBe('safe');
    });
    it('danger만 확인을 요구한다', () => {
        expect(needsConfirm(commands, 'stop')).toBe(true);
        expect(needsConfirm(commands, 'kick a')).toBe(false);
    });
});

describe('불러오지 못한 모듈', () => {
    it('경로에서 모듈 폴더 이름을 뽑는다 (윈도우·유닉스 경로 모두)', () => {
        expect(failedModuleName({ path: 'C:\\saba\\modules\\terraria\\' })).toBe('terraria');
        expect(failedModuleName({ path: '/opt/saba/modules/palworld' })).toBe('palworld');
    });
    it('서버 모듈 이름으로 실패 항목을 찾는다', () => {
        const failed = [{ path: 'modules/terraria', error: 'bad toml' }];
        expect(failureForModule(failed, 'terraria')?.error).toBe('bad toml');
        expect(failureForModule(failed, 'minecraft')).toBeNull();
    });
});
