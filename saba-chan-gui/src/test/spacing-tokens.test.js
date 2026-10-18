import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(__dirname, '..');
const cssFiles = (dir) =>
    readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
        e.isDirectory() ? cssFiles(join(dir, e.name)) : e.name.endsWith('.css') ? [join(dir, e.name)] : [],
    );

describe('간격 토큰', () => {
    it('padding·margin·gap은 tokens.css의 --sp-* 만 쓴다 (1px 예외)', () => {
        const stray = cssFiles(SRC)
            .filter((f) => !f.endsWith('tokens.css'))
            .flatMap((f) =>
                readFileSync(f, 'utf8')
                    .split('\n')
                    .map((line, i) => [line, i + 1])
                    .filter(([line]) => /^\s*(padding|margin|gap|row-gap|column-gap)(-[a-z]+)?\s*:/.test(line))
                    .filter(([line]) => /\d+(\.\d+)?px/.test(line.replace(/(?<![\d.])1px/g, '')))
                    .map(([line, n]) => `${f.slice(SRC.length + 1)}:${n} ${line.trim()}`),
            );
        expect(stray).toEqual([]);
    });
});
