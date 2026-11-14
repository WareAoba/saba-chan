/**
 * 인스톨러·업데이터 창의 문구는 locales/<lang>/installer.json(`installer`·`updater`)에 있고 빌드 때 창에 묶인다 (installer build.rs).
 * 지원 언어마다 en 과 같은 키가 있어야 그 언어 사용자에게 영어로 떨어지지 않는다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { SUPPORTED_LANGUAGES } from '../utils/languages';

const LOCALES = path.resolve(__dirname, '../../../locales');
const load = (lang: string) => JSON.parse(fs.readFileSync(path.join(LOCALES, lang, 'installer.json'), 'utf8'));

describe.each(['installer', 'updater'])('installer.json %s', (part) => {
    it('지원 언어마다 en 과 같은 키를 가진다', () => {
        const en = Object.keys(load('en')[part]).sort();
        const problems = [];
        for (const lang of SUPPORTED_LANGUAGES) {
            const t = fs.existsSync(path.join(LOCALES, lang, 'installer.json')) ? load(lang)[part] : null;
            if (!t) problems.push(`${lang}: 없음`);
            else for (const k of en) if (!(k in t)) problems.push(`${lang}: ${k}`);
        }
        expect(problems).toEqual([]);
    });
});
