import clsx from 'clsx';
import type { TFunction } from 'i18next';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { formatBytes } from '../monitor/systemMetrics';
import { Dialog } from '../ui/Dialog';
import { SabaSpinner } from '../ui/SabaUI';
import { PATH_ERRORS } from './FolderPathsSection';
import { driveLabel, itemLabel, type StorageItem, type StorageVolume } from './StorageSection';

// 데몬 `storage_<코드>` 중 문구가 있는 것 (나머지는 `paths_<코드>`와 같은 뜻이라 folder_paths 문구를 쓴다)
const STORAGE_ERRORS = [
    'server_running',
    'no_space',
    'not_movable',
    'no_server_folder',
    'root',
    'overlaps_data',
    'overlaps_other',
    'busy',
    'instance_busy',
];

/** 옮기기 거절·실패 → 한 줄. 접두어(`storage_`·`paths_`)가 없으면(작업의 오류) 옮기던 것의 종류로 고른다 */
export function storageErrorText(
    code: string | null | undefined,
    detail: string | null | undefined,
    kind: string,
    t: TFunction,
) {
    const raw = String(code || '');
    const bare = raw.replace(/^(storage|paths)_/, '');
    const server = raw.startsWith('storage_') || (!raw.startsWith('paths_') && kind === 'server');
    const vars = { detail: detail || '' };
    if (server && STORAGE_ERRORS.includes(bare)) return t(`storage.err_${bare}`, vars);
    if (PATH_ERRORS.includes(bare)) return t(`folder_paths.err_${bare}`, vars);
    if (STORAGE_ERRORS.includes(bare)) return t(`storage.err_${bare}`, vars);
    return detail || t('storage.err_generic');
}

type Pick = { mount: string } | { target: string };

interface Props {
    volumes: StorageVolume[];
    items: StorageItem[];
    onClose: () => void;
    /** 옮기기를 마쳤다 (다시 잰다) */
    onMoved: () => void;
}

const POLL_MS = 500;
const optionValue = (i: StorageItem) => (i.kind === 'server' ? `server:${i.id}` : i.kind);

/**
 * 파일 이동 창: 무엇을(서버 폴더·모듈·익스텐션) → 어디로(드라이브, 또는 폴더 직접 고르기) → 데몬이 정한 자리·공간 확인 → 옮기기.
 * 자리·공간 판단과 복사는 데몬이 한다 (POST /api/system/storage/move, dryRun으로 먼저 묻는다). 옮기는 동안 진행을 읽는다
 */
export function FileMoveDialog({ volumes, items, onClose, onMoved }: Props) {
    const { t } = useTranslation('gui');
    const movable = items.filter((i) => i.kind === 'server' || i.kind === 'modules' || i.kind === 'extensions');
    const [what, setWhat] = useState('');
    const [pick, setPick] = useState<Pick | null>(null);
    const [preview, setPreview] = useState<ApiBody | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [asking, setAsking] = useState(false);
    const [job, setJob] = useState<ApiBody | null>(null);
    const alive = useRef(true);
    useEffect(() => {
        alive.current = true;
        return () => {
            alive.current = false;
        };
    }, []);

    const item = movable.find((i) => optionValue(i) === what);
    const running = !!job && !['done', 'failed', 'canceled'].includes(job.phase);
    const finished = job && ['done', 'canceled'].includes(job.phase);

    const body = (it: StorageItem, p: Pick) => ({
        kind: it.kind,
        ...(it.kind === 'server' ? { id: it.id } : {}),
        ...p,
    });

    const ask = async (p: Pick) => {
        if (!item) return;
        setPick(p);
        setPreview(null);
        setError(null);
        setAsking(true);
        try {
            const res = await window.api.systemStorageMove({ ...body(item, p), dryRun: true });
            if (failed(res)) setError(storageErrorText(res?.error_code, res?.error, item.kind, t));
            else setPreview(res);
        } finally {
            setAsking(false);
        }
    };

    const chooseFolder = async () => {
        const target = await window.api.openFolderDialog?.();
        if (target) await ask({ target });
    };

    const poll = () => {
        setTimeout(async () => {
            if (!alive.current) return;
            const res = await window.api.systemStorageMoveStatus();
            const next = res?.job;
            if (!next) return;
            setJob(next);
            if (next.phase === 'failed') setError(storageErrorText(next.error_code, next.error, next.kind, t));
            if (next.phase === 'done') onMoved();
            if (!['done', 'failed', 'canceled'].includes(next.phase)) poll();
        }, POLL_MS);
    };

    const start = async () => {
        if (!item || !pick) return;
        setError(null);
        const res = await window.api.systemStorageMove(body(item, pick));
        if (failed(res)) {
            setError(storageErrorText(res?.error_code, res?.error, item.kind, t));
            return;
        }
        setJob(res.job);
        poll();
    };

    const options = movable.map((i) => ({
        value: optionValue(i),
        label: `${i.kind === 'server' ? t('storage.option_server', { name: itemLabel(i, t) }) : itemLabel(i, t)} — ${formatBytes(i.bytes)}`,
    }));
    const pickedMount = pick && 'mount' in pick ? pick.mount : null;
    const targetVolume = volumes.find((v) => v.mount === pickedMount);
    const checks: [string, string][] = preview
        ? [
              ...(item?.kind === 'server' ? [['ok', t('storage.check_stopped')] as [string, string]] : []),
              [
                  'ok',
                  t('storage.check_space', {
                      need: formatBytes(preview.bytes),
                      free: formatBytes(preview.available_bytes),
                  }),
              ],
              ...(item?.kind === 'server' && targetVolume?.kind === 'hdd'
                  ? [['warn', t('storage.check_hdd')] as [string, string]]
                  : []),
          ]
        : [];
    const pct = job?.total_bytes ? Math.min(100, (job.done_bytes / job.total_bytes) * 100) : -1;

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && !running && onClose()}
            title={t('storage.move_files')}
            description={t('storage.move_desc')}
            dismissible={!running}
            className="storage-move"
            footer={
                running ? (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        onClick={() => window.api.systemStorageMoveCancel()}
                        disabled={job?.phase === 'moving'}
                    >
                        {t('storage.cancel')}
                    </button>
                ) : finished ? null : (
                    <button type="button" className="btn btn-primary" onClick={start} disabled={!preview || asking}>
                        <Icon name="check" size="sm" /> {t('storage.move')}
                    </button>
                )
            }
        >
            {job?.phase === 'done' && (
                <>
                    <div className="notice tone-ok" role="status">
                        <Icon name="checkCircle" size="sm" />
                        <span>{t('storage.moved', { path: job.target })}</span>
                    </div>
                    {job.left_behind?.length > 0 && (
                        <div className="notice tone-warn" role="status">
                            <Icon name="alertTriangle" size="sm" />
                            <span>{t('folder_paths.left_behind', { files: job.left_behind.join(', ') })}</span>
                        </div>
                    )}
                </>
            )}
            {job?.phase === 'canceled' && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('storage.canceled')}</span>
                </div>
            )}
            {running && (
                <div className="storage-progress">
                    <div className="storage-progress-row">
                        <span>{t(`storage.phase_${job.phase}`)}</span>
                        {pct >= 0 && (
                            <span>
                                {formatBytes(job.done_bytes)} / {formatBytes(job.total_bytes)}
                            </span>
                        )}
                    </div>
                    <div className={clsx('ss-progress', { indeterminate: pct < 0 || job.phase === 'moving' })}>
                        <i style={pct < 0 || job.phase === 'moving' ? undefined : { width: `${pct}%` }} />
                    </div>
                </div>
            )}
            {!running && !finished && (
                <div className="storage-steps">
                    <div className="storage-step">
                        <span className="storage-step-label">
                            <span className="storage-step-n">1</span>
                            {t('storage.step_what')}
                        </span>
                        <CustomDropdown
                            className="storage-what"
                            ariaLabel={t('storage.step_what')}
                            value={what || undefined}
                            placeholder={t('storage.pick')}
                            onChange={(v: string) => {
                                setWhat(v);
                                setPick(null);
                                setPreview(null);
                                setError(null);
                            }}
                            options={options}
                        />
                    </div>
                    {item && (
                        <div className="storage-step">
                            <span className="storage-step-label" id="storage-where">
                                <span className="storage-step-n">2</span>
                                {t('storage.step_where')}
                            </span>
                            <div className="storage-drives" role="radiogroup" aria-labelledby="storage-where">
                                {volumes.map((v) => {
                                    const here = v.mount === item.mount;
                                    const full = !here && item.bytes > v.available_bytes;
                                    const after = Math.max(0, v.available_bytes - item.bytes);
                                    const used = ((v.total_bytes - v.available_bytes) / v.total_bytes) * 100;
                                    const adds = here ? 0 : Math.min(100 - used, (item.bytes / v.total_bytes) * 100);
                                    return (
                                        <button
                                            key={v.mount}
                                            type="button"
                                            role="radio"
                                            aria-checked={pickedMount === v.mount}
                                            aria-label={driveLabel(v)}
                                            className="storage-drive"
                                            disabled={here || full || asking}
                                            onClick={() => ask({ mount: v.mount })}
                                        >
                                            <span className="storage-drive-top">
                                                {driveLabel(v)}
                                                {v.kind !== 'unknown' && (
                                                    <span className="storage-chip">{v.kind.toUpperCase()}</span>
                                                )}
                                            </span>
                                            <span className="storage-mini" aria-hidden="true">
                                                <span style={{ width: `${used}%` }} />
                                                <span className="adds" style={{ width: `${adds}%` }} />
                                            </span>
                                            <span className="storage-drive-free">
                                                {t('storage.free_of', {
                                                    free: formatBytes(v.available_bytes),
                                                    total: formatBytes(v.total_bytes),
                                                })}
                                            </span>
                                            <span className="storage-drive-why">
                                                {here
                                                    ? t('storage.current_drive')
                                                    : full
                                                      ? t('storage.no_space')
                                                      : t('storage.after_move', { free: formatBytes(after) })}
                                            </span>
                                        </button>
                                    );
                                })}
                            </div>
                        </div>
                    )}
                    {item && (
                        <div className="storage-step">
                            <span className="storage-step-label">
                                <span className="storage-step-n">3</span>
                                {t('storage.step_check')}
                            </span>
                            <div className="storage-path">
                                <div className="storage-path-row">
                                    <span>{t('storage.new_location')}</span>
                                    <button
                                        type="button"
                                        className="btn btn-ghost btn-sm"
                                        onClick={chooseFolder}
                                        disabled={asking}
                                    >
                                        <Icon name="folder" size="sm" /> {t('storage.choose_folder')}
                                    </button>
                                </div>
                                <code>{asking ? <SabaSpinner size="xs" /> : preview?.target || '—'}</code>
                            </div>
                            {checks.length > 0 && (
                                <ul className="storage-checks">
                                    {checks.map(([tone, text]) => (
                                        <li key={text} className={`tone-${tone}`}>
                                            <Icon name={tone === 'ok' ? 'check' : 'alertTriangle'} size="sm" />
                                            <span>{text}</span>
                                        </li>
                                    ))}
                                </ul>
                            )}
                        </div>
                    )}
                </div>
            )}
            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
        </Dialog>
    );
}
