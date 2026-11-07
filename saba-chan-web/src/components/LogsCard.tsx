import { localLogLine } from '@gui/utils/logTime';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import type { Server, WebApi } from '../types';
import { Icon } from './icons';
import { Button, Card, errorText, failed, Loading } from './ui';

/** 로그 묶음 한 개 (실행 한 번) */
interface LogSource {
    id: string;
    label: string;
}

/** 읽어 온 로그 */
interface LogsView {
    sources: LogSource[];
    source?: string;
    lines: string[];
    before?: number;
}

const PAGE = 200;

/** 받은 내용을 이 기기에 파일로 내려받는다 */
function download(fileName: string, content: string) {
    const url = URL.createObjectURL(new Blob([content], { type: 'text/plain;charset=utf-8' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = fileName;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/**
 * 지난 로그 (2026-11-03 사용자 지정 시안: 점검 탭 안 카드). 실행 한 번이 한 묶음이다.
 * 고른 묶음의 끝에서 200줄을 보이고 [더 보기]로 앞을 붙인다. 내보내기는 그 묶음을 .log 파일로 받는다.
 * 데몬 GET /api/instance/:id/logs, /logs/export (웹으로 나가며 PC 경로는 가려진다)
 */
export function LogsCard({ api, server }: { api: WebApi; server: Pick<Server, 'id' | 'name'> }) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    // null = 읽는 중, false = 읽지 못함
    const [view, setView] = useState<LogsView | false | null>(null);
    const [busy, setBusy] = useState<'load' | 'more' | 'export' | null>(null);

    const load = useCallback(
        async (source: string | undefined) => {
            setBusy('load');
            const res = await api.instanceLogs(server.id, { source, lines: PAGE });
            setBusy(null);
            if (failed(res)) {
                toast(errorText(t, res), 'error');
                return setView(false);
            }
            setView({ sources: res.sources || [], source: res.source, lines: res.lines || [], before: res.before });
        },
        [api, server.id, t, toast],
    );

    useEffect(() => {
        setView(null);
        load(undefined);
    }, [load]);

    const more = async () => {
        if (!view) return;
        setBusy('more');
        const res = await api.instanceLogs(server.id, { source: view.source, lines: PAGE, before: view.before });
        setBusy(null);
        if (failed(res)) return toast(errorText(t, res), 'error');
        setView((v) => (v ? { ...v, lines: [...(res.lines || []), ...v.lines], before: res.before } : v));
    };

    const exportLog = async () => {
        if (!view) return;
        setBusy('export');
        const res = await api.instanceLogsExport(server.id, { source: view.source });
        setBusy(null);
        if (failed(res)) return toast(errorText(t, res), 'error');
        download(res.fileName || `${server.name}.log`, res.content || '');
        toast(res.truncated ? t('logs.exported_truncated') : t('logs.exported'), 'ok');
    };

    const action = (view || undefined)?.source && (
        <Button busy={busy === 'export'} disabled={!!busy} onClick={exportLog}>
            <Icon name="download" size={16} /> {t('logs.export')}
        </Button>
    );
    return (
        <Card icon="file" tone="7" title={t('logs.title')} action={action}>
            {view === false ? null : view === null ? (
                <Loading />
            ) : view.sources.length === 0 ? (
                <p className="hint">{t('logs.empty')}</p>
            ) : (
                <>
                    <div className="chips">
                        {view.sources.map((s) => (
                            <button
                                key={s.id}
                                type="button"
                                aria-pressed={s.id === view.source}
                                className={s.id === view.source ? 'chip on' : 'chip'}
                                disabled={!!busy}
                                onClick={() => s.id !== view.source && load(s.id)}
                            >
                                {s.label}
                            </button>
                        ))}
                    </div>
                    {view.before !== undefined && (
                        <Button busy={busy === 'more'} disabled={!!busy} onClick={more}>
                            {t('logs.more')}
                        </Button>
                    )}
                    <pre className="log-lines">{view.lines.map(localLogLine).join('\n')}</pre>
                    <small className="hint">{t('logs.count', { count: view.lines.length })}</small>
                </>
            )}
        </Card>
    );
}
