import { gameNameOf, protocolLabel } from '@gui/utils/serverView';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Section } from './ui';
import type { Server, ServerPanelProps, TFn, WebApi } from '../types';

/** 서버 종료 기록 한 건 (GET /api/instance/:id/events) */
interface ExitEvent {
    at: number | string;
    kind: string;
    exit_code?: number | null;
    signal?: string | null;
    restart_in_secs?: number | null;
    note_code?: string;
    note_args?: { count?: number; reason?: string };
    note?: string;
}

const SHOW_MAX = 8;

/** 데몬이 남기는 재시작 안 함 이유 (GUI ExitHistory 와 같은 note_code). 모르는 코드는 그대로 */
function exitNote(e: ExitEvent, tg: TFn) {
    const args = e.note_args || {};
    if (e.note_code === 'auto_restart_off') return tg('shell.exit_note_auto_off');
    if (e.note_code === 'gave_up') return tg('shell.exit_note_gave_up', { count: args.count });
    if (e.note_code === 'restart_failed') return tg('shell.exit_note_restart_failed', { reason: args.reason });
    return e.note || e.note_code || '';
}

/** 최근 종료 기록: 왜 꺼졌는지와 자동 재시작 여부 */
function ExitHistory({ api, server }: { api: WebApi; server: Pick<Server, 'id' | 'status'> }) {
    const { i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [events, setEvents] = useState<ExitEvent[] | null>(null);

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
    const fmt = (at: number | string) => {
        const d = new Date(typeof at === 'number' && at < 1e12 ? at * 1000 : at);
        return Number.isNaN(d.getTime())
            ? String(at)
            : d.toLocaleString(i18n.language, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });
    };
    const time = (at: number | string) => new Date(typeof at === 'number' && at < 1e12 ? at * 1000 : at).getTime();
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
                                    {e.exit_code != null &&
                                        (e.exit_code === 0
                                            ? tg('shell.exit_code_ok')
                                            : tg('shell.exit_code', { code: e.exit_code }))}{' '}
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
export function InfoPanel({ api, server, moduleData }: Omit<ServerPanelProps, 'running'>) {
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

    // 사용자 11-06 #24·#26: 모르는 값은 줄째 숨기고, 개발 용어(통신 방식)는 '자세히' 안에 (앱 정보 탭과 같다)
    const rows = [
        [tg('shell.info_module'), gameNameOf(t, server.module, moduleData)],
        version && [tg('shell.info_version'), version],
        server.port && [tg('shell.info_port'), String(server.port)],
        [tg('shell.info_id'), server.id],
    ].filter(Boolean) as [string, string][];
    const moreRows = [moduleData && [tg('shell.info_protocol'), protocolLabel(server, moduleData)]].filter(
        Boolean,
    ) as [string, string][];
    const list = (items: [string, string][]) => (
        <dl className="info-list">
            {items.map(([k, v]) => (
                <div key={k} className="info-row">
                    <dt>{k}</dt>
                    <dd>{v}</dd>
                </div>
            ))}
        </dl>
    );

    return (
        <div className="panel">
            <Section title={tg('shell.tab_info')}>
                {list(rows)}
                {moreRows.length > 0 && (
                    <details className="info-more">
                        <summary>{tg('shell.info_more')}</summary>
                        {list(moreRows)}
                    </details>
                )}
            </Section>
            <ExitHistory api={api} server={server} />
        </div>
    );
}
