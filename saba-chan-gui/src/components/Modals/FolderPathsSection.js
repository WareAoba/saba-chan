import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';

export const folderPathsAvailable = () =>
    typeof window.api?.systemPaths === 'function' && typeof window.api?.systemPathsMove === 'function';

// 데몬 `paths_<코드>` (`component_dirs::PathError`) 중 문구가 있는 것
const MOVE_ERRORS = [
    'busy',
    'servers_running',
    'env_fixed',
    'not_absolute',
    'root_or_home',
    'contains_data_dir',
    'same_as_current',
    'overlaps_current',
    'overlaps_other',
    'not_empty',
    'not_writable',
    'has_link',
    'copy_failed',
    'verify_failed',
    'commit_failed',
];

const failed = (res) => !res || res.ok === false || res.success === false;

/** 옮기기 실패 → 한 줄 (아는 코드는 번역, 나머지는 데몬 문구) */
export function folderPathsErrorText(res, t) {
    const code = String(res?.error_code || '').replace(/^paths_/, '');
    if (MOVE_ERRORS.includes(code)) return t(`folder_paths.err_${code}`, { detail: res.error || '' });
    return res?.error || t('folder_paths.err_generic');
}

/**
 * 설정 > 고급: 폴더 위치 (데몬 GET /api/system/paths) 와 모듈·익스텐션 폴더 옮기기 (POST /api/system/paths/move).
 * 고른 폴더 아래 `modules`·`extensions`를 데몬이 만든다. CLI `config paths`·`config move`와 같은 기능이다.
 */
export function FolderPathsSection() {
    const { t } = useTranslation('gui');
    const [paths, setPaths] = useState(null);
    const [pending, setPending] = useState(null); // {kind, target}
    const [busy, setBusy] = useState(false);
    const [moved, setMoved] = useState(null);
    const [error, setError] = useState(null);

    const load = useCallback(async () => {
        const res = await window.api.systemPaths();
        if (failed(res)) setError(res?.error || t('folder_paths.err_generic'));
        else setPaths(res);
    }, [t]);

    useEffect(() => {
        load();
    }, [load]);

    const pick = async (kind) => {
        setError(null);
        setMoved(null);
        const target = await window.api.openFolderDialog?.();
        if (target) setPending({ kind, target });
    };

    const move = async () => {
        setBusy(true);
        setError(null);
        try {
            const res = await window.api.systemPathsMove(pending.kind, pending.target);
            if (failed(res)) {
                setError(folderPathsErrorText(res, t));
                return;
            }
            setMoved(res);
            setPending(null);
            await load();
        } catch (e) {
            setError(e.message);
        } finally {
            setBusy(false);
        }
    };

    const dirRow = (kind) => {
        const d = paths?.[kind] || {};
        return (
            <div key={kind} className="diagnostics-row">
                <dt>{t(`folder_paths.${kind}`)}</dt>
                <dd className="folder-paths-dir">
                    <code>{d.path}</code>
                    <span className="muted">
                        {t(`folder_paths.source_${d.source || 'default'}`)}
                        {d.source && d.source !== 'default' && <> · {t('folder_paths.default_is', { path: d.default })}</>}
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost"
                        onClick={() => pick(kind)}
                        disabled={busy || d.source === 'env'}
                    >
                        <Icon name="folder" size="sm" /> {t('folder_paths.move')}
                    </button>
                </dd>
            </div>
        );
    };

    const plain = (key, value) => (
        <div key={key} className="diagnostics-row">
            <dt>{t(`folder_paths.${key}`)}</dt>
            <dd>
                <code>{value || '-'}</code>
            </dd>
        </div>
    );

    return (
        <div className="data-transfer">
            <h4 className="data-transfer-title">
                <Icon name="folder" size="sm" /> {t('folder_paths.title')}
            </h4>
            <p className="setting-description">{t('folder_paths.description')}</p>
            {paths && (
                <dl className="diagnostics-grid">
                    {plain('data', paths.data_dir)}
                    {plain('install', paths.install_dir)}
                    {plain('default_install', paths.default_install_dir)}
                    {plain('instances', paths.instances_dir)}
                    {dirRow('modules')}
                    {dirRow('extensions')}
                </dl>
            )}
            {pending && (
                <div className="data-transfer-summary" role="group">
                    <div>{t(`folder_paths.confirm_${pending.kind}`, { dir: pending.target })}</div>
                    <div className="data-transfer-actions">
                        <button type="button" className="btn btn-ghost" onClick={() => setPending(null)} disabled={busy}>
                            {t('folder_paths.cancel')}
                        </button>
                        <button type="button" className="btn btn-primary" onClick={move} disabled={busy}>
                            {busy ? <SabaSpinner size="xs" /> : <Icon name="check" size="sm" />}{' '}
                            {t('folder_paths.confirm_move')}
                        </button>
                    </div>
                </div>
            )}
            {moved && (
                <div className="notice tone-ok" role="status">
                    <Icon name="checkCircle" size="sm" />
                    <span>{t('folder_paths.moved', { path: moved.path })}</span>
                </div>
            )}
            {moved?.left_behind?.length > 0 && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('folder_paths.left_behind', { files: moved.left_behind.join(', ') })}</span>
                </div>
            )}
            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
        </div>
    );
}
