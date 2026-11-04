/**
 * 언어 파일은 영어만 번들에 넣고, 나머지는 그 언어로 바꿀 때 불러온다.
 */
import { afterAll, describe, expect, it } from 'vitest';
import i18n, { i18nReady } from '../i18n';

describe('언어 파일 지연 로드', () => {
    afterAll(() => i18n.changeLanguage('en'));

    it('다른 언어로 바꾸면 그 언어의 gui·common·bot을 불러와 쓴다', async () => {
        await i18nReady;
        expect(i18n.hasResourceBundle('ja', 'gui')).toBe(false);
        await i18n.changeLanguage('ja');
        expect(i18n.t('discord.agent_stopped')).toBe('オフ');
        expect(i18n.hasResourceBundle('ja', 'common')).toBe(true);
        expect(i18n.hasResourceBundle('ja', 'bot')).toBe(true);
    });

    it('없는 키는 영어로 떨어진다', async () => {
        await i18n.changeLanguage('ja');
        expect(i18n.t('discord.agent_stopped', { lng: 'en' })).toBe('Off');
    });
});
