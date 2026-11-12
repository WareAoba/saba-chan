import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// 그라디언트·color-mix는 보간 색 공간을 지정한다 (지정이 없으면 sRGB로 섞여 중간색이 탁해진다).
// 회색·투명이 끼면 oklab을 쓴다: 크로미움은 채도가 아주 낮은 색을 oklch로 섞을 때 색상각을 버리고
// 채도만 남겨 0°(분홍)로 그린다. 반투명 카드가 불그스름해진 원인 (사용자 11-13)
const SRC = join(__dirname, '..');
const walk = (dir: string): string[] =>
    readdirSync(dir).flatMap((name) => {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) return name === 'test' ? [] : walk(p);
        return /\.(css|[jt]sx?)$/.test(name) ? [p] : [];
    });

describe('색 보간 공간', () => {
    it('모든 gradient()가 oklch나 oklab을 쓴다', () => {
        const bad = [];
        for (const file of walk(SRC)) {
            const text = readFileSync(file, 'utf8');
            for (const m of text.matchAll(/(?:linear|radial|conic)-gradient\(\s*([^\n]{0,24})/g)) {
                if (!/^in okl(ch|ab)/.test(m[1])) bad.push(`${file.slice(SRC.length)}: ${m[0].trim()}`);
            }
        }
        expect(bad).toEqual([]);
    });

    it('color-mix()는 oklab으로 섞는다 (섞기는 양만 정하므로 색상각 경로가 필요 없다)', () => {
        const bad = [];
        for (const file of walk(SRC)) {
            const text = readFileSync(file, 'utf8');
            for (const m of text.matchAll(/color-mix\(\s*([^\n]{0,24})/g)) {
                if (!m[1].startsWith('in oklab')) bad.push(`${file.slice(SRC.length)}: ${m[0].trim()}`);
            }
        }
        expect(bad).toEqual([]);
    });
});
