import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomDropdown from '../CustomDropdown/CustomDropdown';

const LOCAL = '';

/** 설정에서 원격 데몬을 연결하거나 잊으면 보내는 창 이벤트 (상단 바가 목록을 다시 읽는다) */
export const REMOTE_DAEMONS_CHANGED = 'saba:remote-daemons-changed';

export const reloadWindow = () => window.location.reload();

/** 지금 다루는 데몬: 원격 데몬 hostId, 이 PC면 null, 아직 모르면 undefined */
export function useDaemonTarget() {
    const [target, setTarget] = useState(undefined);
    useEffect(() => {
        let alive = true;
        Promise.resolve(window.api?.daemonTarget?.())
            .then((res) => alive && setTarget(res?.target || null))
            .catch(() => alive && setTarget(null));
        return () => {
            alive = false;
        };
    }, []);
    return target;
}

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

    const load = useCallback(async () => {
        const [list, current] = await Promise.all([
            window.api.remoteDaemons().catch(() => null),
            window.api.daemonTarget().catch(() => null),
        ]);
        const saved = Array.isArray(list?.daemons) ? list.daemons : [];
        const chosen = current?.target || LOCAL;
        // 잊은 원격 데몬을 고른 채 남아 있으면 이 PC로 되돌린다 (목록을 읽지 못했으면 그대로 둔다).
        // 저장하지 못했으면 다시 읽지 않는다: 다시 읽어도 같은 값이라 되풀이된다
        if (chosen !== LOCAL && Array.isArray(list?.daemons) && !saved.some((d) => d.hostId === chosen)) {
            const res = await window.api.daemonTargetSet(null).catch(() => null);
            if (res?.success) {
                onSwitched();
                return null;
            }
        }
        return { saved, chosen };
    }, [onSwitched]);

    // 고른 원격 데몬이 이 PC를 잊었으면(그 PC의 데몬을 지우고 다시 설치함) 다시 연결하기 전에는 쓸 수 없다:
    // 이 PC로 돌아간다. 닿지 않는 것은 꺼진 PC일 수 있어 그대로 둔다 (설정의 원격 데몬 절이 까닭을 보인다)
    const leaveForgotten = useCallback(
        async (chosen) => {
            if (chosen === LOCAL || typeof window.api.remoteServers !== 'function') return;
            const all = await window.api.remoteServers().catch(() => null);
            const mine = all?.daemons?.find((d) => d.hostId === chosen);
            if (mine?.error_code !== 'remote_unauthorized') return;
            const current = await window.api.daemonTarget().catch(() => null);
            if (current?.target !== chosen) return;
            const res = await window.api.daemonTargetSet(null).catch(() => null);
            if (res?.success) onSwitched();
        },
        [onSwitched],
    );

    useEffect(() => {
        let alive = true;
        const refresh = () =>
            load().then((r) => {
                if (!alive || !r) return null;
                setDaemons(r.saved);
                setTarget(r.chosen);
                return r;
            });
        refresh().then((r) => r && leaveForgotten(r.chosen));
        window.addEventListener(REMOTE_DAEMONS_CHANGED, refresh);
        return () => {
            alive = false;
            window.removeEventListener(REMOTE_DAEMONS_CHANGED, refresh);
        };
    }, [load, leaveForgotten]);

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
                ...daemons.map((d) => ({ value: d.hostId, label: d.label || d.name || d.hostId, icon: 'cloud' })),
            ]}
        />
    );
}
