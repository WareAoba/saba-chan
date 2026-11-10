import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

// 반투명(data-backdrop="on")일 때 카드·컴포넌트 재질이 유리로 바뀌는지, 떠 있는 창은 단단하게 남는지 지킨다.
const SRC = path.resolve(__dirname, '..');
const read = (p: string) => fs.readFileSync(path.join(SRC, p), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

function cssFiles(dir: string): string[] {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((d) => {
        const p = path.join(dir, d.name);
        if (d.isDirectory()) return cssFiles(p);
        return d.name.endsWith('.css') ? [p] : [];
    });
}

function rules(css: string): { sel: string[]; body: string }[] {
    return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({
        sel: m[1].split(',').map((s) => s.trim().replace(/\s+/g, ' ')),
        body: m[2],
    }));
}

/** 선택자 목록이 들어 있는 규칙들의 본문 */
function bodyFor(css: string, selector: string): string {
    return rules(css)
        .filter((r) => r.sel.includes(selector))
        .map((r) => r.body)
        .join('\n');
}

// 카드·버튼·입력칸이 칠하는 면 토큰. 테마는 *-solid만 정하고 이 이름은 재질 층이 고른다
const SURFACES = [
    '--bg-1',
    '--bg-2',
    '--bg-3',
    '--bg-4',
    '--surface-gradient',
    '--surface-gradient-hover',
    '--inset',
    '--field',
];
const DEPTH = ['--neu-up', '--neu-up-hi', '--neu-in'];

describe('glass material when the window backdrop is on', () => {
    const tokens = read('styles/tokens.css');
    const backdrop = read('styles/backdrop.css');
    const glass = bodyFor(backdrop, ':root[data-backdrop="on"]');

    it('themes define only the solid surfaces; the material layer maps them', () => {
        for (const t of SURFACES) {
            expect(tokens, `${t} must not be set by a theme block`).not.toMatch(new RegExp(`\\s${t}:\\s*#`));
            expect(tokens).toMatch(new RegExp(`\\s${t}:\\s*var\\(${t}-solid\\)`));
        }
    });

    it('every surface becomes a see-through mix of its solid colour', () => {
        for (const t of SURFACES) {
            expect(glass, t).toMatch(
                new RegExp(
                    `${t}:\\s*color-mix\\(in oklch, var\\(${t}-solid\\) var\\(--glass-[a-z]+\\), transparent\\)`,
                ),
            );
        }
    });

    it('raised and sunken depth drops the light-side halo on glass', () => {
        for (const t of DEPTH) {
            const m = glass.match(new RegExp(`${t}:([^;]*);`));
            expect(m, t).not.toBeNull();
            expect(m?.[1]).not.toContain('--neu-light');
            expect(m?.[1]).not.toMatch(/-\d+px -\d+px/);
        }
    });

    it('every floating window (popover, modal, toast) keeps the solid material', () => {
        const islands = new Set(
            rules(backdrop)
                .filter((r) => r.body.includes('--bg-2: var(--bg-2-solid)'))
                .flatMap((r) => r.sel)
                .map((s) => s.replace(':root[data-backdrop="on"] ', '')),
        );
        const floating = cssFiles(SRC)
            .filter((f) => !f.endsWith('tokens.css'))
            .flatMap((f) => rules(fs.readFileSync(f, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '')))
            .filter((r) => /box-shadow:[^;]*var\(--shadow-float\)/.test(r.body))
            .flatMap((r) => r.sel)
            .filter((s) => !s.includes(':') && !s.includes('.dragging'));
        expect(floating.filter((s) => !islands.has(s))).toEqual([]);
        // 떠 있는 창 안에서는 입체도 원래 뉴모피즘으로 돌아온다
        const island = rules(backdrop).find((r) => r.body.includes('--bg-2: var(--bg-2-solid)'));
        for (const t of DEPTH) expect(island?.body).toContain(`${t}: var(${t}-solid)`);
    });
});

describe('dropdown is a value holder', () => {
    it('closed trigger is a sunken field like .input, not a raised button', () => {
        const css = read('components/CustomDropdown/CustomDropdown.css');
        const body = bodyFor(css, '.custom-dropdown-trigger');
        expect(body).toMatch(/background:\s*var\(--field\)/);
        expect(body).toMatch(/box-shadow:\s*var\(--neu-in\)/);
        expect(css).not.toMatch(/\.custom-dropdown-trigger:hover\s*\{[^}]*--neu-up/);
    });
});
