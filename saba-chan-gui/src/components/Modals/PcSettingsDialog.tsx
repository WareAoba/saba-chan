import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { OsLogo, type Platform } from '../OsLogo';
import { Dialog } from '../ui/Dialog';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { type AddressDraft, addressDraft, addressPatch, PublicAddressFields, usePublicAddress } from './PublicAddress';

interface PcSettingsDialogProps {
    /** null이면 이 PC */
    hostId: string | null;
    name: string;
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

/** 내 PC 카드의 수정 창: 게임 서버 주소, 먼저 보이는 PC, 업데이트, 지우기를 한곳에서 */
export function PcSettingsDialog(props: PcSettingsDialogProps) {
    const { hostId, name, sub, platform, hasAddress, main, updateTo, canUpdate, canRemove } = props;
    const { t } = useTranslation('gui');
    const address = usePublicAddress(hostId, hasAddress);
    const [draft, setDraft] = useState<AddressDraft | null>(null);
    const [mainDraft, setMainDraft] = useState(Boolean(main));
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
    const invalid = Boolean(draft && !patch);

    const save = async () => {
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
        props.onClose(addressChanged || mainChanged);
    };

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && !busy && props.onClose(false)}
            title={t('pc_settings.title', { name })}
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
                        disabled={busy || invalid || !(addressChanged || mainChanged)}
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

            {main !== undefined && (
                <section className="pc-settings-section">
                    <div className="pc-settings-row">
                        <label className="setting-label" htmlFor="pc-settings-main">
                            <span className="setting-title">
                                <Icon name="star" size="sm" /> {t('pc_settings.main_title')}
                            </span>
                            <span className="setting-description">
                                {main ? t('pc_settings.main_on_desc') : t('pc_settings.main_desc')}
                            </span>
                        </label>
                        <SabaToggle
                            id="pc-settings-main"
                            role="switch"
                            aria-label={t('pc_settings.main_title')}
                            checked={mainDraft}
                            disabled={busy || main}
                            onChange={setMainDraft}
                        />
                    </div>
                </section>
            )}

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
