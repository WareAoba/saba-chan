import { useSettingsStore } from '../../../stores/useSettingsStore';
import { TOKENS } from '../../../utils/syntaxHighlight';

/** GUI 설정 일부를 저장한다 (설정 화면 탭들이 같이 쓴다) */
export const setOption = (patch: Parameters<ReturnType<typeof useSettingsStore.getState>['update']>[0]) =>
    useSettingsStore.getState().update(patch);

/** 모든 구문 강조 색을 기본색으로 (빈 값) */
export const clearedTokenColors = () => Object.fromEntries(TOKENS.map((token) => [token, '']));
