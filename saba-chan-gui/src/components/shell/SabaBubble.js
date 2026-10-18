import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { BUBBLE_MAX_HOLD_MS, useSabaBubble, WAKE_WINDOW_MS } from '../../stores/useSabaBubble';
import { useServerStore } from '../../stores/useServerStore';
import { useUIStore } from '../../stores/useUIStore';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { SabaArt } from '../ui/SabaArt';
import './SabaBubble.css';

const LEAVE_MS = 320;

/**
 * 말풍선을 띄울 순간을 고른다:
 * - 앱이 켜진 뒤 데몬을 손으로 깨우면 (꺼짐 → 실행): 졸린 사바쨩
 * - 서버가 넘어져 자동 복구를 기다리면: 복구 중 (다시 켜지거나 복구를 멈출 때까지, 최소 5초)
 * - 복구를 포기했거나 실패하면: 멈춤
 */
function useBubbleTriggers() {
    const { t } = useTranslation('gui');
    const tRef = useRef(t);
    tRef.current = t;
    // 다시 연결됨(resync): 끊긴 사이 끝 이벤트를 놓쳤을 수 있어, 다음 목록에서 꺼져 있으면 놓는다 (그때의 말풍선 id)
    const resyncRef = useRef(null);

    // 데몬 깨우기
    useEffect(() => {
        // 깨우기를 누르고, 꺼짐·오류를 본 뒤에 실행으로 돌아오면 깨운 것이다 (사이에 'checking'이 끼어도)
        const down = (st) => st === 'stopped' || st === 'error';
        let wasDown = down(useUIStore.getState().backgroundDaemonStatus);
        return useUIStore.subscribe((s) => {
            const now = s.backgroundDaemonStatus;
            if (down(now)) wasDown = true;
            else if (now === 'running' && wasDown) {
                wasDown = false;
                // 상태 확인이 잠깐 끊겼다 돌아온 것은 깨운 것이 아니다
                const { wokeAt, show, markWoken } = useSabaBubble.getState();
                if (Date.now() - wokeAt < WAKE_WINDOW_MS) {
                    markWoken(false);
                    show({ kind: 'sleepy', text: tRef.current('saba.sleepy') });
                }
            }
        });
    }, []);

    // 서버 크래시 → 복구 중 / 멈춤
    useEffect(() => {
        // 이름을 못 찾으면 id라도 넣어 주어 없는 문장을 만들지 않는다
        const nameOf = (d) =>
            d.instance || useServerStore.getState().servers.find((s) => s.id === d.instanceId)?.name || d.instanceId;
        // 같은 서버의 다른 복구 시도(늦게 온 이전 이벤트)인가
        const staleFor = (d) => {
            const b = useSabaBubble.getState().bubble;
            return (
                b?.holdKey === d.instanceId &&
                typeof b.restartAttempt === 'number' &&
                typeof d.restartAttempt === 'number' &&
                b.restartAttempt !== d.restartAttempt
            );
        };
        return subscribeDaemonEvents((ev) => {
            const d = ev.data;
            const bubble = useSabaBubble.getState();
            if (ev.type === 'resync') {
                // 그때 떠 있던 복구 중 말풍선에만 쓴다
                const held = bubble.bubble?.holdKey && !bubble.bubble.released;
                resyncRef.current = held ? bubble.bubble.id : null;
            } else if (ev.type === 'instance.exit' && d.restartInSecs != null) {
                const srv = useServerStore.getState().servers.find((s) => s.id === d.instanceId);
                bubble.show({
                    heldFrom: { status: srv?.status, startTime: srv?.start_time },
                    kind: 'recovering',
                    text: tRef.current('saba.recovering', { name: nameOf(d) || '' }),
                    holdKey: d.instanceId,
                    restartAttempt: d.restartAttempt,
                    restartAt: Date.now() + d.restartInSecs * 1000,
                });
            } else if (ev.type === 'instance.exit' && (d.kind === 'crash' || d.noteCode === 'gave_up')) {
                bubble.show({
                    kind: 'stopped',
                    text: tRef.current('saba.stopped', { name: nameOf(d) || '' }),
                    minMs: 6000,
                });
            } else if (ev.type === 'instance.restarted' || ev.type === 'instance.restart_cancelled') {
                // 다시 켰거나 사용자가 복구를 멈췄다: 최소 시간만 채우고 조용히 들어간다
                if (!staleFor(d)) bubble.release(d.instanceId);
            } else if (ev.type === 'instance.restart_failed' && !staleFor(d)) {
                bubble.show({
                    kind: 'stopped',
                    text: tRef.current('saba.stopped', { name: nameOf(d) || '' }),
                    minMs: 6000,
                });
            }
        });
    }, []);

    // 복구 중인 서버가 다시 켜지면 말풍선을 놓는다.
    // 넘어지기 전 목록이 남아 있을 수 있어, 꺼진 모습을 봤거나 시작 시각이 바뀐 뒤의 실행만 믿는다
    useEffect(() => {
        let watch = null;
        return useServerStore.subscribe((s, prev) => {
            const b = useSabaBubble.getState().bubble;
            // 시계 같은 다른 갱신은 건너뛰고, 목록을 새로 받은 갱신에서만 resync 표시를 쓴다
            const resyncId = s.servers !== prev.servers ? resyncRef.current : null;
            // 다른 말풍선으로 바뀌었거나 이미 놓였으면 표시는 끝났다
            if (resyncId && (resyncId !== b?.id || b.released)) resyncRef.current = null;
            if (!b?.holdKey || b.released) return;
            const srv = s.servers.find((x) => x.id === b.holdKey);
            // 재시작 예정 시각 전이면 아직 기다리는 중이라 꺼져 있는 것이 맞다: 표시를 남겨 그 뒤 목록에서 다시 본다
            const restartDue = !(b.restartAt > Date.now());
            if (resyncId === b.id && restartDue) {
                resyncRef.current = null;
                if (srv?.status !== 'running') {
                    useSabaBubble.getState().release(b.holdKey);
                    return;
                }
            }
            if (watch?.id !== b.id) {
                const from = b.heldFrom ?? {};
                watch = { id: b.id, startTime: from.startTime, sawDown: from.status !== 'running' };
            }
            if (srv?.status !== 'running') watch.sawDown = true;
            else if (watch.sawDown || srv.start_time !== watch.startTime) useSabaBubble.getState().release(b.holdKey);
        });
    }, []);
}

/** 좌상단 아이콘에서 늘어져 나와 둥실 떠 있다가 다시 아이콘으로 들어간다 */
export function SabaBubble() {
    useBubbleTriggers();
    const bubble = useSabaBubble((s) => s.bubble);
    // 들어가는 중인 말풍선 id (바뀐 말풍선이 이전 것의 상태를 물려받지 않게)
    const [leavingId, setLeavingId] = useState(null);
    const leaving = Boolean(bubble) && leavingId === bubble.id;
    // 최소 시간이 지난 말풍선 id
    const [minDone, setMinDone] = useState({ id: null, force: false });

    // 새 말풍선이 오면 처음부터 다시 잰다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 말풍선이 바뀔 때(id)만 다시 잰다
    useEffect(() => {
        if (!bubble) return undefined;
        const id = bubble.id;
        const min = setTimeout(() => setMinDone((m) => (m.id === id ? m : { id, force: false })), bubble.minMs);
        const max = setTimeout(() => setMinDone({ id, force: true }), Math.max(bubble.minMs, BUBBLE_MAX_HOLD_MS));
        return () => {
            clearTimeout(min);
            clearTimeout(max);
        };
    }, [bubble?.id, bubble?.minMs]);

    // 최소 시간이 지났고 붙잡을 이유가 없으면 들어간다
    useEffect(() => {
        if (!bubble || leaving || minDone.id !== bubble.id) return;
        if (minDone.force || bubble.released) setLeavingId(bubble.id);
    }, [bubble, minDone, leaving]);

    // 들어가는 움직임이 끝나면 지운다
    const bubbleId = bubble?.id;
    useEffect(() => {
        if (!leaving || !bubbleId) return undefined;
        const timer = setTimeout(() => useSabaBubble.getState().clear(bubbleId), LEAVE_MS);
        return () => clearTimeout(timer);
    }, [leaving, bubbleId]);

    // 알림 영역은 늘 두고 내용만 바꾼다 (새로 붙은 영역은 읽기 프로그램이 놓칠 수 있다)
    return (
        <>
            <output className="sr-only">{bubble?.text ?? ''}</output>
            {bubble && (
                <div
                    key={bubble.id}
                    aria-hidden="true"
                    className={clsx('saba-bubble', `saba-bubble-${bubble.kind}`, { leaving })}
                >
                    <div className="saba-bubble-body">
                        <SabaArt kind={bubble.kind} />
                    </div>
                </div>
            )}
        </>
    );
}
