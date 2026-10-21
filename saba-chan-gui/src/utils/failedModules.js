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
 * 무엇이 필요한지 나열하고, module.toml이 스키마에 맞지 않으면(module_schema_invalid) 문제 목록을,
 * 데몬보다 새 스키마면(module_schema_unsupported) 필요한 버전을 보인다. 나머지는 데몬의 오류 문구를 그대로 쓴다.
 */
export const failedModuleReason = (f, t) => {
    if (f?.error_code === 'module_dependency_unsatisfied' && f.missing_dependencies?.length) {
        return t('shell.module_needs', { deps: f.missing_dependencies.join(', ') });
    }
    if (f?.error_code === 'module_schema_invalid' && f.problems?.length) {
        return t('modules.schema_invalid', { problems: f.problems.join('; ') });
    }
    if (f?.error_code === 'module_schema_unsupported') {
        return t('modules.schema_unsupported', { required: f.required ?? '?' });
    }
    return f?.error || '';
};
