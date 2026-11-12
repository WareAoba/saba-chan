import { useEffect } from 'react';
import { useServerStore } from '../stores/useServerStore';

const RETRY_MS = 15_000;

/**
 * 모듈 목록을 받지 못한 채 데몬이 켜져 있으면 받을 때까지 다시 받는다.
 * 처음 한 번만 실패를 알리고, 그 뒤 다시 받는 동안의 실패는 알리지 않는다.
 */
export function useModulesRetry(daemonStatus: string) {
    const loaded = useServerStore((s) => s.modulesLoaded);
    useEffect(() => {
        if (daemonStatus !== 'running' || loaded) return;
        useServerStore.getState().fetchModules();
        const timer = setInterval(() => useServerStore.getState().fetchModules({ quiet: true }), RETRY_MS);
        return () => clearInterval(timer);
    }, [daemonStatus, loaded]);
}
