import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

/**
 * GUI를 띄운 프로그램의 job이 떼어 내기를 막아 데몬이 그 안에서 돌 때 상단 바에 남는 경고.
 * 그 프로그램이 꺼지면 데몬도 함께 꺼질 수 있으니, 시작 메뉴에서 다시 열도록 안내한다.
 */
export function ParentJobNotice() {
    const { t } = useTranslation('gui');
    const label = t('shell.parent_job');
    return (
        <button
            type="button"
            className="appbar-preflight failed appbar-parent-job"
            data-tooltip={label}
            aria-label={label}
            onClick={() => safeShowToast(label, 'warning', 10000)}
        >
            <Icon name="alertTriangle" size="sm" />
            <span className="appbar-preflight-text">{t('shell.parent_job_short')}</span>
        </button>
    );
}
