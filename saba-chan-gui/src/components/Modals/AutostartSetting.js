import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';

export const autostartAvailable = () =>
    typeof window.api?.systemAutostart === 'function' && typeof window.api?.systemAutostartSet === 'function';

const MODES = ['off', 'daemon', 'full'];
// 데몬 `autostart_<코드>` (`saba_chan_updater_lib::autostart::AutostartError`) 중 문구가 있는 것
const ERRORS = ['gui_missing', 'write_failed', 'unsupported'];

const failed = (res) => !res || res.ok === false || res.success === false;

/**
 * 설정 > 일반: 로그인할 때 자동 실행 (데몬 GET·PUT /api/system/autostart). 고르면 바로 저장한다.
 * 데몬이 OS 시작 프로그램에 등록한다. CLI `config autostart`와 같은 기능이다.
 */
export function AutostartSetting() {
    const { t } = useTranslation('gui');
    const [state, setState] = useState(null); // {supported, mode}
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState(null);

    useEffect(() => {
        let alive = true;
        window.api
            .systemAutostart()
            .then((res) => {
                if (alive && !failed(res)) setState(res);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    if (!state?.supported) return null;

    const change = async (mode) => {
        if (mode === state.mode) return;
        setBusy(true);
        setError(null);
        try {
            const res = await window.api.systemAutostartSet(mode);
            if (failed(res)) {
                const code = String(res?.error_code || '').replace(/^autostart_/, '');
                setError(ERRORS.includes(code) ? t(`autostart.err_${code}`) : res?.error || t('autostart.err_generic'));
                return;
            }
            setState(res);
        } catch (e) {
            setError(e.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <>
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="power" size="sm" /> {t('autostart.label')}
                    </span>
                    <span className="setting-description">{t('autostart.description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    ariaLabel={t('autostart.label')}
                    value={state.mode}
                    disabled={busy}
                    onChange={change}
                    options={MODES.map((m) => ({ value: m, label: t(`autostart.${m}`) }))}
                />
            </div>
            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
        </>
    );
}
