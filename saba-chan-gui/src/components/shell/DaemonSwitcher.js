import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomDropdown from '../CustomDropdown/CustomDropdown';

const LOCAL = '';

const reloadWindow = () => window.location.reload();

/**
 * 상단 바: 다룰 데몬 고르기 (이 PC / 연결한 원격 데몬, G1). 고르면 셸이 데몬 요청마다 `X-Saba-Target`을 싣고
 * (이 PC의 데몬이 그 데몬에 중계한다, 01 §1.4.2 7) 화면을 다시 읽는다. 연결한 원격 데몬이 없으면 보이지 않는다.
 * CLI `--daemon <이름>`과 같은 기능이다.
 */
export function DaemonSwitcher({ onSwitched = reloadWindow }) {
    const { t } = useTranslation('gui');
    const [daemons, setDaemons] = useState(null);
    const [target, setTarget] = useState(LOCAL);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let alive = true;
        (async () => {
            const [list, current] = await Promise.all([
                window.api.remoteDaemons().catch(() => null),
                window.api.daemonTarget().catch(() => null),
            ]);
            if (!alive) return;
            const saved = Array.isArray(list?.daemons) ? list.daemons : [];
            const chosen = current?.target || LOCAL;
            // 잊은 원격 데몬을 고른 채 남아 있으면 이 PC로 되돌린다 (목록을 읽지 못했으면 그대로 둔다)
            if (chosen !== LOCAL && Array.isArray(list?.daemons) && !saved.some((d) => d.hostId === chosen)) {
                await window.api.daemonTargetSet(null);
                onSwitched();
                return;
            }
            setDaemons(saved);
            setTarget(chosen);
        })();
        return () => {
            alive = false;
        };
    }, [onSwitched]);

    if (!daemons || (daemons.length === 0 && target === LOCAL)) return null;

    const change = async (value) => {
        if (value === target) return;
        setBusy(true);
        try {
            const res = await window.api.daemonTargetSet(value || null);
            if (res?.success === false) return;
            setTarget(value);
            onSwitched();
        } finally {
            setBusy(false);
        }
    };

    return (
        <CustomDropdown
            className="appbar-daemon-switcher"
            ariaLabel={t('daemon_switcher.label')}
            value={target}
            disabled={busy}
            onChange={change}
            options={[
                { value: LOCAL, label: t('daemon_switcher.local'), icon: 'desktop' },
                ...daemons.map((d) => ({ value: d.hostId, label: d.name || d.hostId, icon: 'cloud' })),
            ]}
        />
    );
}
