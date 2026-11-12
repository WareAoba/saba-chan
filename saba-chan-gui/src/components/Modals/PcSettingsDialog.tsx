import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { OsLogo, type Platform } from '../OsLogo';
import { Dialog } from '../ui/Dialog';
import { SabaSpinner } from '../ui/SabaUI';
import { type AddressDraft, addressDraft, addressPatch, PublicAddressFields, usePublicAddress } from './PublicAddress';

/** PC 이름 상한 (서버쨩 `MAX_HOST_NAME`) */
const PC_NAME_MAX = 64;

interface PcSettingsDialogProps {
    /** null이면 이 PC */
    hostId: string | null;
    /** 창 제목에 쓰는 이름 (주인 이름이 붙을 수 있다) */
    name: string;
    /** 바꿀 PC 이름 그대로 (서버쨩의 PC 이름) */
    pcName?: string;
    /** PC 이름을 바꿀 수 있으면 (서버쨩에 연결된 내 PC). 실패하면 까닭을 돌려준다 */
    onRename?: (name: string) => Promise<string | null>;
    sub: string;
    platform?: Platform | null;
    /** 그 PC의 게임 서버 주소를 읽을 수 있을 때만 (켜져 있고 연결된 데몬) */
    hasAddress: boolean;
    /** 앱을 열면 먼저 보이는 PC를 고를 수 있으면 지금 값, 고를 수 없으면 undefined */
    main?: boolean;
    /** 이 PC보다 옛 판이면 업데이트할 판 */
    updateTo?: string;
    canUpdate?: boolean;
    /** 지울 수 있으면 (이 PC는 지울 수 없다) */
    canRemove: boolean;
    onMain: () => void;
    onUpdate: () => void;
    onRemove: () => void;
    onClose: (changed: boolean) => void;
}

/** 내 PC 카드의 수정 창: PC 이름(제목 자리)과 메인 별, 게임 서버 주소, 업데이트, 지우기를 한곳에서 */
export function PcSettingsDialog(props: PcSettingsDialogProps) {
    const { hostId, name, sub, platform, hasAddress, main, updateTo, canUpdate, canRemove } = props;
    const { t } = useTranslation('gui');
    const address = usePublicAddress(hostId, hasAddress);
    const [draft, setDraft] = useState<AddressDraft | null>(null);
    const [mainDraft, setMainDraft] = useState(Boolean(main));
    const [nameDraft, setNameDraft] = useState(props.pcName || '');
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        if (address.current && !draft) setDraft(addressDraft(address.current));
    }, [address.current, draft]);

    const patch = draft ? addressPatch(draft) : null;
    const cur = address.current;
    const addressChanged =
        Boolean(cur && patch) &&
        (patch?.address !== (cur?.address || '') || patch?.shareAddress !== (cur?.shareAddress !== false));
    const mainChanged = main !== undefined && mainDraft !== main;
    const newName = nameDraft.replace(/\s+/g, ' ').trim();
    const nameChanged = Boolean(props.onRename) && newName !== (props.pcName || '');
    const invalid = Boolean(draft && !patch) || (nameChanged && !newName);

    const save = async () => {
        if (nameChanged && props.onRename) {
            setBusy(true);
            setError(null);
            const err = await props.onRename(newName);
            setBusy(false);
            if (err) {
                setError(err);
                return;
            }
        }
        if (addressChanged && patch) {
            setBusy(true);
            setError(null);
            const err = await address.save(patch);
            setBusy(false);
            if (err) {
                setError(err);
                return;
            }
        }
        if (mainChanged && mainDraft) props.onMain();
        props.onClose(addressChanged || mainChanged || nameChanged);
    };

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && !busy && props.onClose(false)}
            title={
                // 제목 자리가 곧 PC 이름 칸이고, 바로 오른쪽 별이 메인 데몬이다 (사용자 11-12)
                <span className="pc-settings-titlerow">
                    {props.onRename ? (
                        <input
                            id="pc-settings-name"
                            type="text"
                            className="input pc-settings-name"
                            aria-label={t('pc_settings.name_title')}
                            maxLength={PC_NAME_MAX}
                            value={nameDraft}
                            disabled={busy}
                            onChange={(e) => setNameDraft(e.target.value)}
                        />
                    ) : (
                        <span className="pc-settings-name-text">{name}</span>
                    )}
                    {main !== undefined && (
                        <button
                            type="button"
                            className={`remote-pc-star${mainDraft ? ' on' : ''}`}
                            aria-pressed={mainDraft}
                            aria-label={mainDraft ? t('remote_daemon.star_on') : t('remote_daemon.star_set')}
                            data-tooltip={mainDraft ? t('remote_daemon.star_on') : t('remote_daemon.star_set')}
                            // 지금 메인이면 끌 수 없다 (다른 PC를 고르면 바뀐다)
                            disabled={busy || main}
                            onClick={() => setMainDraft((m) => !m)}
                        >
                            <Icon name="star" size="sm" />
                        </button>
                    )}
                </span>
            }
            description={sub}
            media={
                <span className="remote-pc-os large">
                    <OsLogo platform={platform} size={32} />
                </span>
            }
            className="pc-settings"
            footer={
                <>
                    <button
                        type="button"
                        className="btn btn-ghost"
                        onClick={() => props.onClose(false)}
                        disabled={busy}
                    >
                        {t('remote_daemon.cancel')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-primary"
                        disabled={busy || invalid || !(addressChanged || mainChanged || nameChanged)}
                        onClick={save}
                    >
                        {busy ? <SabaSpinner size="xs" /> : null} {t('public_address.save')}
                    </button>
                </>
            }
        >
            <section className="pc-settings-section">
                <h4 className="pc-settings-heading">
                    <Icon name="globe" size="sm" /> {t('public_address.label')}
                </h4>
                {!hasAddress && <p className="setting-description">{t('pc_settings.address_unavailable')}</p>}
                {hasAddress && !address.loaded && <SabaSpinner size="sm" />}
                {hasAddress && address.loaded && !address.current && (
                    <p className="setting-description">{t('pc_settings.address_unavailable')}</p>
                )}
                {address.current && draft && (
                    <PublicAddressFields current={address.current} draft={draft} busy={busy} onChange={setDraft} />
                )}
            </section>

            {updateTo && (
                <section className="pc-settings-section">
                    <div className="pc-settings-row">
                        <span className="setting-label">
                            <span className="setting-title">
                                <Icon name="download" size="sm" />{' '}
                                {t('remote_daemon.update_needed', { version: updateTo })}
                            </span>
                            <span className="setting-description">{t('pc_settings.update_desc')}</span>
                        </span>
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            disabled={busy || !canUpdate}
                            onClick={props.onUpdate}
                        >
                            <Icon name="download" size="sm" /> {t('remote_daemon.update')}
                        </button>
                    </div>
                </section>
            )}

            {canRemove && (
                <section className="pc-settings-section pc-settings-danger">
                    <div className="pc-settings-row">
                        <span className="setting-label">
                            <span className="setting-title">{t('pc_settings.remove_title')}</span>
                            <span className="setting-description">{t('pc_settings.remove_desc')}</span>
                        </span>
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm danger"
                            disabled={busy}
                            onClick={props.onRemove}
                        >
                            <Icon name="trash" size="sm" /> {t('remote_daemon.delete')}
                        </button>
                    </div>
                </section>
            )}

            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
        </Dialog>
    );
}
