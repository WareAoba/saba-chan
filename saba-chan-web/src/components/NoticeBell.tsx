import type { Notice } from '@gui/types/global';
import { groupNotices, noticeHintKey, noticeScreen, noticeTime, toNotice } from '@gui/utils/noticeView';
import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useData } from '../store';
import type { PcEvent, WebApi } from '../types';
import { Icon } from './icons';
import { webScreen } from './ScreenLink';
import { failed } from './ui';

/** 이 이벤트가 오면 알림함이 바뀌었을 수 있다 (`notices.changed`, 그것을 보내지 않는 예전 데몬은 알림함에 넣는 계기 중 웹 세션에 오는 것, 01 §2.2 `/api/notices`) */
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
 * 패널을 열면 모두 읽음이 된다. 누르면 그 서버의 화면을 여는 알림(예약 백업·예약 작업 실패)은 앱처럼 그 화면으로 가고,
 * 앱에서만 할 수 있는 일(패치노트·업데이트 창·서버 업데이트)의 알림은 보여 주기만 한다
 */
export function NoticeBell({ api }: { api: WebApi }) {
    const { t } = useTranslation();
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
        // 브리지는 실패해도 던지지 않고 실패 본문을 돌려준다: 데몬 목록으로 되돌린다
        if (failed(await api.noticesRead().catch(() => null))) load();
    };
    const clear = async () => {
        setNotices([]);
        if (failed(await api.noticesClear().catch(() => null))) load();
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
                                {g.notices.map((n) => {
                                    const body = (
                                        <>
                                            <span
                                                className={clsx('notice-dot', n.read ? 'read' : n.type)}
                                                aria-hidden="true"
                                            />
                                            <div className="notice-msg">
                                                {n.message}
                                                {n.detail && <small>{n.detail}</small>}
                                            </div>
                                            <span className="notice-time">{noticeTime(n.timestamp, tg)}</span>
                                        </>
                                    );
                                    const target = noticeScreen(n);
                                    const screen = target && webScreen(target, t);
                                    if (!screen || !n.action) {
                                        return (
                                            <div key={n.id} className="notice-item">
                                                {body}
                                            </div>
                                        );
                                    }
                                    return (
                                        <button
                                            key={n.id}
                                            type="button"
                                            className="notice-item notice-item-link"
                                            onClick={() => {
                                                setOpen(false);
                                                screen.open();
                                            }}
                                        >
                                            {body}
                                            <span className="notice-action">{tg(noticeHintKey(n.action))} ↗</span>
                                        </button>
                                    );
                                })}
                            </section>
                        ))
                    )}
                </div>
            )}
        </div>
    );
}
