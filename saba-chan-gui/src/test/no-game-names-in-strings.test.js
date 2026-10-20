import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { gameNameOf } from '../utils/serverView';

// 설치 문구는 모든 모듈이 같이 쓴다: 특정 게임 이름이나 그 게임의 파일 형식(JAR)을 박지 않고 모듈 이름을 넣는다
const SHARED = [
    'servers.install_completed',
    'servers.progress_downloading',
    'server_settings.install_version_confirm',
    'server_settings.version_description_install',
];
const LANGS = ['ko', 'en', 'ja', 'zh-CN', 'zh-TW', 'de', 'es', 'fr', 'pt-BR', 'ru'];
const get = (obj, key) => key.split('.').reduce((o, k) => o?.[k], obj);

describe('모듈이 같이 쓰는 설치 문구', () => {
    for (const lang of LANGS) {
        it(`${lang}: 게임 이름과 JAR을 박지 않는다`, () => {
            const gui = JSON.parse(readFileSync(join(__dirname, '../../../locales', lang, 'gui.json'), 'utf8'));
            for (const key of SHARED) {
                const text = get(gui, key);
                expect(text, key).toBeTypeOf('string');
                expect(text, `${lang} ${key}`).not.toMatch(/minecraft|マインクラフト|마인크래프트|我的世界|jar/i);
            }
            for (const key of ['servers.install_completed', 'servers.progress_downloading']) {
                expect(get(gui, key), `${lang} ${key}`).toContain('{{game}}');
            }
        });
    }

    it('게임 이름은 모듈 번역, 없으면 모듈의 game_name, 그것도 없으면 모듈 id', () => {
        const t = (key, opts) => (key === 'mod_mc:module.display_name' ? '마크' : opts.defaultValue);
        expect(gameNameOf(t, 'mc')).toBe('마크');
        expect(gameNameOf(t, 'game', { name: 'game', game_name: 'Terraria' })).toBe('Terraria');
        expect(gameNameOf(t, 'game')).toBe('game');
    });
});
