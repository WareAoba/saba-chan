import type { Notice } from '@gui/types/global';
import { groupNotices, noticeTime, toNotice } from '@gui/utils/noticeView';
import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useData } from '../store';
import type { PcEvent, WebApi } from '../types';
import { Icon } from './icons';

/** 이 이벤트가 오면 알림함이 바뀌었을 수 있다 (데몬이 알림함에 넣는 계기 중 웹 세션에 오는 것, 01 §2.2 `/api/notices`) */
const MAY_ADD = new Set([
    'instance.exit',
    'instance.started',
    'instance.unresponsive',
    'instance.update_notice',
    'instance.update_waiting',
    'instance.update_failed',
    'instance.updated',
    'notices.changed',
    'resync',
]);

/**
 * 머리의 종 버튼과 알림 패널. 데몬 알림함(앱 알림 창과 같은 목록·같은 읽음 상태)을 서버별로 묶어 보인다.
 * 패널을 열면 모두 읽음이 된다. 웹은 알림을 눌러 할 일(패치노트·업데이트 창)이 없어 보여 주기만 한다
 */
export function NoticeBell({ api }: { api: WebApi }) {
    const { t: tg } = useTranslation('gui');
    const [notices, setNotices] = useState<Notice[]>([]);
    const [open, setOpen] = useState(false);
    const box = useRef<HTMLDivElement>(null);
    const servers = useData((s) => s.servers);

    const load = useCallback(async () => {
        const res = await api.noticesList().catch(() => null);
        if (!res?.success || !Array.isArray(res.notices)) return null;
        const list: Notice[] = res.notices.map(toNotice);
        setNotices(list);
        return list;
    }, [api]);

    useEffect(() => {
        load();
        const off = api.onDaemonEvent((e: PcEvent) => {
            if (MAY_ADD.has(e?.type)) load();
        });
        return () => {
            off?.();
        };
    }, [api, load]);

    useEffect(() => {
        if (!open) return;
        const away = (e: Event) => {
            if (!box.current?.contains(e.target as Node)) setOpen(false);
        };
        const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
        document.addEventListener('pointerdown', away);
        document.addEventListener('keydown', esc);
        return () => {
            document.removeEventListener('pointerdown', away);
            document.removeEventListener('keydown', esc);
        };
    }, [open]);

    const unread = notices.filter((n) => !n.read).length;
    // 열 때 다시 읽고, 읽지 않은 것이 있으면 데몬에도 모두 읽음을 보낸다
    const show = async () => {
        setOpen(true);
        const list = (await load()) ?? notices;
        if (!list.some((n) => !n.read)) return;
        setNotices(list.map((n) => ({ ...n, read: true })));
        api.noticesRead().catch(() => load());
    };
    const clear = () => {
        setNotices([]);
        api.noticesClear().catch(() => load());
    };
    const groups = groupNotices(notices, (id) => servers.find((s) => s.id === id)?.name);
    const title = tg('notice_modal.title');

    return (
        <div className="notice-bell" ref={box}>
            <button
                type="button"
                className="notice-bell-btn"
                aria-label={title}
                aria-haspopup="dialog"
                aria-expanded={open}
                onClick={() => (open ? setOpen(false) : show())}
            >
                <Icon name="bell" size={17} />
                {unread > 0 && (
                    <span className="notice-bell-badge" aria-hidden="true">
                        {unread > 99 ? '99+' : unread}
                    </span>
                )}
            </button>
            {open && (
                <div className="notice-panel" role="dialog" aria-label={title}>
                    <div className="notice-panel-head">
                        <h3>{title}</h3>
                        {notices.length > 0 && (
                            <button type="button" className="notice-panel-clear" onClick={clear}>
                                {tg('notice_modal.clear_all')}
                            </button>
                        )}
                    </div>
                    {notices.length === 0 ? (
                        <p className="notice-panel-empty">{tg('notice_modal.empty')}</p>
                    ) : (
                        groups.map((g) => (
                            <section key={g.key} className="notice-group">
                                <div className="notice-group-name">{g.label}</div>
                                {g.notices.map((n) => (
                                    <div key={n.id} className="notice-item">
                                        <span
                                            className={clsx('notice-dot', n.read ? 'read' : n.type)}
                                            aria-hidden="true"
                                        />
                                        <div className="notice-msg">
                                            {n.message}
                                            {n.detail && <small>{n.detail}</small>}
                                        </div>
                                        <span className="notice-time">{noticeTime(n.timestamp, tg)}</span>
                                    </div>
                                ))}
                            </section>
                        ))
                    )}
                </div>
            )}
        </div>
    );
}
