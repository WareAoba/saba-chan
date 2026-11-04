import { type MouseEventHandler, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { Icon } from '../Icon';
import { startAurora, startStars } from './aboutSky';
import './AboutPage.css';

/**
 * 설정 › 정보 페이지 본문 (2026-11-05 시안 확정).
 * 위에서부터 버전 카드 → 사바쨩 자기소개 → 제작자 카드(별밤·오로라).
 * active 일 때만 밤하늘을 움직인다 (정보 페이지는 닫혀 있어도 그려져 있다).
 */

const COMPONENTS = [
    { key: 'saba-core', label: 'Core', icon: 'package' },
    { key: 'gui', label: 'GUI', icon: 'monitor' },
    { key: 'cli', label: 'CLI', icon: 'terminal' },
    { key: 'updater', label: 'Updater', icon: 'refresh' },
];

const DAY = [
    { key: 'cook', src: './saba/cooking.webp', tone: 'pink' },
    { key: 'deliver', src: './saba/relay.webp', tone: 'sky' },
    { key: 'care', src: './saba/recovering.webp', tone: 'mint' },
    { key: 'sleep', src: './saba/sleep.webp', tone: 'lilac' },
];

/** 컴포넌트 버전 표시. 모르면 대시 */
const versionLabel = (v?: string | null) => (v ? `v${v}` : '—');

function openExternal(url: string) {
    if (window.electron?.shell?.openExternal) window.electron.shell.openExternal(url);
    else window.open(url, '_blank');
}

/* 펜으로 그린 낙서들. 질감은 .about-intro-card 의 #about-pen 필터가 입힌다 */
const Heart = ({ fill = 'none', stroke = '#d9649a', size = 18 }) => (
    <svg className="about-pen" width={size} height={size * 0.9} viewBox="0 0 18 16" aria-hidden="true">
        <path d="M9 14C4 11 1.5 8 1.5 5 1.5 3 3 1.5 5 1.5c1.6 0 3 1 4 2.4C10 2.5 11.4 1.5 13 1.5c2 0 3.5 1.5 3.5 3.5 0 3-2.5 6-7.5 9z" fill={fill} stroke={stroke} strokeWidth="1.7" strokeLinejoin="round" />
    </svg>
);

const CHIP_ICONS: Record<string, React.ReactElement> = {
    talk: (
        <path d="M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2z" fill="none" stroke="#7a6a8a" strokeWidth="1.5" strokeLinejoin="round" />
    ),
    sing: (
        <>
            <path d="M6 12.5V3.5l7-1.5v8.5" fill="none" stroke="#e0679a" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
            <ellipse cx="4.6" cy="12.6" rx="1.8" ry="1.4" fill="#ffd0e2" stroke="#e0679a" strokeWidth="1.3" />
            <ellipse cx="11.6" cy="10.8" rx="1.8" ry="1.4" fill="#ffd0e2" stroke="#e0679a" strokeWidth="1.3" />
        </>
    ),
    read: (
        <>
            <path d="M2 6.5h2.5L8 3.5v9L4.5 9.5H2z" fill="#d6f5ec" stroke="#5aa891" strokeWidth="1.5" strokeLinejoin="round" />
            <path d="M10.5 5.5c1.3 1.4 1.3 3.6 0 5M12.5 4c2.2 2.3 2.2 5.7 0 8" fill="none" stroke="#5aa891" strokeWidth="1.4" strokeLinecap="round" />
        </>
    ),
};

/* 하루 그림 둘레의 손그림 동그라미 (조금씩 다르게 떨린다) */
const RINGS: Record<string, string[]> = {
    pink: ['M50 4C76 3 97 22 96 50c-1 27-22 46-47 45C22 94 3 75 5 48 7 23 26 5 52 6', '#fff3f8', '#ff9fc4'],
    sky: ['M50 5C75 4 96 24 95 51c-2 26-22 45-48 43C21 92 4 73 6 47 8 22 27 4 53 7', '#f1f6ff', '#a9c7ff'],
    mint: ['M50 4C77 5 96 25 95 50c-1 26-21 46-47 45C21 94 4 74 5 48 7 23 27 3 51 6', '#effbf7', '#9fe3d0'],
    lilac: ['M50 5C76 3 97 23 95 50c-2 27-22 45-47 44C22 93 4 74 6 48 7 22 27 4 52 6', '#f6f2ff', '#c9b6ff'],
};

interface VersionCardProps {
    appVersion?: string | null;
    componentInfo?: ApiBody | null;
    onLogoClick?: MouseEventHandler<HTMLImageElement>;
    lastUpdated?: string | null;
}

function VersionCard({ appVersion, componentInfo, onLogoClick, lastUpdated }: VersionCardProps) {
    const { t } = useTranslation('gui');
    return (
        <section className="about-card about-app-card">
            <div className="about-app-ring">
                {appVersion && <span className="about-version-badge">v{appVersion}</span>}
                <img src="./title.png" alt="Saba-chan" className="about-app-logo" onClick={onLogoClick} />
            </div>
            <div className="about-app-info">
                <div className="about-app-name">
                    <strong>Saba-chan</strong>
                    <span>{t('settings_modal.about_tagline')}</span>
                </div>
                <div className="about-component-list">
                    {COMPONENTS.map((c) => (
                        <div key={c.key} className="about-component-item">
                            <span className="about-component-icon"><Icon name={c.icon} size="sm" /></span>
                            <span className="about-component-text">
                                <span className="about-component-label">{c.label}</span>
                                <span className="about-component-ver">{versionLabel(componentInfo?.components?.[c.key] || appVersion)}</span>
                            </span>
                        </div>
                    ))}
                </div>
                <div className="about-app-footer">
                    {lastUpdated && (
                        <span className="about-app-meta"><Icon name="clock" size="xs" /> {lastUpdated}</span>
                    )}
                    <span className="about-app-meta"><Icon name="file" size="xs" /> MIT License</span>
                </div>
            </div>
        </section>
    );
}

function IntroCard() {
    const { t } = useTranslation('gui');
    const k = (key: string) => t(`settings_modal.about_intro.${key}`);
    return (
        <section className="about-card about-intro-card">
            <svg width="0" height="0" className="about-pen-defs" aria-hidden="true">
                {/* 선을 살짝 떨게 하고, 잉크가 군데군데 옅게 남는다 */}
                <filter id="about-pen" x="-10%" y="-10%" width="120%" height="120%">
                    <feTurbulence type="fractalNoise" baseFrequency="0.8" numOctaves="2" seed="4" result="n" />
                    <feDisplacementMap in="SourceGraphic" in2="n" scale="1.8" xChannelSelector="R" yChannelSelector="G" result="w" />
                    <feTurbulence type="fractalNoise" baseFrequency="2.2" numOctaves="1" seed="9" result="g" />
                    <feColorMatrix in="g" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 -1.4 1.25" result="gm" />
                    <feComposite in="w" in2="gm" operator="in" />
                </filter>
            </svg>
            <svg className="about-pen about-intro-sparkles" viewBox="0 0 70 60" aria-hidden="true">
                <path className="about-twinkle" d="M20 6c1 8 3 10 11 11-8 1-10 3-11 11-1-8-3-10-11-11 8-1 10-3 11-11z" stroke="#f3b33d" strokeWidth="2.2" fill="#fff6c9" strokeLinejoin="round" />
                <path className="about-twinkle" d="M52 30c.6 5 2 6 7 7-5 .6-6 2-7 7-.6-5-2-6-7-7 5-1 6-2 7-7z" stroke="#e0679a" strokeWidth="2" fill="none" strokeLinejoin="round" />
            </svg>

            <div className="about-intro-top">
                <figure className="about-polaroid">
                    <span className="about-polaroid-pin"><Heart fill="#ffd0e2" size={26} /></span>
                    <div className="about-polaroid-shot">
                        <img src="./saba/mermaid-upper.webp" alt="" />
                    </div>
                    <figcaption>{k('photo_caption')} <Heart /></figcaption>
                </figure>
                <div className="about-intro-hello">
                    <h3>{k('hello')}</h3>
                    <span className="about-intro-alias">サバちゃん · {k('alias')}</span>
                    <p>{k('body')}</p>
                    <dl className="about-intro-notes">
                        <dt>{k('motif_label')}</dt>
                        <dd>
                            <mark className="about-hl-mint">{k('motif')}</mark>
                            <svg className="about-pen" width="40" height="19" viewBox="0 0 46 22" aria-hidden="true">
                                <path d="M3 11c6-8 18-9 28-3l9-6-2 9 2 9-9-6C21 20 9 19 3 11z" fill="#dbe7ff" stroke="#5f79d6" strokeWidth="1.8" strokeLinejoin="round" />
                                <circle cx="10" cy="9.5" r="1.6" fill="#5f79d6" />
                                <path d="M16 6c2 3 2 7 0 10M21 5c2 4 2 8 0 12" stroke="#5f79d6" strokeWidth="1.4" fill="none" strokeLinecap="round" />
                            </svg>
                        </dd>
                        <dt>{k('personality_label')}</dt>
                        <dd>{k('personality')} <small>{k('personality_note')}</small></dd>
                        <dt>{k('likes_label')}</dt>
                        <dd>{k('likes')}</dd>
                    </dl>
                </div>
            </div>

            <div className="about-intro-notes-row">
                <div className="about-intro-sticky about-intro-sticky-pink">
                    <h4>{k('sister_title')} <Heart fill="#ffe1ec" stroke="#c85b8b" /></h4>
                    <div className="about-intro-sister">
                        <img src="./server-chan.webp" alt="" />
                        <span><b>{k('sister_name')}</b><br />{k('sister_desc')}</span>
                    </div>
                </div>
                <div className="about-intro-sticky">
                    <h4>{k('discord_title')}</h4>
                    <div className="about-intro-chips">
                        {['talk', 'sing', 'read'].map((c) => (
                            <span key={c} className={`about-intro-chip about-intro-chip-${c}`}>
                                <svg className="about-pen" width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">{CHIP_ICONS[c]}</svg>
                                {k(c)}
                            </span>
                        ))}
                    </div>
                </div>
            </div>

            <div className="about-intro-day">
                <h4>
                    {k('day_title')}
                    <svg className="about-pen" width="34" height="20" viewBox="0 0 34 20" aria-hidden="true">
                        <path d="M8 17h18a6 6 0 0 0 0-12 8 8 0 0 0-15 1 5 5 0 0 0-3 11z" fill="#eef4ff" stroke="#8aa6e8" strokeWidth="1.8" strokeLinejoin="round" />
                    </svg>
                </h4>
                <div className="about-intro-day-cells">
                    {DAY.map(({ key, src, tone }) => {
                        const [d, fill, stroke] = RINGS[tone];
                        return (
                            <div key={key} className="about-intro-day-cell">
                                <div className="about-intro-day-frame">
                                    <svg className="about-pen" viewBox="0 0 100 100" aria-hidden="true">
                                        <path d={d} fill={fill} stroke={stroke} strokeWidth="2.4" strokeLinecap="round" />
                                    </svg>
                                    <img src={src} alt="" />
                                </div>
                                <span>{k(`day_${key}`)}</span>
                            </div>
                        );
                    })}
                </div>
            </div>
        </section>
    );
}

function DevCard({ active }: { active: boolean }) {
    const { t } = useTranslation('gui');
    const stars = useRef<HTMLCanvasElement>(null);
    const aurora = useRef<HTMLCanvasElement>(null);
    useEffect(() => {
        if (!active) return undefined;
        const stopStars = startStars(stars.current!);
        const stopAurora = startAurora(aurora.current!);
        return () => { stopStars(); stopAurora(); };
    }, [active]);
    return (
        <section className="about-card about-dev-card">
            <canvas ref={stars} className="about-sky-stars" aria-hidden="true" />
            <canvas ref={aurora} className="about-sky-aurora" aria-hidden="true" />
            <div className="about-dev-avatar">
                <img
                    src="https://github.com/WareAoba.png"
                    alt="WareAoba"
                    className="about-dev-img"
                    onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
                />
            </div>
            <div className="about-dev-info">
                <span className="about-made-by">{t('settings_modal.made_by')}</span>
                <span className="about-dev-id">@WareAoba</span>
                <span className="about-dev-bio">{t('settings_modal.dev_bio')}</span>
            </div>
            <button className="about-dev-github-btn" onClick={() => openExternal('https://github.com/WareAoba/saba-chan')}>
                <Icon name="github" size="sm" />
                <span>{t('settings_modal.github_repo')}</span>
            </button>
        </section>
    );
}

interface AboutPageProps extends VersionCardProps {
    active: boolean;
}

export function AboutPage({ active, appVersion, componentInfo, lastUpdated, onLogoClick }: AboutPageProps) {
    return (
        <div className="about-scroll-area">
            <VersionCard appVersion={appVersion} componentInfo={componentInfo} lastUpdated={lastUpdated} onLogoClick={onLogoClick} />
            <IntroCard />
            <DevCard active={active} />
        </div>
    );
}
