import { createContext, type ReactNode, useContext, useEffect } from 'react';
import { createPortal } from 'react-dom';

/*
 * 서버 추가 창의 화면 틀 (사용자 11-11). 창 껍데기는 공통 Dialog 하나(AddInstanceModal)이고,
 * 화면마다 다른 제목·설명·그림·바닥 단추는 그 자리에 옮겨 그린다. 그래서 화면을 바꿔도 창이 닫혔다 열리지 않는다.
 * 틀이 없으면(화면 하나만 그리는 시험) 제목·본문·바닥을 차례로 그린다.
 */

interface FrameSlots {
    title: HTMLElement | null;
    description: HTMLElement | null;
    media: HTMLElement | null;
    footer: HTMLElement | null;
    /** 넓은 창 (상세 설정) */
    setWide: (wide: boolean) => void;
}

export const AddServerFrameContext = createContext<FrameSlots | null>(null);

interface AddServerFrameProps {
    title: ReactNode;
    description?: ReactNode;
    media?: ReactNode;
    footer?: ReactNode;
    wide?: boolean;
    children?: ReactNode;
}

export function AddServerFrame({ title, description, media, footer, wide = false, children }: AddServerFrameProps) {
    const slots = useContext(AddServerFrameContext);
    const setWide = slots?.setWide;
    useEffect(() => {
        setWide?.(wide);
    }, [setWide, wide]);

    if (!slots) {
        return (
            <>
                <h2>{title}</h2>
                {description && <p>{description}</p>}
                {children}
                {footer}
            </>
        );
    }
    return (
        <>
            {slots.title && createPortal(title, slots.title)}
            {slots.description && description && createPortal(description, slots.description)}
            {slots.media && media && createPortal(media, slots.media)}
            {slots.footer && footer && createPortal(footer, slots.footer)}
            {children}
        </>
    );
}
