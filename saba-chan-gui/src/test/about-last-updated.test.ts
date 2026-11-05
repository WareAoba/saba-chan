import { describe, expect, it } from 'vitest';
import { formatLastUpdated } from '../components/Modals/SettingsModal';

describe('정보 페이지의 마지막 업데이트 날짜', () => {
    it('데몬이 문자열로 준 unix 초를 날짜로 읽는다', () => {
        // 2026-10-19T12:00:00Z
        expect(formatLastUpdated('1792411200', 'en-US')).toBe('October 19, 2026');
    });

    it('읽을 수 없는 값이면 날짜를 그리지 않는다', () => {
        expect(formatLastUpdated('nope', 'en-US')).toBeNull();
    });
});
