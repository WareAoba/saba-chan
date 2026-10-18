import { ErrorBoundary } from 'react-error-boundary';
import { useExtensions } from '../contexts/ExtensionContext';

// 확장 컴포넌트 하나가 throw하면 그 자리만 비운다 (앱 전체 오류 화면으로 번지지 않게)
const logSlotError = (slotId) => (error) => console.warn(`[Extension] slot '${slotId}' component failed:`, error);

export default function ExtensionSlot({ slotId, ...props }) {
    const { slots } = useExtensions();
    const components = slots[slotId] || [];

    if (components.length === 0) return null;

    return (
        <>
            {components.map((Comp, i) => (
                <ErrorBoundary key={`${slotId}-${i}`} fallback={null} onError={logSlotError(slotId)}>
                    <Comp {...props} />
                </ErrorBoundary>
            ))}
        </>
    );
}
