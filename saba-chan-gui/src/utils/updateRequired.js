/**
 * 사바쨩 버전이 모자라 거절된 응답을 "업데이트가 필요해요" 문구로. 그런 거절이 아니면 null.
 *
 * - 릴레이: 디스코드 상태의 lastErrorCode "agent_too_old" + minVersion
 * - 모듈: module_dependency_unsatisfied 중 saba-core 요구(`"saba-core >=0.1.10 (installed 0.1.9)"`), module_schema_unsupported
 * - 익스텐션: app_version_unsatisfied(related[0] = 요구 버전), component_version_unsatisfied 중 saba-core(related[1])
 */

const CORE = 'saba-core';

/** ">=0.1.10" → "0.1.10" */
const bare = (requirement) =>
    String(requirement || '')
        .replace(/^\s*>=\s*/, '')
        .trim();

/** missing_dependencies 중 saba-core 요구 버전 */
const coreRequirement = (deps) => {
    const dep = (deps || []).find((d) => typeof d === 'string' && d.startsWith(`${CORE} `));
    return dep ? bare(dep.slice(CORE.length + 1).split(' (')[0]) : null;
};

export const updateRequiredText = (t, body, name = '') => {
    if (!body) return null;
    if (body.lastErrorCode === 'agent_too_old') {
        return t('update_required.relay', { version: body.minVersion || '?' });
    }
    switch (body.error_code) {
        case 'module_dependency_unsatisfied': {
            const version = coreRequirement(body.missing_dependencies);
            return version ? t('update_required.module', { name, version }) : null;
        }
        case 'module_schema_unsupported':
            return t('update_required.module_newer', { name });
        case 'app_version_unsatisfied':
            return t('update_required.extension', { name, version: bare(body.related?.[0]) || '?' });
        case 'component_version_unsatisfied':
            return body.related?.[0] === CORE
                ? t('update_required.extension', { name, version: bare(body.related[1]) || '?' })
                : null;
        default:
            return null;
    }
};
