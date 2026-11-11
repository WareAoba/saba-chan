import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';

const failed = (res: ApiBody | null | undefined) => !res || res.success === false || Boolean(res.error_code);

/** 고칠 값: 공개 주소(빈 값이면 공인 IP 그대로)와 디스코드 `/<서버> 주소`로 알려 주기 */
interface Draft {
    custom: boolean;
    address: string;
    shareAddress: boolean;
}

interface DialogProps {
    name: string;
    current: ApiBody;
    busy: boolean;
    error: string | null;
    onCancel: () => void;
    onSave: (patch: { address: string; shareAddress: boolean }) => void;
}

/** 게임 서버 주소 바꾸기 창: 공인 IP 그대로 / 공개 주소 직접 정하기, 디스코드에 알려 주기 */
export function PublicAddressDialog({ name, current, busy, error, onCancel, onSave }: DialogProps) {
    const { t } = useTranslation('gui');
    const [draft, setDraft] = useState<Draft>({
        custom: Boolean(current.address),
        address: current.address || '',
        shareAddress: current.shareAddress !== false,
    });
    const example = `${draft.custom && draft.address.trim() ? draft.address.trim() : current.publicIp || 'mc.example.com'}:25565`;
    const option = (custom: boolean, title: string, description: string) => (
        <label className={`public-address-option${draft.custom === custom ? ' on' : ''}`}>
            <input
                type="radio"
                name="public-address-mode"
                checked={draft.custom === custom}
                disabled={busy}
                onChange={() => setDraft({ ...draft, custom })}
            />
            <span>
                <b>{title}</b>
                <span className="setting-description">{description}</span>
            </span>
        </label>
    );
    return (
        <div className="modal-overlay" role="presentation">
            <div className="modal remote-pc-dialog" role="dialog" aria-modal="true" aria-label={t('public_address.title', { name })}>
                <h3 className="remote-pc-dialog-title">{t('public_address.title', { name })}</h3>
                {option(
                    false,
                    t('public_address.auto_title'),
                    current.publicIp ? t('public_address.auto_desc_ip', { ip: current.publicIp }) : t('public_address.auto_desc'),
                )}
                {option(true, t('public_address.custom_title'), t('public_address.custom_desc'))}
                {draft.custom && (
                    <input
                        id="public-address-input"
                        type="text"
                        className="setting-input-text"
                        aria-label={t('public_address.custom_title')}
                        placeholder="mc.example.com"
                        value={draft.address}
                        disabled={busy}
                        onChange={(e) => setDraft({ ...draft, address: e.target.value })}
                    />
                )}
                <p className="setting-description">{t('public_address.preview', { example })}</p>
                <div className="setting-item">
                    <label className="setting-label" htmlFor="public-address-share">
                        <span className="setting-title">{t('public_address.share_title')}</span>
                        <span className="setting-description">{t('public_address.share_desc')}</span>
                    </label>
                    <SabaToggle
                        id="public-address-share"
                        role="switch"
                        aria-label={t('public_address.share_title')}
                        checked={draft.shareAddress}
                        disabled={busy}
                        onChange={(shareAddress) => setDraft({ ...draft, shareAddress })}
                    />
                </div>
                {error && (
                    <div className="setting-validation-error" role="alert">
                        <Icon name="alertCircle" size="sm" /> {error}
                    </div>
                )}
                <div className="remote-pc-dialog-actions">
                    <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy}>
                        {t('remote_daemon.cancel')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-primary"
                        disabled={busy || (draft.custom && !draft.address.trim())}
                        onClick={() => onSave({ address: draft.custom ? draft.address.trim() : '', shareAddress: draft.shareAddress })}
                    >
                        {busy ? <SabaSpinner size="xs" /> : null} {t('public_address.save')}
                    </button>
                </div>
            </div>
        </div>
    );
}

/**
 * 내 PC 카드의 "게임 서버 주소" 줄 (`<dl>` 안). `hostId`가 없으면 이 PC, 있으면 그 원격 데몬의 `/api/public-address`.
 * 읽지 못하면(예전 데몬·닿지 않음) 그리지 않는다
 */
export function PublicAddressRow({ hostId, name }: { hostId: string | null; name: string }) {
    const { t } = useTranslation('gui');
    const [current, setCurrent] = useState<ApiBody | null>(null);
    const [editing, setEditing] = useState(false);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const load = useCallback(async () => {
        if (typeof window.api?.publicAddress !== 'function') return;
        const res = await window.api.publicAddress(hostId).catch(() => null);
        setCurrent(failed(res) ? null : res);
    }, [hostId]);
    useEffect(() => {
        load();
    }, [load]);

    if (!current) return null;

    const save = async (patch: { address: string; shareAddress: boolean }) => {
        setBusy(true);
        setError(null);
        try {
            const res = await window.api.publicAddressSet(hostId, patch);
            if (failed(res)) {
                setError(res?.error_code === 'invalid_address' ? t('public_address.err_invalid') : res?.error || t('public_address.err_failed'));
                return;
            }
            setCurrent(res);
            setEditing(false);
        } catch (e) {
            setError((e as Error).message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <>
            <dt>{t('public_address.label')}</dt>
            <dd className="public-address-value">
                {current.address ? (
                    <code>{current.address}</code>
                ) : (
                    <span>
                        {t('public_address.auto')}
                        {current.publicIp && (
                            <>
                                {' · '}
                                <code>{current.publicIp}</code>
                            </>
                        )}
                    </span>
                )}
                {current.shareAddress === false && <span className="public-address-private muted">{t('public_address.share_off')}</span>}
                <button type="button" className="btn btn-ghost btn-sm public-address-change" onClick={() => setEditing(true)}>
                    {t('public_address.change')}
                </button>
            </dd>
            {editing && (
                <PublicAddressDialog
                    name={name}
                    current={current}
                    busy={busy}
                    error={error}
                    onCancel={() => {
                        setEditing(false);
                        setError(null);
                    }}
                    onSave={save}
                />
            )}
        </>
    );
}
