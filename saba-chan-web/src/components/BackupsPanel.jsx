import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Button, Empty, errorText, failed, Loading, Section } from './ui';

export function sizeText(bytes) {
    if (!Number.isFinite(bytes)) return '';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let v = bytes;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        i++;
    }
    return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export function BackupsPanel({ api, server, running }) {
    const { t, i18n } = useTranslation();
    const { toast, ask } = useApp.getState();
    const [list, setList] = useState(null);
    const [busy, setBusy] = useState(false);

    const load = useCallback(async () => {
        const res = await api.instanceBackups(server.id);
        if (failed(res)) {
            toast(errorText(t, res), 'error');
            setList([]);
        } else setList(Array.isArray(res.backups) ? res.backups : []);
    }, [api, server.id, t, toast]);

    useEffect(() => {
        load();
    }, [load]);

    const act = async (fn, okText) => {
        setBusy(true);
        const res = await fn();
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        toast(okText, 'ok');
        load();
    };

    const create = () => act(() => api.instanceBackupCreate(server.id), t('backups.created'));
    const restore = async (b) => {
        const ok = await ask({
            title: t('backups.restore_title'),
            message: t('backups.restore_message', { name: b.name }),
            confirmLabel: t('backups.restore'),
            danger: true,
        });
        if (ok) act(() => api.instanceBackupRestore(server.id, b.name), t('backups.restored'));
    };
    const remove = async (b) => {
        const ok = await ask({
            title: t('backups.delete_title'),
            message: t('backups.delete_message', { name: b.name }),
            confirmLabel: t('backups.delete'),
            danger: true,
        });
        if (ok) act(() => api.instanceBackupDelete(server.id, b.name), t('backups.deleted'));
    };

    const when = (sec) =>
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
                    {list.map((b) => (
                        <li key={b.name} className="row">
                            <div className="row-main">
                                <div className="row-title">
                                    {when(b.created)}
                                    <span className="pill">
                                        {t(`backups.kind.${b.kind}`, { defaultValue: b.kind })}
                                    </span>
                                </div>
                                <div className="row-sub">{sizeText(b.size)}</div>
                            </div>
                            <div className="row-actions">
                                <Button disabled={running || busy} onClick={() => restore(b)}>
                                    {t('backups.restore')}
                                </Button>
                                <Button tone="ghost-danger" disabled={busy} onClick={() => remove(b)}>
                                    {t('backups.delete')}
                                </Button>
                            </div>
                        </li>
                    ))}
                </ul>
            )}
            {running && list?.length > 0 && <p className="hint">{t('backups.stop_first')}</p>}
        </Section>
    );
}
