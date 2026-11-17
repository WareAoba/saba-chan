import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(__dirname, '..');
const GUIDES = join(__dirname, '..', '..', '..', 'guides');

function sources(dir: string): string[] {
    return readdirSync(dir).flatMap((name) => {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) return name === 'test' ? [] : sources(path);
        return /\.tsx?$/.test(name) ? [readFileSync(path, 'utf8')] : [];
    });
}

/** 화면이 내놓는 자리: 글자 그대로 쓴 것과, 컴포넌트가 `<앞>.<id>`로 붙이는 앞부분 */
function knownSpots() {
    const code = sources(SRC).join('\n');
    const literal = (re: RegExp) => [...code.matchAll(re)].map((m) => m[1]);
    // spot="…"는 컴포넌트에 넘기는 값: 그대로 쓰거나(Card) 탭 id를 붙인다(spot="server.tab")
    const props = literal(/[\s{]spot="([^"]+)"/g);
    const exact = new Set([
        ...literal(/(?:data-spot|saveSpot)="([^"]+)"/g),
        // data-spot={조건 ? 'port.check' : undefined}
        ...literal(/data-spot=\{[^}]*?'([\w.]+)'/g),
        ...props,
    ]);
    const prefixes = new Set([...literal(/data-spot=\{`([^`$]+)\.\$\{/g), ...props]);
    return { exact, prefixes };
}

describe('가이드가 짚는 자리', () => {
    it('모든 가이드 단계의 spot(a|b면 각각)이 화면에 있다', () => {
        const { exact, prefixes } = knownSpots();
        const missing: string[] = [];
        for (const name of readdirSync(GUIDES)) {
            const toml = readFileSync(join(GUIDES, name, 'guide.toml'), 'utf8');
            for (const m of toml.matchAll(/^spot\s*=\s*"([^"]+)"/gm)) {
                for (const spot of m[1].split('|')) {
                    const prefix = spot.slice(0, spot.lastIndexOf('.'));
                    if (!exact.has(spot) && !prefixes.has(prefix)) missing.push(`${name}: ${spot}`);
                }
            }
        }
        expect(missing).toEqual([]);
    });
});
