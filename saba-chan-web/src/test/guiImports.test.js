import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/** 웹이 GUI 에서 가져다 쓰는 것. 화면(컴포넌트)은 가져오지 않는다 — 웹은 할 수 있는 일이 달라 따로 만든다 */
const ALLOWED = new Set([
    '@gui/api/createApi',
    '@gui/api/events',
    '@gui/utils/serverView',
    '@gui/utils/localizedText',
    '@gui/utils/commandRisk',
    // 권한 단계 계산과 사다리 편집 (화면 없는 순수 함수)
    '@gui/components/discord/levels',
    '@gui/styles/tokens.css',
]);

function sources(dir) {
    return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' ? [] : sources(p);
        return /\.(jsx?|css)$/.test(e.name) ? [p] : [];
    });
}

describe('GUI 에서 가져오는 것', () => {
    it('정해 둔 목록만 가져온다', () => {
        const found = new Set();
        for (const file of sources(join(__dirname, '..'))) {
            for (const m of readFileSync(file, 'utf8').matchAll(/['"](@gui\/[^'"]+)['"]/g)) found.add(m[1]);
        }
        expect([...found].filter((p) => !ALLOWED.has(p))).toEqual([]);
    });
});
