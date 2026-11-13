import { type ReactNode, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { Dialog } from '../ui/Dialog';
import { Popover } from '../ui/Popover';
import { AppBarPill, PillRing } from './AppBarPill';

/** 업데이트가 진행 중이면 자주, 할 일이 없으면 드물게 읽는다 (이 PC 데몬에 묻는 로컬 요청) */
const ACTIVE_POLL_MS = 5000;
const IDLE_POLL_MS = 60000;
/** '완료'가 뜬 뒤 다른 동작(움직이기·누르기·치기)을 하면 이만큼 지나 알약을 누르지 않아도 지운다 (사용자 11-13) */
const DONE_SEEN_MS = 5000;
/** 팝오버에 먼저 보이는 패치노트 줄 수. 나머지는 "패치노트 전체 보기"로 편다 */
const NOTES_PREVIEW = 3;

type State = 'idle' | 'waiting' | 'applying' | 'done' | 'ready';
const SHOWN: State[] = ['applying', 'done', 'waiting', 'ready'];

interface WaitingFor {
    kind: string;
    instance_id?: string;
    name?: string;
}

/** 데몬 GET /api/updates/status 의 phase (계약 D) */
interface Phase {
    state: State;
    version: string | null;
    since?: string | null;
    waiting_for?: WaitingFor[];
    reason?: string | null;
    notes?: Record<string, string> | null;
}

const LABEL_KEYS: Record<string, string> = {
    applying: 'update_pill.applying',
    done: 'update_pill.done',
    waiting: 'update_pill.waiting',
    ready: 'update_pill.ready',
};

/** 무엇을 기다리는지. 앞의 넷은 데몬 작업이 끝나길, 뒤의 셋은 그 서버가 꺼지길 기다린다 */
const WAIT_KEYS: Record<string, string> = {
    backup: 'update_pill.wait_backup',
    provision: 'update_pill.wait_provision',
    server_update: 'update_pill.wait_server_update',
    module_call: 'update_pill.wait_module_call',
    non_runner_server: 'update_pill.wait_server_stop',
    extension_bridge: 'update_pill.wait_server_stop',
    runner_format: 'update_pill.wait_server_stop',
};

/** 이 언어의 패치노트 → 줄 목록 (글로만 그린다: HTML로 해석하지 않는다) */
export function patchNoteLines(notes: Record<string, string> | null | undefined, lang: string): string[] {
    if (!notes) return [];
    const base = lang.split('-')[0];
    const text = notes[lang] ?? notes[base] ?? notes.ko ?? notes.en ?? Object.values(notes)[0] ?? '';
    return String(text)
        .split(/\r?\n/)
        .map((l) => l.replace(/^\s*(?:[-*•]\s*)?/, '').trim())
        .filter((l) => l && !l.startsWith('#'));
}

function readPhase(res: ApiBody | null): Phase | null {
    const p = res?.phase;
    if (!res || res.ok === false || !p || typeof p !== 'object' || typeof p.state !== 'string') return null;
    return p as Phase;
}

const pillIcon = (state: State) => {
    if (state === 'done') return <Icon name="check" size="sm" />;
    if (state === 'waiting') return <Icon name="clock" size="sm" />;
    if (state === 'ready') return <Icon name="arrowUp" size="sm" />;
    return <PillRing />;
};

/**
 * 다시 시작해 {version}으로 업데이트할지 묻는 창. 상단 바 알약 '준비됨'과 사바 스토리지의 사바쨩 버전 고르기가 같이 쓴다.
 * children은 안내 위에 보인다(패치노트)
 */
export function RestartDialog({
    open,
    version,
    onClose,
    onRestart,
    children,
}: {
    open: boolean;
    version: string;
    onClose: () => void;
    onRestart: () => void;
    children?: ReactNode;
}) {
    const { t } = useTranslation('gui');
    return (
        <Dialog
            open={open}
            onOpenChange={(o) => !o && onClose()}
            size="sm"
            className="update-pill-dialog"
            title={t('update_pill.restart_title', { version })}
            footer={
                <>
                    <button type="button" className="btn" onClick={onClose}>
                        {t('update_pill.later')}
                    </button>
                    <button type="button" className="btn btn-primary" onClick={onRestart}>
                        {t('update_pill.restart_now')}
                    </button>
                </>
            }
        >
            {children}
            <p className="update-pill-note">{t('update_pill.restart_note')}</p>
        </Dialog>
    );
}

/**
 * 상단 바 업데이트 알약 (업데이트 알림 시안 v8, 계약 G). 데몬이 묻지 않고 적용하는 사바쨩 업데이트의 네 상태를 보인다:
 * 적용 중·완료·대기 중은 누르면 버전과 패치노트, 준비됨은 다시 시작할지 묻는다.
 * 데몬이 꺼진 동안(적용 중)은 읽기가 실패하므로 마지막 상태를 그대로 둔다.
 * 모듈·익스텐션·번역 자동 교체는 여기 띄우지 않는다 (알림함 기록만).
 */
export function UpdatePill() {
    const { t, i18n } = useTranslation('gui');
    const [phase, setPhase] = useState<Phase | null>(null);
    const [current, setCurrent] = useState<string | null>(null);
    const [open, setOpen] = useState(false);
    const [asking, setAsking] = useState(false);
    const [allNotes, setAllNotes] = useState(false);
    const stateRef = useRef<State | null>(null);
    // 지금 다시 시작을 누른 때(ms): 그 전에 정해진 '준비됨'은 아직 적용 중으로 둔다. 누른 뒤에 정해진 '준비됨'
    // (되돌림·미룸으로 다시 뜬 데몬)이나 데몬이 내려갔다 돌아온 뒤의 답은 받아들인다
    const restarting = useRef<number | null>(null);
    const daemonWentDown = useRef(false);
    // 본 '완료': 데몬에 확인을 보냈고, 팝오버를 닫으면 사라진다
    const acked = useRef(false);
    const [dismissed, setDismissed] = useState(false);
    const anchor = useRef<HTMLSpanElement>(null);
    const running = useServerStore((s) => s.servers.filter((x) => x.status === 'running').length);
    const nameOf = (id?: string) => useServerStore.getState().servers.find((s) => s.id === id)?.name || id || '';

    // 누르기 전에 정해진 '준비됨'이 아직 오는 중인가
    const stillRestarting = (next: Phase) => {
        if (restarting.current === null || daemonWentDown.current) return false;
        const since = next.since ? Date.parse(next.since) : Number.NaN;
        return !(since > restarting.current);
    };

    // biome-ignore lint/correctness/useExhaustiveDependencies: 한 번만 시작하는 읽기 반복이다 (참조는 ref로 본다)
    useEffect(() => {
        if (!window.api?.updaterStatus) return undefined;
        let alive = true;
        let timer: ReturnType<typeof setTimeout> | undefined;
        const poll = async () => {
            let res: ApiBody | null = null;
            try {
                res = await window.api.updaterStatus();
            } catch {
                /* 데몬이 내려가 있다: 마지막 상태를 둔다 */
            }
            if (!alive) return;
            const next = readPhase(res);
            if (!next && restarting.current !== null) daemonWentDown.current = true;
            if (next && !(next.state === 'ready' && stillRestarting(next))) {
                restarting.current = null;
                if (next.state !== 'done') {
                    acked.current = false;
                    setDismissed(false);
                }
                stateRef.current = next.state;
                setPhase(next);
                const core = (Array.isArray(res?.components) ? res.components : []).find(
                    (c: ApiBody) => c?.component === 'saba-core',
                );
                if (core?.current_version) setCurrent(String(core.current_version));
            }
            // 아직 한 번도 못 읽었거나 진행 중이면 자주 본다
            const idle = stateRef.current === 'idle';
            timer = setTimeout(poll, idle ? IDLE_POLL_MS : ACTIVE_POLL_MS);
        };
        poll();
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, []);

    const state: State = phase && !(phase.state === 'done' && dismissed) ? phase.state : 'idle';
    const visible = SHOWN.includes(state);
    // 사라지는 움직임 동안에는 마지막 모양을 그린다
    const lastShown = useRef<State>('applying');
    if (visible) lastShown.current = state;
    const drawn = visible ? state : lastShown.current;

    const close = () => {
        setOpen(false);
        setAllNotes(false);
        if (acked.current && stateRef.current === 'done') setDismissed(true);
    };

    // 완료는 다른 동작을 하면 5초 뒤 지운다: 알약·팝오버 밖에서 움직이거나 누르거나 치면 그때부터 잰다.
    // 알약 위에 올리거나 팝오버를 열어 보는 중이면 지우지 않고 다음 동작을 기다린다
    const openRef = useRef(open);
    openRef.current = open;
    useEffect(() => {
        if (state !== 'done') return undefined;
        let timer: ReturnType<typeof setTimeout> | undefined;
        const onUse = (e: Event) => {
            const target = e.target instanceof Element ? e.target : null;
            if (timer || target?.closest('.appbar-pill, .update-pill-pop')) return;
            timer = setTimeout(() => {
                timer = undefined;
                if (openRef.current || anchor.current?.matches(':hover')) return;
                if (!acked.current) {
                    acked.current = true;
                    Promise.resolve(window.api?.updaterPhaseAck?.()).catch(() => {});
                }
                setDismissed(true);
            }, DONE_SEEN_MS);
        };
        const events = ['pointermove', 'pointerdown', 'keydown', 'wheel'];
        for (const name of events) window.addEventListener(name, onUse, true);
        return () => {
            clearTimeout(timer);
            for (const name of events) window.removeEventListener(name, onUse, true);
        };
    }, [state]);

    // 사라진 뒤에는 열린 것도 닫는다
    useEffect(() => {
        if (!visible) {
            setOpen(false);
            setAsking(false);
        }
    }, [visible]);

    const onPillClick = () => {
        if (state === 'ready') {
            setOpen(false);
            setAsking(true);
            return;
        }
        if (state === 'done' && !acked.current) {
            acked.current = true;
            Promise.resolve(window.api?.updaterPhaseAck?.()).catch(() => {});
        }
        if (open) close();
        else setOpen(true);
    };

    const restartNow = async () => {
        setAsking(false);
        let res: ApiBody | null = null;
        try {
            res = await window.api?.updaterRestart?.();
        } catch (e) {
            res = { ok: false, error: (e as Error)?.message };
        }
        if (res?.ok === false) {
            safeShowToast(t('update_pill.restart_failed', { error: res.error || '-' }), 'error', 6000);
            return;
        }
        restarting.current = Date.now();
        daemonWentDown.current = false;
        stateRef.current = 'applying';
        setPhase((p) => (p ? { ...p, state: 'applying' } : p));
    };

    const version = phase?.version || '';
    const lines = patchNoteLines(phase?.notes, i18n.language || 'ko');
    const shownLines = allNotes ? lines : lines.slice(0, NOTES_PREVIEW);
    const notesList = (items: string[]) =>
        items.length > 0 && (
            <ul>
                {items.map((l, i) => (
                    // biome-ignore lint/suspicious/noArrayIndexKey: 같은 문장이 두 번 나올 수 있다
                    <li key={i}>{l}</li>
                ))}
            </ul>
        );

    let title = '';
    let subs: string[] = [];
    if (state === 'applying') {
        title = current && current !== version ? `${current} → ${version}` : version;
        subs = [t('update_pill.applying_sub')];
    } else if (state === 'done') {
        title = t('update_pill.done_title', { version });
        subs = running > 0 ? [t('update_pill.done_sub', { count: running })] : [];
    } else if (state === 'waiting') {
        title = t('update_pill.waiting_title', { version });
        subs = (phase?.waiting_for || []).map((w) =>
            t(WAIT_KEYS[w.kind] || 'update_pill.wait_other', { name: w.name || nameOf(w.instance_id) }),
        );
    }

    const popover = (
        <Popover
            open={open && state !== 'ready'}
            onOpenChange={(o) => !o && close()}
            anchorRef={anchor}
            label={t(LABEL_KEYS[state] || 'update_pill.applying')}
            className="update-pill-pop"
            autoFocus={false}
        >
            <div>
                <p className="update-pill-version">{title}</p>
                {subs.map((s, i) => (
                    // biome-ignore lint/suspicious/noArrayIndexKey: 같은 문장이 두 번 나올 수 있다
                    <p className="update-pill-sub" key={i}>
                        {s}
                    </p>
                ))}
            </div>
            {notesList(shownLines)}
            {lines.length > NOTES_PREVIEW && !allNotes && (
                <button type="button" className="update-pill-more" onClick={() => setAllNotes(true)}>
                    {t('update_pill.notes_more')}
                </button>
            )}
        </Popover>
    );

    const dialog = (
        <RestartDialog open={asking} version={version} onClose={() => setAsking(false)} onRestart={restartNow}>
            {notesList(lines)}
        </RestartDialog>
    );

    return (
        <>
            <AppBarPill
                visible={visible}
                as="button"
                tone={drawn}
                anchorRef={anchor}
                aria-haspopup="dialog"
                aria-expanded={open || asking}
                onClick={onPillClick}
                icon={pillIcon(drawn)}
                label={t(LABEL_KEYS[drawn])}
                after={popover}
            />
            {dialog}
        </>
    );
}
