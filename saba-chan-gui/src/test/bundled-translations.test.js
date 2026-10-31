/**
 * 인스톨러·업데이터 창은 번역을 JS 안에 들고 있다 (Tauri 웹뷰라 locales 를 못 읽는다).
 * 지원 언어마다 en 과 같은 키가 있어야 그 언어 사용자에게 영어로 떨어지지 않는다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const ROOT = path.resolve(__dirname, '../../..');
const LANGS = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

/** `const NAME = {` 부터 줄 처음의 `};` 까지를 객체로 읽는다 */
function bundle(file, name) {
    const src = fs.readFileSync(path.join(ROOT, file), 'utf8');
    const start = src.indexOf(`const ${name} = {`);
    const end = src.indexOf('\n};', start);
    return new Function(`return ${src.slice(src.indexOf('{', start), end + 2)}`)();
}

describe.each([
    ['installer/gui/src/installer.js', 'T'],
    ['installer/gui/src/updater.js', 'TRANSLATIONS'],
])('%s', (file, name) => {
    it('지원 언어마다 en 과 같은 키를 가진다', () => {
        const t = bundle(file, name);
        const en = Object.keys(t.en).sort();
        const problems = [];
        for (const lang of LANGS) {
            if (!t[lang]) problems.push(`${lang}: 없음`);
            else for (const k of en) if (!(k in t[lang])) problems.push(`${lang}: ${k}`);
        }
        expect(problems).toEqual([]);
    });
});
