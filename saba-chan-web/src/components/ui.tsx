import { useEffect, useId, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { wordmarkFor } from '../art';
import { useApp } from '../store';
import { Icon } from './icons';
import type { ApiBody, TFn } from '../types';

/** 결과가 실패인지 (GUI 브리지는 실패를 {error} 또는 {success:false}·{ok:false}로 돌려준다) */
export const failed = (res: ApiBody | null | undefined) => !res || !!res.error || res.success === false || res.ok === false;

/** 실패 결과 → 사람이 읽을 한 줄. 문지기·중계가 막은 것은 이유를 번역해 보인다 */
export function errorText(t: TFn, res: ApiBody | null | undefined) {
    const code = res?.error_code;
    if (typeof code === 'string' && code.startsWith('remote_')) {
        const fields = Array.isArray(res?.fields) ? res.fields.join(', ') : '';
        return t(`errors.${code}`, { fields, defaultValue: t('errors.generic') });
    }
    return res?.error || res?.message || t('errors.generic');
}

/** 기다리는 동안(`busy`)은 `disabled`를 무엇으로 넘겨도 누를 수 없다 (같은 요청을 두 번 보내지 않게) */
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
    tone?: string;
    busy?: boolean;
}

export function Button({ tone = 'plain', busy = false, disabled = false, children, ...rest }: ButtonProps) {
    return (
        <button type="button" className={`btn btn-${tone}`} {...rest} disabled={busy || disabled}>
            {busy ? <span className="spinner" aria-hidden="true" /> : null}
            <span>{children}</span>
        </button>
    );
}

export function StatusDot({ status }: { status: string }) {
    const { t } = useTranslation();
    return (
        <span className={`status status-${status}`}>
            <span className="status-dot" aria-hidden="true" />
            {t(`status.${status}`)}
        </span>
    );
}

/** 공식 글자 로고. 색은 글자색을 따른다 (그림을 가림막으로 쓴다) */
export function Wordmark({ className = '' }) {
    const { t, i18n } = useTranslation();
    const mark = wordmarkFor(i18n.language);
    return (
        <span
            className={`wordmark ${className}`}
            style={{
                '--wordmark': `url("${new URL(mark.src, document.baseURI).href}")`,
                aspectRatio: String(mark.ratio),
            } as React.CSSProperties}
        >
            <span className="sr-only">{t('app_name')}</span>
        </span>
    );
}

export function Empty({ art, children }: { art?: string; children?: React.ReactNode }) {
    return (
        <div className="empty">
            {art && <img className="empty-art" src={art} alt="" />}
            <p>{children}</p>
        </div>
    );
}

export function Loading() {
    const { t } = useTranslation();
    return (
        <p className="empty">
            <span className="spinner" aria-hidden="true" /> {t('common_loading')}
        </p>
    );
}

interface SectionProps {
    title?: React.ReactNode;
    action?: React.ReactNode;
    children?: React.ReactNode;
}

export function Section({ title, action, children }: SectionProps) {
    return (
        <section className="section">
            {(title || action) && (
                <header className="section-head">
                    <h2>{title}</h2>
                    {action}
                </header>
            )}
            {children}
        </section>
    );
}

/**
 * 디스코드 프로필 사진·서버 사진 (cdn.discordapp.com, 데몬·릴레이가 준 주소). 없거나 못 읽으면 이름 첫 글자.
 * 크기는 className 으로 정한다 (멤버 줄 `avatar`, 서버 칩 `guild-initial`)
 */
export function Avatar({ url, name, className = 'avatar' }: { url?: unknown; name?: string; className?: string }) {
    const [broken, setBroken] = useState<string | null>(null);
    const ok = typeof url === 'string' && url.startsWith('https://') && broken !== url;
    return (
        <span className={className} aria-hidden="true">
            {ok ? (
                <img
                    src={url}
                    alt=""
                    loading="lazy"
                    referrerPolicy="no-referrer"
                    draggable={false}
                    onError={() => setBroken(url)}
                />
            ) : (
                String(name ?? '')
                    .trim()
                    .charAt(0)
                    .toUpperCase() || '?'
            )}
        </span>
    );
}

interface CardProps {
    icon: string;
    /** 아이콘 바탕색 (styles.css .set-card-icon[data-tone]) */
    tone: string;
    title: React.ReactNode;
    desc?: React.ReactNode;
    action?: React.ReactNode;
    children?: React.ReactNode;
}

/** 설정 한 묶음 (앱 설정 화면의 카드와 같은 짜임): 색 아이콘·제목·한 줄 설명, 아래 칸들은 선으로 나눈다 */
export function Card({ icon, tone, title, desc, action, children }: CardProps) {
    const id = useId();
    return (
        <section className="set-card" aria-labelledby={id}>
            <header className="set-card-head">
                <span className="set-card-icon" data-tone={tone}>
                    <Icon name={icon} size={16} />
                </span>
                <h3 id={id}>{title}</h3>
                {action}
            </header>
            {desc && <p className="set-card-desc">{desc}</p>}
            <div className="set-card-body">{children}</div>
        </section>
    );
}

/** 고르기 줄. `scroll`이면 칸이 많아도 줄이지 않고 옆으로 민다 (서버 상세 탭) */
interface SegmentsProps {
    value: string;
    options: { value: string; label: React.ReactNode }[];
    onChange: (value: string) => void;
    scroll?: boolean;
    ariaLabel?: string;
}

export function Segments({ value, options, onChange, scroll = false, ariaLabel }: SegmentsProps) {
    return (
        <div className={scroll ? 'segments segments-scroll' : 'segments'} role="tablist" aria-label={ariaLabel}>
            {options.map((o) => (
                <button
                    key={o.value}
                    type="button"
                    role="tab"
                    aria-selected={value === o.value}
                    className={value === o.value ? 'on' : ''}
                    onClick={() => onChange(o.value)}
                >
                    {o.label}
                </button>
            ))}
        </div>
    );
}

/** 열린 창을 Esc 로 닫는다 */
function useEscape(onClose: (() => void) | null | undefined) {
    useEffect(() => {
        if (!onClose) return;
        const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
    }, [onClose]);
}

export function ConfirmHost() {
    const { t } = useTranslation();
    const confirm = useApp((s) => s.confirm);
    useEscape(confirm ? () => confirm.done(false) : null);
    if (!confirm) return null;
    // 겹치는 창은 body 바로 아래에 그린다: 움직이는 화면 상자 안이면 fixed가 그 상자를 기준으로 놓인다 (사용자 11-06 #1)
    return createPortal(
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 눌러 닫는 보조 동작 (키보드는 Esc)
        <div className="scrim" role="presentation" onClick={(e) => e.target === e.currentTarget && confirm.done(false)}>
            <div className="dialog" role="dialog" aria-modal="true">
                <h3>{confirm.title}</h3>
                {confirm.message && <p className="dialog-text">{confirm.message}</p>}
                <div className="dialog-actions">
                    <Button onClick={() => confirm.done(false)}>{t('cancel')}</Button>
                    <Button tone={confirm.danger ? 'danger' : 'accent'} onClick={() => confirm.done(true)}>
                        {confirm.confirmLabel || t('confirm')}
                    </Button>
                </div>
            </div>
        </div>,
        document.body,
    );
}

export function ToastHost() {
    const toasts = useApp((s) => s.toasts);
    return (
        <div className="toasts" aria-live="polite">
            {toasts.map((t) => (
                <div key={t.id} className={`toast toast-${t.tone}`}>
                    {t.message}
                </div>
            ))}
        </div>
    );
}

interface SheetProps {
    title: React.ReactNode;
    onClose: () => void;
    children?: React.ReactNode;
}

export function Sheet({ title, onClose, children }: SheetProps) {
    const { t } = useTranslation();
    useEscape(onClose);
    return createPortal(
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 눌러 닫는 보조 동작 (키보드는 Esc)
        <div className="scrim" role="presentation" onClick={(e) => e.target === e.currentTarget && onClose()}>
            <div className="sheet" role="dialog" aria-modal="true">
                <header className="sheet-head">
                    <h3>{title}</h3>
                    <button type="button" className="icon-btn" onClick={onClose} aria-label={t('close')}>
                        <Icon name="close" />
                    </button>
                </header>
                <div className="sheet-body">{children}</div>
            </div>
        </div>,
        document.body,
    );
}
