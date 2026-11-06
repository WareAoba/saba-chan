import type { ComponentType } from 'react';
import { ErrorBoundary } from 'react-error-boundary';
import { useExtensions } from '../contexts/ExtensionContext';

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

interface ExtensionSlotProps {
    slotId: string;
    [prop: string]: unknown;
}

export default function ExtensionSlot({ slotId, ...props }: ExtensionSlotProps) {
    const { slots } = useExtensions();
    const components: ComponentType<Record<string, unknown>>[] = (slots as Record<string, ComponentType<Record<string, unknown>>[]>)[slotId] || [];

    if (components.length === 0) return null;

    return (
        <>
            {components.map((Comp, i) => (
                <ErrorBoundary
                    key={`${slotId}-${Comp && (typeof Comp === 'function' || typeof Comp === 'object') ? keyOf(Comp) : `i${i}`}`}
                    fallback={null}
                    onError={logSlotError(slotId)}
                >
                    <Comp {...props} />
                </ErrorBoundary>
            ))}
        </>
    );
}
