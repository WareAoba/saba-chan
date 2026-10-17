import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

const ICONS = { verified: 'checkCircle', modified: 'alertTriangle', unverified: 'alertCircle' };

/**
 * 모듈·익스텐션 신뢰 배지 (데몬 GET /api/modules·/api/extensions의 `trust`).
 * verified = 서명된 카탈로그에서 받았고 그 뒤로 파일이 그대로다. 값이 없으면(옛 데몬) 그리지 않는다.
 */
export function TrustBadge({ trust }) {
    const { t } = useTranslation('gui');
    if (!ICONS[trust]) return null;
    const tip = t(`trust.${trust}_tip`);
    return (
        <span className={`trust-badge trust-${trust}`} data-tooltip={tip} aria-label={tip} role="img">
            <Icon name={ICONS[trust]} size="xs" />
            {t(`trust.${trust}`)}
        </span>
    );
}
