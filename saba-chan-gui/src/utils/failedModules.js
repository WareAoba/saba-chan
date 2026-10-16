/** 데몬이 불러오지 못한 모듈 항목 { path, error } 에서 모듈 폴더 이름을 뽑는다. */
export const failedModuleName = (f) =>
    (f?.path || '')
        .replace(/[\\/]+$/, '')
        .split(/[\\/]/)
        .pop() || '?';

/** 서버가 쓰는 모듈이 로드 실패 목록에 있으면 그 항목을 돌려준다. */
export const failureForModule = (failed, moduleName) =>
    (failed || []).find((f) => failedModuleName(f) === moduleName) || null;
