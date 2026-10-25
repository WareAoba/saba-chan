import { useEffect, useState } from 'react';
import { startPoll } from './poll';

/** 데몬 설치 진행(`/api/provision-progress/:name`) → 화면 상태. 설치 중이 아니면 null */
export function provisionView(p) {
    if (!p?.active) return null;
    if (!p.done) return { state: 'running', percent: p.percent ?? 0, message: p.message ?? '' };
    if (p.success === false || p.error) return { state: 'failed', error: p.error ?? '', pcNeeded: !!p.action_required };
    return { state: 'done' };
}

/** 설치 중인 서버의 진행을 2초마다 읽는다. 성공으로 끝나면 알아서 치운다 */
export function useProvision(api, server) {
    const [view, setView] = useState(null);
    const name = server.provisioning ? server.name : null;
    useEffect(() => {
        if (!name) return setView(null);
        let stopped = false;
        const load = async () => {
            const v = provisionView(await api.instanceProvisionProgress(name));
            if (stopped) return;
            setView(v);
            if (v?.state === 'done') api.instanceDismissProvision(name);
        };
        const stop = startPoll(load, 2000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api, name]);
    return { view, dismiss: () => name && api.instanceDismissProvision(name).then(() => setView(null)) };
}
