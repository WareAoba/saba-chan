import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { BUBBLE_MAX_HOLD_MS, useSabaBubble } from '../../stores/useSabaBubble';
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

    // 데몬 깨우기
    useEffect(() => {
        // 한 번이라도 꺼짐·오류를 본 뒤에 실행으로 돌아오면 깨운 것이다 (사이에 'checking'이 끼어도)
        const down = (st) => st === 'stopped' || st === 'error';
        let wasDown = down(useUIStore.getState().backgroundDaemonStatus);
        return useUIStore.subscribe((s) => {
            const now = s.backgroundDaemonStatus;
            if (down(now)) wasDown = true;
            else if (now === 'running' && wasDown) {
                wasDown = false;
                useSabaBubble.getState().show({ kind: 'sleepy', text: tRef.current('saba.sleepy') });
            }
        });
    }, []);

    // 서버 크래시 → 복구 중 / 멈춤
    useEffect(() => {
        const nameOf = (d) => d.instance || useServerStore.getState().servers.find((s) => s.id === d.instanceId)?.name;
        return subscribeDaemonEvents((ev) => {
            const d = ev.data;
            const bubble = useSabaBubble.getState();
            if (ev.type === 'instance.exit' && d.kind === 'crash') {
                const name = nameOf(d) || '';
                if (d.restartInSecs != null) {
                    const srv = useServerStore.getState().servers.find((s) => s.id === d.instanceId);
                    bubble.show({
                        heldFrom: { status: srv?.status, startTime: srv?.start_time },
                        kind: 'recovering',
                        text: tRef.current('saba.recovering', { name }),
                        holdKey: d.instanceId,
                    });
                } else {
                    bubble.show({ kind: 'stopped', text: tRef.current('saba.stopped', { name }), minMs: 6000 });
                }
            } else if (ev.type === 'instance.restarted' || ev.type === 'instance.restart_cancelled') {
                // 다시 켰거나 사용자가 복구를 멈췄다: 최소 시간만 채우고 들어간다
                bubble.release(d.instanceId);
            } else if (ev.type === 'instance.restart_failed') {
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
        return useServerStore.subscribe((s) => {
            const b = useSabaBubble.getState().bubble;
            if (!b?.holdKey || b.released) return;
            const srv = s.servers.find((x) => x.id === b.holdKey);
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
    const [leaving, setLeaving] = useState(false);
    const [minDone, setMinDone] = useState(false);
    const shownId = useRef(null);

    // 새 말풍선이 오면 처음부터 다시 잰다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 말풍선이 바뀔 때(id)만 다시 잰다
    useEffect(() => {
        if (!bubble) return undefined;
        if (shownId.current !== bubble.id) {
            shownId.current = bubble.id;
            setLeaving(false);
            setMinDone(false);
        }
        const min = setTimeout(() => setMinDone(true), bubble.minMs);
        const max = setTimeout(() => setMinDone('force'), Math.max(bubble.minMs, BUBBLE_MAX_HOLD_MS));
        return () => {
            clearTimeout(min);
            clearTimeout(max);
        };
    }, [bubble?.id, bubble?.minMs]);

    // 최소 시간이 지났고 붙잡을 이유가 없으면 들어간다
    useEffect(() => {
        if (bubble && !leaving && (minDone === 'force' || (minDone && bubble.released))) setLeaving(true);
    }, [bubble, minDone, leaving]);

    // 들어가는 움직임이 끝나면 지운다
    const bubbleId = bubble?.id;
    useEffect(() => {
        if (!leaving || !bubbleId) return undefined;
        const timer = setTimeout(() => useSabaBubble.getState().clear(bubbleId), LEAVE_MS);
        return () => clearTimeout(timer);
    }, [leaving, bubbleId]);

    if (!bubble) return null;

    return (
        <output key={bubble.id} className={clsx('saba-bubble', `saba-bubble-${bubble.kind}`, { leaving })}>
            <div className="saba-bubble-body">
                <SabaArt kind={bubble.kind} size={84} />
                <p className="saba-bubble-text">{bubble.text}</p>
            </div>
        </output>
    );
}
