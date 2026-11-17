import { type ReactNode, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AddonPin, AddonPinResponse, AddonVersionsResponse, ApiBody, ApiResult } from '../../types/daemon';
import { useUpdateStore, watchUpdates } from '../../stores/useUpdateStore';
import { failed } from '../../utils/apiResult';
import { createTranslateError } from '../../utils/helpers';
import { Icon } from '../Icon';
import type { ContextMenuItem } from '../ui/ContextMenu';
import { Dialog } from '../ui/Dialog';
import { SabaSpinner } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { QuestionModal } from './Modals';

// 모듈·익스텐션 버전 고르기 (사바 스토리지 카드 우클릭 메뉴)와 업데이트 단추
//
// 새 버전이 있는지는 데몬의 업데이트 상태(`update.status`)가 알린다. 업데이트 단추는 "자동 업데이트로 돌아가기"와 같이
// 데몬에 가장 새 버전으로 바꾸라고 보낸다 (`DELETE /api/updates/addons/{key}/pin`).
//
// 고를 수 있는 버전, 받기·서명 확인·설치, 켜진 서버 기다리기, 고정과 풀기는 모두 데몬이 한다.
// 여기서는 데몬이 준 목록을 보여 주고 고른 것을 보낸 뒤 답을 알린다.

/** 버전을 고를 카드 하나. key는 릴리스 키(module-<이름>·ext-<id>) */
export interface AddonCard {
    key: string;
    name: string;
    kind: 'module' | 'ext';
    version?: string;
    pinned?: AddonPin;
    pending_version?: string;
}

interface Change {
    card: AddonCard;
    version: string;
    from: string;
    until: string;
}

export type Banner = { tone: 'success' | 'error'; text: string } | null;

/** 탭 위 한 줄 알림 (모듈·익스텐션·사바쨩 버전 고르기가 같이 쓴다) */
export function StatusBanner({ banner }: { banner: Banner }) {
    if (!banner) return null;
    return (
        <div className={`update-modal-status-bar ${banner.tone}`}>
            <Icon name={banner.tone === 'success' ? 'checkCircle' : 'xCircle'} size="sm" /> {banner.text}
        </div>
    );
}

/** 버전 고르기 창의 한 줄 (모듈·익스텐션·사바쨩이 같이 쓴다): 버전과 표시, 그 아래 설명 줄 */
export function VersionRadio({
    version,
    chips,
    hints,
    checked,
    disabled,
    onChoose,
}: {
    version: string;
    chips?: ReactNode;
    hints?: ReactNode;
    checked: boolean;
    disabled?: boolean;
    onChoose: () => void;
}) {
    return (
        // biome-ignore lint/a11y/useSemanticElements: 버전 줄은 버전과 표시를 함께 담는 버튼 모양 칸이다
        <button
            type="button"
            role="radio"
            aria-checked={checked}
            className="radio-card"
            disabled={disabled}
            onClick={onChoose}
        >
            <span className="radio-card-title">
                {version}
                {chips}
            </span>
            {hints}
        </button>
    );
}

/** 데몬이 준 버전 목록을 보여 주고 하나를 고르게 한다 */
function VersionDialog({
    card,
    onPick,
    onClose,
}: {
    card: AddonCard;
    onPick: (change: Change) => void;
    onClose: () => void;
}) {
    const { t } = useTranslation('gui');
    const [res, setRes] = useState<ApiResult<AddonVersionsResponse> | null>(null);
    const [chosen, setChosen] = useState<string | null>(null);

    useEffect(() => {
        let live = true;
        (async () => {
            const body = (await window.api.addonVersions(card.key)) ?? { success: false };
            if (live) setRes(body);
        })();
        return () => {
            live = false;
        };
    }, [card.key]);

    const versions = res && !failed(res) ? res.versions || [] : [];
    const excluded = res && !failed(res) ? res.excluded || [] : [];
    const pick = () => {
        if (!chosen) return;
        const offered = versions.filter((v) => !v.yanked);
        onPick({
            card,
            version: chosen,
            from: res?.installed || card.version || '',
            until: (offered.find((v) => v.latest) || offered[0])?.version || '',
        });
    };

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onClose()}
            size="sm"
            title={t('saba_storage.version_title', { name: card.name, kind: kindWord(t, card) })}
            description={res?.core ? t('saba_storage.version_subtitle', { core: res.core }) : undefined}
            footer={
                <>
                    <button type="button" className="btn btn-ghost" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                    <button type="button" className="btn btn-primary" disabled={!chosen} onClick={pick}>
                        {t('saba_storage.version_install')}
                    </button>
                </>
            }
        >
            {res === null ? (
                <SabaSpinner size="sm" />
            ) : failed(res) ? (
                <p className="ss-version-note">{reasonOf(t, res)}</p>
            ) : (
                <>
                    <div className="ss-versions" role="radiogroup" aria-label={t('saba_storage.version_choose')}>
                        {versions.map((v) => (
                            <VersionRadio
                                key={v.version}
                                version={v.version}
                                checked={chosen === v.version}
                                disabled={v.yanked || v.installed}
                                onChoose={() => setChosen(v.version)}
                                chips={
                                    <>
                                        {v.installed && <StatusChip tone="ok" label={t('saba_storage.version_installed')} />}
                                        {v.latest && <StatusChip tone="info" label={t('saba_storage.version_latest')} />}
                                        {v.yanked && <StatusChip tone="stop" label={t('saba_storage.version_yanked')} />}
                                    </>
                                }
                                hints={v.yanked && <span className="ss-version-hint">{t('saba_storage.version_yanked_hint')}</span>}
                            />
                        ))}
                    </div>
                    {versions.length === 0 && <p className="ss-version-note">{t('saba_storage.version_none')}</p>}
                    {excluded.length > 0 && (
                        <p className="ss-version-note">
                            {t('saba_storage.version_excluded', { count: excluded.length, versions: excluded.join(', ') })}
                        </p>
                    )}
                </>
            )}
        </Dialog>
    );
}

type T = ReturnType<typeof useTranslation>['t'];

const kindWord = (t: T, card: AddonCard) =>
    card.kind === 'module' ? t('saba_storage.version_kind_module') : t('saba_storage.version_kind_ext');

/**
 * 데몬이 준 error_code를 문장으로. 직접 고친 폴더 거절은 모듈·익스텐션이 같은 문장을 쓰고(`module_modified` → `modified`),
 * 모르는 코드는 일반 실패 문장으로. 코드가 없으면(데몬에 닿지 못함) 원문을 번역한다
 */
export const reasonOf = (t: T, res: ApiBody) =>
    res.error_code
        ? t(`saba_storage.version_error.${res.error_code.replace(/^(module|extension)_/, '')}`, {
              defaultValue: t('saba_storage.version_error.other'),
          })
        : createTranslateError(t)(res.error);

/**
 * 모듈 탭·익스텐션 탭이 같이 쓴다. reload는 바꾼 뒤 목록(버전·고정·기다림)을 다시 받는다.
 * 카드에는 menuItems(우클릭 메뉴), chip(새 버전·고정 표시), progress(받는 중·기다리는 중 줄), pinNote(고정 설명),
 * updateButton(업데이트 단추)을 붙이고, 설치됨 머리에 updateAllButton, 탭 위에 banner, 아무 곳에 dialogs를 둔다.
 */
export function useAddonVersions(reload?: () => unknown) {
    const { t } = useTranslation('gui');
    const [picking, setPicking] = useState<AddonCard | null>(null);
    const [confirming, setConfirming] = useState<Change | null>(null);
    const [unpinning, setUnpinning] = useState<AddonCard | null>(null);
    // 받는 중인 카드 → 버전
    const [busy, setBusy] = useState<Record<string, string>>({});
    // 켜진 서버를 기다리는 카드 → 데몬이 알려 준 서버 (목록에는 pending_version만 온다)
    const [waiting, setWaiting] = useState<Record<string, { version: string; servers: string[] }>>({});
    const [banner, setBanner] = useState<Banner>(null);
    const status = useUpdateStore((s) => s.status);

    useEffect(() => watchUpdates(), []);

    const change = useCallback(
        async ({ card, version, from }: Change, send: () => Promise<ApiBody> | undefined, batch = false) => {
            setBusy((prev) => ({ ...prev, [card.key]: version }));
            // 모두 업데이트 중에는 앞 카드의 실패 알림을 뒤 카드가 지우지 않는다
            if (!batch) setBanner(null);
            const res: ApiResult<AddonPinResponse> = (await send()) ?? { success: false };
            const label = { name: card.name, kind: kindWord(t, card) };
            if (failed(res)) {
                setBanner({ tone: 'error', text: t('saba_storage.version_failed', { ...label, version, from, reason: reasonOf(t, res) }) });
            } else if (res.state === 'waiting') {
                const servers = res.waiting_for || [];
                setWaiting((prev) => ({ ...prev, [card.key]: { version: res.version || version, servers } }));
            } else {
                const done: Banner = { tone: 'success', text: t('saba_storage.version_done', { ...label, version: res.version || version }) };
                setBanner((prev) => (batch && prev?.tone === 'error' ? prev : done));
            }
            setBusy((prev) => {
                const { [card.key]: _, ...rest } = prev;
                return rest;
            });
            await reload?.();
            await useUpdateStore.getState().refresh();
        },
        [t, reload],
    );

    /** 데몬이 판정한 업데이트할 새 버전 (고정했거나 바꾸기를 기다리면 없다) */
    const newerVersion = (card: AddonCard): string | null => {
        const comp = (status?.components as ApiBody[] | undefined)?.find((c) => c.component === card.key);
        return comp?.update_version ? String(comp.update_version) : null;
    };
    // 받는 중인 카드도 단추를 내지 않는다
    const updatable = (card: AddonCard) => !!newerVersion(card) && !(card.key in busy);

    const update = (card: AddonCard, batch = false) =>
        change({ card, version: newerVersion(card) || '', from: card.version || '', until: '' }, () => window.api.addonUnpin(card.key), batch);

    /** 카드 오른쪽 업데이트 단추 */
    const updateButton = (card: AddonCard, disabled?: boolean) => {
        if (card.key in busy) {
            return (
                <button type="button" className="btn btn-primary btn-sm ss-update-btn" disabled>
                    {t('saba_storage.updating')}
                </button>
            );
        }
        if (!updatable(card)) return null;
        return (
            <button type="button" className="btn btn-primary btn-sm ss-update-btn" disabled={disabled} onClick={() => update(card)}>
                <Icon name="arrowUp" size="sm" />
                {t('saba_storage.update')}
            </button>
        );
    };

    /** 설치됨 머리의 모두 업데이트. 바꿀 것이 둘 이상일 때만 보이고, 하나씩 차례로 바꾼다 */
    const updateAllButton = (cards: AddonCard[]) => {
        const targets = cards.filter(updatable);
        if (targets.length < 2) return null;
        return (
            <button
                type="button"
                className="btn btn-ghost btn-sm ss-update-btn"
                disabled={Object.keys(busy).length > 0}
                onClick={async () => {
                    setBanner(null);
                    for (const card of targets) await update(card, true);
                }}
            >
                <Icon name="arrowUp" size="sm" />
                {t('saba_storage.update_all')}
                <span className="ss-section-count">{targets.length}</span>
            </button>
        );
    };

    const menuItems = (card: AddonCard, disabled: boolean): ContextMenuItem[] => [
        {
            id: 'version',
            label: t('saba_storage.version_choose'),
            icon: 'branch',
            disabled: disabled || card.key in busy,
            onSelect: () => setPicking(card),
        },
        card.pinned && {
            id: 'unpin',
            label: t('saba_storage.version_unpin'),
            icon: 'undo',
            disabled: disabled || card.key in busy,
            onSelect: () => setUnpinning(card),
        },
    ];

    /** 이름 옆 표시: 새 버전, 고정 */
    const chip = (card: AddonCard) => {
        const latest = newerVersion(card);
        return (
            <>
                {latest && <span className="ss-update-badge">v{latest}</span>}
                {card.pinned && <StatusChip tone="info" label={t('saba_storage.version_pinned')} />}
            </>
        );
    };

    /** 받는 중이거나 켜진 서버를 기다리는 동안 카드 설명 자리에 보인다 */
    const progress = (card: AddonCard): ReactNode => {
        const downloading = busy[card.key];
        if (downloading) {
            return (
                <span className="ss-card-desc ss-card-progress">
                    <SabaSpinner size="xs" /> {t('saba_storage.version_downloading', { version: downloading })}
                </span>
            );
        }
        const pending = card.pending_version;
        if (!pending) return null;
        const servers = waiting[card.key]?.version === pending ? waiting[card.key].servers : [];
        return (
            <span className="ss-card-desc ss-card-progress">
                <Icon name="clock" size="sm" />
                {servers.length > 0
                    ? t('saba_storage.version_waiting', { servers: servers.join(', '), version: pending })
                    : t('saba_storage.version_waiting_any', { version: pending })}
            </span>
        );
    };

    const pinNote = (card: AddonCard) =>
        card.pinned && (
            <span className="ss-card-desc">{t('saba_storage.version_pinned_note', { until: card.pinned.until_newer_than })}</span>
        );

    const bannerEl = <StatusBanner banner={banner} />;

    const dialogs = (
        <>
            {picking && (
                <VersionDialog
                    card={picking}
                    onClose={() => setPicking(null)}
                    onPick={(c) => {
                        setPicking(null);
                        setConfirming(c);
                    }}
                />
            )}
            {confirming && (
                <QuestionModal
                    title={t('saba_storage.version_confirm_title', { version: confirming.version })}
                    message={t('saba_storage.version_confirm_message', {
                        name: confirming.card.name,
                        kind: kindWord(t, confirming.card),
                        from: confirming.from,
                        version: confirming.version,
                        until: confirming.until,
                    })}
                    onCancel={() => setConfirming(null)}
                    buttons={[
                        {
                            label: t('saba_storage.version_confirm_go'),
                            action: () => {
                                setConfirming(null);
                                change(confirming, () => window.api.addonPin(confirming.card.key, confirming.version));
                            },
                        },
                        { label: t('modals.cancel'), action: () => setConfirming(null) },
                    ]}
                />
            )}
            {unpinning?.pinned && (
                <QuestionModal
                    title={t('saba_storage.version_unpin_title')}
                    message={t('saba_storage.version_unpin_message', {
                        name: unpinning.name,
                        kind: kindWord(t, unpinning),
                        version: unpinning.pinned.until_newer_than,
                    })}
                    onCancel={() => setUnpinning(null)}
                    buttons={[
                        {
                            label: t('saba_storage.version_unpin_go'),
                            action: () => {
                                const card = unpinning;
                                const until = card.pinned?.until_newer_than || '';
                                setUnpinning(null);
                                change({ card, version: until, from: card.version || '', until }, () =>
                                    window.api.addonUnpin(card.key),
                                );
                            },
                        },
                        { label: t('modals.cancel'), action: () => setUnpinning(null) },
                    ]}
                />
            )}
        </>
    );

    return { menuItems, chip, progress, pinNote, updateButton, updateAllButton, banner: bannerEl, dialogs };
}
