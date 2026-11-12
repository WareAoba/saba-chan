import { useEffect, useRef } from 'react';
import { useSabaBubble } from '../stores/useSabaBubble';
import type { ProgressBarConfig } from '../stores/useUIStore';

interface ProgressSnapshot {
    percent: number;
    timestamp: number;
    /** 이번 멈춤에 말풍선을 이미 띄웠다 */
    shown: boolean;
}
/** 느린 진행 말풍선의 holdKey */
const SLOW_PROGRESS_BUBBLE = 'slow-progress';

/**
 * 진행이 5초 넘게 멈춰 보이면 사바쨩 말풍선으로 "멈춘 게 아니에요"를 알린다 (다시 움직이거나 끝나면 내린다).
 * 퍼센트가 있는 진행(받기·설치)만 본다: 서버 시작·정지처럼 끝을 모르는 진행은 서버 상태가 따로 보여 준다 (사용자 11-12)
 */
export function useSlowProgressBubble(progressBar: ProgressBarConfig | null, text: string) {
    const textRef = useRef(text);
    textRef.current = text;
    const stallTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
    const progressSnapshotRef = useRef<ProgressSnapshot | null>(null);
    useEffect(() => {
        const settle = () => {
            useSabaBubble.getState().release(SLOW_PROGRESS_BUBBLE);
            if (stallTimerRef.current) clearInterval(stallTimerRef.current);
            stallTimerRef.current = null;
            progressSnapshotRef.current = null;
        };
        if (!progressBar || progressBar.indeterminate || progressBar.percent === 100) {
            settle();
            return;
        }
        if (!progressSnapshotRef.current) {
            progressSnapshotRef.current = { percent: progressBar.percent || 0, timestamp: Date.now(), shown: false };
        }
        if (!stallTimerRef.current) {
            stallTimerRef.current = setInterval(() => {
                const snap = progressSnapshotRef.current;
                if (!snap || snap.shown || (Date.now() - snap.timestamp) / 1000 < 5) return;
                snap.shown = true;
                useSabaBubble
                    .getState()
                    .show({ kind: 'cooking', text: textRef.current, holdKey: SLOW_PROGRESS_BUBBLE });
            }, 1000);
        }
        const currentPercent = progressBar.percent || 0;
        const snap = progressSnapshotRef.current;
        if (snap && currentPercent - snap.percent > 5) {
            progressSnapshotRef.current = { percent: currentPercent, timestamp: Date.now(), shown: false };
            useSabaBubble.getState().release(SLOW_PROGRESS_BUBBLE);
        }
    }, [progressBar]);
    useEffect(
        () => () => {
            if (stallTimerRef.current) clearInterval(stallTimerRef.current);
        },
        [],
    );
}
