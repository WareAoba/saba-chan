import clsx from 'clsx';
import type { CSSProperties, ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import './SabaArt.css';

/**
 * 사바쨩 그림 (public/saba/*.webp).
 * 원칙: 한 화면에 한 명, 알리는 순간에만 쓴다. 다크·라이트는 그림자만 다르다 (--art-shadow).
 */
export const SABA_ART = ['sleep', 'sleepy', 'cleaning', 'undefined', 'recovering', 'stopped', 'relay', 'cooking', 'served'];

interface SabaArtProps {
    kind: string;
    className?: string;
    size?: number;
    alt?: string;
}

export function SabaArt({ kind, className, size, alt = '' }: SabaArtProps) {
    if (!SABA_ART.includes(kind)) return null;
    return (
        <img
            src={`./saba/${kind}.webp`}
            alt={alt}
            className={clsx('saba-art', `saba-art-${kind}`, className)}
            style={size ? ({ '--art-size': `${size}px` } as CSSProperties) : undefined}
            draggable={false}
            // 풀린 뒤에 그린다: 비동기 해독이면 탭을 열 때 그림 없는 프레임이 먼저 보인다 (warmTabArt가 미리 풀어 둔다)
            decoding="sync"
        />
    );
}

/** 빈 목록: 청소하는 사바쨩 + "아직 추가하지 않았어요" + 목록별 안내 */
interface SabaEmptyProps {
    body?: ReactNode;
    title?: ReactNode;
    size?: number;
    compact?: boolean;
    children?: ReactNode;
}

export function SabaEmpty({ body, title, size = 180, compact, children }: SabaEmptyProps) {
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
interface SabaFailProps {
    message?: ReactNode;
    onRetry?: () => void;
    retryLabel?: ReactNode;
    size?: number;
}

export function SabaFail({ message, onRetry, retryLabel, size = 180 }: SabaFailProps) {
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
