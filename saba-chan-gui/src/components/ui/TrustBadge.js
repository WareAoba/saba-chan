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

/**
 * 서버를 만들거나 가져올 때 고른 모듈이 확인되지 않았거나 바뀌었으면 알린다.
 * 모듈 Python만 격리되고 게임 서버 프로세스는 격리되지 않으므로, 믿을 수 있는 모듈인지 이때 묻는다.
 */
export function TrustCreateNotice({ trust }) {
    const { t } = useTranslation('gui');
    if (trust !== 'unverified' && trust !== 'modified') return null;
    return (
        <div className="validation-warning-banner trust-create-notice" role="note">
            <Icon name="alertTriangle" size="sm" />
            <span>{t(`trust.create_notice.${trust}`)}</span>
        </div>
    );
}
