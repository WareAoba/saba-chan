import clsx from 'clsx';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { openScreen } from '../../utils/openScreen';
import { closeGuide, openAddServer, useGuideStore } from '../guides/guides';
import { Icon } from '../Icon';

/** 단계 id → 그 단계로 가는 버튼 (없으면 버튼 없이 설명만) */
const GO: Record<string, (() => void) | undefined> = {
    discord: () => openScreen('discord/connection'),
    module: () => useGuideStore.getState().setStep('modules'),
    server: () => openAddServer(),
    chatbot: () => openScreen('discord/saba'),
};

/**
 * 처음 시작하기 체크리스트 (J1-a, 사용자 11-08~10). 데몬 `GET /api/onboarding`의 단계를 순서대로 보이고,
 * 다음 단계만 펼쳐 설명과 버튼을 단다. 남은 필수 단계가 없거나 ✕로 닫았으면 그리지 않는다.
 */
export function OnboardingCard() {
    const { t } = useTranslation('gui');
    const state = useGuideStore((s) => s.onboarding);
    const closed = useSettingsStore((s) => s.closedGuides.includes('checklist'));

    const next = state?.next;
    if (!next || closed) return null;

    const required = state.steps.filter((s) => !s.optional);
    const optional = state.steps.filter((s) => s.optional && !s.done);
    const done = required.filter((s) => s.done).length;

    return (
        <section className="onboarding-card" aria-label={t('onboarding.title')}>
            <div className="onboarding-head">
                <span className="onboarding-title">{t('onboarding.title')}</span>
                <span className="onboarding-progress">{`${done} / ${required.length}`}</span>
                <button
                    type="button"
                    className="icon-btn onboarding-close"
                    onClick={() => closeGuide('checklist')}
                    aria-label={t('guides.close')}
                >
                    <Icon name="close" size="xs" />
                </button>
            </div>
            <ol className="onboarding-steps">
                {required.map((step, i) => {
                    const current = step.id === next;
                    const go = GO[step.id];
                    return (
                        <li key={step.id} className={clsx('onboarding-step', { done: step.done, current })}>
                            <span className="onboarding-mark" aria-hidden="true">
                                {step.done ? <Icon name="check" size="xs" /> : i + 1}
                            </span>
                            <span className="onboarding-text">
                                <strong>{t(`onboarding.step_${step.id}`)}</strong>
                                {current && <span className="onboarding-desc">{t(`onboarding.desc_${step.id}`)}</span>}
                                {current && go && (
                                    <button type="button" className="btn btn-primary btn-sm" onClick={go}>
                                        {t(`onboarding.go_${step.id}`)}
                                    </button>
                                )}
                            </span>
                        </li>
                    );
                })}
                {optional.map((step) => (
                    <li key={step.id} className="onboarding-step optional">
                        <span className="onboarding-mark" aria-hidden="true">
                            <Icon name="plus" size="xs" />
                        </span>
                        <span className="onboarding-text">
                            <button type="button" className="guide-link" onClick={GO[step.id]}>
                                {t(`onboarding.step_${step.id}`)}
                            </button>
                            <span className="onboarding-optional">{t('onboarding.optional')}</span>
                        </span>
                    </li>
                ))}
            </ol>
        </section>
    );
}

/** 시작 버튼 말풍선: 체크리스트의 다음 단계가 첫 시작일 때 (닫으면 이번 실행 동안 안 보인다) */
export function StartCoach() {
    const { t } = useTranslation('gui');
    const next = useGuideStore((s) => s.onboarding?.next);
    const dismissed = useGuideStore((s) => s.coachDismissed);
    const closed = useSettingsStore((s) => s.closedGuides.includes('checklist'));
    if (next !== 'start' || dismissed || closed) return null;
    return (
        <div className="start-coach" role="note">
            <span>{t('guides.start_coach')}</span>
            <button type="button" className="guide-link" onClick={() => useGuideStore.setState({ coachDismissed: true })}>
                {t('guides.close')}
            </button>
        </div>
    );
}
