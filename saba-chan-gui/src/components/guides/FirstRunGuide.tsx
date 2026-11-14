import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import { localizedDescription, localizedName } from '../../utils/localizedText';
import type { CatalogModule } from '../../utils/moduleSupport';
import { claimOpeningSound } from '../../utils/opening';
import { openScreen } from '../../utils/openScreen';
import { cachedModuleCatalog, loadModuleCatalog } from '../../utils/storeCache';
import { Icon } from '../Icon';
import { AddonIcon } from '../ui/AddonIcon';
import { SabaSpinner } from '../ui/SabaUI';
import { closeGuide, type GuideStep, openAddServer, startAddServer, useGuideStore } from './guides';

/**
 * 처음 시작하기 (사용자 11-08~10): 서버가 없을 때 서버 화면.
 * welcome 디스코드 연결부터 → fork 쓰던 서버 불러오기 / 새로 만들기 → (모듈이 없으면) modules 게임 모듈 받기.
 */
export function FirstRunGuide({ step }: { step: GuideStep }) {
    const { t } = useTranslation('gui');
    const modules = useServerStore((s) => s.modules);
    const discordDone = useGuideStore((s) => s.onboarding?.steps.find((x) => x.id === 'discord')?.done);
    const setStep = useGuideStore((s) => s.setStep);
    const hasModules = modules.length > 0;
    // 가이드가 열릴 때 한 번 가이드 열림음 (셸 `sound_play`, 설정 sounds를 따른다). 단계를 넘길 때는 내지 않는다
    useEffect(() => {
        // 앱을 막 연 참이면 오프닝이 이 소리를 맡아 기동음 대신 화음에 맞춰 낸다 (utils/opening.ts)
        if (!claimOpeningSound('guideOpen')) {
            (window.api as { playSound?: (name: 'guideOpen') => unknown } | undefined)?.playSound?.('guideOpen');
        }
    }, []);
    // 모듈 받기 화면의 ✕는 그 화면만 닫는다 (처음 시작하기 전체는 welcome·fork의 ✕)
    const close = () => (step === 'modules' ? setStep(null) : closeGuide('firstRun'));

    return (
        <div className="page page-empty guide-page">
            <section className="guide-card" aria-label={t('guides.first_run')}>
                <button type="button" className="icon-btn guide-close" onClick={close} aria-label={t('guides.close')}>
                    <Icon name="close" size="sm" />
                </button>
                {step === 'welcome' && (
                    <div className="guide-welcome">
                        {/* 로딩 화면과 같은 빛 번지는 그라데이션 링 안의 사바쨩 프로필 (사용자 11-10) */}
                        <div className="guide-hero" aria-hidden="true">
                            <span className="guide-hero-glow" />
                            <span className="guide-hero-ring" />
                            <span className="guide-hero-mask" />
                            <img src="./icon.png" alt="" className="guide-avatar" />
                        </div>
                        <h2>{t('guides.welcome_title')}</h2>
                        <p>{t('guides.welcome_body')}</p>
                        <div className="guide-actions">
                            <button type="button" className="btn btn-ghost" onClick={() => setStep('fork')}>
                                {t('guides.later')}
                            </button>
                            <button
                                type="button"
                                className="btn btn-primary"
                                onClick={() => {
                                    // 돌아오면 다음 단계(갈림길)부터
                                    setStep('fork');
                                    openScreen('discord/connection');
                                }}
                            >
                                <Icon name="discord" size="sm" /> {t('guides.connect_discord')}
                            </button>
                        </div>
                    </div>
                )}
                {step === 'fork' && (
                    <div className="guide-fork">
                        {discordDone && (
                            <span className="guide-pill">
                                <Icon name="check" size="xs" /> {t('guides.discord_linked')}
                            </span>
                        )}
                        <h2>{t('guides.fork_title')}</h2>
                        <p>{t('guides.fork_body')}</p>
                        <div className="guide-choices">
                            <button type="button" className="guide-choice" onClick={() => startAddServer('migrate', hasModules)}>
                                <Icon name="folder" size="md" />
                                <strong>{t('guides.import_title')}</strong>
                                <span>{t('guides.import_desc')}</span>
                                <span className="guide-choice-go">
                                    {t('guides.import_go')} <Icon name="chevronRight" size="xs" />
                                </span>
                            </button>
                            <button type="button" className="guide-choice" onClick={() => startAddServer('new', hasModules)}>
                                <Icon name="plus" size="md" />
                                <strong>{t('guides.new_title')}</strong>
                                <span>{t('guides.new_desc')}</span>
                                <span className="guide-choice-go">
                                    {t('guides.new_go')} <Icon name="chevronRight" size="xs" />
                                </span>
                            </button>
                        </div>
                        <p className="guide-note">{t('guides.fork_note')}</p>
                    </div>
                )}
                {step === 'modules' && <ModuleGuide />}
            </section>
        </div>
    );
}

/** 게임 모듈 받기: 사바 스토리지가 받는 목록 그대로, 받으면 원래 하던 곳으로 이어 간다 */
function ModuleGuide() {
    const { t, i18n } = useTranslation('gui');
    const installed = useServerStore((s) => s.modules);
    const after = useGuideStore((s) => s.after);
    const [catalog, setCatalog] = useState<CatalogModule[] | null>(cachedModuleCatalog);
    const [failed, setFailed] = useState(false);
    const [busy, setBusy] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        let alive = true;
        (async () => {
            try {
                const res = await loadModuleCatalog();
                if (!alive) return;
                if (res.ok) setCatalog(res.modules);
                else setFailed(true);
            } catch {
                if (alive) setFailed(true);
            }
        })();
        return () => {
            alive = false;
        };
    }, []);

    const install = async (id: string) => {
        setBusy(id);
        setError(null);
        try {
            const res = await window.api?.moduleInstallFromManifest?.(id);
            if (!res?.success) {
                setError(t('guides.module_failed'));
                return;
            }
            await useServerStore.getState().fetchModules();
            // 체크리스트의 "게임 받기"도 바로 끝난 것으로 보이게
            void useGuideStore.getState().refreshOnboarding();
            if (after) openAddServer(after);
        } catch {
            setError(t('guides.module_failed'));
        } finally {
            setBusy(null);
        }
    };

    const shown = (catalog || []).filter((m) => m.platform_supported !== false);
    return (
        <div className="guide-modules">
            <h2>{t('guides.modules_title')}</h2>
            <p>{t('guides.modules_body')}</p>
            {!catalog && !failed && (
                <div className="guide-loading">
                    <SabaSpinner size="md" />
                </div>
            )}
            {failed && !catalog && <p className="guide-error">{t('guides.modules_load_failed')}</p>}
            {shown.length > 0 && (
                <ul className="guide-module-list">
                    {shown.map((m) => {
                        const have = installed.some((x) => x.name === m.id);
                        return (
                            <li key={m.id} className="guide-module">
                                <AddonIcon storeKey={`module-${m.id}`} hasIcon={m.has_icon} version={m.version} fallback="gamepad" />
                                <span className="guide-module-text">
                                    <strong>{localizedName(m, i18n.language) || m.id}</strong>
                                    <span>{localizedDescription(m, i18n.language)}</span>
                                </span>
                                {have ? (
                                    <span className="guide-module-done">
                                        <Icon name="check" size="xs" /> {t('guides.module_installed')}
                                    </span>
                                ) : (
                                    <button
                                        type="button"
                                        className="btn btn-primary btn-sm"
                                        disabled={busy !== null}
                                        onClick={() => install(m.id)}
                                    >
                                        {busy === m.id ? <SabaSpinner size="sm" /> : t('guides.module_get')}
                                    </button>
                                )}
                            </li>
                        );
                    })}
                </ul>
            )}
            {error && <p className="guide-error">{error}</p>}
            <p className="guide-note">
                {t('guides.modules_more')}{' '}
                <button type="button" className="btn btn-ghost btn-sm" onClick={() => openScreen('settings/sabaStorage/modules')}>
                    {t('guides.open_storage')}
                </button>
            </p>
        </div>
    );
}
