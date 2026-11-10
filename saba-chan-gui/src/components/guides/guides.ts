import { create } from 'zustand';
import i18n from '../../i18n';
import { useServerStore } from '../../stores/useServerStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { useUIStore } from '../../stores/useUIStore';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';

/**
 * 처음 사용자 가이드 (사용자 11-08~10). 모든 가이드는 ✕로 닫고, 설정 → 일반의 가이드 목록이나 명령 팔레트에서 다시 연다.
 * 닫은 가이드는 GUI 설정 `closedGuides`에 남는다 (데몬 GuiSettings).
 *
 *   firstRun   서버가 없을 때 서버 화면: 디스코드 연결 → 쓰던 서버 불러오기 / 새로 만들기 (→ 모듈이 없으면 모듈 받기)
 *   checklist  처음 시작하기 체크리스트 (데몬 `GET /api/onboarding`) + 시작 버튼 말풍선
 */
export type GuideId = 'firstRun' | 'checklist';
/** 가이드 화면 안의 단계. modules는 게임 모듈 받기 (불러오기·새로 만들기 어느 쪽이든 모듈이 없으면 먼저) */
export type GuideStep = 'welcome' | 'fork' | 'modules';
export type AddServerMode = 'new' | 'migrate';

/** 설정 → 일반의 가이드 목록·명령 팔레트에 보이는 순서 */
export const GUIDE_ENTRIES = [
    { id: 'firstRun', icon: 'rocket' },
    { id: 'import', icon: 'folder' },
    { id: 'modules', icon: 'package' },
] as const;
export type GuideEntryId = (typeof GUIDE_ENTRIES)[number]['id'];

export interface OnboardingStep {
    id: string;
    done: boolean;
    optional?: boolean;
}
export interface OnboardingState {
    steps: OnboardingStep[];
    next: string | null;
}

interface GuideState {
    /** 지금 보이는 가이드 단계 (null이면 서버가 없을 때만 저절로 welcome·fork) */
    step: GuideStep | null;
    /** 모듈을 받은 뒤 이어 갈 곳 */
    after: AddServerMode | null;
    /** 서버 추가 창을 열 때 바로 갈 흐름 (창이 읽고 비운다) */
    addServerMode: AddServerMode | null;
    onboarding: OnboardingState | null;
    /** 시작 버튼 말풍선을 이번 실행 동안 닫음 */
    coachDismissed: boolean;
    setStep: (step: GuideStep | null, after?: AddServerMode | null) => void;
    takeAddServerMode: () => AddServerMode | null;
    refreshOnboarding: () => Promise<void>;
}

export const useGuideStore = create<GuideState>()((set, get) => ({
    step: null,
    after: null,
    addServerMode: null,
    onboarding: null,
    coachDismissed: false,
    setStep: (step, after = null) => set({ step, after }),
    takeAddServerMode: () => {
        const mode = get().addServerMode;
        if (mode) set({ addServerMode: null });
        return mode;
    },
    refreshOnboarding: async () => {
        if (!window.api?.onboarding) return;
        try {
            const res = (await window.api.onboarding()) as ApiBody;
            set({ onboarding: Array.isArray(res?.steps) ? { steps: res.steps, next: res.next ?? null } : null });
        } catch {
            set({ onboarding: null });
        }
    },
}));


/** ✕: 닫힌 것으로 남기고, 처음 닫을 때 다시 여는 곳을 알린다 */
export function closeGuide(id: GuideId) {
    const closed = useSettingsStore.getState().closedGuides;
    if (closed.includes(id)) return;
    useSettingsStore.getState().update({ closedGuides: [...closed, id] });
    if (id === 'firstRun') useGuideStore.getState().setStep(null);
    if (closed.length === 0) safeShowToast(i18n.t('gui:guides.closed_toast'), 'info', 5000);
}

/** 서버 추가 창을 연다. mode면 고르기 화면을 건너뛰고 그 흐름으로 */
export function openAddServer(mode: AddServerMode | null = null) {
    useGuideStore.setState({ addServerMode: mode });
    useGuideStore.getState().setStep(null);
    useUIStore.getState().setShowModuleManager(true);
}

/** 모듈이 없으면 모듈 받기를 먼저, 있으면 그 흐름의 서버 추가 창을 연다 */
export function startAddServer(mode: AddServerMode, hasModules: boolean) {
    if (hasModules) openAddServer(mode);
    else useGuideStore.getState().setStep('modules', mode);
}

/** 설정·명령 팔레트의 "열기": 닫은 표시를 지우고 서버 화면에 그 가이드를 띄운다 */
export function openGuide(id: GuideEntryId) {
    const closed = useSettingsStore.getState().closedGuides;
    const reopen = id === 'firstRun' ? ['firstRun', 'checklist'] : [];
    if (reopen.some((g) => closed.includes(g))) {
        useSettingsStore.getState().update({ closedGuides: closed.filter((g) => !reopen.includes(g)) });
    }
    useUIStore.getState().setActiveTab('servers');
    if (id === 'import') startAddServer('migrate', useServerStore.getState().modules.length > 0);
    else useGuideStore.getState().setStep(id === 'modules' ? 'modules' : 'welcome');
}
