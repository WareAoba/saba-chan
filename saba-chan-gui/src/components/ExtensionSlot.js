
import { useExtensions } from '../contexts/ExtensionContext';

const componentIds = new WeakMap();
let nextComponentId = 0;

function componentKey(component) {
    if (!componentIds.has(component)) componentIds.set(component, ++nextComponentId);
    return componentIds.get(component);
}

export default function ExtensionSlot({ slotId, ...props }) {
    const { slots } = useExtensions();
    const components = slots[slotId] || [];

    if (components.length === 0) return null;

    return (
        <>
            {components.map((Comp) => (
                <Comp key={componentKey(Comp)} {...props} />
            ))}
        </>
    );
}
