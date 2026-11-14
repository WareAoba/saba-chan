import { describe, expect, it } from 'vitest';
import { pickLanguage } from '../utils/languages';

/** 브라우저·OS 언어 → 지원 언어. 데몬·CLI·셸(`constants::resolve_locale`)과 같은 규칙 */
describe('pickLanguage', () => {
    it('지역 없는 언어와 지역 붙은 언어를 지원 언어로 맞춘다', () => {
        expect(pickLanguage(['ko-KR'])).toBe('ko');
        expect(pickLanguage(['pt'])).toBe('pt-BR');
        expect(pickLanguage(['zh_TW'])).toBe('zh-TW');
    });

    it('중국어는 스크립트 태그가 지역보다 먼저다', () => {
        expect(pickLanguage(['zh-HK'])).toBe('zh-TW');
        expect(pickLanguage(['zh-Hans-HK'])).toBe('zh-CN');
        expect(pickLanguage(['zh-Hant-SG'])).toBe('zh-TW');
        expect(pickLanguage(['zh'])).toBe('zh-CN');
    });

    it('지원하지 않는 언어는 다음 후보, 없으면 영어', () => {
        expect(pickLanguage(['nl-NL', 'de-AT'])).toBe('de');
        expect(pickLanguage(['nl-NL'])).toBe('en');
    });
});
