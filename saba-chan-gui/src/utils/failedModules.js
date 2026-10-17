/** 데몬이 불러오지 못한 모듈 항목 { path, error } 에서 모듈 폴더 이름을 뽑는다. */
export const failedModuleName = (f) =>
    (f?.path || '')
        .replace(/[\\/]+$/, '')
        .split(/[\\/]/)
        .pop() || '?';

/** 서버가 쓰는 모듈이 로드 실패 목록에 있으면 그 항목을 돌려준다. */
export const failureForModule = (failed, moduleName) =>
    (failed || []).find((f) => failedModuleName(f) === moduleName) || null;

/**
 * 로드 실패 이유를 사람 말로. 의존성 버전이 안 맞으면(module_dependency_unsatisfied)
 * 무엇이 필요한지 나열하고, 나머지는 데몬의 오류 문구를 그대로 쓴다.
 */
export const failedModuleReason = (f, t) =>
    f?.error_code === 'module_dependency_unsatisfied' && f.missing_dependencies?.length
        ? t('shell.module_needs', { deps: f.missing_dependencies.join(', ') })
        : f?.error || '';
