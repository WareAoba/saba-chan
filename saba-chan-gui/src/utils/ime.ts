import type { KeyboardEvent } from 'react';

/**
 * IME 조합 중이거나 변환을 확정하는 Enter인지.
 * WebKit(맥·리눅스 웹뷰)은 확정 Enter의 keydown을 조합이 끝난 뒤에 보내 isComposing이 거짓이다. 그때도 keyCode는 229다.
 */
export const isImeKey = (e: KeyboardEvent): boolean => e.nativeEvent.isComposing || e.keyCode === 229;
