import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { sizeText } from './BackupsPanel';
import { startPoll } from './poll';
import { Empty, Loading, Section, Segments } from './ui';

function Meter({ label, percent, detail }) {
    const p = Math.max(0, Math.min(100, percent || 0));
    return (
        <div className="meter">
            <div className="meter-head">
                <span>{label}</span>
                <strong>{Math.round(p)}%</strong>
            </div>
            <div className="meter-track" aria-hidden="true">
                <div className="meter-fill" style={{ width: `${p}%` }} />
            </div>
            {detail && <small className="hint">{detail}</small>}
        </div>
    );
}

function Metrics({ api }) {
    const { t } = useTranslation();
    const [snap, setSnap] = useState(null);
    useEffect(() => {
        let stopped = false;
        const load = async () => {
            const res = await api.systemMetrics({ sizes: false });
            if (!stopped && res?.cpu) setSnap(res);
        };
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api]);

    if (!snap) return <Loading />;
    const mem = snap.memory || {};
    const running = (snap.instances || []).filter((i) => i.running);
    return (
        <>
            <Meter label={t('more.cpu')} percent={snap.cpu?.total_percent} />
            <Meter
                label={t('more.memory')}
                percent={mem.total_bytes ? (mem.used_bytes / mem.total_bytes) * 100 : 0}
                detail={`${sizeText(mem.used_bytes)} / ${sizeText(mem.total_bytes)}`}
            />
            <h3 className="sub-title">{t('more.servers_usage')}</h3>
            {running.length === 0 ? (
                <Empty>{t('more.none_running')}</Empty>
            ) : (
                <ul className="rows">
                    {running.map((i) => (
                        <li key={i.id} className="row">
                            <div className="row-main">
                                <div className="row-title">{i.name}</div>
                                <div className="row-sub">
                                    CPU {Math.round(i.cpu_percent)}% · {sizeText(i.memory_bytes)}
                                </div>
                            </div>
                        </li>
                    ))}
                </ul>
            )}
        </>
    );
}

function DaemonLog({ api }) {
    const { t } = useTranslation();
    const [lines, setLines] = useState(null);
    const next = useRef(undefined);
    const box = useRef(null);
    useEffect(() => {
        let stopped = false;
        const load = async () => {
            const res = await api.daemonConsole(next.current === undefined ? {} : { since: next.current });
            if (stopped || !Array.isArray(res?.lines)) return;
            if (res.lines.length) next.current = res.lines[res.lines.length - 1].id + 1;
            setLines((prev) => [...(prev ?? []), ...res.lines].slice(-300));
        };
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api]);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 새 줄이 올 때마다 맨 아래로 내린다
    useEffect(() => {
        if (box.current) box.current.scrollTop = box.current.scrollHeight;
    }, [lines]);

    if (lines === null) return <Loading />;
    return (
        <pre className="console-lines console-tall" ref={box}>
            {lines.length === 0 ? (
                <span className="console-empty">{t('console.empty')}</span>
            ) : (
                lines.map((l) => (
                    <div key={l.id} className={`line line-${l.level || 'info'}`}>
                        {l.message}
                    </div>
                ))
            )}
        </pre>
    );
}

export function MoreTab({ api }) {
    const { t } = useTranslation();
    const [view, setView] = useState('metrics');
    return (
        <div className="panel">
            <Segments
                value={view}
                onChange={setView}
                options={[
                    { value: 'metrics', label: t('more.metrics') },
                    { value: 'log', label: t('more.daemon_log') },
                ]}
            />
            <Section>{view === 'metrics' ? <Metrics api={api} /> : <DaemonLog api={api} />}</Section>
            <p className="hint">{t('more.pc_only_note')}</p>
        </div>
    );
}
