import { describe, expect, it } from 'vitest';
import { noticeRows } from '../components/PermissionsTab';

// 사용자 요청(2026-10-26): 켜짐 알림은 그 디스코드 서버가 쓰기로 한 서버만 고를 수 있고, 기본은 꺼짐
describe('켜짐 알림 목록', () => {
    const servers = [
        { id: 'a', name: 'survival' },
        { id: 'b', name: 'creative' },
    ];
    it('쓰기로 한 서버만, 켠 것만 켜짐으로', () => {
        expect(noticeRows(servers, { instances: ['a'], allowedInstances: null })).toEqual([
            { id: 'a', name: 'survival', on: true },
            { id: 'b', name: 'creative', on: false },
        ]);
        expect(noticeRows(servers, { instances: [], allowedInstances: ['b'] })).toEqual([
            { id: 'b', name: 'creative', on: false },
        ]);
        expect(noticeRows(servers, null)).toEqual([]);
    });
});
