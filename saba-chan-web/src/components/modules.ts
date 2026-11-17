import type { Module } from '../types';
/** 설치된 모듈의 id (폴더 이름. 데몬 목록의 name 과 같을 때가 많지만 path 가 기준이다) */
export const moduleIdOf = (m: Pick<Module, 'path' | 'name'>) => (m.path || '').replace(/\\/g, '/').split('/').pop() || m.name;

/** 데몬이 업데이트할 판(`update_version`)을 준 설치된 모듈. 새 판·고정·바꾸기 대기는 데몬이 판정한다 */
export function moduleUpdates(modules: Pick<Module, 'path' | 'name' | 'version' | 'update_version'>[]) {
    return modules
        .filter((m) => !!m.update_version)
        .map((m) => ({ id: moduleIdOf(m), name: m.name, current: m.version, latest: m.update_version as string }));
}

/** 모듈 검사 결과(`{permission, file, line, detail}` 목록)에서 선언하지 않은 권한 이름만 겹치지 않게 꺼낸다 */
export function undeclaredPermissions(findings: { permission?: unknown; [key: string]: unknown }[] | null | undefined) {
    return [...new Set((findings || []).map((f) => f?.permission).filter((p): p is string => typeof p === 'string' && !!p))];
}
