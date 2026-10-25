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
});
