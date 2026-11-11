import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import { safeShowToast } from '../../utils/helpers';
import { localLogLine } from '../../utils/logTime';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { failed } from '../../utils/apiResult';

interface LogSource {
    id: string;
    label: string;
}

interface LogView {
    sources: LogSource[];
    source?: string;
    lines: string[];
    before?: string | number;
}

const PAGE = 200;
/**
 * 지난 로그 (데몬 GET /api/instance/:id/logs): 실행 한 번이 한 묶음이다. 고른 묶음의 끝에서 200줄,
 * [더 보기]로 앞을 붙인다. 줄 머리의 UTC 시각은 이 PC 시간대로 바꿔 보인다 (utils/logTime). 내보내기는 저장 창에서 고른 곳에 데몬이 쓴다 (`instanceLogsSave`). 웹은 점검 탭의 같은 카드
 */
export function ServerLogs({ server }: { server: Server }) {
    const { t } = useTranslation('gui');
    const [view, setView] = useState<LogView | false | null>(null);
    const [busy, setBusy] = useState<'load' | 'more' | 'export' | null>(null);
    const available = typeof window.api?.instanceLogs === 'function';

    const load = useCallback(
        async (source: string | undefined) => {
            setBusy('load');
            const res = await window.api.instanceLogs(server.id, { source, lines: PAGE });
            setBusy(null);
            if (failed(res)) {
                safeShowToast(t('shell.logs_failed', { error: res?.error || '' }), 'error', 5000);
                return setView(false);
            }
            setView({ sources: res.sources || [], source: res.source, lines: res.lines || [], before: res.before });
        },
        [server.id, t],
    );

    useEffect(() => {
        if (!available) return;
        setView(null);
        load(undefined);
    }, [available, load]);

    if (!available || !view) return null;

    const more = async () => {
        setBusy('more');
        const res = await window.api.instanceLogs(server.id, { source: view.source, lines: PAGE, before: view.before });
        setBusy(null);
        if (failed(res)) return safeShowToast(t('shell.logs_failed', { error: res?.error || '' }), 'error', 5000);
        setView((v) => ({ ...(v as LogView), lines: [...(res.lines || []), ...(v as LogView).lines], before: res.before }));
    };

    const save = async () => {
        setBusy('export');
        const res = await window.api.instanceLogsSave(server.id, {
            source: view.source,
            defaultName: `${server.name || server.id}-${view.source}.log`,
        });
        setBusy(null);
        if (res?.canceled) return;
        if (failed(res)) return safeShowToast(t('shell.logs_failed', { error: res?.error || '' }), 'error', 5000);
        safeShowToast(
            res.truncated ? t('shell.logs_saved_truncated', { path: res.path }) : t('shell.logs_saved', { path: res.path }),
            'success',
            4000,
        );
    };

    return (
        <section className="dc-card ss-card server-logs" aria-label={t('shell.logs_title')}>
            <div className="dc-card-head server-logs-head">
                <h2 className="ss-card-title">
                    <Icon name="file" size="sm" /> {t('shell.logs_title')}
                </h2>
                {view.sources.length > 0 && (
                    <>
                        <CustomDropdown
                            className="server-logs-run"
                            value={view.source || ''}
                            ariaLabel={t('shell.logs_runs')}
                            disabled={!!busy}
                            onChange={(v) => v !== view.source && load(v)}
                            options={view.sources.map((s) => ({ value: s.id, label: s.label }))}
                        />
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm server-logs-export"
                            disabled={!!busy}
                            onClick={save}
                        >
                            <Icon name="download" size="sm" /> {t('shell.logs_export')}
                        </button>
                    </>
                )}
            </div>
            {view.sources.length === 0 ? (
                <p className="muted">{t('shell.logs_empty')}</p>
            ) : (
                <>
                    {view.before !== undefined && (
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm server-logs-more"
                            disabled={!!busy}
                            onClick={more}
                        >
                            {t('shell.logs_more')}
                        </button>
                    )}
                    <pre>{view.lines.map(localLogLine).join('\n')}</pre>
                </>
            )}
        </section>
    );
}
