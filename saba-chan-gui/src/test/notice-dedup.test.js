/**
 * Codex 리뷰 #553: 액션이 없는 알림(게임 서버 새 빌드 안내)은 dedup을 줘도 같은 안내가 쌓였다
 */
import { beforeEach, describe, expect, it } from 'vitest';
import '../components/Modals/NoticeModal';

describe('알림 중복 방지', () => {
    beforeEach(() => {
        localStorage.clear();
    });

    it('액션이 없는 같은 안내는 하나만 남기고, 다른 안내는 따로 둔다', () => {
        const add = (message) => window.__sabaNotice.addNotice({ message, source: 'Updater', dedup: true });
        add('🔔 pal-1 server update');
        add('🔔 pal-1 server update');
        add('🔔 mc-2 server update');
        expect(window.__sabaNotice.loadNotices().map((n) => n.message)).toEqual(['🔔 mc-2 server update', '🔔 pal-1 server update']);
    });
});
