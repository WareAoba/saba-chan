import type { TFunction } from 'i18next';

/** 데몬이 모듈마다 싣는 지원 OS·CPU 판정 (설치된 모듈 목록과 카탈로그가 같은 이름이다) */
export interface ModuleSupport {
    platforms?: string[];
    architectures?: string[];
    platform_supported?: boolean;
    unsupported_reason?: string;
}

/** 이 데몬에서 돌지 않는 모듈의 짧은 표시와 풀어 쓴 이유. 돌면 null */
export function unsupportedText(t: TFunction, m: ModuleSupport): { label: string; detail: string } | null {
    if (m.platform_supported !== false) return null;
    if (m.unsupported_reason === 'arch') {
        return {
            label: t('add_server_modal.arch_unsupported'),
            detail: t('add_server_modal.arch_unsupported_detail', { architectures: (m.architectures || []).join(', ') }),
        };
    }
    return {
        label: t('add_server_modal.platform_unsupported'),
        detail: t('add_server_modal.platform_unsupported_detail', { platforms: (m.platforms || []).join(', ') }),
    };
}

/** 받기 전 카탈로그 모듈 하나 (`GET /api/modules/manifest`의 `manifest.modules.<id>`) */
export interface CatalogModule extends ModuleSupport {
    id: string;
    version?: string;
    display_name?: string;
    display_names?: Record<string, string>;
    description?: string;
    descriptions?: Record<string, string>;
    icon?: string;
    size_bytes?: number;
}
