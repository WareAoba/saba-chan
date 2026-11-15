import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { Icon } from '../Icon';
import { Disclosure } from '../ui/Disclosure';

export const folderPathsAvailable = () => typeof window.api?.systemPaths === 'function';

// 데몬 `paths_<코드>` (`component_dirs::PathError`) 중 문구가 있는 것 (`folder_paths.err_<코드>`). 파일 이동 창이 쓴다
export const PATH_ERRORS = [
    'busy',
    'servers_running',
    'env_fixed',
    'not_absolute',
    'root_or_home',
    'contains_data_dir',
    'same_as_current',
    'overlaps_current',
    'overlaps_other',
    'overlaps_install',
    'not_empty',
    'not_writable',
    'has_link',
    'copy_failed',
    'verify_failed',
    'commit_failed',
    'acl_failed',
];

/**
 * 설정 > 저장 공간: 폴더 위치 (데몬 GET /api/system/paths). 옮기기는 파일 이동 창(`FileMoveDialog`)이 한다
 */
export function FolderPathsSection({ defaultOpen = true }: { defaultOpen?: boolean }) {
    const { t } = useTranslation('gui');
    const [paths, setPaths] = useState<ApiBody | null>(null);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        window.api.systemPaths().then((res: ApiBody) => {
            if (failed(res)) setError(res?.error || t('folder_paths.err_generic'));
            else setPaths(res);
        });
    }, [t]);

    const row = (key: string, value: string | undefined, note?: string) => (
        <div key={key} className="diagnostics-row">
            <dt>{t(`folder_paths.${key}`)}</dt>
            <dd className="folder-paths-dir">
                <code>{value || '-'}</code>
                {note && <span className="muted">{note}</span>}
            </dd>
        </div>
    );

    const dirRow = (kind: string) => {
        const d: ApiBody = paths?.[kind] || {};
        const source = t(`folder_paths.source_${d.source || 'default'}`);
        const note =
            d.source && d.source !== 'default'
                ? `${source} · ${t('folder_paths.default_is', { path: d.default })}`
                : source;
        return row(kind, d.path, note);
    };

    return (
        <section className="saba-card">
            <Disclosure
                defaultOpen={defaultOpen}
                summary={
                    <span className="saba-card-title">
                        <Icon name="folder" size="sm" /> {t('folder_paths.title')}
                    </span>
                }
            >
                <p className="setting-description">{t('folder_paths.description')}</p>
                {paths && (
                    <dl className="diagnostics-grid">
                        {row('data', paths.data_dir)}
                        {row('install', paths.install_dir)}
                        {row('default_install', paths.default_install_dir)}
                        {row('instances', paths.instances_dir)}
                        {dirRow('modules')}
                        {dirRow('extensions')}
                    </dl>
                )}
                {error && (
                    <div className="setting-validation-error" role="alert">
                        <Icon name="alertCircle" size="sm" /> {error}
                    </div>
                )}
            </Disclosure>
        </section>
    );
}
