import clsx from 'clsx';
import { type CSSProperties, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useModalClose } from '../../hooks/useModalClose';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { Icon } from '../Icon';
import { type GuideItem, pendingGuides, startGuide, useGuideStore } from './guides';

/** 카드에 펼쳐 보일 안 끝낸 가이드 수 */
const SHOWN = 3;

/** 둘째 줄: 진행 중이면 "N단계 중 M단계", 아니면 요약 */
function subtitle(g: GuideItem, t: (key: string, o?: Record<string, unknown>) => string) {
    if (g.progress && g.progress.done > 0) {
        return t('guides.step_of', { step: Math.min(g.progress.done + 1, g.progress.total), total: g.progress.total });
    }
    if (g.step > 0) return t('guides.step_of', { step: g.step + 1, total: g.steps.length });
    return g.summary;
}

function GuideRow({ guide, onPick, current }: { guide: GuideItem; onPick: () => void; current?: boolean }) {
    const { t } = useTranslation('gui');
    const sub = guide.done ? '' : subtitle(guide, t);
    return (
        <button type="button" className={clsx('guide-row', { current, done: guide.done })} onClick={onPick}>
            <span className="guide-row-icon">
                <Icon name={guide.done ? 'checkCircle' : guide.icon} size="sm" />
            </span>
            <span className="guide-row-text">
                <span className="guide-row-title">
                    {guide.title}
                    {guide.new && <span className="guide-new">{t('guides.new')}</span>}
                </span>
                {sub && <span className="guide-row-sub">{sub}</span>}
            </span>
        </button>
    );
}

/** 더 보기: 끝낸 것까지 모든 가이드를 묶음별로. `from`: 가이드 카드가 창 가운데에서 떨어진 거리 (그 자리에서 커지며 열린다) */
function GuideList({ guides, from, onClose }: { guides: GuideItem[]; from: { x: number; y: number }; onClose: () => void }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);
    // 데몬이 묶음 순서대로 준다
    const groups = [...new Set(guides.map((g) => g.group))];
    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div
                className="modal guide-sheet"
                style={{ '--from-x': `${from.x}px`, '--from-y': `${from.y}px` } as CSSProperties}
                role="dialog"
                aria-label={t('guides.title')}
                onClick={(e) => e.stopPropagation()}
            >
                <div className="guide-sheet-head">
                    <Icon name="lightbulb" size="sm" />
                    <strong>{t('guides.title')}</strong>
                    <button type="button" className="icon-btn" onClick={requestClose} aria-label={t('guides.close')}>
                        <Icon name="close" size="xs" />
                    </button>
                </div>
                <div className="guide-sheet-groups">
                    {groups.map((group) => (
                        <section key={group}>
                            <h3>{t(`guides.group_${group}`)}</h3>
                            {guides
                                .filter((g) => g.group === group)
                                .map((g) => (
                                    <GuideRow
                                        key={g.id}
                                        guide={g}
                                        onPick={() => {
                                            onClose();
                                            startGuide(g.id);
                                        }}
                                    />
                                ))}
                        </section>
                    ))}
                </div>
            </div>
        </div>
    );
}

/**
 * 가이드 카드 (사용자 11-11 시안 v4): 서버 목록 아래 늘 있는 얇은 "가이드" 카드.
 * 누르면 안 끝낸 가이드를 세 개까지 펼치고(진행 중 → new! → 나머지), "더 보기"는 끝낸 것까지 전체 목록.
 */
export function GuideCard() {
    const { t } = useTranslation('gui');
    const guides = useGuideStore((s) => s.guides);
    const [open, setOpen] = useState(false);
    const [all, setAll] = useState<{ x: number; y: number } | null>(null);
    const ref = useRef<HTMLElement>(null);
    // 데몬이 새 가이드를 받으면 다시 읽는다 (받기는 목록을 부를 때 뒤에서 한다)
    useEffect(
        () =>
            subscribeDaemonEvents((ev) => {
                if (ev.type === 'guides.updated') useGuideStore.getState().refreshGuides();
            }),
        [],
    );
    if (!guides?.length) return null;
    const pending = pendingGuides(guides).slice(0, SHOWN);
    // 펼칠 때 완료를 다시 묻는다 (백업·예약처럼 서버 목록이 바뀌지 않는 완료도 보이게)
    const toggle = () => {
        if (!open) useGuideStore.getState().refreshGuides();
        setOpen(!open);
    };
    return (
        <section ref={ref} className={clsx('guide-dock', { open })}>
            <button type="button" className="guide-dock-head" aria-expanded={open} onClick={toggle}>
                <Icon name="lightbulb" size="sm" />
                <span className="guide-dock-label">{t('guides.title')}</span>
                <Icon name="chevronUp" size="xs" />
            </button>
            {/* 접혀도 그려 두고 높이만 접는다: 펼치고 접는 움직임 (사용자 11-13) */}
            <div className="guide-dock-wrap" inert={!open}>
                <div className="guide-dock-body">
                    {pending.map((g) => (
                        <GuideRow
                            key={g.id}
                            guide={g}
                            current={g.step > 0 || (g.progress?.done ?? 0) > 0}
                            onPick={() => startGuide(g.id)}
                        />
                    ))}
                    <button
                        type="button"
                        className="guide-dock-more"
                        onClick={() => {
                            useGuideStore.getState().refreshGuides();
                            const r = ref.current?.getBoundingClientRect();
                            setAll(
                                r
                                    ? { x: r.left + r.width / 2 - window.innerWidth / 2, y: r.top + r.height / 2 - window.innerHeight / 2 }
                                    : { x: 0, y: 0 },
                            );
                        }}
                    >
                        {t('guides.more')}
                    </button>
                </div>
            </div>
            {all && <GuideList guides={guides} from={all} onClose={() => setAll(null)} />}
        </section>
    );
}
