import { describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

describe('언어는 다른 창에서 바꿔도 따라간다', () => {
    it('메인 창이 언어를 바꾸면 바탕화면 펫 창도 그 언어로 바꾼다', async () => {
        await i18n.changeLanguage('ko');
        // 다른 창의 저장은 이 창에 storage 이벤트로만 온다
        localStorage.setItem('i18nextLng', 'ja');
        window.dispatchEvent(new StorageEvent('storage', { key: 'i18nextLng', newValue: 'ja' }));
        await vi.waitFor(() => expect(i18n.language).toBe('ja'));
    });

    it('지원하지 않는 값은 무시한다', async () => {
        await i18n.changeLanguage('ko');
        window.dispatchEvent(new StorageEvent('storage', { key: 'i18nextLng', newValue: 'xx' }));
        expect(i18n.language).toBe('ko');
    });
});
