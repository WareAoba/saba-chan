import { type ReactNode, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { art } from '../art';
import { useApp } from '../store';
import { Icon } from './icons';
import { LanguageMenu } from './LanguageMenu';
import { InstallBand } from './NoPc';
import { NoticeBell } from './NoticeBell';
import { Wordmark } from './ui';
import type { WebApi, WebSession } from '../types';

/** 남은 시간 "14:05" */
export function remainingText(expiresAt: number | null, now: number) {
    const left = Math.max(0, Math.floor(((expiresAt ?? now) - now) / 1000));
    return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
}

/** children: 탭 메뉴. 넓은 화면은 이 줄 가운데, 휴대폰은 화면 아래에 붙는다 (styles.css .tabs).
 * PC가 붙은 세션이면 언어 버튼 앞에 알림 종 (PC 데몬의 알림함) */
export function SessionBar({
    session,
    api,
    children,
}: {
    session: WebSession | null;
    api?: WebApi;
    children?: ReactNode;
}) {
    const { t } = useTranslation();
    const { expiresAt, warning, phase, hostId } = useApp((s) => s.session);
    const [now, setNow] = useState(Date.now());
    useEffect(() => {
        const timer = setInterval(() => setNow(Date.now()), 1000);
        return () => clearInterval(timer);
    }, []);

    return (
        <header className="topbar">
            <div className="topbar-row">
                <h1 className="topbar-title">
                    <img className="topbar-logo" src={art.icon} alt="" />
                    <Wordmark className="topbar-wordmark" />
                    {!hostId && <span className="pill">{t('no_pc.badge')}</span>}
                </h1>
                {children}
                <div className={`timer${warning ? ' timer-warn' : ''}`} title={t('session.remaining')}>
                    <Icon name="clock" size={14} /> {remainingText(expiresAt, now)}
                </div>
                <button type="button" className="btn btn-plain btn-small" onClick={() => session?.extend()}>
                    {t('session.extend')}
                </button>
                {hostId && api && <NoticeBell api={api} />}
                <LanguageMenu />
            </div>
            {warning && <p className="banner banner-warn">{t('session.ending_soon')}</p>}
            {/* 머리 안에 둔다: 넓은 화면에서도 메뉴·내용 칸을 밀지 않고 두 칸을 다 쓴다 */}
            {!hostId && <InstallBand />}
            {phase === 'reconnecting' && (
                <p className="banner banner-info">
                    <span className="spinner" aria-hidden="true" /> {t('session.reconnecting')}
                </p>
            )}
        </header>
    );
}
