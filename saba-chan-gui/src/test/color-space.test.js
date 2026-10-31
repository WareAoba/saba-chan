import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// 그라디언트·color-mix는 보간 색 공간을 oklch로 지정한다 (지정이 없으면 sRGB로 섞여 중간색이 탁해진다)
const SRC = join(__dirname, '..');
const walk = (dir) =>
    readdirSync(dir).flatMap((name) => {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) return name === 'test' ? [] : walk(p);
        return /\.(css|js|jsx)$/.test(name) ? [p] : [];
    });

describe('색 보간 공간', () => {
    it('모든 gradient()·color-mix()가 oklch를 쓴다', () => {
        const bad = [];
        for (const file of walk(SRC)) {
            const text = readFileSync(file, 'utf8');
            for (const m of text.matchAll(/(?:linear|radial|conic)-gradient\(\s*([^\n]{0,24})|color-mix\(\s*([^\n]{0,24})/g)) {
                const head = m[1] ?? m[2];
                if (!head.startsWith('in oklch')) bad.push(`${file.slice(SRC.length)}: ${m[0].trim()}`);
            }
        }
        expect(bad).toEqual([]);
    });
});
