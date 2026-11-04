/**
 * 사용법 안내의 명령 단어는 데몬이 알아듣는 문자열이어야 한다.
 * 데몬 목록 명령은 list / 목록, 시작 명령은 start / 시작만 인식하므로 다른 언어로 번역하지 않는다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const LOCALES = path.resolve(__dirname, '../../../locales');
const LANGS = fs.readdirSync(LOCALES).filter((l) => l !== 'ko');

describe('usage command words', () => {
    it.each(LANGS)('%s keeps list/start as typed commands', (lang) => {
        const gui = JSON.parse(fs.readFileSync(path.join(LOCALES, lang, 'gui.json'), 'utf8'));
        expect(gui.discord.usage_list_word).toBe('list');
        expect(gui.discord.usage_start_word).toBe('start');
    });
});
