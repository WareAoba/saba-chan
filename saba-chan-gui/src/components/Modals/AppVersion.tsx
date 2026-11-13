import type { TFunction } from 'i18next';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUIStore } from '../../stores/useUIStore';
import type { ApiBody, AppVersionsResponse, RevertResult } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { Icon } from '../Icon';
import { useDaemonTarget } from '../shell/DaemonSwitcher';
import { RestartDialog } from '../shell/UpdatePill';
import { Dialog } from '../ui/Dialog';
import { SabaSpinner } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { type Banner, reasonOf, StatusBanner, VersionRadio } from './AddonVersions';
import { QuestionModal } from './Modals';

// ─────────────────────────────────────────────────────────────
// 사바쨩 버전 고르기 (사바 스토리지 컴포넌트 탭, 업데이트 설정 맨 아래 줄)
//
// 고를 수 있는 버전(지금·이 PC에 남겨 둔 직전·기다리는 새 버전)과 고정 여부는 데몬이 정하고,
// 되돌리기·자동 업데이트로 돌아가기는 셸 명령이 데몬에 업데이터를 띄우게 한다(상단 바 알약의 지금 다시 시작과 같은 길).
// 이 PC의 데몬을 볼 때만 보인다: 원격 PC는 데몬이 되돌리기·다시 시작을 막는다.
// ─────────────────────────────────────────────────────────────

type Choice = 'previous' | 'latest';
/** 확인을 받는 일: 직전 버전으로 되돌리기 · 고정을 풀고 가장 새 버전으로 · 기다리는 업데이트 적용 */
type Asking = 'rollback' | 'unhold' | 'update' | null;
/** 되돌리는 중: 업데이터를 띄웠는지, 이 창이 닫히고 다시 열리는지 */
type Reverting = { launched: boolean; reopen: boolean } | null;

/** 되돌리기 결과 한 줄 (알림과 컴포넌트 탭 위 줄이 같이 쓴다) */
export const revertMessage = (r: RevertResult, t: TFunction) =>
    t(r.success ? 'saba_storage.app_version_reverted' : 'saba_storage.app_version_revert_incomplete', {
        from: r.replaced_by,
        version: r.version,
    });

/** 마지막으로 알린 되돌리기 결과의 시각: 같은 결과는 다시 켤 때마다 알리지 않는다 */
const REVERT_SEEN_KEY = 'saba-chan-revert-announced';

/**
 * 시작할 때 읽은 마지막 결과가 사용자 되돌리기면 한 번만 알린다. 데몬은 이 결과를 확인한 것으로 남기므로(acknowledged)
 * 업데이트 결과 알림과 따로 센다. 알린 결과는 이번 실행 동안 컴포넌트 탭 위에도 보인다
 */
export function announceRevert(r: RevertResult, t: TFunction) {
    let seen: string | null = null;
    try {
        seen = localStorage.getItem(REVERT_SEEN_KEY);
    } catch {
        /* 저장소를 못 쓰면 이번만 알린다 */
    }
    if (!r.timestamp || seen === r.timestamp) return;
    try {
        localStorage.setItem(REVERT_SEEN_KEY, r.timestamp);
    } catch {
        /* quota exceeded 등 무시 */
    }
    window.__sabaNotice?.addNotice({
        message: revertMessage(r, t),
        type: r.success ? 'success' : 'error',
        source: 'Updater',
    });
    useUIStore.getState().setRevertResult(r);
}

/** 채널 선택지 문구("베타 (먼저 받기, 불안정할 수 있음)")에서 채널 이름만 */
const channelName = (label: string) => label.split(/\s*[（(]/)[0];

/** 앱 버전 문자열 비교용 (`v0.1.13`·`0.1.13`) */
const bare = (v: string) => v.replace(/^v/, '');

/** 데몬이 준 버전을 새 버전부터 보여 주고 하나를 고르게 한다 */
function AppVersionDialog({
    versions,
    onPick,
    onClose,
}: {
    versions: AppVersionsResponse;
    onPick: (choice: Choice) => void;
    onClose: () => void;
}) {
    const { t, i18n } = useTranslation('gui');
    const [chosen, setChosen] = useState<Choice | null>(null);
    const { installed, kept_previous: kept, latest } = versions;
    const held = !!versions.held_until_newer_than;
    const when = (iso: string) => {
        const d = new Date(iso);
        return Number.isNaN(d.getTime())
            ? iso
            : d.toLocaleString(i18n.language, { month: 'long', day: 'numeric', hour: 'numeric', minute: '2-digit' });
    };
    const hint = (text: string) => <span className="ss-version-hint">{text}</span>;
    const pinnedChip = <StatusChip tone="info" label={t('saba_storage.version_pinned')} />;
    const latestChip = <StatusChip tone="info" label={t('saba_storage.version_latest')} />;

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onClose()}
            size="sm"
            title={t('saba_storage.app_version_title')}
            description={t('saba_storage.app_version_subtitle', {
                channel: channelName(t(`updates.channel_${versions.channel}`, { defaultValue: versions.channel })),
            })}
            footer={
                <>
                    <button type="button" className="btn btn-ghost" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-primary"
                        disabled={!chosen}
                        onClick={() => chosen && onPick(chosen)}
                    >
                        {chosen === 'latest' && held
                            ? t('saba_storage.version_unpin')
                            : t('saba_storage.app_version_go')}
                    </button>
                </>
            }
        >
            {/* 새 버전부터: 기다리는 새 버전 > 지금 > 남겨 둔 직전 */}
            <div className="ss-versions" role="radiogroup" aria-label={t('saba_storage.app_version_choose')}>
                {latest && (
                    <VersionRadio
                        version={latest}
                        checked={chosen === 'latest'}
                        onChoose={() => setChosen('latest')}
                        chips={latestChip}
                        hints={held && hint(t('saba_storage.app_version_latest_held_hint'))}
                    />
                )}
                <VersionRadio
                    version={installed}
                    checked={false}
                    disabled
                    onChoose={() => {}}
                    chips={
                        <>
                            <StatusChip tone="ok" label={t('saba_storage.version_installed')} />
                            {!latest && latestChip}
                            {held && pinnedChip}
                        </>
                    }
                />
                {kept && (
                    <VersionRadio
                        version={kept.version}
                        checked={chosen === 'previous'}
                        onChoose={() => setChosen('previous')}
                        hints={
                            <>
                                {hint(t('saba_storage.app_version_previous'))}
                                {hint(t('saba_storage.app_version_previous_note', { when: when(kept.updated_at) }))}
                            </>
                        }
                    />
                )}
            </div>
            {!kept && !latest && <p className="ss-version-note">{t('saba_storage.app_version_none')}</p>}
            <p className="ss-version-note">
                {t('saba_storage.app_version_older_note', { lowest: kept?.version ?? installed })}
            </p>
        </Dialog>
    );
}

/**
 * 컴포넌트 탭이 쓴다. nameOf는 구성요소 키를 보이는 이름으로 바꾼다(되돌리면 못 쓰게 되는 모듈·익스텐션).
 * 업데이트 설정 맨 아래에 row, 탭 위에 banner(되돌린 결과·되돌리는 중·실패), 아무 곳에 dialogs를 두고,
 * 구성요소 카드의 버전 앞에는 was(되돌리기 전 버전)를 지워 보인다. core는 탭이 읽은 지금 코어 버전이다(바뀌면 줄을 다시 읽는다).
 */
export function useAppVersion(nameOf: (key: string) => string, core?: string) {
    const { t } = useTranslation('gui');
    const target = useDaemonTarget();
    const local = target === null;
    const [versions, setVersions] = useState<AppVersionsResponse | null>(null);
    const [picking, setPicking] = useState(false);
    const [asking, setAsking] = useState<Asking>(null);
    const [reverting, setReverting] = useState<Reverting>(null);
    const [banner, setBanner] = useState<Banner>(null);
    const reverted = useUIStore((s) => s.revertResult);

    const load = useCallback(async () => {
        const res: ApiBody | undefined = await window.api?.updaterVersions?.();
        // 읽지 못했거나 이 길을 모르는 옛 데몬이면 줄을 그리지 않는다
        setVersions(!failed(res) && typeof res?.installed === 'string' ? (res as AppVersionsResponse) : null);
    }, []);

    // 되돌린 뒤(GUI가 그대로 남아 다시 뜬 데몬에 붙었을 때 포함) 다시 읽는다
    // biome-ignore lint/correctness/useExhaustiveDependencies: reverted가 바뀌면 다시 읽는다
    useEffect(() => {
        if (local) load();
    }, [local, load, reverted]);
    useEffect(() => {
        if (reverted) setReverting(null);
    }, [reverted]);
    // 앱이 바뀌면 다시 읽는다 (자동 업데이트로 돌아간 뒤 GUI가 그대로 남았을 때)
    const coreBefore = useRef(core);
    useEffect(() => {
        if (local && coreBefore.current && core && coreBefore.current !== core) load();
        coreBefore.current = core;
    }, [local, load, core]);

    const installed = versions?.installed || '';
    const kept = versions?.kept_previous || null;
    const held = versions?.held_until_newer_than || null;
    const latest = versions?.latest || '';

    const rollBack = async () => {
        const version = kept?.version || '';
        setAsking(null);
        setBanner(null);
        setReverting({ launched: false, reopen: false });
        let res: ApiBody | undefined;
        try {
            res = await window.api?.updaterRollback?.();
        } catch (e) {
            res = { ok: false, error: (e as Error)?.message };
        }
        if (failed(res)) {
            setReverting(null);
            // 코드가 없으면 업데이터를 띄우지 못한 것이다 (설치기 없음·실행 표 실패·셸이 데몬에 닿지 못함)
            const reason = res?.error_code ? reasonOf(t, res) : t('saba_storage.version_error.updater_not_started');
            setBanner({
                tone: 'error',
                text: t('saba_storage.app_version_failed', { version, reason, current: installed }),
            });
            return;
        }
        // GUI가 바뀌면 셸이 곧 이 창을 닫고, 업데이터가 바꾼 사바쨩을 다시 띄운다
        setReverting({ launched: true, reopen: res?.gui_restart !== false });
    };

    // 고정을 풀거나 기다리는 업데이트를 적용한다: 상단 바 알약의 지금 다시 시작과 같다
    const restart = async () => {
        setAsking(null);
        setBanner(null);
        let res: ApiBody | undefined;
        try {
            res = await window.api?.updaterRestart?.();
        } catch (e) {
            res = { ok: false, error: (e as Error)?.message };
        }
        if (res?.ok === false)
            setBanner({ tone: 'error', text: t('update_pill.restart_failed', { error: res.error || '-' }) });
        // 데몬은 적용하기 전에 고정부터 푼다: 실패했거나 받을 것이 없어 데몬이 그대로면 줄을 다시 읽는다
        if (res?.daemon_stopping !== true) await load();
    };

    const pick = (choice: Choice) => {
        setPicking(false);
        setAsking(choice === 'previous' ? 'rollback' : held ? 'unhold' : 'update');
    };

    const row = local && versions && (
        <div className="update-panel-setting-row app-version-row">
            <div className="app-version-text">
                <span className="update-panel-setting-label">
                    <Icon name="branch" size="sm" />
                    {held
                        ? t('saba_storage.app_version_held_label', { current: installed })
                        : t('saba_storage.app_version_label')}
                    {held && <StatusChip tone="info" label={t('saba_storage.version_pinned')} />}
                </span>
                <span className="ss-version-hint">
                    {held
                        ? t('saba_storage.version_pinned_note', { until: held })
                        : kept
                          ? t('saba_storage.app_version_now_kept', { current: installed, previous: kept.version })
                          : t('saba_storage.app_version_now', { current: installed })}
                </span>
            </div>
            <span className="app-version-actions">
                <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    disabled={!!reverting}
                    onClick={() => setPicking(true)}
                >
                    <Icon name="branch" size="sm" />
                    {t('saba_storage.app_version_choose')}
                </button>
                {held && (
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={!!reverting}
                        onClick={() => setAsking('unhold')}
                    >
                        <Icon name="undo" size="sm" />
                        {t('saba_storage.version_unpin')}
                    </button>
                )}
            </span>
        </div>
    );

    const bannerEl = (
        <>
            {reverted && (
                <StatusBanner
                    banner={{ tone: reverted.success ? 'success' : 'error', text: revertMessage(reverted, t) }}
                />
            )}
            {reverting && (
                <>
                    <div className="update-modal-status-bar">
                        <SabaSpinner size="xs" /> {t('saba_storage.app_version_reverting')}
                    </div>
                    {reverting.launched && (
                        <div className="update-modal-status-bar">
                            <Icon name="info" size="sm" /> {t('saba_storage.app_version_reverting_launched')}
                        </div>
                    )}
                    {reverting.reopen && (
                        <p className="ss-version-note app-version-reopen">
                            {t('saba_storage.app_version_reverting_reopen')}
                        </p>
                    )}
                </>
            )}
            <StatusBanner banner={banner} />
        </>
    );

    /** 되돌린 버전인 앱 구성요소: 되돌리기 전 버전 (카드 버전 앞에 지워 보인다) */
    const was = (current: string): ReactNode =>
        reverted &&
        bare(current) === bare(reverted.version) && (
            <>
                <s className="app-version-was">v{bare(reverted.replaced_by)}</s>{' '}
            </>
        );

    const needs = (kept?.needs_newer_core || []).map(nameOf).join(', ');
    const dialogs = (
        <>
            {picking && versions && (
                <AppVersionDialog versions={versions} onPick={pick} onClose={() => setPicking(false)} />
            )}
            {asking === 'rollback' && kept && (
                <QuestionModal
                    title={t('saba_storage.version_confirm_title', { version: kept.version })}
                    message={t('saba_storage.app_version_rollback_message', { version: kept.version })}
                    detail={
                        <>
                            {needs && (
                                <>
                                    {t('saba_storage.app_version_rollback_addons', {
                                        version: kept.version,
                                        list: needs,
                                    })}
                                    <br />
                                </>
                            )}
                            {t('saba_storage.app_version_rollback_hold', {
                                current: kept.hold_until_newer_than || installed,
                            })}
                        </>
                    }
                    onCancel={() => setAsking(null)}
                    buttons={[
                        { label: t('saba_storage.version_confirm_go'), action: rollBack },
                        { label: t('modals.cancel'), action: () => setAsking(null) },
                    ]}
                />
            )}
            {asking === 'unhold' && (
                <QuestionModal
                    title={t('saba_storage.version_unpin_title')}
                    message={
                        latest
                            ? t('saba_storage.app_version_unhold_message', { version: latest })
                            : t('saba_storage.app_version_unhold_only')
                    }
                    detail={t('saba_storage.app_version_unhold_keeps')}
                    onCancel={() => setAsking(null)}
                    buttons={[
                        { label: t('saba_storage.version_unpin_go'), action: restart },
                        { label: t('modals.cancel'), action: () => setAsking(null) },
                    ]}
                />
            )}
            {/* 고정되지 않은 채 기다리는 업데이트: 상단 바 알약의 다시 시작 확인과 같은 문구 */}
            <RestartDialog
                open={asking === 'update'}
                version={latest}
                onClose={() => setAsking(null)}
                onRestart={restart}
            />
        </>
    );

    return { row, banner: bannerEl, dialogs, was };
}
