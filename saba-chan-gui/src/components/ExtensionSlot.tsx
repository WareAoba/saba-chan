import type { ComponentType, ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { ErrorBoundary } from 'react-error-boundary';
import { type SlotEntry, useExtensions } from '../contexts/ExtensionContext';
import { useExtensionHost } from '../extensions/host';

// 확장 컴포넌트 하나가 throw하면 그 자리만 비운다 (앱 전체 오류 화면으로 번지지 않게)
// 컴포넌트마다 고정 key: 앞의 컴포넌트가 빠져도 남은 것을 새로 만들지 않게 (배열 순번은 자리가 바뀐다)
// memo·forwardRef·lazy가 돌려주는 컴포넌트는 객체라서 함수와 같이 다룬다
const componentKeys = new WeakMap<object, number>();
let nextComponentKey = 0;
const keyOf = (Comp: object) => {
    if (!componentKeys.has(Comp)) componentKeys.set(Comp, ++nextComponentKey);
    return componentKeys.get(Comp);
};

const logSlotError = (slotId: string) => (error: unknown) => console.warn(`[Extension] slot '${slotId}' component failed:`, error);

interface SlotViewProps {
    entry: SlotEntry;
    /** 오류 기록에 쓸 slot 이름 */
    slotId?: string;
    /** 컴포넌트가 없는 칸에 그릴 것 (설정 탭의 자동 폼) */
    fallback?: ReactNode;
    [prop: string]: unknown;
}

/** slot 한 칸: 그 익스텐션의 host를 붙여 그리고, throw하면 그 칸만 비운다 */
export function SlotView({ entry, slotId = entry.ext, fallback = null, ...props }: SlotViewProps) {
    const host = useExtensionHost(entry.ext);
    const Comp = entry.Component as ComponentType<Record<string, unknown>> | undefined;
    if (!Comp) return <>{fallback}</>;
    return (
        <ErrorBoundary fallback={null} onError={logSlotError(slotId)}>
            <Comp {...props} host={host} />
        </ErrorBoundary>
    );
}

interface ExtensionSlotProps {
    slotId: string;
    [prop: string]: unknown;
}

/** 그 자리에 그대로 그리는 slot (inline): 등록된 칸을 모두 그린다 */
export default function ExtensionSlot({ slotId, ...props }: ExtensionSlotProps) {
    const { slots } = useExtensions();
    const entries = (slots[slotId] || []).filter((e) => e.Component);

    if (entries.length === 0) return null;

    return (
        <>
            {entries.map((entry, i) => {
                const Comp = entry.Component as object;
                return (
                    <SlotView
                        key={`${slotId}-${Comp && (typeof Comp === 'function' || typeof Comp === 'object') ? keyOf(Comp) : `i${i}`}`}
                        entry={entry}
                        slotId={slotId}
                        {...props}
                    />
                );
            })}
        </>
    );
}

export interface SlotPage {
    /** 탭·목록에서 쓰는 id: `ext:<익스텐션 id>` */
    id: string;
    ext: string;
    label: string;
    icon: string;
    description?: string;
    entry: SlotEntry;
}

/** 탭·목록 한 줄이 되는 slot (page·button·card): 익스텐션마다 첫 칸 하나. 이름은 manifest label(익스텐션 번역) → 익스텐션 이름 */
export function useSlotPages(slotId: string): SlotPage[] {
    const { slots, extensions } = useExtensions();
    const { t } = useTranslation();
    const seen = new Set<string>();
    const pages: SlotPage[] = [];
    for (const entry of slots[slotId] || []) {
        if (seen.has(entry.ext)) continue;
        seen.add(entry.ext);
        const ns = `ext_${entry.ext}`;
        const name = extensions.find((e) => e.id === entry.ext)?.name || entry.ext;
        const { label, icon, description } = entry.meta;
        pages.push({
            id: `ext:${entry.ext}`,
            ext: entry.ext,
            label: label ? t(label, { ns, defaultValue: label }) : name,
            icon: icon || 'extension',
            ...(description ? { description: t(description, { ns, defaultValue: description }) } : {}),
            entry,
        });
    }
    return pages;
}
