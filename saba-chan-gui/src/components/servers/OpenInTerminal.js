import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

/** 이 PC에서 터미널을 못 여는 이유 (데몬 POST /api/clients/launch). 한 번 받으면 버튼을 숨긴다 */
const UNAVAILABLE = ['unsupported_platform', 'cli_not_found'];
let unavailable = false;

export const openInTerminalAvailable = () => !unavailable && typeof window.api?.openInTerminal === 'function';

/** 테스트용: 숨긴 상태를 되돌린다 */
export const resetOpenInTerminal = () => {
    unavailable = false;
};

const codeOf = (res) => res?.error_code || res?.code || (typeof res?.error === 'string' ? res.error : null);

/** 데몬 오류 → 한 줄 */
export function openInTerminalErrorText(res, t) {
    const code = codeOf(res);
    if (UNAVAILABLE.includes(code) || code === 'not_found') return t(`terminal.err_${code}`);
    return res?.message || res?.error || t('terminal.err_generic');
}

/**
 * 서버 머리의 "터미널로 열기": 데몬이 그 서버의 CLI 콘솔을 새 터미널 창에 띄운다 (I4).
 * 이 PC에서 못 여는 경우(지원 안 하는 OS, CLI 없음)를 알게 되면 알리고 버튼을 숨긴다.
 */
export function OpenInTerminalButton({ server }) {
    const { t } = useTranslation('gui');
    const [busy, setBusy] = useState(false);
    const [, setHidden] = useState(false);
    if (!openInTerminalAvailable()) return null;

    const open = async () => {
        setBusy(true);
        try {
            const res = await window.api.openInTerminal({ view: 'console', instance: server.id });
            if (!res || res.success === false || codeOf(res)) {
                if (UNAVAILABLE.includes(codeOf(res))) {
                    unavailable = true;
                    setHidden(true);
                }
                safeShowToast(openInTerminalErrorText(res, t), 'warning', 6000);
            }
        } catch (e) {
            safeShowToast(e.message || t('terminal.err_generic'), 'error', 6000);
        }
        setBusy(false);
    };

    return (
        <button
            type="button"
            className="icon-btn"
            onClick={open}
            disabled={busy}
            data-tooltip={t('terminal.open')}
            aria-label={t('terminal.open')}
        >
            <Icon name="terminal" size="sm" />
        </button>
    );
}
