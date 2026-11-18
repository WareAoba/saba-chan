import { formatBytes } from '@gui/components/monitor/systemMetrics';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type ToastTone, useApp } from '../store';
import { Button, Empty, failed, Loading, maintenanceError, Section, unsureText } from './ui';
import type { ApiBody, Server, WebApi } from '../types';

/** 백업 한 개 (GET /api/instance/:id/backups) */
interface Backup {
    name: string;
    created: number;
    kind: string;
    size: number;
    /** false: zip으로 열리지 않는 백업 (복원할 수 없다) */
    readable?: boolean;
}

export function BackupsPanel({ api, server, running }: { api: WebApi; server: Pick<Server, 'id'>; running: boolean }) {
    const { t, i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const { toast, ask } = useApp.getState();
    const [list, setList] = useState<Backup[] | null>(null);
    const [busy, setBusy] = useState(false);

    const load = useCallback(async () => {
        const res = await api.instanceBackups(server.id);
        if (failed(res)) {
            toast(maintenanceError(t, tg, res), 'error');
            setList([]);
        } else setList(Array.isArray(res.backups) ? res.backups : []);
    }, [api, server.id, t, tg, toast]);

    useEffect(() => {
        load();
    }, [load]);

    /** okText: 성공 문구, 또는 답을 보고 고르는 문구·색 */
    const act = async (fn: () => Promise<ApiBody>, okText: string | ((res: ApiBody) => [string, ToastTone])) => {
        setBusy(true);
        const res = await fn();
        setBusy(false);
        const unsure = unsureText(t, res);
        if (unsure) {
            toast(unsure, 'info');
            return load();
        }
        if (failed(res)) return toast(maintenanceError(t, tg, res), 'error');
        const [text, tone]: [string, ToastTone] = typeof okText === 'string' ? [okText, 'ok'] : okText(res);
        toast(text, tone);
        load();
    };

    const create = () => act(() => api.instanceBackupCreate(server.id), t('backups.created'));
    const restore = async (b: Backup) => {
        const ok = await ask({
            title: t('backups.restore_title'),
            message: t('backups.restore_message', { name: b.name }),
            confirmLabel: t('backups.restore'),
            danger: true,
        });
        // 복원은 끝났어도 폴더 안 링크 일부를 되살리지 못했으면 앱과 같은 문구로 경고한다 (경로는 데몬 로그에만)
        if (ok)
            act(
                () => api.instanceBackupRestore(server.id, b.name),
                (res) =>
                    res?.links_not_restored > 0
                        ? [tg('maintenance.restored_links_missed', { count: res.links_not_restored }), 'warn']
                        : [t('backups.restored'), 'ok'],
            );
    };
    const remove = async (b: Backup) => {
        const ok = await ask({
            title: t('backups.delete_title'),
            message: t('backups.delete_message', { name: b.name }),
            confirmLabel: t('backups.delete'),
            danger: true,
        });
        if (ok) act(() => api.instanceBackupDelete(server.id, b.name), t('backups.deleted'));
    };

    const when = (sec: number) =>
        Number.isFinite(sec)
            ? new Date(sec * 1000).toLocaleString(i18n.language, { dateStyle: 'medium', timeStyle: 'short' })
            : '';

    return (
        <Section
            title={t('detail.backups')}
            action={
                <Button tone="accent" busy={busy} onClick={create}>
                    + {t('backups.create')}
                </Button>
            }
        >
            {list === null ? (
                <Loading />
            ) : list.length === 0 ? (
                <Empty>{t('backups.empty')}</Empty>
            ) : (
                <ul className="rows">
                    {list.map((b) => {
                        // 열리지 않는 백업은 복원할 수 없다 (앱 MaintenanceBackups와 같은 기준)
                        const unreadable = b.readable === false;
                        return (
                            <li key={b.name} className="row">
                                <div className="row-main">
                                    <div className="row-title">
                                        {when(b.created)}
                                        <span className="pill">
                                            {t(`backups.kind.${b.kind}`, { defaultValue: b.kind })}
                                        </span>
                                        {unreadable && (
                                            <span
                                                className="pill pill-warn"
                                                title={tg('maintenance.backup_unreadable_hint')}
                                            >
                                                {tg('maintenance.backup_unreadable')}
                                            </span>
                                        )}
                                    </div>
                                    <div className="row-sub">{formatBytes(b.size)}</div>
                                </div>
                                <div className="row-actions">
                                    <Button disabled={running || busy || unreadable} onClick={() => restore(b)}>
                                        {t('backups.restore')}
                                    </Button>
                                    <Button tone="ghost-danger" disabled={busy} onClick={() => remove(b)}>
                                        {t('backups.delete')}
                                    </Button>
                                </div>
                            </li>
                        );
                    })}
                </ul>
            )}
            {running && (list?.length ?? 0) > 0 && <p className="hint">{t('backups.stop_first')}</p>}
        </Section>
    );
}
