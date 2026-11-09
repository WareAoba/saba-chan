import { useCallback, useState } from 'react';

/**
 * 설치 중인 항목 id (사바 스토리지 모듈·익스텐션 탭). `run(id, work)`가 일하는 동안 id를 넣어 두고 끝나면 뺀다
 */
export function useInstallingIds() {
    const [installingIds, setInstallingIds] = useState<Set<string>>(new Set());
    const run = useCallback(async <R>(id: string, work: () => Promise<R>) => {
        setInstallingIds((prev) => new Set(prev).add(id));
        try {
            return await work();
        } finally {
            setInstallingIds((prev) => {
                const n = new Set(prev);
                n.delete(id);
                return n;
            });
        }
    }, []);
    return { installingIds, run };
}
