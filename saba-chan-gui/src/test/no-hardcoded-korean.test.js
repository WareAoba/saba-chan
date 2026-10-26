/**
 * 화면에 보이는 한국어 문구는 번역 키를 거친다 (한글 하드코딩 점검 2026-10-25).
 * 주석과 t() 의 기본 문구는 빼고, 그 밖의 코드에 한글이 있으면 다른 언어 사용자에게도 한국어가 보인다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { hangulOutsideT } from './hangulScan';

const SRC = path.resolve(__dirname, '..');
const LOCALES = path.resolve(__dirname, '../../../locales');

// 언어 고르기 목록은 각 언어를 그 언어 이름으로 보인다
const ALLOWED = new Set(['utils/languages.js']);

function sourceFiles(dir) {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' || e.name === 'dev' ? [] : sourceFiles(p);
        return /\.jsx?$/.test(e.name) && !e.name.includes('.test.') ? [p] : [];
    });
}

describe('한글 하드코딩', () => {
    it('검사기는 주석과 t() 기본 문구를 빼고 나머지 한글을 찾는다', () => {
        const code = [
            '// 주석',
            '/* 블록 주석 */',
            "t('a.b', '기본 문구')",
            "t('a.b', { defaultValue: `기본 ${x}` })",
            'const url = "https://x.y/z"; // 끝 주석',
            "const bad = '하드코딩';",
            '<span>화면 글자</span>',
        ].join('\n');
        expect(hangulOutsideT(code).map((h) => h.line)).toEqual([6, 7]);
    });

    it('GUI 코드에 번역 키를 거치지 않은 한글이 없다', () => {
        const found = [];
        for (const file of sourceFiles(SRC)) {
            const rel = path.relative(SRC, file).split(path.sep).join('/');
            if (ALLOWED.has(rel)) continue;
            for (const h of hangulOutsideT(fs.readFileSync(file, 'utf8'))) found.push(`${rel}:${h.line}: ${h.text}`);
        }
        expect(found).toEqual([]);
    });

    // 한국어 명령 이름(/사바쨩 연결)을 다른 언어 문구에 그대로 쓰면 그 언어의 디스코드에서는 /saba connect 다
    it('ko 밖의 locales 문구에 한글이 없다', () => {
        const found = [];
        const walk = (lang, file, value, key) => {
            if (value && typeof value === 'object') {
                for (const [k, v] of Object.entries(value)) walk(lang, file, v, key ? `${key}.${k}` : k);
            } else if (typeof value === 'string' && /[\uAC00-\uD7A3]/.test(value)) found.push(`${lang}/${file}: ${key}`);
        };
        for (const lang of fs.readdirSync(LOCALES).filter((l) => l !== 'ko')) {
            for (const file of fs.readdirSync(path.join(LOCALES, lang)).filter((f) => f.endsWith('.json'))) {
                walk(lang, file, JSON.parse(fs.readFileSync(path.join(LOCALES, lang, file), 'utf8')), '');
            }
        }
        expect(found).toEqual([]);
    });
});
