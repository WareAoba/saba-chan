import { useState, useEffect, useRef } from 'react';

/**
 * Manages the waiting image display logic.
 * Shows waiting.png when progress is slow (>5s stall) or when timeout toast is detected.
 *
 * @param {Object|null} progressBar - Current progress bar state { message, percent?, indeterminate? }
 * @returns {{ showWaitingImage: boolean, setShowWaitingImage: Function }}
 */
export function useWaitingImage(progressBar) {
    const [showWaitingImage, setShowWaitingImage] = useState(false);
    const waitingTimerRef = useRef(null);
    const progressSnapshotRef = useRef(null);

    // Monitor progress bar speed — show waiting image if stalled >5s
    useEffect(() => {
        if (!progressBar) {
            setShowWaitingImage(false);
            if (waitingTimerRef.current) clearInterval(waitingTimerRef.current);
            progressSnapshotRef.current = null;
            return;
        }

        if (progressBar.percent === 100) {
            setShowWaitingImage(false);
            if (waitingTimerRef.current) clearInterval(waitingTimerRef.current);
            progressSnapshotRef.current = null;
            return;
        }

        if (!progressSnapshotRef.current) {
            progressSnapshotRef.current = { percent: progressBar.percent || 0, timestamp: Date.now() };
        }

        if (!waitingTimerRef.current) {
            waitingTimerRef.current = setInterval(() => {
                const snap = progressSnapshotRef.current;
                if (!snap) return;
                const elapsed = (Date.now() - snap.timestamp) / 1000;
                if (elapsed >= 5) {
                    setShowWaitingImage(true);
                }
            }, 1000);
        }

        // If progress jumped >5%, reset snapshot
        const currentPercent = progressBar.percent || 0;
        const snap = progressSnapshotRef.current;
        if (snap && currentPercent - snap.percent > 5) {
            progressSnapshotRef.current = { percent: currentPercent, timestamp: Date.now() };
            setShowWaitingImage(false);
        }

        return () => {
            if (waitingTimerRef.current) {
                clearInterval(waitingTimerRef.current);
                waitingTimerRef.current = null;
            }
        };
    }, [progressBar]);

    // Detect timeout toasts (e.g., "시간이 걸릴 수 있습니다")
    useEffect(() => {
        const origUpdateToast = window.updateToast;
        const wrappedUpdateToast = (id, message, type, duration) => {
            if (message && message.includes('시간이 걸릴')) {
                setShowWaitingImage(true);
                setTimeout(() => setShowWaitingImage(false), duration || 5000);
            }
            if (origUpdateToast) origUpdateToast(id, message, type, duration);
        };
        window.updateToast = wrappedUpdateToast;
        return () => { window.updateToast = origUpdateToast; };
    }, []);

    return { showWaitingImage, setShowWaitingImage };
}
