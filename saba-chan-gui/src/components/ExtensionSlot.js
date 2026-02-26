
import { useExtensions } from '../contexts/ExtensionContext';

export default function ExtensionSlot({ slotId, ...props }) {
    const { slots } = useExtensions();
    const components = slots[slotId] || [];

    if (components.length === 0) return null;

    return (
        <>
            {components.map((Comp, i) => (
                <Comp key={`${slotId}-${i}`} {...props} />
            ))}
        </>
    );
}
