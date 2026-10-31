/** 설치된 모듈의 id (폴더 이름. 데몬 목록의 name 과 같을 때가 많지만 path 가 기준이다) */
export const moduleIdOf = (m) => (m.path || '').replace(/\\/g, '/').split('/').pop() || m.name;

// "v1.2.3-beta.4" → { nums: [1,2,3], pre: ['beta', 4] } (pre 가 없으면 정식판)
function parse(v) {
    const [core, ...rest] = String(v).trim().replace(/^v/i, '').split('-');
    const nums = core.split('.').map((x) => Number.parseInt(x, 10) || 0);
    const pre = rest.length
        ? rest
              .join('-')
              .split('.')
              .map((x) => (/^\d+$/.test(x) ? Number(x) : x))
        : null;
    return { nums, pre };
}

function comparePre(a, b) {
    if (!a || !b) return a ? -1 : b ? 1 : 0; // 같은 판이면 시험판이 정식판보다 낮다
    for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
        if (a[i] === undefined || b[i] === undefined) return a[i] === undefined ? -1 : 1;
        if (a[i] === b[i]) continue;
        if (typeof a[i] === typeof b[i]) return a[i] > b[i] ? 1 : -1;
        return typeof a[i] === 'number' ? -1 : 1;
    }
    return 0;
}

/** latest 가 current 보다 새 판인지 (데몬은 공식 목록이 더 낮으면 받지 않는다) */
export function isNewerVersion(latest, current) {
    if (!latest || !current) return false;
    const a = parse(latest);
    const b = parse(current);
    for (let i = 0; i < Math.max(a.nums.length, b.nums.length); i += 1) {
        const d = (a.nums[i] || 0) - (b.nums[i] || 0);
        if (d) return d > 0;
    }
    return comparePre(a.pre, b.pre) > 0;
}

/** 공식 목록(`[{id, version}]`)에 더 새 판이 있는 설치된 모듈 */
export function moduleUpdates(modules, catalog) {
    if (!Array.isArray(catalog)) return [];
    const latest = new Map(catalog.map((m) => [m.id, m.version]));
    return modules
        .map((m) => ({ id: moduleIdOf(m), name: m.name, current: m.version, latest: latest.get(moduleIdOf(m)) }))
        .filter((u) => isNewerVersion(u.latest, u.current));
}

/** 모듈 검사 결과(`{permission, file, line, detail}` 목록)에서 선언하지 않은 권한 이름만 겹치지 않게 꺼낸다 */
export function undeclaredPermissions(findings) {
    return [...new Set((findings || []).map((f) => f?.permission).filter((p) => typeof p === 'string' && p))];
}
