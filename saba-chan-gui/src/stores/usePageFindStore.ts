import { create } from 'zustand';

/**
 * 서버 상세 탭 안 찾기(Ctrl+F)의 열림·찾는 글자.
 * 찾기 창(PageFind)과 원문 키 꼬리표(FindKeyTag)가 함께 본다.
 * 꼬리표는 맞는지 여부만 구독하므로 글자를 칠 때 바뀐 줄만 다시 그린다.
 */
interface PageFindState {
    open: boolean;
    query: string;
    setOpen: (open: boolean) => void;
    setQuery: (query: string) => void;
    /** 창(대화상자) 안의 찾기가 떠 있다: 뒤 화면의 찾기는 쉰다 (서버 추가 창의 상세 설정) */
    modal: boolean;
    setModal: (modal: boolean) => void;
}

export const usePageFindStore = create<PageFindState>()((set) => ({
    open: false,
    query: '',
    setOpen: (open) => set({ open }),
    setQuery: (query) => set({ query }),
    modal: false,
    setModal: (modal) => set({ modal }),
}));
