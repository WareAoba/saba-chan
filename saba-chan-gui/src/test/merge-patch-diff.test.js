import { describe, expect, it } from 'vitest';
import { mergePatchDiff } from '../utils/mergePatch';

// 데몬의 merge_patch와 같은 규칙으로 얹어 본다 (RFC 7386)
function apply(target, patch) {
    if (patch === null || typeof patch !== 'object' || Array.isArray(patch)) return patch;
    const out = target !== null && typeof target === 'object' && !Array.isArray(target) ? { ...target } : {};
    for (const [k, v] of Object.entries(patch)) {
        if (v === null) delete out[k];
        else out[k] = apply(out[k], v);
    }
    return out;
}

describe('바뀐 칸만 보내는 patch (0.1.10 결정 1)', () => {
    it('한 길드의 한 칸만 바꾸면 그 칸만 싣는다', () => {
        const before = { g1: { memo: 'a', members: { u: 2 } }, g2: { memo: 'b' } };
        const after = { g1: { memo: 'c', members: { u: 2 } }, g2: { memo: 'b' } };
        expect(mergePatchDiff(before, after)).toEqual({ g1: { memo: 'c' } });
    });

    it('없어진 키는 null로 지우고, 배열은 통째로 바꾼다', () => {
        expect(mergePatchDiff({ a: 1, b: [1, 2] }, { b: [2] })).toEqual({ a: null, b: [2] });
        expect(mergePatchDiff({ a: 1 }, { a: 1 })).toBeUndefined();
    });

    it('얹으면 새 값이 된다 (null이 없는 값)', () => {
        const cases = [
            [{}, { a: { b: { c: 1 } } }],
            [{ a: { b: 1, c: [1] } }, { a: { c: [2] }, d: 'x' }],
            [{ a: 1 }, { a: { b: 2 } }],
            [{ a: { b: 2 } }, { a: 'flat' }],
            [{ prefixes: ['a', 'b'] }, { prefixes: [] }],
        ];
        for (const [before, after] of cases) {
            const patch = mergePatchDiff(before, after);
            expect(patch === undefined ? before : apply(before, patch)).toEqual(after);
        }
    });
});
