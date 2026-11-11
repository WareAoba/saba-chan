import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { Segmented } from '../ui/StatusChip';
import { SabaToggle } from '../ui/SabaUI';

const failed = (res: ApiBody | null | undefined) => !res || res.success === false || Boolean(res.error_code);

/** 고칠 값: 공개 주소(빈 값이면 공인 IP 그대로)와 디스코드 `/<서버> 주소`로 알려 주기 */
export interface AddressDraft {
    custom: boolean;
    address: string;
    shareAddress: boolean;
}

export const addressDraft = (current: ApiBody): AddressDraft => ({
    custom: Boolean(current.address),
    address: current.address || '',
    shareAddress: current.shareAddress !== false,
});

/** 저장할 값. 직접 정하기인데 비어 있으면 null (저장할 수 없다) */
export const addressPatch = (draft: AddressDraft) =>
    draft.custom && !draft.address.trim() ? null : { address: draft.custom ? draft.address.trim() : '', shareAddress: draft.shareAddress };

/**
 * 그 데몬의 `/api/public-address`. `hostId`가 없으면 이 PC.
 * 읽지 못하면(예전 데몬·닿지 않음) current는 null
 */
export function usePublicAddress(hostId: string | null, enabled = true) {
    const { t } = useTranslation('gui');
    const [current, setCurrent] = useState<ApiBody | null>(null);
    const [loaded, setLoaded] = useState(false);
    const load = useCallback(async () => {
        if (!enabled || typeof window.api?.publicAddress !== 'function') {
            setLoaded(true);
            return;
        }
        const res = await window.api.publicAddress(hostId).catch(() => null);
        setCurrent(failed(res) ? null : res);
        setLoaded(true);
    }, [hostId, enabled]);
    useEffect(() => {
        load();
    }, [load]);

    /** 저장하고 실패 문구를 돌려준다 (성공이면 null) */
    const save = async (patch: { address: string; shareAddress: boolean }): Promise<string | null> => {
        try {
            const res = await window.api.publicAddressSet(hostId, patch);
            if (failed(res)) {
                return res?.error_code === 'invalid_address' ? t('public_address.err_invalid') : res?.error || t('public_address.err_failed');
            }
            setCurrent(res);
            return null;
        } catch (e) {
            return (e as Error).message;
        }
    };
    return { current, loaded, save };
}

/** PC 설정 창의 게임 서버 주소 절: 공인 IP 그대로 / 직접 정하기, 디스코드에 알려 주기 */
export function PublicAddressFields({
    current,
    draft,
    busy,
    onChange,
}: {
    current: ApiBody;
    draft: AddressDraft;
    busy: boolean;
    onChange: (draft: AddressDraft) => void;
}) {
    const { t } = useTranslation('gui');
    const example = `${draft.custom && draft.address.trim() ? draft.address.trim() : current.publicIp || 'mc.example.com'}:25565`;
    return (
        <>
            <Segmented
                ariaLabel={t('public_address.label')}
                value={draft.custom ? 'custom' : 'auto'}
                onChange={(id) => !busy && onChange({ ...draft, custom: id === 'custom' })}
                items={[
                    { id: 'auto', label: t('public_address.auto_title') },
                    { id: 'custom', label: t('public_address.custom_title') },
                ]}
            />
            <p className="setting-description">
                {draft.custom
                    ? t('public_address.custom_desc')
                    : current.publicIp
                      ? t('public_address.auto_desc_ip', { ip: current.publicIp })
                      : t('public_address.auto_desc')}
            </p>
            {draft.custom && (
                <input
                    id="public-address-input"
                    type="text"
                    className="input setting-field"
                    aria-label={t('public_address.custom_title')}
                    placeholder="mc.example.com"
                    value={draft.address}
                    disabled={busy}
                    onChange={(e) => onChange({ ...draft, address: e.target.value })}
                />
            )}
            <p className="setting-description">{t('public_address.preview', { example })}</p>
            <div className="pc-settings-row">
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
                    onChange={(shareAddress) => onChange({ ...draft, shareAddress })}
                />
            </div>
        </>
    );
}

/**
 * 내 PC 카드의 "게임 서버 주소" 줄 (`<dl>` 안). 바꾸기는 카드의 수정 창에서 한다.
 * 읽지 못하면(예전 데몬·닿지 않음) 그리지 않는다
 */
export function PublicAddressRow({ hostId }: { hostId: string | null }) {
    const { t } = useTranslation('gui');
    const { current } = usePublicAddress(hostId);
    if (!current) return null;
    return (
        <>
            <dt>{t('public_address.label')}</dt>
            <dd className="public-address-value">
                {current.address ? (
                    <code>{current.address}</code>
                ) : (
                    <span>{t('public_address.auto')}</span>
                )}
                {current.shareAddress === false && <span className="public-address-private muted">{t('public_address.share_off')}</span>}
            </dd>
        </>
    );
}
