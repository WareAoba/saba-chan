import clsx from 'clsx';
import { createContext, type ReactNode, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

/** 내려가는 움직임 길이 (CSS sabaSavebarDown과 맞춘다) */
const LEAVE_MS = 200;

interface SaveBarProps {
    dirty: boolean;
    saving: boolean;
    /** 값이 틀려 저장할 수 없을 때 */
    blocked?: boolean;
    onSave: () => void;
    onRevert: () => void;
    /** 안내 글 (기본: 저장하지 않은 변경이 있어요) */
    message?: string;
    /** 저장 단추 이름 (기본: 저장) */
    saveLabel?: string;
    /** 위험한 저장: 빨간 단추 */
    danger?: boolean;
    /** 안내 글 옆에 붙일 것 (예: 같은 일을 하는 CLI 줄) */
    extra?: ReactNode;
    className?: string;
}

/**
 * 저장 막대. 바뀐 게 있을 때만 화면 아래에서 올라와 되돌리기·저장을 묻고, 저장하거나 되돌리면 내려간다.
 * 즉시 저장할 수 없는 설정(포트처럼 다 친 뒤에 적용해야 하는 값, 여러 칸을 한 번에 보내야 하는 값)에만 쓴다.
 */
export function SaveBar({
    dirty,
    saving,
    blocked = false,
    onSave,
    onRevert,
    message,
    saveLabel,
    danger,
    extra,
    className,
}: SaveBarProps) {
    const { t } = useTranslation('gui');
    const [leaving, setLeaving] = useState(false);
    const [shown, setShown] = useState(dirty);
    if (dirty && !shown) setShown(true);
    if (dirty && leaving) setLeaving(false);
    if (!dirty && shown && !leaving) setLeaving(true);

    // 내려가는 움직임이 끝나면 그리지 않는다 (움직임을 꺼도 같은 때에)
    useEffect(() => {
        if (!leaving) return;
        const timer = setTimeout(() => {
            setShown(false);
            setLeaving(false);
        }, LEAVE_MS);
        return () => clearTimeout(timer);
    }, [leaving]);

    if (!shown) return null;
    const text = message || t('discord.unsaved');
    return (
        <section className={clsx('saba-savebar', className, { dirty: dirty, leaving })} aria-label={text}>
            <span className="saba-savebar-dot" aria-hidden="true" />
            <span className="saba-savebar-text">{text}</span>
            {extra}
            <span className="saba-savebar-actions">
                <button type="button" className="btn btn-ghost" disabled={saving || !dirty} onClick={onRevert}>
                    <Icon name="undo" size="sm" /> {t('discord.revert')}
                </button>
                <button
                    type="button"
                    className={clsx('btn', danger ? 'btn-danger' : 'btn-primary')}
                    disabled={!dirty || saving || blocked}
                    onClick={onSave}
                >
                    <Icon name="save" size="sm" /> {saveLabel || t('discord.save')}
                </button>
            </span>
        </section>
    );
}

/** 즉시 저장할 수 없는 칸 하나가 알리는 것 */
export interface PendingChange {
    dirty: boolean;
    /** 값이 틀려 저장할 수 없을 때 */
    blocked?: boolean;
    /** false를 돌려주면 저장하지 못한 것이다 (칸이 사라진 뒤에도 막대에 남긴다) */
    save: () => unknown;
    revert: () => void;
}

/** change가 null이면 뺀다. only를 주면 그 항목이 아직 등록돼 있을 때만 뺀다 */
type Register = (id: string, change: PendingChange | null, only?: PendingChange) => void;
const PendingContext = createContext<Register | null>(null);
/** 저장하지 않은 입력 초안: 칸이 사라졌다 다시 그려져도(탭 전환) 남도록 막대 자리가 쥔다 */
interface Drafts {
    values: Record<string, string>;
    set: (id: string, value: string | undefined) => void;
}
const DraftContext = createContext<Drafts | null>(null);

/**
 * 화면 하나(예: 설정)의 저장 막대 자리. 안쪽 칸들이 usePendingChange로 알린 바뀜을 모아
 * 막대 하나로 한꺼번에 저장하거나 되돌린다.
 */
export function SaveBarHost({ children, className }: { children: ReactNode; className?: string }) {
    const [items, setItems] = useState<Record<string, PendingChange>>({});
    const [draftValues, setDraftValues] = useState<Record<string, string>>({});
    const setDraft = useCallback((id: string, value: string | undefined) => {
        setDraftValues((prev) => {
            if (value !== undefined) return { ...prev, [id]: value };
            if (!(id in prev)) return prev;
            const { [id]: _gone, ...rest } = prev;
            return rest;
        });
    }, []);
    const [saving, setSaving] = useState(false);
    const register = useCallback<Register>((id, change, only) => {
        setItems((prev) => {
            if (change) return { ...prev, [id]: change };
            if (!(id in prev) || (only && prev[id] !== only)) return prev;
            const { [id]: _gone, ...rest } = prev;
            return rest;
        });
    }, []);
    const pending = Object.values(items).filter((c) => c.dirty);
    const saveAll = async () => {
        setSaving(true);
        try {
            for (const c of pending) await c.save();
        } finally {
            setSaving(false);
        }
    };
    return (
        <PendingContext.Provider value={register}>
            <DraftContext.Provider value={{ values: draftValues, set: setDraft }}>{children}</DraftContext.Provider>
            <SaveBar
                className={className}
                dirty={pending.length > 0}
                saving={saving}
                blocked={pending.some((c) => c.blocked)}
                onSave={saveAll}
                onRevert={() => {
                    for (const c of pending) c.revert();
                }}
            />
        </PendingContext.Provider>
    );
}

/** 즉시 저장할 수 없는 칸이 바뀜을 가장 가까운 SaveBarHost 막대에 알린다 */
export function usePendingChange(id: string, change: PendingChange) {
    const register = useContext(PendingContext);
    const latest = useRef(change);
    latest.current = change;
    useEffect(() => {
        register?.(id, {
            dirty: change.dirty,
            blocked: change.blocked,
            save: () => latest.current.save(),
            revert: () => latest.current.revert(),
        });
    }, [register, id, change.dirty, change.blocked]);
    // 칸이 사라져도(설정 탭 전환) 저장하지 않은 바뀜은 막대에 남긴다. 저장하거나 되돌리면 그때 뺀다
    useEffect(
        () => () => {
            const last = latest.current;
            if (!register) return;
            if (!last.dirty) return register(id, null);
            // 그 사이 칸이 다시 그려져 새로 등록했으면 그 등록은 두고 이 항목만 뺀다
            const orphan: PendingChange = {
                dirty: true,
                blocked: last.blocked,
                save: async () => {
                    if ((await last.save()) !== false) register(id, null, orphan);
                },
                revert: () => {
                    last.revert();
                    register(id, null, orphan);
                },
            };
            register(id, orphan);
        },
        [register, id],
    );
}

/**
 * 저장 막대로 묻는 입력칸의 초안. 손대지 않은 동안은 저장된 값을 따라가고, 고친 값은 SaveBarHost가 쥐어
 * 칸이 사라졌다 다시 그려져도 남는다. 저장된 값과 같아지면 초안을 지운다.
 */
export function useHostDraft(id: string, saved: string) {
    const host = useContext(DraftContext);
    const [local, setLocal] = useState<string | undefined>(undefined);
    const stored = host ? host.values[id] : local;
    const set = useCallback(
        (value: string) => {
            const next = value === saved ? undefined : value;
            if (host) host.set(id, next);
            else setLocal(next);
        },
        [host, id, saved],
    );
    // 저장해서 저장된 값이 초안과 같아졌으면 초안을 내려놓는다 (그 뒤로는 다시 저장된 값을 따라간다)
    useEffect(() => {
        if (stored !== undefined && stored === saved) {
            if (host) host.set(id, undefined);
            else setLocal(undefined);
        }
    }, [stored, saved, host, id]);
    return [stored ?? saved, set] as const;
}
