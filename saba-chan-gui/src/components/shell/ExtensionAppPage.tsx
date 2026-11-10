import { useEffect } from 'react';
import { useExtensions } from '../../contexts/ExtensionContext';
import { SlotView, useSlotPages } from '../ExtensionSlot';

/** 맨 위 탭으로 연 익스텐션 화면 (`App.page`). 그 익스텐션이 꺼졌으면 서버 화면으로 돌아간다 */
export function ExtensionAppPage({ page, onMissing }: { page: string; onMissing: () => void }) {
    const { loading } = useExtensions();
    const found = useSlotPages('App.page').find((p) => p.id === page);
    useEffect(() => {
        if (!loading && !found) onMissing();
    }, [loading, found, onMissing]);
    if (!found) return null;
    return (
        <div className="page page-scroll page-extension" data-extension={found.ext}>
            <SlotView entry={found.entry} slotId="App.page" />
        </div>
    );
}
