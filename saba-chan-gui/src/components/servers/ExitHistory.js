import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { StatusChip } from '../ui/StatusChip';

const KIND_TONE = { crash: 'stop', start_failed: 'stop', unknown_exit: 'warn', in_game_stop: 'info', user_stop: 'idle' };
const SHOW_MAX = 8;

/**
 * 최근 종료 기록 (데몬 A1: GET /api/instance/:id/events).
 * 왜 꺼졌는지(직접 끔·게임 안에서 끔·크래시·모름)와 자동 재시작 여부를 보여준다.
 */
export function ExitHistory({ server }) {
    const { t, i18n } = useTranslation('gui');
    const [events, setEvents] = useState(null);

    // biome-ignore lint/correctness/useExhaustiveDependencies: 상태가 바뀔 때마다 다시 읽는다
    useEffect(() => {
        if (!window.api?.instanceEvents) return;
        let alive = true;
        window.api
            .instanceEvents(server.id)
            .then((r) => {
                if (alive && Array.isArray(r?.events)) setEvents(r.events);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [server.id, server.status]);

    if (!events) return null;

    const fmt = (at) => {
        const d = new Date(typeof at === 'number' && at < 1e12 ? at * 1000 : at);
        return Number.isNaN(d.getTime())
            ? String(at)
            : d.toLocaleString(i18n.language, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });
    };
    // 최신이 위로
    const list = [...events].sort((a, b) => new Date(b.at) - new Date(a.at)).slice(0, SHOW_MAX);

    return (
        <section className="exit-history" aria-label={t('shell.exit_title')}>
            <h3>{t('shell.exit_title')}</h3>
            {list.length === 0 ? (
                <p className="muted">{t('shell.exit_empty')}</p>
            ) : (
                <ul>
                    {list.map((e, i) => (
                        <li key={`${e.at}-${i}`}>
                            <time>{fmt(e.at)}</time>
                            <StatusChip
                                tone={KIND_TONE[e.kind] || 'idle'}
                                label={t(`shell.exit_${e.kind}`, { defaultValue: e.kind })}
                            />
                            <span className="exit-detail">
                                {e.exit_code != null && <code>exit {e.exit_code}</code>}
                                {e.signal && <code>{e.signal}</code>}
                                {e.restart_in_secs != null && (
                                    <span className="exit-restart">
                                        {t('shell.exit_restart_in', { secs: e.restart_in_secs })}
                                    </span>
                                )}
                                {(e.note || e.evidence) && (
                                    <span className="muted" title={e.evidence || undefined}>
                                        {e.note || e.evidence}
                                    </span>
                                )}
                            </span>
                        </li>
                    ))}
                </ul>
            )}
        </section>
    );
}
