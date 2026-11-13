import clsx from 'clsx';
import { type MouseEvent, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import type { ToastType } from '../../types/global';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import { formatBytes } from '../../utils/schedule';
import { gameNameOf } from '../../utils/serverView';
import { Icon } from '../Icon';
import { useContextMenu } from '../ui/ContextMenu';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { failed } from '../../utils/apiResult';
import { Dialog } from '../ui/Dialog';
import { errorText } from './maintenanceText';

/** 백업 목록 한 줄 (GET /api/instance/:id/backups) */
interface Backup {
    name: string;
    created: number;
    size: number;
    stored?: number;
    kind: string;
    readable?: boolean;
    /** 가져온 백업이면 그 백업을 만든 서버의 이름 */
    imported_from?: string;
}

/** 가져오기 전에 데몬이 알려 준 것 (POST …/backups/import `{preview: true}`) */
interface ImportPreview {
    path: string;
    server: string;
    module: string;
    created: number;
    size: number;
}

const KIND_TONE: Record<string, 'ok' | 'stop' | 'warn' | 'info' | 'idle'> = {
    manual: 'info',
    scheduled: 'idle',
    'pre-update': 'warn',
    'pre-restore': 'warn',
    snapshot: 'info',
    imported: 'info',
    'game-snapshot': 'idle',
};
// 게임이 스스로 만든 자동 백업 (데몬 `game-snapshot:<id>`). 내보내려면 먼저 보관한다
const GAME_SNAPSHOT = 'game-snapshot';

export function BackupsCard({ server }: { server: Server }) {
    const { t, i18n } = useTranslation('gui');
    const openMenu = useContextMenu();
    const [backups, setBackups] = useState<Backup[] | null>(null);
    // 백업 폴더가 디스크에 차지하는 크기 (데몬이 계산. 같은 내용은 한 번만 센다)
    const [totalBytes, setTotalBytes] = useState<number | null>(null);
    // 목록을 읽지 못한 이유. 빈 목록("백업 없음")과 구별해 보인다
    const [loadError, setLoadError] = useState<string | null>(null);
    // 모듈이 게임 자동 백업을 알려 주는가 (`[backup] snapshots`)
    const [snapshots, setSnapshots] = useState(false);
    const [busy, setBusy] = useState<string | null>(null); // 'create' | `restore:${name}` | `delete:${name}`
    const [armed, setArmed] = useState<string | null>(null); // 두 번 눌러 확정할 동작
    // 가져오려고 고른 zip (데몬이 검사해 알려 준 내용). 확인하면 목록에 올린다
    const [importing, setImporting] = useState<ImportPreview | null>(null);
    const alive = useRef(true);
    useEffect(
        () => () => {
            alive.current = false;
        },
        [],
    );

    const refresh = useCallback(async () => {
        try {
            const res = await window.api.instanceBackups(server.id);
            if (!alive.current) return;
            if (res?.success === false || (res && !Array.isArray(res.backups))) {
                setLoadError(errorText(res, t('maintenance.load_failed'), t));
                return;
            }
            setLoadError(null);
            setBackups(res.backups);
            setTotalBytes(typeof res?.total_bytes === 'number' ? res.total_bytes : null);
            setSnapshots(res?.snapshots_supported === true);
        } catch (e) {
            if (alive.current) setLoadError(e instanceof Error && e.message ? e.message : t('maintenance.load_failed'));
        }
    }, [server.id, t]);

    useEffect(() => {
        refresh();
    }, [refresh]);

    const run = async (
        key: string,
        call: () => Promise<ApiBody>,
        okMessage?: (res: ApiBody) => string | { text: string; tone: ToastType },
    ) => {
        setBusy(key);
        setArmed(null);
        try {
            const res = await call();
            if (failed(res)) {
                safeShowToast(errorText(res, t('maintenance.action_failed'), t), 'error', 6000);
            } else if (okMessage) {
                // 일부만 된 결과(예: 되살리지 못한 링크)는 경고로 알린다
                const msg = okMessage(res);
                if (typeof msg === 'string') {
                    if (msg) safeShowToast(msg, 'success', 4000);
                }
                else safeShowToast(msg.text, msg.tone, 6000);
            }
        } catch (err) {
            safeShowToast((err as Error).message || String(err), 'error', 6000);
        } finally {
            if (alive.current) {
                setBusy(null);
                refresh();
            }
        }
    };

    const stopped = server.status === 'stopped';
    const fmt = (s: number) =>
        new Date(s * 1000).toLocaleString(i18n.language, {
            year: 'numeric',
            month: 'numeric',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
        });
    const list = [...(backups || [])].sort((a, b) => b.created - a.created);
    const canTransfer =
        typeof window.api?.instanceBackupExport === 'function' && typeof window.api?.instanceBackupImport === 'function';
    // 파일 이름에 쓸 수 없는 글자는 뺀다
    const exportName = () => {
        const d = new Date();
        const day = [d.getFullYear(), d.getMonth() + 1, d.getDate()].map((n) => String(n).padStart(2, '0')).join('-');
        return `${(server.name || 'backup').replace(/[\\/:*?"<>|]/g, '_')}-${day}.zip`;
    };
    const exportBackup = (name: string) =>
        run(
            `export:${name}`,
            () => window.api.instanceBackupExport(server.id, name, exportName()),
            (res) => (res?.canceled ? '' : t('maintenance.backup_exported', { path: res?.path, size: formatBytes(res?.size || 0) })),
        );
    // 고른 파일을 데몬이 먼저 검사한다. 사바쨩 백업이 아니면 창을 띄우지 않는다
    const pickImport = async () => {
        setBusy('import');
        try {
            const res = await window.api.instanceBackupImport(server.id, { preview: true });
            if (res?.canceled || !alive.current) return;
            if (failed(res) || !res?.preview) {
                safeShowToast(errorText(res, t('maintenance.action_failed'), t), 'error', 6000);
                return;
            }
            setImporting({ ...res.preview, path: res.path });
        } catch (err) {
            safeShowToast((err as Error).message || String(err), 'error', 6000);
        } finally {
            if (alive.current) setBusy(null);
        }
    };
    const confirmImport = async (path: string) => {
        await run(
            'import',
            () => window.api.instanceBackupImport(server.id, { path }),
            () => t('maintenance.backup_imported'),
        );
        if (alive.current) setImporting(null);
    };

    return (
        <section className="dc-card" aria-label={t('maintenance.backups_title')}>
            <div className="dc-card-head">
                <h2>{t('maintenance.backups_title')}</h2>
                {canTransfer && (
                    <button type="button" className="btn btn-ghost btn-sm" disabled={!!busy} onClick={pickImport}>
                        {busy === 'import' && !importing ? <SabaSpinner size="sm" /> : <Icon name="download" size="sm" />}{' '}
                        {t('maintenance.backup_import')}
                    </button>
                )}
                {snapshots && (
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={!!busy}
                        data-tooltip={t('maintenance.backup_keep_snapshot_hint')}
                        onClick={() =>
                            run(
                                'keep',
                                () => window.api.instanceBackupCreate(server.id, { source: 'snapshot' }),
                                () => t('maintenance.backup_snapshot_kept'),
                            )
                        }
                    >
                        {busy === 'keep' && <SabaSpinner size="sm" />} {t('maintenance.backup_keep_snapshot')}
                    </button>
                )}
                <button data-spot="backup.create"
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={!!busy}
                    onClick={() =>
                        run(
                            'create',
                            () => window.api.instanceBackupCreate(server.id),
                            (res) => (res?.backup ? t('maintenance.backup_created') : t('maintenance.backup_nothing')),
                        )
                    }
                >
                    {busy === 'create' ? <SabaSpinner size="sm" light /> : <Icon name="plus" size="sm" />}{' '}
                    {t('maintenance.backup_now')}
                </button>
            </div>
            <p className="dc-card-desc">{t('maintenance.backups_desc')}</p>
            {!stopped && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('maintenance.restore_stop_first')}
                </p>
            )}
            {loadError ? (
                <SabaFail message={loadError} onRetry={refresh} retryLabel={t('maintenance.retry')} />
            ) : backups === null ? (
                <SabaSpinner />
            ) : list.length === 0 ? (
                <p className="muted">{t('maintenance.backups_empty')}</p>
            ) : (
                <>
                {totalBytes !== null && (
                    <p className="dc-card-desc">
                        {t('maintenance.backup_total', { size: formatBytes(totalBytes), count: list.length })}
                    </p>
                )}
                <ul className="dc-list">
                    {list.map((b) => {
                        const restoreKey = `restore:${b.name}`;
                        const deleteKey = `delete:${b.name}`;
                        // zip으로 열리지 않는 백업: 복원할 수 없고 지울 때까지 데몬이 쓰지 않는 내용을 정리하지 못한다
                        const unreadable = b.readable === false;
                        const game = b.kind === GAME_SNAPSHOT;
                        // 우클릭도 버튼과 같이 한 번 누르면 확인 상태로 바꿀 뿐이다 (실제 실행은 버튼을 한 번 더)
                        const menu = (e: MouseEvent) =>
                            openMenu(e, [
                                {
                                    id: 'restore',
                                    label: t('maintenance.restore'),
                                    icon: 'refresh',
                                    disabled: !stopped || !!busy || unreadable,
                                    onSelect: () => setArmed(restoreKey),
                                },
                                !game &&
                                    canTransfer && {
                                        id: 'export',
                                        label: t('maintenance.backup_export'),
                                        icon: 'arrowUp',
                                        disabled: !!busy || unreadable,
                                        onSelect: () => exportBackup(b.name),
                                    },
                                {
                                    id: 'delete',
                                    label: t('maintenance.delete'),
                                    icon: 'trash',
                                    danger: true,
                                    disabled: !!busy,
                                    onSelect: () => setArmed(deleteKey),
                                },
                            ]);
                        return (
                            <li key={b.name} className="dc-list-row" onContextMenu={menu}>
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{fmt(b.created)}</span>
                                    <span
                                        className={clsx('dc-list-sub', unreadable && 'warn')}
                                        data-tooltip={unreadable ? t('maintenance.backup_unreadable_hint') : b.name}
                                    >
                                        {[
                                            unreadable
                                                ? t('maintenance.backup_unreadable')
                                                : game
                                                  ? t('maintenance.backup_game_snapshot')
                                                  : b.kind === 'pre-restore'
                                                    ? t('maintenance.backup_pre_restore_hint')
                                                    : b.imported_from
                                                      ? t('maintenance.backup_imported_from', { server: b.imported_from })
                                                      : null,
                                            b.name,
                                        ]
                                            .filter(Boolean)
                                            .join(' · ')}
                                    </span>
                                </span>
                                <StatusChip
                                    tone={KIND_TONE[b.kind] || 'idle'}
                                    label={t(`maintenance.kind_${b.kind}`, { defaultValue: b.kind })}
                                />
                                {!game && canTransfer && (
                                    <button
                                        type="button"
                                        className="btn btn-ghost btn-sm"
                                        disabled={!!busy || unreadable}
                                        onClick={() => exportBackup(b.name)}
                                    >
                                        {busy === `export:${b.name}` ? <SabaSpinner size="sm" /> : <Icon name="arrowUp" size="sm" />}{' '}
                                        {t('maintenance.backup_export')}
                                    </button>
                                )}
                                <button data-spot="backup.restore"
                                    type="button"
                                    className={clsx('btn btn-sm', armed === restoreKey ? 'btn-primary' : 'btn-ghost')}
                                    disabled={!stopped || !!busy || unreadable}
                                    data-tooltip={
                                        unreadable
                                            ? t('maintenance.backup_unreadable_hint')
                                            : stopped
                                              ? t('maintenance.restore_hint')
                                              : t('maintenance.restore_stop_first')
                                    }
                                    onClick={() =>
                                        armed === restoreKey
                                            ? run(
                                                  restoreKey,
                                                  () => window.api.instanceBackupRestore(server.id, b.name),
                                                  (res) =>
                                                      res?.links_not_restored > 0
                                                          ? {
                                                                text: t('maintenance.restored_links_missed', {
                                                                    count: res.links_not_restored,
                                                                }),
                                                                tone: 'warning',
                                                            }
                                                          : t('maintenance.restored'),
                                              )
                                            : setArmed(restoreKey)
                                    }
                                >
                                    {busy === restoreKey && <SabaSpinner size="sm" />}
                                    {armed === restoreKey ? t('maintenance.restore_confirm') : t('maintenance.restore')}
                                </button>
                                <button
                                    type="button"
                                    className={clsx('btn btn-sm', armed === deleteKey ? 'btn-danger' : 'btn-ghost')}
                                    disabled={!!busy}
                                    onClick={() =>
                                        armed === deleteKey
                                            ? run(deleteKey, () =>
                                                  window.api.instanceBackupDelete(server.id, b.name),
                                              )
                                            : setArmed(deleteKey)
                                    }
                                >
                                    {armed === deleteKey
                                        ? t('maintenance.delete_confirm')
                                        : t('maintenance.delete')}
                                </button>
                            </li>
                        );
                    })}
                </ul>
                </>
            )}
            <Dialog
                open={!!importing}
                onOpenChange={(open) => !open && busy !== 'import' && setImporting(null)}
                title={t('maintenance.backup_import_title')}
                description={importing?.path.split(/[\\/]/).pop()}
                media={<Icon name="download" />}
                size="sm"
                footer={
                    <>
                        <button type="button" className="btn btn-ghost" disabled={busy === 'import'} onClick={() => setImporting(null)}>
                            {t('maintenance.backup_import_cancel')}
                        </button>
                        <button
                            type="button"
                            className="btn btn-primary"
                            disabled={busy === 'import'}
                            onClick={() => importing && confirmImport(importing.path)}
                        >
                            {busy === 'import' && <SabaSpinner size="sm" light />} {t('maintenance.backup_import')}
                        </button>
                    </>
                }
            >
                {importing && (
                    <>
                        <dl className="dc-facts">
                            <dt>{t('maintenance.backup_import_server')}</dt>
                            <dd>
                                {importing.server} ({gameNameOf(t, importing.module)})
                            </dd>
                            <dt>{t('maintenance.backup_import_created')}</dt>
                            <dd>{fmt(importing.created)}</dd>
                            <dt>{t('maintenance.backup_import_size')}</dt>
                            <dd>{formatBytes(importing.size)}</dd>
                        </dl>
                        <p className="muted">{t('maintenance.backup_import_note')}</p>
                    </>
                )}
            </Dialog>
        </section>
    );
}