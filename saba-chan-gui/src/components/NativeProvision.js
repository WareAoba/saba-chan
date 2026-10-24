/**
 * NativeProvision — 서버 설치(프로비저닝) 카드. 설치를 "요리"로 보여 준다.
 * 설치 중: 요리하는 사바쨩 + 진행 막대 / 끝: 접시에 서버 아이콘을 올린 완성 카드 / 실패: 쓰러진 사바쨩.
 *
 * Props:
 *   server: 서버 인스턴스
 *   provisionProgress: 데몬의 설치 진행 상태 (done, error, message, percent)
 *   cooked: 설치가 성공으로 끝난 것을 봤다 (데몬이 진행 표시를 지운 뒤에도 남는다)
 *   icon: 접시에 올릴 서버 아이콘
 *   onDismiss: 실패 카드 닫기 (실패한 새 인스턴스는 데몬이 롤백한다)
 *   onClose: 완성 카드 닫기
 *   onStart, onSettings: 완성 카드의 켜기·설정 먼저 보기
 */
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { SabaArt } from './ui/SabaArt';

export function NativeProvision({ server, provisionProgress, cooked, icon, onDismiss, onClose, onStart, onSettings }) {
    const { t } = useTranslation('gui');
    const error = provisionProgress?.error;

    if (server.provisioning && error) {
        return (
            <div className="cook cook-failed" role="alert">
                <SabaArt kind="stopped" size={140} />
                <div className="cook-text">
                    <p className="cook-title">{t('saba.cook_failed')}</p>
                    <p className="cook-error mt-pre">{error}</p>
                    <div className="cook-actions">
                        <button type="button" className="btn btn-ghost btn-sm" onClick={onDismiss}>
                            {t('modals.close')}
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    if (server.provisioning && !provisionProgress?.done) {
        const percent = provisionProgress?.percent ?? 0;
        return (
            <div className="cook" aria-live="polite">
                <SabaArt kind="cooking" size={140} />
                <div className="cook-text">
                    <p className="cook-title">{t('saba.cooking')}</p>
                    <div className="cook-bar">
                        <div
                            className={`cook-bar-fill${percent > 0 ? '' : ' indeterminate'}`}
                            style={percent > 0 ? { width: `${percent}%` } : undefined}
                        />
                    </div>
                    <p className="cook-step">
                        <span>{provisionProgress?.message}</span>
                        {percent > 0 && <span className="cook-pct">{percent}%</span>}
                    </p>
                </div>
            </div>
        );
    }

    // 완성 카드는 사용자가 닫을 때까지만 (데몬이 진행 표시를 지우기 전에 닫아도 사라진다)
    if (!cooked) return null;

    const close = (then) => () => {
        onClose?.();
        then?.();
    };
    return (
        <div className="cook cook-done" aria-live="polite">
            <div className="cook-plate">
                <SabaArt kind="served" size={140} />
                <span className="cook-dish">{icon}</span>
            </div>
            <div className="cook-text">
                <p className="cook-title">{t('saba.cooked')}</p>
                <div className="cook-actions">
                    <button type="button" className="btn btn-primary btn-sm" onClick={close(onStart)}>
                        <Icon name="play" size="sm" /> {t('shell.start')}
                    </button>
                    <button type="button" className="btn btn-ghost btn-sm" onClick={close(onSettings)}>
                        {t('saba.settings_first')}
                    </button>
                </div>
            </div>
            <button type="button" className="cook-close" onClick={close()} aria-label={t('modals.close')}>
                <Icon name="x" size="sm" />
            </button>
        </div>
    );
}
