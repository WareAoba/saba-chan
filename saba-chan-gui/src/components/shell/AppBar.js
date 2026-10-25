import clsx from 'clsx';
import { useTranslation } from 'react-i18next';
import BackgroundModal from '../Modals/BackgroundModal';
import NoticeModal from '../Modals/NoticeModal';
import ExtensionSlot from '../ExtensionSlot';
import { Icon } from '../Icon';
import { ConnectionRail } from './ConnectionRail';
import { ParentJobNotice } from './ParentJobNotice';
import { RuntimePreflight } from './RuntimePreflight';
import { SwitchToCliButton } from './SwitchToCli';
import { TaskIndicator } from './TaskIndicator';

/** 공식 글자 로고: 한국어·일본어·영어는 각자, 나머지 언어는 영어. ratio = 가로/세로 */
const WORDMARKS = {
    ko: { src: './logo-kr.png', ratio: 996 / 300 },
    ja: { src: './logo-jp.png', ratio: 1459 / 300 },
    en: { src: './logo-en.png', ratio: 2236 / 300 },
};
export function wordmarkFor(lang) {
    const l = String(lang || 'en').toLowerCase();
    if (l.startsWith('ko')) return WORDMARKS.ko;
    if (l.startsWith('ja')) return WORDMARKS.ja;
    return WORDMARKS.en;
}

/** 상단 탭 = 큰 기능 한 개 = 한 페이지 (A7). 새 페이지는 여기에 한 줄 추가한다. */
export const APP_PAGES = [
    { id: 'servers', icon: 'server', labelKey: 'shell.nav_servers' },
    { id: 'discord', icon: 'discord', labelKey: 'shell.nav_discord' },
    { id: 'settings', icon: 'cog', labelKey: 'shell.nav_settings' },
];

/**
 * 창 맨 위 한 줄: 로고 · 페이지 탭 · 상태 · 알림 · 창 버튼.
 * 예전의 제목 표시줄 + 큰 로고 머리 + 버튼 줄(세 줄)을 하나로 합쳤다.
 */
export function AppBar({
    page,
    onNavigate,
    daemonStatus,
    daemonInParentJob,
    discordStatus,
    unreadCount,
    extInitializing,
    extInitLabel,
    showNotice,
    onToggleNotice,
    noticeProps,
    showDaemon,
    onToggleDaemon,
    daemonProps,
}) {
    const { t, i18n } = useTranslation('gui');
    const wordmark = wordmarkFor(i18n.language);

    return (
        <header className="appbar">
            <div className="appbar-brand">
                <img src="./icon.png" alt="" className="appbar-logo" />
                {/* 검은 글자 그림을 마스크로 써서 테마 글자색으로 칠한다.
                    커스텀 속성 안 상대 url()은 스타일시트(assets/) 기준으로 풀리므로 절대 주소로 넘긴다 */}
                <span
                    className="appbar-name appbar-wordmark"
                    style={{ '--wordmark': `url("${new URL(wordmark.src, document.baseURI).href}")`, aspectRatio: String(wordmark.ratio) }}
                >
                    <span className="sr-only">{t('common:app_name')}</span>
                </span>
            </div>

            <nav className="appbar-tabs" aria-label={t('shell.nav_label')}>
                {APP_PAGES.map((p) => (
                    <button
                        key={p.id}
                        type="button"
                        className={clsx('appbar-tab', { active: page === p.id })}
                        aria-current={page === p.id ? 'page' : undefined}
                        onClick={() => onNavigate(p.id)}
                    >
                        <Icon name={p.icon} size="sm" />
                        <span>{t(p.labelKey)}</span>
                    </button>
                ))}
            </nav>

            <div className="appbar-spacer" />

            <div className="appbar-status">
                <ExtensionSlot slotId="AppBar.status" />
                <TaskIndicator />
                <RuntimePreflight daemonStatus={daemonStatus} />
                {daemonInParentJob && <ParentJobNotice />}
                {extInitializing && (
                    <span className="appbar-ext-init" title={extInitLabel}>
                        <span className="spinner" aria-hidden="true" />
                    </span>
                )}
                <div className="popover-anchor">
                    <ConnectionRail
                        daemonStatus={daemonStatus}
                        botStatus={discordStatus}
                        onDaemonClick={onToggleDaemon}
                        daemonExpanded={showDaemon}
                        onBotClick={() => onNavigate('discord')}
                    />
                    <BackgroundModal {...daemonProps} isOpen={showDaemon} />
                </div>
                <div className="popover-anchor">
                    <button
                        type="button"
                        className="icon-btn"
                        onClick={onToggleNotice}
                        data-tooltip={t('notice_modal.tooltip')}
                        aria-label={t('notice_modal.tooltip')}
                        aria-expanded={showNotice}
                    >
                        <Icon name="bell" size="sm" />
                        {unreadCount > 0 && <span className="badge-count">{unreadCount > 9 ? '9+' : unreadCount}</span>}
                    </button>
                    <NoticeModal {...noticeProps} isOpen={showNotice} />
                </div>
                <SwitchToCliButton />
            </div>

            <WindowControls />
        </header>
    );
}

/** 창 최소화·최대화·닫기 (프레임 없는 창). 로딩 화면에서도 쓴다. */
export function WindowControls() {
    const { t } = useTranslation('gui');
    return (
        <div className="appbar-window">
            <button
                type="button"
                className="win-btn"
                onClick={() => window.electron?.minimizeWindow()}
                title={t('title_bar.minimize')}
            >
                <Icon name="minimize" size={12} />
            </button>
            <button
                type="button"
                className="win-btn"
                onClick={() => window.electron?.maximizeWindow()}
                title={t('title_bar.maximize')}
            >
                <Icon name="maximize" size={12} />
            </button>
            <button
                type="button"
                className="win-btn win-close"
                onClick={() => window.electron?.closeWindow()}
                title={t('title_bar.close')}
            >
                <Icon name="close" size={12} />
            </button>
        </div>
    );
}
