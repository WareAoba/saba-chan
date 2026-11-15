import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { useServerStore } from '../../stores/useServerStore';
import { useUIStore } from '../../stores/useUIStore';
import { refreshNotices } from '../../utils/noticeInbox';
import {
    historyEntryOf,
    SERVER_CHANGING,
    showEventToast,
    showPortMoved,
    subscribeDaemonEvents,
} from '../../utils/daemonEvents';

const REFETCH_DEBOUNCE_MS = 300;

/**
 * 데몬 이벤트를 받아 토스트를 띄우고, 서버 상태가 바뀌었을 만한 이벤트면 목록을 다시 읽는다.
 * 알림함은 데몬이 보관한다: 바뀌었다고 하면(`notices.changed`)·다시 붙으면(`resync`) 다시 읽는다.
 * 그리는 것은 없다. onDaemonEvent가 없으면 마운트하지 않는다 (App.tsx).
 */
export function DaemonEvents({ onServersChanged }: { onServersChanged?: () => void }) {
    const { t } = useTranslation('gui');
    const tRef = useRef(t);
    tRef.current = t;
    const refetchRef = useRef(onServersChanged);
    refetchRef.current = onServersChanged;

    useEffect(() => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const nameOf = (id: string) => useServerStore.getState().servers.find((s) => s.id === id)?.name || id;
        void refreshNotices();
        const off = subscribeDaemonEvents((ev: { id?: unknown; type: string; data: ApiBody }) => {
            const tr = tRef.current;
            if (ev.type === 'notices.changed' || ev.type === 'resync') void refreshNotices();
            // 설치가 시작되거나 끝나면 목록의 설치 중 표시가 바뀐다 (진행 중의 잦은 갱신에는 다시 읽지 않는다)
            const provisionFlip =
                ev.type === 'provision.progress' &&
                (ev.data.done === true || !useServerStore.getState().servers.some((s) => s.name === ev.data.instance && s.provisioning));
            if (SERVER_CHANGING.includes(ev.type) || provisionFlip) {
                // 한꺼번에 여러 건이 와도 한 번만 다시 읽는다
                clearTimeout(timer);
                timer = setTimeout(() => refetchRef.current?.(), REFETCH_DEBOUNCE_MS);
            }
            if (ev.type === 'action.run') {
                // 콘솔·다른 곳에서 보낸 명령도 명령 탭의 실행 기록에 (#18)
                const entry = historyEntryOf(ev.data, tr);
                if (entry) useUIStore.getState().pushCommandHistory(ev.data.instanceId, entry);
            }
            // 새 판은 데몬이 묻지 않고 적용하고 상단 바 업데이트 알약이 보인다: "N개 발견" 알림은 만들지 않는다
            if (ev.type === 'update.available') return;
            if (ev.type === 'instance.port_moved') {
                // 자동·예약 재시작처럼 시작 응답을 아무도 보지 않는 경로도 알린다
                const d = ev.data;
                showPortMoved(
                    { instanceId: d.instanceId, name: d.instance || nameOf(d.instanceId), from: d.from, to: d.to },
                    tr,
                );
                return;
            }
            showEventToast(ev, tr, nameOf);
        });
        return () => {
            clearTimeout(timer);
            off();
        };
    }, []);

    return null;
}
