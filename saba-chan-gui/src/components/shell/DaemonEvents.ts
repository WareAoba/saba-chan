import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { useServerStore } from '../../stores/useServerStore';
import { useUIStore } from '../../stores/useUIStore';
import { SERVER_UPDATE_ACTION } from '../../utils/serverUpdateNotice';
import { historyEntryOf, SERVER_CHANGING, showEventToast, showPortMoved, subscribeDaemonEvents } from '../../utils/daemonEvents';

const REFETCH_DEBOUNCE_MS = 300;

/** 누르면 그 게임 서버를 업데이트하는 알림. 서버마다 하나만 둔다 (출처=서버 이름) */
const serverUpdateNotice = (text: string, instanceId: string, name: string) => ({
    message: text,
    type: 'info' as const,
    source: name,
    instanceId,
    action: SERVER_UPDATE_ACTION,
    data: { instanceId, name },
    dedup: true,
});

/**
 * 데몬 이벤트를 받아 토스트·알림을 띄우고, 서버 상태가 바뀌었을 만한 이벤트면 목록을 다시 읽는다.
 * 그리는 것은 없다. onDaemonEvent가 없으면 마운트하지 않는다 (App.js).
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
        const off = subscribeDaemonEvents((ev: { type: string; data: ApiBody }) => {
            const tr = tRef.current;
            if (SERVER_CHANGING.includes(ev.type)) {
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
            if (ev.type === 'instance.update_notice') {
                // 켠 서버의 모듈 새 버전(업데이트 화면으로)·게임 서버 새 빌드 — 어디서 켰든 앱에도 남긴다
                const d = ev.data;
                const name = d.instance || nameOf(d.instanceId);
                const module = d.kind === 'module';
                if (!module && d.kind !== 'server') return;
                window.__sabaNotice?.addNotice(
                    module
                        ? {
                              message: tr('events.module_update', { name, module: d.module, version: d.latest }),
                              type: 'info',
                              source: 'Updater',
                              action: 'openUpdateModal',
                              dedup: true,
                          }
                        : serverUpdateNotice(tr('events.server_update', { name }), d.instanceId, name),
                );
                return;
            }
            if (ev.type === 'instance.update_waiting') {
                // 자동 업데이트가 접속자 때문에 기다린다 — 누르면 5분 예고 뒤 업데이트
                const d = ev.data;
                const name = d.instance || nameOf(d.instanceId);
                window.__sabaNotice?.addNotice(
                    serverUpdateNotice(
                        typeof d.players === 'number'
                            ? tr('events.server_update_waiting', { name, count: d.players })
                            : tr('events.server_update_waiting_unknown', { name }),
                        d.instanceId,
                        name,
                    ),
                );
                return;
            }
            if (ev.type === 'instance.update_failed') {
                const d = ev.data;
                const name = d.instance || nameOf(d.instanceId);
                window.__sabaNotice?.addNotice({
                    message: tr('events.server_update_failed', { name, error: d.error || '' }).trim(),
                    type: 'error',
                    source: name,
                    instanceId: d.instanceId,
                });
                return;
            }
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
