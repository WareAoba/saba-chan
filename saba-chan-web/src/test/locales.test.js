import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { hangulOutsideT } from '../../../saba-chan-gui/src/test/hangulScan';
import { initI18n, SUPPORTED_LANGUAGES } from '../i18n';

const LOCALES = join(__dirname, '../../../locales');
const SRC = join(__dirname, '..');

function flatten(obj, prefix = '', out = {}) {
    for (const [k, v] of Object.entries(obj)) {
        const key = prefix ? `${prefix}.${k}` : k;
        if (v && typeof v === 'object') flatten(v, key, out);
        else out[key] = String(v);
    }
    return out;
}

const catalog = (lang, file = 'web.json') => flatten(JSON.parse(readFileSync(join(LOCALES, lang, file), 'utf8')));
const placeholders = (s) => (s.match(/\{\{\w+\}\}/g) ?? []).sort();

function sources(dir) {
    return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' ? [] : sources(p);
        return /\.jsx?$/.test(e.name) ? [p] : [];
    });
}

describe('web.json', () => {
    it('모든 언어가 en 과 같은 키와 자리 표시자를 가진다', () => {
        const en = catalog('en');
        const problems = [];
        for (const lang of SUPPORTED_LANGUAGES.filter((l) => l !== 'en')) {
            const other = catalog(lang);
            for (const k of Object.keys(en)) {
                if (!(k in other)) problems.push(`${lang}: missing ${k}`);
                else if (placeholders(en[k]).join() !== placeholders(other[k]).join())
                    problems.push(`${lang}: placeholders ${k}`);
            }
            for (const k of Object.keys(other)) if (!(k in en)) problems.push(`${lang}: not in en ${k}`);
        }
        expect(problems).toEqual([]);
    });

    it('코드가 부르는 고정 키는 en 에 있다', () => {
        const en = catalog('en');
        const missing = [];
        for (const file of sources(SRC)) {
            for (const m of readFileSync(file, 'utf8').matchAll(/\bt\(\s*'([\w.-]+)'/g)) {
                if (!(m[1] in en)) missing.push(`${file.slice(SRC.length + 1)}: ${m[1]}`);
            }
        }
        expect(missing).toEqual([]);
    });

    it('앱 문구를 빌려 쓰는 고정 키(tg)는 gui.json en 에 있다', () => {
        const gui = catalog('en', 'gui.json');
        const missing = [];
        for (const file of sources(SRC)) {
            for (const m of readFileSync(file, 'utf8').matchAll(/\btg\(\s*'([\w.-]+)'/g)) {
                if (!(m[1] in gui)) missing.push(`${file.slice(SRC.length + 1)}: ${m[1]}`);
            }
        }
        expect(missing).toEqual([]);
    });
});

describe('탭 제목', () => {
    it('브라우저 탭 제목은 고른 언어의 app_name 이다', async () => {
        await initI18n('ja');
        expect(document.title).toBe(catalog('ja').app_name);
        expect(document.documentElement.lang).toBe('ja');
    });

    it('index.html 의 처음 제목·언어는 기본 언어(en)다', () => {
        const html = readFileSync(join(__dirname, '../../index.html'), 'utf8');
        expect(html).toContain(`<title>${catalog('en').app_name}</title>`);
        expect(html).toContain('<html lang="en">');
    });
});

describe('한글 하드코딩', () => {
    it('화면 코드에 번역 키를 거치지 않은 한글이 없다', () => {
        const found = [];
        for (const file of sources(SRC)) {
            for (const h of hangulOutsideT(readFileSync(file, 'utf8')))
                found.push(`${file.slice(SRC.length + 1)}:${h.line}: ${h.text}`);
        }
        expect(found).toEqual([]);
    });
});
