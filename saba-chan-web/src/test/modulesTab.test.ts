import { describe, expect, it } from 'vitest';
import { moduleUpdates, undeclaredPermissions } from '../components/modules';
import { resolveTheme } from '../prefs';

describe('모듈 업데이트', () => {
    it('데몬이 업데이트할 판(update_version)을 준 모듈만 고른다 (고정한 모듈은 데몬이 빼고 보낸다, 모듈 id는 폴더 이름)', () => {
        const modules = [
            { name: 'minecraft', path: 'C:\\saba\\modules\\minecraft', version: '0.9.3', update_version: '0.9.4' },
            { name: 'palworld', path: '/srv/modules/palworld', version: '0.6.1' },
        ];
        expect(moduleUpdates(modules)).toEqual([{ id: 'minecraft', name: 'minecraft', current: '0.9.3', latest: '0.9.4' }]);
    });
});

describe('이 화면 설정', () => {
    it('테마는 고른 것, 기기 따라면 기기 설정을 따른다', () => {
        expect(resolveTheme('light', false)).toBe('light');
        expect(resolveTheme('dark', true)).toBe('dark');
        expect(resolveTheme('system', true)).toBe('light');
        expect(resolveTheme('system', false)).toBe('dark');
        expect(resolveTheme('bogus', false)).toBe('dark');
    });
});

describe('모듈 검사', () => {
    it('검사 결과(권한·파일·줄)에서 권한 이름만 겹치지 않게 꺼낸다', () => {
        const findings = [
            { permission: 'network', file: 'lifecycle.py', line: 3, detail: 'urllib' },
            { permission: 'process', file: 'lifecycle.py', line: 9, detail: 'subprocess' },
            { permission: 'network', file: 'util.py', line: 1, detail: 'socket' },
        ];
        expect(undeclaredPermissions(findings)).toEqual(['network', 'process']);
        expect(undeclaredPermissions([{ file: 'x.py' }])).toEqual([]);
    });
});
