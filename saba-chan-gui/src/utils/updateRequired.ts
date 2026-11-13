import type { TFunction } from 'i18next';
import type { ApiBody } from '../types/daemon';

/**
 * 사바쨩 버전이 모자라 거절된 응답을 "업데이트가 필요해요" 문구로. 그런 거절이 아니면 null.
 *
 * - 릴레이: 디스코드 상태의 lastErrorCode "agent_too_old" + minVersion
 * - 모듈: module_dependency_unsatisfied 중 saba-core 요구(`"saba-core >=0.1.10 (installed 0.1.9)"`), module_schema_unsupported
 * - 익스텐션: app_version_unsatisfied, component_version_unsatisfied 중 saba-core
 *
 * 요구 버전은 데몬이 `required_core`로 싣는다(0.1.14-beta.2~). 그 전 데몬(0.1.14 정식)이면 문구·`related`에서 읽는다
 * (원격 데몬 최소 버전이 beta.2를 넘을 때 그 대체를 지운다, `MIN_REMOTE_DAEMON` 참고)
 */

const CORE = 'saba-core';

/** ">=0.1.10" → "0.1.10" */
const bare = (requirement?: string | null) =>
    String(requirement || '')
        .replace(/^\s*>=\s*/, '')
        .trim();

/** missing_dependencies 중 saba-core 요구 버전 */
const coreRequirement = (deps?: string[] | null) => {
    const dep = (deps || []).find((d) => typeof d === 'string' && d.startsWith(`${CORE} `));
    return dep ? bare(dep.slice(CORE.length + 1).split(' (')[0]) : null;
};

export const updateRequiredText = (t: TFunction, body?: ApiBody | null, name = '') => {
    if (!body) return null;
    if (body.lastErrorCode === 'agent_too_old') {
        return t('update_required.relay', { version: body.minVersion || '?' });
    }
    const required: string | null = body.required_core || null;
    switch (body.error_code) {
        case 'module_dependency_unsatisfied': {
            const version = required || coreRequirement(body.missing_dependencies);
            return version ? t('update_required.module', { name, version }) : null;
        }
        case 'module_schema_unsupported':
            return t('update_required.module_newer', { name });
        case 'app_version_unsatisfied':
            return t('update_required.extension', { name, version: required || bare(body.related?.[0]) || '?' });
        case 'component_version_unsatisfied':
            return body.related?.[0] === CORE
                ? t('update_required.extension', { name, version: required || bare(body.related[1]) || '?' })
                : null;
        default:
            return null;
    }
};
