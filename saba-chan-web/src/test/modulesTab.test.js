import { describe, expect, it } from 'vitest';
import { isNewerVersion, moduleUpdates } from '../components/modules';
import { resolveTheme } from '../prefs';

describe('모듈 업데이트', () => {
    it('버전을 숫자 조각으로 비교하고, 같은 판의 시험판은 정식판보다 낮다', () => {
        expect(isNewerVersion('0.9.10', '0.9.4')).toBe(true);
        expect(isNewerVersion('1.0.0', '1.0.0')).toBe(false);
        expect(isNewerVersion('1.0.0', '1.0.0-beta.2')).toBe(true);
        expect(isNewerVersion('1.0.0-beta.2', '1.0.0')).toBe(false);
        expect(isNewerVersion('1.0.0-beta.10', '1.0.0-beta.9')).toBe(true);
        expect(isNewerVersion('v1.2', '1.1.9')).toBe(true);
        expect(isNewerVersion(undefined, '1.0.0')).toBe(false);
        expect(isNewerVersion('1.0.0', undefined)).toBe(false);
    });

    it('설치된 모듈 중 공식 목록에 더 새 판이 있는 것만 고른다 (모듈 id는 폴더 이름)', () => {
        const modules = [
            { name: 'minecraft', path: 'C:\\saba\\modules\\minecraft', version: '0.9.3' },
            { name: 'palworld', path: '/srv/modules/palworld', version: '0.6.1' },
            { name: 'local-only', path: '/srv/modules/local-only', version: '1.0.0' },
        ];
        const catalog = [
            { id: 'minecraft', version: '0.9.4' },
            { id: 'palworld', version: '0.6.1' },
        ];
        expect(moduleUpdates(modules, catalog)).toEqual([
            { id: 'minecraft', name: 'minecraft', current: '0.9.3', latest: '0.9.4' },
        ]);
        expect(moduleUpdates(modules, { error: 'x' })).toEqual([]);
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
