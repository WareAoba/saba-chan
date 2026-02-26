/**
 * useExtensionInitStatus — 익스텐션 초기화(daemon.startup) 진행 상태를 폴링
 *
 * 반환값:
 *   { initializing, inProgress, completed }
 *   - initializing: boolean — 하나라도 초기화 중이면 true
 *   - inProgress: { [extId]: message }
 *   - completed: [{ ext_id, success, message, timestamp }]
 *
 * 폴링 라이프사이클:
 *   1. 마운트 시 2초 간격으로 폴링 시작
 *   2. initializing: true → false 전환을 감지하면 폴링 중지
 *   3. 한 번도 true가 되지 않더라도 MAX_IDLE_POLLS 이후 자동 중지
 *      (데몬이 훅 디스패치 전에 응답하는 race condition 대비)
 */
import { useCallback, useEffect, useRef, useState } from 'react';

const POLL_INTERVAL_MS = 2000;
const MAX_IDLE_POLLS = 15; // true를 한 번도 못 봤을 때 최대 폴링 횟수 (30초)

export default function useExtensionInitStatus() {
    const [initializing, setInitializing] = useState(false);
    const [inProgress, setInProgress] = useState({});
    const [completed, setCompleted] = useState([]);
    const timerRef = useRef(null);
    /** true를 한 번이라도 관측했는지 */
    const sawTrueRef = useRef(false);
    /** true를 본 적 없을 때의 idle 폴 카운터 */
    const idlePollsRef = useRef(0);
    /** 폴링 종료 여부 */
    const doneRef = useRef(false);

    const stopPolling = useCallback(() => {
        doneRef.current = true;
        if (timerRef.current) {
            clearInterval(timerRef.current);
            timerRef.current = null;
        }
    }, []);

    const fetchStatus = useCallback(async () => {
        if (!window.api?.extensionInitStatus) return;
        try {
            const data = await window.api.extensionInitStatus();

            // daemon_unreachable → 아직 데몬 연결 안 됨, 상태 변경 없이 대기
            if (data.daemon_unreachable) return;

            const isInit = data.initializing ?? false;
            setInitializing(isInit);
            setInProgress(data.in_progress ?? {});
            setCompleted(data.completed ?? []);

            if (isInit) {
                sawTrueRef.current = true;
                idlePollsRef.current = 0;
            } else if (sawTrueRef.current) {
                // true → false 전환 감지: 초기화 완료
                stopPolling();
            } else {
                // 아직 true를 본 적 없음 — race condition 대비 대기
                idlePollsRef.current += 1;
                if (idlePollsRef.current >= MAX_IDLE_POLLS) {
                    stopPolling();
                }
            }
        } catch {
            // 데몬 연결 안 됐을 때는 무시
        }
    }, [stopPolling]);

    useEffect(() => {
        fetchStatus();

        timerRef.current = setInterval(() => {
            if (doneRef.current) {
                clearInterval(timerRef.current);
                return;
            }
            fetchStatus();
        }, POLL_INTERVAL_MS);

        return () => {
            if (timerRef.current) clearInterval(timerRef.current);
        };
    }, [fetchStatus]);

    return { initializing, inProgress, completed };
}
