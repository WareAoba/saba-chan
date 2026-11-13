import { renderHook } from '@testing-library/react';
import i18n from 'i18next';
import { type Mock, vi } from 'vitest';
import { setServerConsole } from '../stores/serverActionsState';
import { setServerI18n } from '../stores/serverI18n';
import { useServerSettingsStore } from '../stores/useServerSettingsStore';
import { type Server, useServerStore } from '../stores/useServerStore';
import { type ModalConfig, useUIStore } from '../stores/useUIStore';
import type { ModuleInfo } from '../types/daemon';

/**
 * 서버 동작(useServerStore 액션) 시험 준비: 서버 스토어를 비우고 서버·모듈을 채운 뒤 스토어를 구독한다
 * (`result.current`가 지금 상태와 액션). 창·진행 막대·모듈 관리자·콘솔 열기는 가짜로 바꿔 돌려준다
 * (창은 UI 스토어에도 실제로 띄운다).
 */
export function setupServerStore({
    servers = [],
    modules = [],
    loading = false,
}: { servers?: unknown[]; modules?: unknown[]; loading?: boolean } = {}) {
    useServerStore.getState()._resetForTest();
    setServerI18n(i18n.getFixedT(null, 'gui'), i18n);
    useServerStore.setState({ servers: servers as Server[], modules: modules as ModuleInfo[], loading });
    // 시험이 창을 느슨하게 읽도록 Mock 그대로 (인자 any)
    const setModal: Mock = vi.fn((modal: ModalConfig | null) => useUIStore.setState({ modal }));
    const setProgressBar = vi.fn();
    const setShowModuleManager = vi.fn();
    const openConsole = vi.fn();
    useUIStore.setState({ openModal: setModal, closeModal: () => setModal(null), setProgressBar, setShowModuleManager });
    setServerConsole(openConsole);
    const { result } = renderHook(() => useServerStore());
    return { result, setModal, setProgressBar, setShowModuleManager, openConsole };
}

/**
 * 서버 설정(useServerSettingsStore) 시험 준비: 설정·서버 스토어를 비우고 서버·모듈을 채운 뒤 설정 스토어를 구독한다.
 * 창·진행 막대, 서버 스토어의 목록 받기(fetchServers)·다시 시작(onRestart → restartServer)은 넘긴 가짜를 쓴다
 */
export function setupSettingsStore({
    servers = [],
    modules = [],
    setModal = vi.fn(),
    setProgressBar = vi.fn(),
    fetchServers = vi.fn(),
    onRestart = vi.fn(),
}: {
    servers?: unknown[];
    modules?: unknown[];
    setModal?: Mock | ((modal: ModalConfig | null) => void);
    setProgressBar?: Mock;
    fetchServers?: Mock;
    onRestart?: Mock;
}) {
    useServerSettingsStore.getState()._resetForTest();
    useServerStore.getState()._resetForTest();
    setServerI18n(i18n.getFixedT(null, 'gui'), i18n);
    useServerStore.setState({
        servers: servers as Server[],
        modules: modules as ModuleInfo[],
        fetchServers,
        restartServer: onRestart,
    });
    useUIStore.setState({ openModal: setModal, closeModal: () => setModal(null), setProgressBar });
    return renderHook(() => useServerSettingsStore());
}
