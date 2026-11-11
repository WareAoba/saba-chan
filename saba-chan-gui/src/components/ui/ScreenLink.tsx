import type { TFunction } from 'i18next';
import { Fragment } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { openScreen } from '../../utils/openScreen';
import { type ScreenAddress, parseScreen } from '../../utils/screenAddress';
import { SETTINGS_TAB_LABELS } from '../Modals/settingsTabs';

// 디스코드 서버 탭 중 이름 키가 discord.tab_<탭>이 아닌 것
const NODE_TAB_LABELS: Record<string, string> = {
    chatbot: 'chatbot.nav',
    music: 'discord.bot_settings.music_title',
    tts: 'discord.bot_settings.tts_title',
};

/** 화면 → 지금 화면에서 가는 길 (디스코드 › 서버 관리 › 채널). 이름을 모르는 탭은 길에서 뺀다 */
export function screenPath(screen: ScreenAddress, t: TFunction): string[] {
    const known = (key: string) => (t(key, { defaultValue: '' }) as string) || null;
    const steps: (string | null | undefined)[] = [];
    switch (screen.page) {
        case 'server':
            steps.push(t('shell.nav_servers'), screen.tab && known(`shell.tab_${screen.tab}`));
            break;
        case 'discord':
            steps.push(t('shell.nav_discord'));
            if (screen.view !== 'node') steps.push(t(`discord.nav_${screen.view}`));
            else if (screen.tab) {
                steps.push(known(NODE_TAB_LABELS[screen.tab] ?? `discord.tab_${screen.tab}`));
                if (screen.tab === 'moderation' && screen.sub) steps.push(known(`discord.mod.view_${screen.sub}`));
            }
            break;
        case 'settings':
            steps.push(t('shell.nav_settings'));
            if (screen.tab && screen.tab in SETTINGS_TAB_LABELS) {
                steps.push(t(SETTINGS_TAB_LABELS[screen.tab as keyof typeof SETTINGS_TAB_LABELS]));
                if (screen.tab === 'sabaStorage' && screen.sub) steps.push(known(`saba_storage.tab_${screen.sub}`));
            }
            break;
        case 'activity':
            steps.push(t('shell.nav_activity'));
            break;
    }
    return steps.filter((s): s is string => !!s);
}

/**
 * 안내 문구 속 화면 링크 (사용자 11-06 #19): 가는 길을 보이고, 누르면 그 화면을 연다.
 * 읽지 못한 주소면 아무것도 그리지 않는다.
 */
export function ScreenLink({ to }: { to: string }) {
    const { t } = useTranslation('gui');
    const screen = parseScreen(to);
    if (!screen) return null;
    const path = screenPath(screen, t);
    return (
        <button
            type="button"
            className="screen-link"
            aria-label={t('links.open', { path: path.join(' › ') })}
            onClick={() => openScreen(to)}
        >
            {path.map((step, i) => (
                <Fragment key={step}>
                    {i > 0 && (
                        <span className="screen-link-sep" aria-hidden="true">
                            ›
                        </span>
                    )}
                    {step}
                </Fragment>
            ))}
            <span className="screen-link-arrow" aria-hidden="true">
                ↗
            </span>
        </button>
    );
}

/** 번역 문구의 <link/> 자리에 화면 링크를 넣는다 */
export function LinkedText({ i18nKey, to, values }: { i18nKey: string; to: string; values?: Record<string, unknown> }) {
    const { t } = useTranslation('gui');
    return <Trans t={t} i18nKey={i18nKey} values={values} components={{ link: <ScreenLink to={to} /> }} />;
}
