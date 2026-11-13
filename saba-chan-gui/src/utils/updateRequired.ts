import type { TFunction } from 'i18next';
import type { ApiBody } from '../types/daemon';

/**
 * 사바쨩 버전이 모자라 거절된 응답을 "업데이트가 필요해요" 문구로. 그런 거절이 아니면 null.
 *
 * - 릴레이: 디스코드 상태의 lastErrorCode "agent_too_old" + minVersion
 * - 모듈: module_dependency_unsatisfied 중 saba-core 요구(`"saba-core >=0.1.10 (installed 0.1.9)"`), module_schema_unsupported
 * - 익스텐션: app_version_unsatisfied, component_version_unsatisfied 중 saba-core
 *
 * 요구 버전은 데몬이 `required_core`로 싣는다 (사바쨩이 모자란 거절에만 있다)
 */

export const updateRequiredText = (t: TFunction, body?: ApiBody | null, name = '') => {
    if (!body) return null;
    if (body.lastErrorCode === 'agent_too_old') {
        return t('update_required.relay', { version: body.minVersion || '?' });
    }
    const version: string | null = body.required_core || null;
    switch (body.error_code) {
        case 'module_dependency_unsatisfied':
            return version ? t('update_required.module', { name, version }) : null;
        case 'module_schema_unsupported':
            return t('update_required.module_newer', { name });
        case 'app_version_unsatisfied':
            return t('update_required.extension', { name, version: version || '?' });
        case 'component_version_unsatisfied':
            return version ? t('update_required.extension', { name, version }) : null;
        default:
            return null;
    }
};
