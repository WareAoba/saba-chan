/**
 * useExtensionInitStatus — 익스텐션 초기화(daemon.startup) 진행 상태를 폴링
 *
 * 반환값:
 *   { initializing, inProgress }
 *   - initializing: boolean — 하나라도 초기화 중이면 true
 *   - inProgress: { [extId]: message }
 *
 * 폴링 라이프사이클:
 *   1. 데몬이 떠 있는 동안(`daemonUp`) 2초 간격으로 폴링. 데몬이 다시 뜨면 처음부터 다시 본다
 *   2. initializing: true → false 전환을 감지하면 폴링 중지
 *   3. 한 번도 true가 되지 않더라도 MAX_IDLE_POLLS 이후 자동 중지
 *      (데몬이 훅 디스패치 전에 응답하는 race condition 대비)
 */
import { useEffect, useState } from 'react';
import { startPoll } from '../utils/poll';

const POLL_INTERVAL_MS = 2000;
const sameJson = <T>(prev: T, next: T): T => (JSON.stringify(prev) === JSON.stringify(next) ? prev : next);
const MAX_IDLE_POLLS = 15; // true를 한 번도 못 봤을 때 최대 폴링 횟수 (30초)

export default function useExtensionInitStatus(daemonUp: boolean) {
    const [initializing, setInitializing] = useState(false);
    const [inProgress, setInProgress] = useState<Record<string, string>>({});

    useEffect(() => {
        if (!daemonUp) return undefined;
        /** true를 한 번이라도 관측했는지 */
        let sawTrue = false;
        /** true를 본 적 없을 때의 idle 폴 카운터 */
        let idlePolls = 0;
        let stop = () => {};
        const fetchStatus = async () => {
            try {
                const data = await window.api.extensionInitStatus();

                // daemon_unreachable → 데몬이 잠깐 닿지 않는다, 상태 변경 없이 대기
                if (data.daemon_unreachable) return;

                const isInit = data.initializing ?? false;
                setInitializing(isInit);
                // 폴링마다 같은 내용이면 이전 값을 그대로 둬서 앱 전체를 다시 그리지 않는다
                setInProgress((prev) => sameJson(prev, data.in_progress ?? {}));

                if (isInit) {
                    sawTrue = true;
                    idlePolls = 0;
                } else if (sawTrue || ++idlePolls >= MAX_IDLE_POLLS) {
                    // true → false 전환(초기화 완료)이거나, true를 본 적 없이 기다릴 만큼 기다렸다
                    stop();
                }
            } catch {
                // 데몬 연결 안 됐을 때는 무시
            }
        };
        // 창이 가려진 동안은 쉰다 (startPoll)
        stop = startPoll(fetchStatus, POLL_INTERVAL_MS);
        return () => stop();
    }, [daemonUp]);

    return { initializing, inProgress };
}
