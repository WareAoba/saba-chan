import { describe, expect, it } from 'vitest';
import { storedValue } from '../utils/storedValue';

// F12: 설정 칸의 진짜 저장 값은 데몬이 고른다 (`stored`). 화면은 공통 칸 목록을 따로 들지 않는다
describe('설정 칸의 저장된 값', () => {
    it('데몬이 실어 준 stored를 그대로 쓴다 (새 공통 칸도)', () => {
        const server = { java_path: '/new', module_settings: { java_path: '/old', motd: 'hi' }, stored: { java_path: '/new', motd: 'hi' } };
        expect(storedValue(server, 'java_path')).toBe('/new');
        expect(storedValue(server, 'motd')).toBe('hi');
        expect(storedValue(server, 'seed')).toBeUndefined();
    });

    it('stored가 없는 0.1.14 데몬이면 공통 칸은 인스턴스 칸이 앞선다', () => {
        expect(storedValue({ port: 26000, module_settings: { port: 25565 } }, 'port')).toBe(26000);
        expect(storedValue({ module_settings: { motd: 'hi' } }, 'motd')).toBe('hi');
    });
});
