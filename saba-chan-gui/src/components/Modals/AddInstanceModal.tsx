import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { SlotView, useSlotPages } from '../ExtensionSlot';
import { Dialog } from '../ui/Dialog';
import { usePageFindStore } from '../../stores/usePageFindStore';
import type { Server } from '../../stores/useServerStore';
import type { ModuleInfo } from '../../types/daemon';
import { AddInstanceNewServer } from './AddInstanceNewServer';
import { AddInstanceMigration } from './AddInstanceMigration';
import { AddServerFrame, AddServerFrameContext } from './AddServerFrame';
import { useGuideStore } from '../guides/guides';
import type { AddServerPayload } from '../../stores/serverActions';
import './AddServer.css';

/**
 * AddInstanceModal — 서버 추가 창 (사용자 11-11 A안).
 *
 * 고르기 화면 없이 게임 목록부터 보인다. 쓰던 서버 가져오기·익스텐션이 더한 방법(`AddServer.method`)은
 * 그 목록의 칸이고, 고르면 같은 창에서 그 화면으로 바뀐다. 창은 공통 Dialog(반투명 모드에서 유리)다.
 *
 * mode: 'new' — 게임 목록·요약·상세 설정 / 'migrate' — 가져오기 / 'ext:<id>' — 익스텐션 화면
 */
interface AddInstanceModalProps {
    isOpen: boolean;
    onClose: () => void;
    extensions: ModuleInfo[];
    servers: Server[];
    onRefreshextensions?: () => void;
    onAddServer: (payload: AddServerPayload) => Promise<unknown>;
}

type Mode = 'new' | 'migrate' | `ext:${string}`;

export function AddInstanceModal({
    isOpen,
    onClose,
    extensions,
    servers,
    onRefreshextensions,
    onAddServer,
}: AddInstanceModalProps) {
    const { t } = useTranslation('gui');
    const [mode, setMode] = useState<Mode>('new');
    const [wide, setWide] = useState(false);
    const [title, setTitle] = useState<HTMLElement | null>(null);
    const [description, setDescription] = useState<HTMLElement | null>(null);
    const [media, setMedia] = useState<HTMLElement | null>(null);
    const [footer, setFooter] = useState<HTMLElement | null>(null);
    const slots = useMemo(() => ({ title, description, media, footer, setWide }), [title, description, media, footer]);

    const extMethods = useSlotPages('AddServer.method');
    const extMethod = mode.startsWith('ext:') ? extMethods.find((p) => p.id === mode) : undefined;
    // 그 화면을 보는 사이 익스텐션이 빠지면 게임 목록으로 (빈 창으로 남지 않게)
    const shownMode: Mode = mode.startsWith('ext:') && !extMethod ? 'new' : mode;

    const handleClose = useCallback(() => {
        setMode('new'); // 다시 열면 게임 목록부터
        onClose();
    }, [onClose]);
    const goBack = useCallback(() => setMode('new'), []);

    // ── 창이 열릴 때 모듈 목록 새로고침 ──
    useEffect(() => {
        if (isOpen && onRefreshextensions) {
            onRefreshextensions();
        }
    }, [isOpen]); // eslint-disable-line react-hooks/exhaustive-deps

    // 가이드에서 "쓰던 서버 불러오기"·"새 서버 만들기"로 열었으면 그 화면으로 바로 연다
    useEffect(() => {
        if (!isOpen) return;
        const requested = useGuideStore.getState().takeAddServerMode();
        if (requested) setMode(requested);
    }, [isOpen]);

    return (
        <Dialog
            open={isOpen}
            onOpenChange={(open) => !open && handleClose()}
            // 상세 설정의 찾기 창이 떠 있으면 Esc는 찾기 창만 닫는다
            onEscapeKeyDown={(e) => {
                const find = usePageFindStore.getState();
                if (!find.open || !find.modal) return;
                e.preventDefault();
                find.setOpen(false);
            }}
            className={wide ? 'add-server wide' : 'add-server'}
            size="lg"
            title={<span ref={setTitle} className="as-slot" />}
            description={<span ref={setDescription} className="as-slot" />}
            media={<span ref={setMedia} className="as-slot" />}
            footer={<div ref={setFooter} className="as-slot" />}
        >
            <AddServerFrameContext.Provider value={slots}>
                {shownMode === 'new' && (
                    <AddInstanceNewServer
                        extensions={extensions}
                        onAddServer={onAddServer}
                        onMigrate={() => setMode('migrate')}
                        extMethods={extMethods}
                        onSelectExt={(id: `ext:${string}`) => setMode(id)}
                        onClose={handleClose}
                    />
                )}

                {/* ── 익스텐션이 더한 방법: 만들기는 그 화면이 데몬 API로 한다 ── */}
                {extMethod && (
                    <AddServerFrame title={extMethod.label} description={extMethod.description}>
                        <SlotView
                            entry={extMethod.entry}
                            slotId="AddServer.method"
                            t={t}
                            onBack={goBack}
                            onClose={handleClose}
                        />
                    </AddServerFrame>
                )}

                {shownMode === 'migrate' && (
                    <AddInstanceMigration
                        extensions={extensions}
                        servers={servers}
                        onAddServer={onAddServer}
                        onBack={goBack}
                    />
                )}
            </AddServerFrameContext.Provider>
        </Dialog>
    );
}
