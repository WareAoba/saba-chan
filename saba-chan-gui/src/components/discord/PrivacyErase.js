import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

/** 데몬이 삭제표를 받았는지 묻는 간격 */
const POLL_MS = 3000;

export const privacyEraseAvailable = () =>
    typeof window.api?.relayPrivacyErase === 'function' && typeof window.api?.relayPrivacyEraseStatus === 'function';

/**
 * 개인정보 모두 지우기 (데몬 POST·GET /api/relay/privacy/erase).
 * 누르면 데몬이 릴레이에서 받은 웹사이트 동의 페이지를 연다. 사용자가 동의하면 릴레이가 지우고,
 * 데몬이 이 PC 의 릴레이 연결을 끊는다. 여기서는 끝났는지 묻고 화면만 비운다(`onErased`).
 */
export function PrivacyErase({ onErased }) {
    const { t, i18n } = useTranslation('gui');
    // null | 'starting' | 'waiting'
    const [phase, setPhase] = useState(null);
    // 부모가 그릴 때마다 새 함수를 줘도 묻기를 처음부터 다시 하지 않게
    const erased = useRef(onErased);
    erased.current = onErased;

    useEffect(() => {
        if (phase !== 'waiting') return undefined;
        const timer = setInterval(async () => {
            let status;
            try {
                status = (await window.api.relayPrivacyEraseStatus())?.status;
            } catch {
                return;
            }
            // 데몬에 잠깐 닿지 않으면(오류 본문, status 없음) 다시 묻는다. 데몬은 그동안에도 계속 묻는다
            if (!['done', 'expired', 'failed', 'none'].includes(status)) return;
            setPhase(null);
            if (status === 'done') {
                safeShowToast(t('discord.erase_done'), 'success', 5000);
                erased.current();
            } else if (status === 'failed') {
                safeShowToast(t('discord.erase_disconnect_failed'), 'error', 6000);
            } else {
                // expired, 또는 데몬이 다시 켜져 삭제표를 잊었다(none)
                safeShowToast(t('discord.erase_expired'), 'warning', 5000);
            }
        }, POLL_MS);
        return () => clearInterval(timer);
    }, [phase, t]);

    const start = async () => {
        setPhase('starting');
        try {
            const res = await window.api.relayPrivacyErase(i18n.language);
            if (!res?.url) throw new Error(res?.error || 'no url');
            const openExternal = window.electron?.shell?.openExternal;
            if (!openExternal) throw new Error('shell bridge unavailable');
            const opened = await openExternal(res.url);
            if (opened?.error) throw new Error(opened.error);
            setPhase('waiting');
        } catch (e) {
            setPhase(null);
            safeShowToast(t('discord.erase_failed', { error: e.message }), 'error', 5000);
        }
    };

    return (
        <div className="dc-row-actions dc-danger-zone">
            <span className="muted">{phase === 'waiting' ? t('discord.erase_waiting') : t('discord.erase_desc')}</span>
            <button type="button" className="btn btn-danger-text" onClick={start} disabled={phase === 'starting'}>
                <Icon name="trash" size="sm" /> {t('discord.erase')}
            </button>
        </div>
    );
}
