import i18next from 'i18next';
import { describe, expect, it } from 'vitest';
import { optionLabel } from '../utils/installFields';

/** 마인크래프트 `level_type` 처럼 값에 `:` 가 든 선택지도 모듈 locales 이름으로 보인다 (i18next 는 `:` 를 네임스페이스 구분자로 읽는다) */
describe('선택지 값에 콜론이 있을 때', () => {
    it('모듈 locales 의 이름을 찾는다', async () => {
        const i18n = i18next.createInstance();
        await i18n.init({
            lng: 'ko',
            resources: {
                ko: { mod_minecraft: { settings: { level_type: { options: { 'minecraft:normal': '기본', plain: '보통' } } } } },
            },
        });
        expect(optionLabel(i18n.t, 'minecraft', { name: 'level_type' }, 'minecraft:normal')).toBe('기본');
        expect(optionLabel(i18n.t, 'minecraft', { name: 'level_type' }, 'plain')).toBe('보통');
        expect(optionLabel(i18n.t, 'minecraft', { name: 'level_type' }, 'minecraft:flat')).toBe('minecraft:flat');
    });
});
