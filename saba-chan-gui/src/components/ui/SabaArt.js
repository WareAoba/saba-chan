import clsx from 'clsx';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import './SabaArt.css';

/**
 * 사바쨩 그림 (public/saba/*.webp).
 * 원칙: 한 화면에 한 명, 알리는 순간에만 쓴다. 다크·라이트는 그림자만 다르다 (--art-shadow).
 */
export const SABA_ART = ['sleep', 'sleepy', 'cleaning', 'undefined', 'recovering', 'stopped', 'relay', 'cooking', 'served'];

export function SabaArt({ kind, className, size, alt = '' }) {
    if (!SABA_ART.includes(kind)) return null;
    return (
        <img
            src={`./saba/${kind}.webp`}
            alt={alt}
            className={clsx('saba-art', `saba-art-${kind}`, className)}
            style={size ? { '--art-size': `${size}px` } : undefined}
            draggable={false}
            decoding="async"
        />
    );
}

/** 빈 목록: 청소하는 사바쨩 + "아직 추가하지 않았어요" + 목록별 안내 */
export function SabaEmpty({ body, title, size = 180, compact, children }) {
    const { t } = useTranslation('gui');
    return (
        <div className={clsx('saba-empty', { compact })}>
            <SabaArt kind="cleaning" size={size} />
            <p className="saba-empty-title">{title || t('saba.empty_title')}</p>
            {body && <p className="saba-empty-body">{body}</p>}
            {children}
        </div>
    );
}

/** 가벼운 실패 (불러오기 실패 등): 고개를 갸웃하는 사바쨩 + 이유 + 다시 시도 */
export function SabaFail({ message, onRetry, retryLabel, size = 180 }) {
    const { t } = useTranslation('gui');
    return (
        <div className="saba-fail" role="alert">
            <SabaArt kind="undefined" size={size} />
            <div className="saba-fail-text">
                <p className="saba-fail-title">{t('saba.fail_title')}</p>
                {message && <p className="saba-fail-body mt-pre">{message}</p>}
                {onRetry && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={onRetry}>
                        <Icon name="refresh" size="sm" /> {retryLabel || t('saba.retry')}
                    </button>
                )}
            </div>
        </div>
    );
}
