import { create } from 'zustand';
import i18n from '../../i18n';
import { useServerStore } from '../../stores/useServerStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { useUIStore } from '../../stores/useUIStore';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import { openScreen } from '../../utils/openScreen';

/**
 * 가이드 (사용자 11-08~11).
 *
 * 기능 가이드는 데몬이 준다 (`GET /api/guides`, 데몬 `guides/<이름>/`): 제목·요약·단계(여는 화면·앱이 할 일·짚을 자리·문구),
 * 완료·new!·보던 단계. 서버 목록 아래 가이드 카드(GuideCard)가 안 끝낸 것을 보이고, 고르면 화면 강조(Spotlight)가 단계를 짚는다.
 *
 * 서버가 없을 때의 처음 시작하기 화면(FirstRunGuide)과 빈 화면 체크리스트는 따로 그린다. ✕로 닫으면 GUI 설정 `closedGuides`에 남는다.
 *   firstRun   서버가 없을 때 서버 화면: 디스코드 연결 → 쓰던 서버 불러오기 / 새로 만들기 (→ 모듈이 없으면 모듈 받기)
 *   checklist  서버가 없는 빈 화면의 처음 시작하기 체크리스트 (데몬 `GET /api/onboarding`)
 */
export type GuideId = 'firstRun' | 'checklist';
/** 가이드 화면 안의 단계. modules는 게임 모듈 받기 (불러오기·새로 만들기 어느 쪽이든 모듈이 없으면 먼저) */
export type GuideStep = 'welcome' | 'fork' | 'modules';
export type AddServerMode = 'new' | 'migrate';

/** 데몬 가이드의 단계 하나 */
export interface GuideStepView {
    id?: string;
    /** 열 화면 주소 (utils/screenAddress). `:first`는 고른 서버(없으면 첫 서버) */
    go?: string;
    /** 앱이 할 일 (GUIDE_ACTIONS) */
    action?: string;
    /** 짚을 자리: 화면 요소의 `data-spot` */
    spot?: string;
    text: string;
}
export interface GuideItem {
    id: string;
    group: string;
    icon: string;
    title: string;
    summary: string;
    steps: GuideStepView[];
    done: boolean;
    new: boolean;
    /** 이어서 볼 단계 (0부터) */
    step: number;
    /** 단계가 체크리스트인 가이드(처음 시작하기)의 진행 */
    progress?: { done: number; total: number };
}

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
    /** 시작 버튼 짚기를 이번 실행 동안 했음 (한 번만) */
    coachDismissed: boolean;
    /** 데몬 가이드 목록 (못 받았으면 null) */
    guides: GuideItem[] | null;
    /** 화면 강조 중인 가이드와 단계 */
    active: { id: string; step: number } | null;
    setStep: (step: GuideStep | null, after?: AddServerMode | null) => void;
    takeAddServerMode: () => AddServerMode | null;
    refreshOnboarding: () => Promise<void>;
    refreshGuides: () => Promise<void>;
}

export const useGuideStore = create<GuideState>()((set, get) => ({
    step: null,
    after: null,
    addServerMode: null,
    onboarding: null,
    coachDismissed: false,
    guides: null,
    active: null,
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
    refreshGuides: async () => {
        if (!window.api?.guides) return;
        try {
            const res = (await window.api.guides(i18n.language)) as ApiBody;
            if (Array.isArray(res?.guides)) set({ guides: res.guides as GuideItem[] });
        } catch {
            // 옛 데몬이거나 잠깐 끊김: 지난 목록을 그대로 둔다
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

/** 소나(Ctrl+K)를 연다 (CommandPalette가 듣는다) */
export const OPEN_SONAR_EVENT = 'saba:open-sonar';

/**
 * 가이드 단계의 `action`: 앱이 하는 일. 화면을 통째로 넘겨받는 일(handoff)이면 그 단계에서 가이드를 마친다
 * (처음 시작하기·서버 추가·모듈 받기 화면, 소나가 그 뒤를 맡는다)
 */
export const GUIDE_ACTIONS: Record<string, { run: () => void; handoff?: boolean }> = {
    modules: {
        run: () => {
            useUIStore.getState().setActiveTab('servers');
            useGuideStore.getState().setStep('modules');
        },
        handoff: true,
    },
    addServer: { run: () => startAddServer('new', useServerStore.getState().modules.length > 0), handoff: true },
    'addServer.migrate': {
        run: () => startAddServer('migrate', useServerStore.getState().modules.length > 0),
        handoff: true,
    },
    sonar: { run: () => window.dispatchEvent(new Event(OPEN_SONAR_EVENT)), handoff: true },
    'page.servers': { run: () => useUIStore.getState().setActiveTab('servers') },
    'page.activity': { run: () => useUIStore.getState().setActiveTab('activity') },
};

/** 단계의 화면 주소에서 `:first`를 고른 서버(없으면 첫 서버)로 바꾼다. 서버가 없으면 null */
export function resolveAddress(go: string): string | null {
    if (!go.includes(':first')) return go;
    const servers = useServerStore.getState().servers;
    const picked = useUIStore.getState().activeServerId;
    const id = servers.find((s) => s.id === picked)?.id ?? servers[0]?.id;
    return id ? go.replace(':first', id) : null;
}

function report(id: string, body: { opened?: boolean; step?: number; finished?: boolean }) {
    return Promise.resolve(window.api?.guideUpdate?.(id, body)).catch(() => {});
}

/** 가이드를 연다: new!를 끄고 이어서 볼 단계부터 화면 강조 */
export function startGuide(id: string) {
    const guide = useGuideStore.getState().guides?.find((g) => g.id === id);
    if (!guide) return;
    if (guide.new) {
        useGuideStore.setState((s) => ({ guides: s.guides?.map((g) => (g.id === id ? { ...g, new: false } : g)) ?? null }));
        report(id, { opened: true });
    }
    goToStep(id, guide.step);
}

/** 단계로 간다: 화면을 열고 앱이 할 일을 한다. 화면을 넘겨받는 일이면 가이드를 마친다 */
export function goToStep(id: string, step: number) {
    const guide = useGuideStore.getState().guides?.find((g) => g.id === id);
    const s = guide?.steps[step];
    if (!guide || !s) return;
    const address = s.go ? resolveAddress(s.go) : null;
    if (address) openScreen(address);
    const action = s.action ? GUIDE_ACTIONS[s.action] : undefined;
    if (action?.handoff) {
        useGuideStore.setState({ active: null });
        finishGuide(id);
        action.run();
        return;
    }
    action?.run();
    useGuideStore.setState({ active: { id, step } });
    if (step !== guide.step) {
        useGuideStore.setState((st) => ({ guides: st.guides?.map((g) => (g.id === id ? { ...g, step } : g)) ?? null }));
        report(id, { step });
    }
}

/** 마지막 단계까지 봤다 (완료 조건이 없는 가이드는 이걸로 끝난다). 다음에 처음부터 보이게 단계를 되돌린다 */
export function finishGuide(id: string) {
    useGuideStore.setState({ active: null });
    report(id, { finished: true, step: 0 }).then(() => useGuideStore.getState().refreshGuides());
}

/** 화면 강조를 닫는다 (바깥 누름·✕·Esc). 보던 단계는 남아 다음에 이어 본다 */
export function closeSpotlight() {
    useGuideStore.setState({ active: null });
}

/** 카드에 보일 안 끝낸 가이드: 진행 중 → 새로 받은 것 → 나머지 (같은 차례면 데몬 순서) */
export function pendingGuides(guides: GuideItem[]): GuideItem[] {
    const rank = (g: GuideItem) => (g.step > 0 || (g.progress?.done ?? 0) > 0 ? 0 : g.new ? 1 : 2);
    return guides
        .map((g, i) => ({ g, i }))
        .filter(({ g }) => !g.done)
        .sort((a, b) => rank(a.g) - rank(b.g) || a.i - b.i)
        .map(({ g }) => g);
}
