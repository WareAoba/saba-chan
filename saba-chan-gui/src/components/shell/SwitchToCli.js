import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUIStore } from '../../stores/useUIStore';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

/**
 * 머리줄의 "CLI로 전환": 데몬이 고른 서버로 CLI 메뉴를 새 터미널에 띄우고, 이 GUI는 인터페이스만 끝낸다 (데몬은 남는다).
 * 데몬이 이 PC에서 CLI를 띄울 수 없다고 하면(`GET /api/clients/launch`의 cli) 그리지 않는다.
 */
export function SwitchToCliButton() {
    const { t } = useTranslation('gui');
    const [available, setAvailable] = useState(false);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let alive = true;
        Promise.resolve(window.api?.clientsLaunchable?.())
            .then((r) => alive && setAvailable(r?.cli === true))
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    if (!available) return null;

    const failed = (reason) => safeShowToast(t('switch_cli.failed', { reason: reason || '' }).trim(), 'warning', 6000);
    const switchToCli = async () => {
        setBusy(true);
        try {
            const instance = useUIStore.getState().activeServerId || undefined;
            // 고른 원격 데몬(G1)은 CLI에도 넘긴다: 그 서버는 그 데몬에 있다
            const current = await window.api.daemonTarget?.().catch(() => null);
            const res = await window.api.clientLaunch({
                kind: 'cli',
                instance,
                ...(current?.target ? { daemon: current.target } : {}),
            });
            if (res?.success) {
                window.api.closeResponse('exit-interface');
                return;
            }
            failed(res?.message || res?.error);
        } catch (e) {
            failed(e?.message);
        }
        setBusy(false);
    };

    return (
        <button
            type="button"
            className="icon-btn"
            onClick={switchToCli}
            disabled={busy}
            data-tooltip={t('switch_cli.label')}
            aria-label={t('switch_cli.label')}
        >
            <Icon name="terminal" size="sm" />
        </button>
    );
}
