import { parseScreen } from '@gui/utils/screenAddress';
import type { TFunction } from 'i18next';
import { Trans, useTranslation } from 'react-i18next';
import { useApp } from '../store';

/**
 * 화면 주소(@gui/utils/screenAddress, 앱과 같다) → 웹에서 가는 길과 여는 일.
 * 웹은 디스코드 탭이 한 화면이라 디스코드 서버 안 탭까지는 가지 않는다. 웹에 없는 화면(설정·작업 기록)은 null
 */
function webScreen(to: string, t: TFunction) {
    const screen = parseScreen(to);
    if (screen?.page === 'discord') {
        return { path: [t('tabs.discord')], open: () => useApp.getState().setTab('discord') };
    }
    if (screen?.page === 'server') {
        return {
            path: [t('tabs.servers')],
            open: () => {
                useApp.getState().setTab('servers');
                useApp.getState().openServer(screen.id);
            },
        };
    }
    return null;
}

/** 안내 문구 속 화면 링크 (사용자 11-06 #19). 앱 ui/ScreenLink와 같은 모양 */
export function ScreenLink({ to }: { to: string }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const target = webScreen(to, t);
    if (!target) return null;
    const path = target.path.join(' › ');
    return (
        <button type="button" className="screen-link" aria-label={tg('links.open', { path })} onClick={target.open}>
            {path}
            <span className="screen-link-arrow" aria-hidden="true">
                ↗
            </span>
        </button>
    );
}

/** 번역 문구의 <link/> 자리에 화면 링크를 넣는다 */
export function LinkedText({ i18nKey, to, ns }: { i18nKey: string; to: string; ns?: string }) {
    const { t } = useTranslation(ns);
    return <Trans t={t} i18nKey={i18nKey} components={{ link: <ScreenLink to={to} /> }} />;
}
