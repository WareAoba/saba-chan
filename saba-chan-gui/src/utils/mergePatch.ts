/**
 * JSON Merge Patch(RFC 7386) 만들기: 데몬 `PATCH /api/config/*`에 바뀐 칸만 보낸다.
 * 객체는 키별로 내려가고, 없어진 키는 null(지우기), 그 밖의 값(배열 포함)은 새 값 그대로.
 * 같으면 undefined를 돌려준다.
 */
const isObject = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);
const same = (a: unknown, b: unknown) => a === b || JSON.stringify(a) === JSON.stringify(b);

export function mergePatchDiff(before: unknown, after: unknown): unknown {
    if (same(before, after)) return undefined;
    if (!isObject(before) || !isObject(after)) return after === undefined ? null : after;
    const patch: Record<string, unknown> = {};
    for (const key of Object.keys(before)) {
        if (!(key in after) || after[key] === undefined) patch[key] = null;
    }
    for (const [key, value] of Object.entries(after)) {
        if (value === undefined) continue;
        const change = mergePatchDiff(before[key], value);
        if (change !== undefined) patch[key] = change;
    }
    return Object.keys(patch).length ? patch : undefined;
}
