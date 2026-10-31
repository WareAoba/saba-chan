import { gameNameOf } from '@gui/utils/serverView';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Section } from './ui';

const SHOW_MAX = 8;

/** 데몬이 남기는 재시작 안 함 이유 (GUI ExitHistory 와 같은 note_code). 모르는 코드는 그대로 */
function exitNote(e, tg) {
    const args = e.note_args || {};
    if (e.note_code === 'auto_restart_off') return tg('shell.exit_note_auto_off');
    if (e.note_code === 'gave_up') return tg('shell.exit_note_gave_up', { count: args.count });
    if (e.note_code === 'restart_failed') return tg('shell.exit_note_restart_failed', { reason: args.reason });
    return e.note || e.note_code || '';
}

/** 최근 종료 기록: 왜 꺼졌는지와 자동 재시작 여부 */
function ExitHistory({ api, server }) {
    const { i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [events, setEvents] = useState(null);

    // biome-ignore lint/correctness/useExhaustiveDependencies: 상태가 바뀔 때마다 다시 읽는다
    useEffect(() => {
        let alive = true;
        api.instanceEvents(server.id).then((r) => {
            if (alive && Array.isArray(r?.events)) setEvents(r.events);
        });
        return () => {
            alive = false;
        };
    }, [api, server.id, server.status]);

    if (!events) return null;
    const fmt = (at) => {
        const d = new Date(typeof at === 'number' && at < 1e12 ? at * 1000 : at);
        return Number.isNaN(d.getTime())
            ? String(at)
            : d.toLocaleString(i18n.language, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });
    };
    const time = (at) => new Date(typeof at === 'number' && at < 1e12 ? at * 1000 : at).getTime();
    const list = [...events].sort((a, b) => time(b.at) - time(a.at)).slice(0, SHOW_MAX);

    return (
        <Section title={tg('shell.exit_title')}>
            {list.length === 0 ? (
                <p className="hint">{tg('shell.exit_empty')}</p>
            ) : (
                <ul className="rows">
                    {list.map((e, i) => (
                        <li key={`${e.at}-${i}`} className="row">
                            <div className="row-main">
                                <div className="row-title">
                                    <span className={`pill exit-${e.kind}`}>
                                        {tg(`shell.exit_${e.kind}`, { defaultValue: e.kind })}
                                    </span>
                                    <time>{fmt(e.at)}</time>
                                </div>
                                <div className="row-sub">
                                    {e.exit_code != null && <code>exit {e.exit_code}</code>}{' '}
                                    {e.signal && <code>{e.signal}</code>}{' '}
                                    {e.restart_in_secs != null &&
                                        tg('shell.exit_restart_in', { secs: e.restart_in_secs })}{' '}
                                    {exitNote(e, tg)}
                                </div>
                            </div>
                        </li>
                    ))}
                </ul>
            )}
        </Section>
    );
}

/** 정보: 게임, 버전(모듈에 물어 확인), 포트, 인스턴스 id, 최근 종료 */
export function InfoPanel({ api, server, moduleData }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [version, setVersion] = useState(server.server_version || null);

    useEffect(() => {
        let alive = true;
        api.instanceGetInstalledVersion(server.id).then((r) => {
            if (alive && typeof r?.version === 'string' && r.version) setVersion(r.version);
        });
        return () => {
            alive = false;
        };
    }, [api, server.id]);

    const rows = [
        [tg('shell.info_module'), gameNameOf(t, server.module, moduleData)],
        [tg('shell.info_version'), version || '—'],
        [tg('shell.info_port'), server.port ? String(server.port) : '—'],
        [tg('shell.info_protocol'), server.protocol_mode || '—'],
        [tg('shell.info_id'), server.id],
    ];

    return (
        <div className="panel">
            <Section title={tg('shell.tab_info')}>
                <dl className="info-list">
                    {rows.map(([k, v]) => (
                        <div key={k} className="info-row">
                            <dt>{k}</dt>
                            <dd>{v}</dd>
                        </div>
                    ))}
                </dl>
            </Section>
            <ExitHistory api={api} server={server} />
        </div>
    );
}
