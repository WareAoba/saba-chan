import { describe, expect, it } from 'vitest';
import { orderMembers } from '../components/PermissionsTab';

describe('멤버 순서', () => {
    it('실제 레벨 높은 순(주인 먼저, 차단은 끝)이고 같은 레벨은 이름순', () => {
        const members = [
            { id: '1', displayName: 'zed' },
            { id: '2', displayName: '하늘' },
            { id: '3', displayName: 'Bob' },
            { id: '4', displayName: 'amy' },
            { id: '5', displayName: 'user10' },
            { id: '6', displayName: 'user2' },
            { id: '7', username: 'cat' },
        ];
        const perm = { ownerDiscordId: '1', members: { 2: 2, 3: -1, 4: 2 } };
        expect(orderMembers(members, perm).map((m) => m.id)).toEqual(['1', '4', '2', '7', '6', '5', '3']);
    });

    it('높고 낮음은 길드 사다리 순서 (더한 단계, 아래로 옮긴 관리자)', () => {
        const members = [
            { id: '1', displayName: 'a' },
            { id: '2', displayName: 'b' },
            { id: '3', displayName: 'c' },
        ];
        const ladder = [{ id: 3 }, { id: 10, name: 'VIP' }, { id: 1 }, { id: 2 }, { id: 0 }, { id: -1 }];
        const perm = { members: { 1: 2, 2: 10 }, defaultLevel: 1, ladder };
        expect(orderMembers(members, perm).map((m) => m.id)).toEqual(['2', '3', '1']);
    });
});
