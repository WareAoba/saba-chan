import type { TFunction } from 'i18next';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { Icon } from '../Icon';
import { formatBytes } from '../monitor/systemMetrics';
import { SabaSpinner } from '../ui/SabaUI';
import { FileMoveDialog } from './FileMoveDialog';
import './StorageSection.css';


/** 디스크 막대의 갈래 (데몬 `storage::Category`) 와 색 */
export const CATEGORIES = ['servers', 'backups', 'components', 'app'] as const;

export interface StorageVolume {
    mount: string;
    name: string;
    kind: 'ssd' | 'hdd' | 'unknown';
    total_bytes: number;
    available_bytes: number;
    saba: Record<(typeof CATEGORIES)[number], number>;
}

export interface StorageItem {
    kind: 'server' | 'backups' | 'modules' | 'extensions' | 'app';
    category: (typeof CATEGORIES)[number];
    id?: string;
    name?: string;
    path?: string;
    mount: string | null;
    bytes: number;
    running?: boolean;
}

/** 드라이브 짧은 이름: Windows는 `C:`, 그 밖은 마운트 자리 */
export const driveShort = (mount: string) =>
    /^[A-Za-z]:[\\/]?$/.test(mount) ? mount.slice(0, 2).toUpperCase() : mount;

/** 드라이브 이름: `이름 (C:)`, 이름이 없으면 짧은 이름 */
export function driveLabel(v: Pick<StorageVolume, 'mount' | 'name'>) {
    const short = driveShort(v.mount);
    return v.name && v.name !== v.mount ? `${v.name} (${short})` : short;
}

/** 사바쨩 파일 한 줄의 이름 */
export function itemLabel(item: StorageItem, t: TFunction) {
    return item.kind === 'server' ? item.name || item.id || '' : t(`storage.item_${item.kind}`);
}

const sabaTotal = (v: StorageVolume) => CATEGORIES.reduce((sum, c) => sum + (v.saba?.[c] || 0), 0);

function measuredText(atMs: number | null | undefined, lang: string, t: TFunction) {
    if (!atMs) return '';
    const minutes = Math.round((Date.now() - atMs) / 60_000);
    const when = new Intl.RelativeTimeFormat(lang, { numeric: 'auto' }).format(-minutes, 'minute');
    return t('storage.measured', { when });
}

function DiskRow({ volume, t }: { volume: StorageVolume; t: TFunction }) {
    const label = driveLabel(volume);
    const used = volume.total_bytes - volume.available_bytes;
    const saba = sabaTotal(volume);
    const other = Math.max(0, used - saba);
    const pct = (b: number) => `${((b / volume.total_bytes) * 100).toFixed(2)}%`;
    return (
        <fieldset className="storage-disk" aria-label={label}>
            <div className="storage-disk-head">
                <span className="storage-disk-name">{label}</span>
                {volume.kind !== 'unknown' && <span className="storage-chip">{volume.kind.toUpperCase()}</span>}
                <span className="storage-disk-size">
                    {t('storage.free_of', {
                        free: formatBytes(volume.available_bytes),
                        total: formatBytes(volume.total_bytes),
                    })}
                </span>
            </div>
            <div className="storage-bar" aria-hidden="true">
                {CATEGORIES.filter((c) => volume.saba?.[c] > 0).map((c) => (
                    // 작아도 보이게 최소 3px
                    <span
                        key={c}
                        className={`storage-seg cat-${c}`}
                        style={{ width: `max(3px, ${pct(volume.saba[c])})` }}
                    />
                ))}
                <span className="storage-seg cat-other" style={{ width: pct(other) }} />
            </div>
            <div className="storage-disk-saba">
                {t('storage.saba')} <strong>{saba ? formatBytes(saba) : t('storage.none')}</strong> ·{' '}
                {t('storage.other_files')} {formatBytes(other)}
            </div>
        </fieldset>
    );
}

/**
 * 설정 > 저장 공간: 내장 디스크와 사바쨩 파일(데몬 GET /api/system/storage), 파일 이동 창.
 * 크기는 데몬이 백그라운드에서 잰다: 재는 동안(`measuring`) 잠깐씩 다시 읽는다
 */
export function StorageSection() {
    const { t, i18n } = useTranslation('gui');
    const [data, setData] = useState<ApiBody | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [moveOpen, setMoveOpen] = useState(false);

    const load = useCallback(
        async (refresh = false) => {
            const res = await window.api.systemStorage(refresh);
            if (failed(res)) setError(res?.error || t('storage.err_generic'));
            else {
                setError(null);
                setData(res);
            }
        },
        [t],
    );

    useEffect(() => {
        load(false);
    }, [load]);

    useEffect(() => {
        if (!data?.measuring) return;
        const timer = setTimeout(() => load(false), 1500);
        return () => clearTimeout(timer);
    }, [data, load]);

    const volumes: StorageVolume[] = data?.volumes || [];
    const items: StorageItem[] = data?.items || [];
    const groups = CATEGORIES.map(
        (c) => [c, items.filter((i) => i.category === c).sort((a, b) => b.bytes - a.bytes)] as const,
    );
    const total = items.reduce((sum, i) => sum + i.bytes, 0);

    return (
        <>
            <section className="saba-card">
                <h4 className="saba-card-title">
                    <Icon name="database" size="sm" /> {t('storage.disks')}
                    <span className="storage-meta">
                        {data?.measuring ? (
                            <>
                                <SabaSpinner size="xs" /> {t('storage.measuring')}
                            </>
                        ) : (
                            measuredText(data?.measured_at_ms, i18n.language, t)
                        )}
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => load(true)}
                            disabled={!!data?.measuring}
                        >
                            {t('storage.refresh')}
                        </button>
                    </span>
                </h4>
                {volumes.map((v) => (
                    <DiskRow key={v.mount} volume={v} t={t} />
                ))}
                <div className="storage-legend">
                    {CATEGORIES.map((c) => (
                        <span key={c}>
                            <i className={`cat-${c}`} />
                            {t(`storage.cat_${c}`)}
                        </span>
                    ))}
                    <span>
                        <i className="cat-other" />
                        {t('storage.other_files')}
                    </span>
                    <span>
                        <i className="cat-free" />
                        {t('storage.cat_free')}
                    </span>
                </div>
                {error && (
                    <div className="setting-validation-error" role="alert">
                        <Icon name="alertCircle" size="sm" /> {error}
                    </div>
                )}
            </section>

            <section className="saba-card">
                <h4 className="saba-card-title">
                    <Icon name="package" size="sm" /> {t('storage.files')}
                    {data?.measured_at_ms && (
                        <span className="storage-meta">{t('storage.total', { size: formatBytes(total) })}</span>
                    )}
                </h4>
                <div className="storage-items">
                    {groups
                        .filter(([, list]) => list.length > 0)
                        .map(([c, list]) => (
                            <div key={c} className="storage-group">
                                <div className="storage-group-label">{t(`storage.cat_${c}`)}</div>
                                {list.map((item) => (
                                    <div key={`${item.kind}:${item.id || item.mount}`} className="storage-item">
                                        <span className="storage-item-name">
                                            <i className={`cat-${c}`} />
                                            {itemLabel(item, t)}
                                            {item.mount && (
                                                <span className="storage-chip">{driveShort(item.mount)}</span>
                                            )}
                                        </span>
                                        <span className="storage-item-size">
                                            {formatBytes(item.bytes)}
                                            {item.running && (
                                                <span className="storage-pill">{t('storage.running')}</span>
                                            )}
                                        </span>
                                        {item.path && <code className="storage-item-path">{item.path}</code>}
                                    </div>
                                ))}
                            </div>
                        ))}
                </div>
                <div className="saba-card-actions">
                    <button
                        type="button"
                        className="btn btn-ghost"
                        onClick={() => setMoveOpen(true)}
                        disabled={!data?.measured_at_ms}
                    >
                        <Icon name="swap" size="sm" /> {t('storage.move_files')}
                    </button>
                </div>
            </section>

            {moveOpen && (
                <FileMoveDialog
                    volumes={volumes}
                    items={items}
                    onClose={() => setMoveOpen(false)}
                    onMoved={() => load(true)}
                />
            )}
        </>
    );
}
