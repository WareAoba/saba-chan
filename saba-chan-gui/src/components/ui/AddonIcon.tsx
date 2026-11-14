import { useCachedIcon } from '../../utils/storeCache';
import { Icon } from '../Icon';

interface AddonIconProps {
    /** 받을 수 있는 목록 항목의 키(`module-<이름>`·`ext-<id>`). 데몬이 받아 둔 그림을 쓴다 */
    storeKey?: string;
    /** 설치된 익스텐션 id. 설치 폴더의 icon.png를 쓴다 */
    extId?: string;
    /** 그림이 있는가 (목록의 `has_icon`). 없으면 묻지 않는다 */
    hasIcon?: boolean;
    /** 같은 항목이라도 버전이 바뀌면 다시 받는다 */
    version?: string;
    /** 이미 받은 그림 (설치된 모듈 목록은 data URL을 싣는다) */
    src?: string | null;
    fallback: string;
    size?: 'sm' | 'md' | 'lg';
}

/** 모듈·익스텐션 아이콘. 한 번 받은 그림은 기억해 두고 다시 그릴 때 바로 쓴다 (`useCachedIcon`) */
export function AddonIcon({ storeKey, extId, hasIcon, version, src, fallback, size = 'md' }: AddonIconProps) {
    const remote = !src && hasIcon && (storeKey || extId);
    const cacheKey = remote ? `${storeKey ? `store:${storeKey}` : `ext:${extId}`}@${version ?? ''}` : null;
    const loaded = useCachedIcon(cacheKey, () =>
        storeKey ? window.api?.storeIcon?.(storeKey) : extId ? window.api?.extensionIcon?.(extId) : undefined,
    );
    const url = src || loaded;
    return url ? <img src={url} alt="" /> : <Icon name={fallback} size={size} />;
}
