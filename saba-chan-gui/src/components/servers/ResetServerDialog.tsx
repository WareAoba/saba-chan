import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { formatBytes } from '../../utils/schedule';
import { Icon } from '../Icon';
import { ConfirmIcon } from '../Modals/Modals';
import { Dialog } from '../ui/Dialog';
import { SabaCheckbox } from '../ui/SabaUI';
import { SegmentedChoice } from '../ui/StatusChip';

/** 초기화 범위 하나 (데몬 GET …/server/reset `scopes[]`): 지금 있는 항목만 크기와 함께 온다 */
export interface ResetScope {
    id: string;
    label?: string;
    note?: string;
    /** 이 범위가 백업이 담는 것을 지우는가 (지우기 전에 백업을 권한다) */
    backup?: boolean;
    items: { label?: string; paths: string[]; size: number }[];
}

export interface ResetChoice {
    scope: string;
    backup: boolean;
}

/** 서버 초기화 확인 창: 고른 범위가 실제로 지우는 파일·폴더를 그대로 보인다 */
export function ResetServerDialog({
    name,
    scopes,
    onClose,
    onConfirm,
}: {
    name: string;
    scopes: ResetScope[];
    onClose: () => void;
    onConfirm: (choice: ResetChoice) => void;
}) {
    const { t } = useTranslation('gui');
    // 좁은 범위(모듈이 먼저 적은 것)가 기본이다
    const [picked, setPicked] = useState(scopes[0].id);
    const [backup, setBackup] = useState(true);
    const scope = scopes.find((s) => s.id === picked) || scopes[0];
    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onClose()}
            title={t('server_settings.reset_confirm_title')}
            description={t('server_settings.reset_preview_desc', { name })}
            media={<ConfirmIcon icon="alertTriangle" tone="stop" />}
            size="sm"
            footer={
                <>
                    <button type="button" className="btn btn-ghost" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-danger"
                        disabled={!scope.items.length}
                        onClick={() => onConfirm({ scope: scope.id, backup: !!scope.backup && backup })}
                    >
                        {t('server_settings.reset_delete_count', { count: scope.items.length })}
                    </button>
                </>
            }
        >
            {scopes.length > 1 && (
                <>
                    <p className="muted">{t('server_settings.reset_scope')}</p>
                    <SegmentedChoice
                        ariaLabel={t('server_settings.reset_scope')}
                        items={scopes.map((s) => ({ id: s.id, label: s.label || s.id }))}
                        value={scope.id}
                        onChange={setPicked}
                    />
                </>
            )}
            {scope.items.length ? (
                <ul className="dc-list">
                    {scope.items.map((item) => (
                        <li key={item.paths.join('|')} className="dc-list-row">
                            <Icon name={item.paths.some((p) => p.endsWith('/')) ? 'folder' : 'file'} size="sm" />
                            <span className="dc-list-text">
                                <code className="dc-list-name">{item.paths.join(' · ')}</code>
                                {item.label && <span className="dc-list-sub">{item.label}</span>}
                            </span>
                            <span className="muted">{formatBytes(item.size)}</span>
                        </li>
                    ))}
                </ul>
            ) : (
                <p className="muted">{t('server_settings.reset_nothing')}</p>
            )}
            {scope.note && (
                <div className="notice tone-info">
                    <Icon name="info" size="sm" />
                    <span>{scope.note}</span>
                </div>
            )}
            {scope.backup && scope.items.length > 0 && (
                <SabaCheckbox checked={backup} onChange={setBackup}>
                    {t('server_settings.reset_backup_first')}
                </SabaCheckbox>
            )}
        </Dialog>
    );
}
