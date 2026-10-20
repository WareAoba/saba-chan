/**
 * 코드가 부르는 고정 번역 키는 en·ko 파일에 있어야 한다 (locales 점검 2026-10-19).
 * 키가 없으면 화면에 키 이름이 그대로 나오거나, 코드에 적힌 기본 문구(한국어 또는 영어)가 모든 언어에 보인다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = path.resolve(__dirname, '..');
const LOCALES = path.resolve(__dirname, '../../../locales');

function flatten(obj, prefix = '', out = new Set()) {
    for (const [k, v] of Object.entries(obj)) {
        const key = prefix ? `${prefix}.${k}` : k;
        if (v && typeof v === 'object') flatten(v, key, out);
        else out.add(key);
    }
    return out;
}

const catalog = (lang, ns) => flatten(JSON.parse(fs.readFileSync(path.join(LOCALES, lang, `${ns}.json`), 'utf8')));

function sourceFiles(dir) {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' || e.name === 'dev' ? [] : sourceFiles(p);
        return /\.jsx?$/.test(e.name) && !e.name.includes('.test.') ? [p] : [];
    });
}

// createApi는 main.js처럼 common 네임스페이스를 기본으로 쓴다
const DEFAULT_NS = { 'api/createApi.js': 'common' };

describe('고정 번역 키', () => {
    it('코드가 부르는 키는 모두 en과 ko에 있다', () => {
        const have = {};
        for (const lang of ['en', 'ko']) for (const ns of ['gui', 'common']) have[`${lang}:${ns}`] = catalog(lang, ns);
        const missing = [];
        for (const file of sourceFiles(SRC)) {
            const rel = path.relative(SRC, file).split(path.sep).join('/');
            const text = fs.readFileSync(file, 'utf8');
            for (const m of text.matchAll(/\bt\(\s*['"]([\w:.-]+)['"]/g)) {
                const [ns, key] = m[1].includes(':') ? m[1].split(':') : [DEFAULT_NS[rel] || 'gui', m[1]];
                if (!['gui', 'common'].includes(ns) || key.endsWith('.') || key.endsWith('_')) continue;
                for (const lang of ['en', 'ko']) {
                    if (!have[`${lang}:${ns}`].has(key)) missing.push(`${rel}: ${lang} ${ns}:${key}`);
                }
            }
        }
        expect(missing).toEqual([]);
    });
});

describe('gui.json 언어별 키', () => {
    // 한 언어에만 키가 빠지면 그 언어 사용자에게만 영어 문구가 보인다 (fallbackLng en)
    it('모든 언어가 en과 같은 키를 가진다', () => {
        const en = catalog('en', 'gui');
        const problems = [];
        for (const lang of fs.readdirSync(LOCALES).filter((l) => l !== 'en')) {
            if (!fs.existsSync(path.join(LOCALES, lang, 'gui.json'))) continue;
            const other = catalog(lang, 'gui');
            for (const key of en) if (!other.has(key)) problems.push(`${lang}: ${key}`);
        }
        expect(problems).toEqual([]);
    });
});
