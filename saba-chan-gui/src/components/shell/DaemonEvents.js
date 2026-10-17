import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import { SERVER_CHANGING, showEventToast, subscribeDaemonEvents } from '../../utils/daemonEvents';

const REFETCH_DEBOUNCE_MS = 300;

/**
 * 데몬 이벤트를 받아 토스트·알림을 띄우고, 서버 상태가 바뀌었을 만한 이벤트면 목록을 다시 읽는다.
 * 그리는 것은 없다. onDaemonEvent가 없으면 마운트하지 않는다 (App.js).
 */
export function DaemonEvents({ onServersChanged }) {
    const { t } = useTranslation('gui');
    const tRef = useRef(t);
    tRef.current = t;
    const refetchRef = useRef(onServersChanged);
    refetchRef.current = onServersChanged;

    useEffect(() => {
        let timer;
        const nameOf = (id) => useServerStore.getState().servers.find((s) => s.id === id)?.name || id;
        const off = subscribeDaemonEvents((ev) => {
            const tr = tRef.current;
            if (SERVER_CHANGING.includes(ev.type)) {
                // 한꺼번에 여러 건이 와도 한 번만 다시 읽는다
                clearTimeout(timer);
                timer = setTimeout(() => refetchRef.current?.(), REFETCH_DEBOUNCE_MS);
            }
            if (ev.type === 'update.available') {
                const names = Array.isArray(ev.data.components) ? ev.data.components : [];
                if (names.length && window.__sabaNotice) {
                    window.__sabaNotice.addNotice({
                        message: `📦 ${tr('updates.found', { count: names.length, names: names.join(', ') })}`,
                        type: 'info',
                        source: 'Updater',
                        action: 'openUpdateModal',
                        dedup: true,
                    });
                }
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
