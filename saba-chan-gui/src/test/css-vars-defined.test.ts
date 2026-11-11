import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

// 지운 변수를 다른 PR이 계속 쓰면 그 칸은 말없이 투명해진다: 대체값 없이 쓰는 변수는 어딘가에서 정해야 한다
const SRC = path.resolve(__dirname, '..');
function files(dir: string, ext: RegExp): string[] {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((d) => {
        const p = path.join(dir, d.name);
        if (d.isDirectory()) return d.name === 'test' ? [] : files(p, ext);
        return ext.test(d.name) ? [p] : [];
    });
}

describe('CSS 변수', () => {
    it('대체값 없이 쓰는 var(--x)는 모두 정해져 있다 (Radix가 넣는 --radix-* 제외)', () => {
        const css = files(SRC, /\.css$/);
        const all = [...css, ...files(SRC, /\.tsx?$/)].map((f) => fs.readFileSync(f, 'utf8'));
        const defined = new Set(all.flatMap((s) => [...s.matchAll(/(--[\w-]+)\s*:|['"`](--[\w-]+)['"`]/g)].map((m) => m[1] || m[2])));
        const missing = css.flatMap((f) =>
            [...fs.readFileSync(f, 'utf8').matchAll(/var\((--[\w-]+)\s*\)/g)]
                .map((m) => m[1])
                .filter((v) => !v.startsWith('--radix-') && !defined.has(v))
                .map((v) => `${path.relative(SRC, f)} ${v}`),
        );
        expect([...new Set(missing)]).toEqual([]);
    });
});
