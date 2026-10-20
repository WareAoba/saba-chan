import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';

export const dataTransferAvailable = () => Boolean(window.api?.dataExport && window.api?.dataImport);

const KNOWN_ERRORS = ['INVALID_PATH', 'INVALID_ARCHIVE', 'INVALID_CONFIG', 'WRITE_FAILED'];
const SKIP_REASONS = ['exists', 'running', 'working_dir_in_use', 'invalid_id', 'save_failed', 'name_taken', 'busy'];

const failed = (res) => !res || res.success === false || Boolean(res.error_code) || (res.error && !res.success);

/** 데몬 오류({success:false, error, error_code}) → 한 줄 */
export function dataTransferErrorText(res, t) {
    const code = res?.error_code;
    if (KNOWN_ERRORS.includes(code)) return t(`data_transfer.err_${code.toLowerCase()}`);
    return res?.error || t('data_transfer.err_generic');
}

const names = (list) => (list || []).map((i) => i.name || i.id).join(', ');

/**
 * 설정 > 고급: 데이터 내보내기·가져오기 (데몬 POST /api/migration/export·import).
 * 가져오기는 미리보기(dryRun) 뒤에 같은 파일로 한 번 더 부른다.
 */
export function DataTransferSection() {
    const { t } = useTranslation('gui');
    const [secrets, setSecrets] = useState(false);
    const [overwrite, setOverwrite] = useState(false);
    const [busy, setBusy] = useState(null); // 'export' | 'preview' | 'import'
    const [exported, setExported] = useState(null);
    const [preview, setPreview] = useState(null);
    const [imported, setImported] = useState(null);
    const [error, setError] = useState(null);

    const run = async (kind, call) => {
        setBusy(kind);
        setError(null);
        try {
            const res = await call();
            if (res?.canceled) return null;
            if (failed(res)) {
                setError(dataTransferErrorText(res, t));
                return null;
            }
            return res;
        } catch (e) {
            setError(e.message);
            return null;
        } finally {
            setBusy(null);
        }
    };

    const doExport = async () => {
        setImported(null);
        const res = await run('export', () => window.api.dataExport({ secrets }));
        if (res) setExported(res);
    };

    const loadPreview = async (path, ow) => {
        const res = await run('preview', () => window.api.dataImport({ path, dryRun: true, overwrite: ow }));
        if (res) setPreview(res);
        return res;
    };

    const pickFile = async () => {
        setExported(null);
        setImported(null);
        await loadPreview(undefined, overwrite);
    };

    // 덮어쓰기를 바꾸면 추가·교체·건너뜀이 달라진다: 같은 파일로 미리보기를 다시 받는다
    const changeOverwrite = async (ow) => {
        setOverwrite(ow);
        // 다시 받지 못하면 보이는 미리보기와 토글이 어긋나지 않게 되돌린다
        if (preview && !(await loadPreview(preview.path, ow))) setOverwrite(!ow);
    };

    const doImport = async () => {
        const res = await run('import', () => window.api.dataImport({ path: preview.path, dryRun: false, overwrite }));
        if (res) {
            setPreview(null);
            setImported(res);
        }
    };

    const summary = preview || imported;
    const inst = summary?.instances || {};

    return (
        <div className="data-transfer">
            <h4 className="data-transfer-title">
                <Icon name="package" size="sm" /> {t('data_transfer.title')}
            </h4>
            <p className="setting-description">{t('data_transfer.description')}</p>

            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">{t('data_transfer.secrets_label')}</span>
                    <span className="setting-description">{t('data_transfer.secrets_description')}</span>
                </label>
                <SabaToggle checked={secrets} onChange={setSecrets} aria-label={t('data_transfer.secrets_label')} />
            </div>

            <div className="data-transfer-actions">
                <button type="button" className="btn btn-primary" onClick={doExport} disabled={busy !== null}>
                    {busy === 'export' ? <SabaSpinner size="xs" /> : <Icon name="download" size="sm" />}{' '}
                    {t('data_transfer.export')}
                </button>
                <button type="button" className="btn btn-ghost" onClick={pickFile} disabled={busy !== null}>
                    {busy === 'preview' && !preview ? <SabaSpinner size="xs" /> : <Icon name="folder" size="sm" />}{' '}
                    {t('data_transfer.import_pick')}
                </button>
            </div>

            {exported && (
                <div className="notice tone-ok" role="status">
                    <Icon name="checkCircle" size="sm" />
                    <span>
                        {t('data_transfer.exported', { count: exported.instances?.length || 0, path: exported.path })}
                        {exported.secrets && <> {t('data_transfer.exported_secrets')}</>}
                    </span>
                </div>
            )}

            {summary && (
                <div className="data-transfer-summary">
                    <div className="data-transfer-head">
                        {preview ? t('data_transfer.preview_title') : t('data_transfer.imported_title')}
                        {summary.appVersion && (
                            <span className="muted">
                                {' '}
                                · {t('data_transfer.from_version', { version: summary.appVersion })}
                            </span>
                        )}
                    </div>
                    <ul>
                        {inst.added?.length > 0 && (
                            <li>{t('data_transfer.added', { count: inst.added.length, names: names(inst.added) })}</li>
                        )}
                        {inst.replaced?.length > 0 && (
                            <li>
                                {t('data_transfer.replaced', {
                                    count: inst.replaced.length,
                                    names: names(inst.replaced),
                                })}
                            </li>
                        )}
                        {(inst.skipped || []).map((s) => (
                            <li key={s.id} className="data-transfer-skip">
                                {t('data_transfer.skipped', {
                                    name: s.name || s.id,
                                    reason: t(
                                        `data_transfer.reason_${SKIP_REASONS.includes(s.reason) ? s.reason : 'other'}`,
                                    ),
                                })}
                            </li>
                        ))}
                        {!inst.added?.length && !inst.replaced?.length && !inst.skipped?.length && (
                            <li>{t('data_transfer.no_servers')}</li>
                        )}
                        {summary.config?.length > 0 && (
                            <li>{t('data_transfer.config', { sections: summary.config.join(', ') })}</li>
                        )}
                        {['imported', 'kept'].includes(summary.nodeToken) && (
                            <li>{t(`data_transfer.node_token_${summary.nodeToken}`)}</li>
                        )}
                        {summary.secrets && <li>{t('data_transfer.includes_secrets')}</li>}
                    </ul>
                    {imported?.config?.includes('gui') && (
                        <p className="setting-description">{t('data_transfer.restart_hint')}</p>
                    )}
                    {summary.missingModules?.length > 0 && (
                        <div className="notice tone-warn" role="status">
                            <Icon name="alertTriangle" size="sm" />
                            <span>
                                {t('data_transfer.missing_modules', { modules: summary.missingModules.join(', ') })}
                            </span>
                        </div>
                    )}
                    {preview && (
                        <>
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">{t('data_transfer.overwrite_label')}</span>
                                    <span className="setting-description">
                                        {t('data_transfer.overwrite_description')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={overwrite}
                                    onChange={changeOverwrite}
                                    disabled={busy !== null}
                                    aria-label={t('data_transfer.overwrite_label')}
                                />
                            </div>
                            <div className="data-transfer-actions">
                                <button
                                    type="button"
                                    className="btn btn-ghost"
                                    onClick={() => setPreview(null)}
                                    disabled={busy !== null}
                                >
                                    {t('data_transfer.cancel')}
                                </button>
                                <button
                                    type="button"
                                    className="btn btn-primary"
                                    onClick={doImport}
                                    disabled={busy !== null}
                                >
                                    {busy === 'import' ? <SabaSpinner size="xs" /> : <Icon name="check" size="sm" />}{' '}
                                    {t('data_transfer.import_confirm')}
                                </button>
                            </div>
                        </>
                    )}
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
