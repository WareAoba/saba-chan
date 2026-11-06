import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { Icon } from './icons';
import { Button, errorText, failed } from './ui';
import type { Server, WebApi } from '../types';

/** from 자리의 것을 to 자리로 옮긴 새 배열 (범위를 벗어나면 그대로) */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
    if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) return list;
    const out = [...list];
    const [item] = out.splice(from, 1);
    out.splice(to, 0, item);
    return out;
}

/**
 * 서버 순서 바꾸기 (2026-11-03 사용자 지정 시안): 손잡이를 끌거나 ▲▼로 옮기고 저장한다.
 * 휴대폰은 HTML 끌어 놓기가 없어 pointer 이벤트로 끌고, 손가락이 지나가는 줄 자리로 바로 옮긴다.
 * 저장은 데몬 PUT /api/instances/reorder (GUI 와 같은 순서)
 */
export function ServerOrder({ api, servers, onDone }: { api: WebApi; servers: Server[]; onDone: () => void }) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const [ids, setIds] = useState(() => servers.map((s) => s.id));
    const [dragging, setDragging] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const stop = useRef<(() => void) | null>(null);
    useEffect(() => () => stop.current?.(), []);
    const nameOf = (id: string) => servers.find((s) => s.id === id)?.name ?? id;

    const startDrag = (e: React.PointerEvent, id: string) => {
        if (busy || (e.button !== undefined && e.button > 0)) return;
        e.preventDefault();
        stop.current?.();
        setDragging(id);
        const move = (ev: PointerEvent) => {
            const row = document.elementFromPoint?.(ev.clientX, ev.clientY)?.closest?.('[data-order-id]');
            const over = (row as HTMLElement | null)?.dataset.orderId;
            if (!over || over === id) return;
            setIds((list) => moveItem(list, list.indexOf(id), list.indexOf(over)));
        };
        const end = () => {
            window.removeEventListener('pointermove', move);
            window.removeEventListener('pointerup', end);
            window.removeEventListener('pointercancel', end);
            stop.current = null;
            setDragging(null);
        };
        stop.current = end;
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', end);
        window.addEventListener('pointercancel', end);
    };

    const save = async () => {
        setBusy(true);
        const res = await api.instanceReorder(ids);
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        // 다음 목록 읽기 전에도 바뀐 순서로 보이게
        const { servers: now, setServers } = useData.getState();
        setServers([...now].sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id)));
        toast(t('servers.order_saved'), 'ok');
        onDone();
    };

    return (
        <div className="server-order">
            <p className="hint">{t('servers.order_hint')}</p>
            <ul className="rows">
                {ids.map((id, i) => (
                    <li
                        key={id}
                        data-order-id={id}
                        className={dragging === id ? 'row order-row is-dragging' : 'row order-row'}
                    >
                        <button
                            type="button"
                            className="order-grip"
                            aria-label={t('servers.order_drag', { name: nameOf(id) })}
                            onPointerDown={(e) => startDrag(e, id)}
                        >
                            <Icon name="grip" size={18} />
                        </button>
                        <div className="row-main">
                            <div className="row-title row-title-one">{nameOf(id)}</div>
                        </div>
                        <div className="row-actions">
                            <button
                                type="button"
                                className="icon-btn"
                                aria-label={t('servers.order_up', { name: nameOf(id) })}
                                disabled={busy || i === 0}
                                onClick={() => setIds((list) => moveItem(list, i, i - 1))}
                            >
                                <Icon name="chevronUp" size={16} />
                            </button>
                            <button
                                type="button"
                                className="icon-btn"
                                aria-label={t('servers.order_down', { name: nameOf(id) })}
                                disabled={busy || i === ids.length - 1}
                                onClick={() => setIds((list) => moveItem(list, i, i + 1))}
                            >
                                <Icon name="chevronDown" size={16} />
                            </button>
                        </div>
                    </li>
                ))}
            </ul>
            <div className="row-actions order-actions">
                <Button onClick={onDone} disabled={busy}>
                    {t('cancel')}
                </Button>
                <Button tone="accent" busy={busy} onClick={save}>
                    {t('servers.order_save')}
                </Button>
            </div>
        </div>
    );
}
