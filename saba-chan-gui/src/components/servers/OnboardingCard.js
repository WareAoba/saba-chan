import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUIStore } from '../../stores/useUIStore';
import { Icon } from '../Icon';

/**
 * 다음 할 일 안내 (J1-a). 데몬 `GET /api/onboarding`이 상태로 계산한 다음 단계를 보인다.
 * 채널이 없거나 남은 필수 단계가 없으면 그리지 않는다. `refreshKey`가 바뀌면 다시 묻는다.
 */
export function OnboardingCard({ onAddServer, refreshKey }) {
    const { t } = useTranslation('gui');
    const setActiveTab = useUIStore((s) => s.setActiveTab);
    const [state, setState] = useState(null);

    useEffect(() => {
        if (!window.api?.onboarding) return;
        let alive = true;
        window.api
            .onboarding()
            .then((res) => alive && setState(res?.steps ? res : null))
            .catch(() => alive && setState(null));
        return () => {
            alive = false;
        };
    }, [refreshKey]);

    const next = state?.next;
    if (!next) return null;

    const required = state.steps.filter((s) => !s.optional);
    const done = required.filter((s) => s.done).length;
    const go = next === 'module' || next === 'server' ? onAddServer : () => setActiveTab('discord');

    return (
        <section className="onboarding-card" aria-label={t('onboarding.title')}>
            <div className="onboarding-head">
                <span className="onboarding-title">
                    <Icon name="lightbulb" size="sm" /> {t('onboarding.title')}
                </span>
                <span className="onboarding-progress">{`${done}/${required.length}`}</span>
            </div>
            <strong className="onboarding-step">{t(`onboarding.step_${next}`)}</strong>
            <p className="onboarding-desc">{t(`onboarding.desc_${next}`)}</p>
            <button type="button" className="btn btn-primary btn-sm" onClick={go}>
                {t(`onboarding.go_${next}`)}
            </button>
        </section>
    );
}
