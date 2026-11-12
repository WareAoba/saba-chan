import type { i18n as I18n, TFunction } from 'i18next';
import { createTranslateError } from '../utils/helpers';

/*
 * 서버 스토어들(목록·동작·설정)이 쓰는 번역. App이 언어가 바뀔 때마다 넣는다 (setServerI18n).
 * 넣기 전에는 기본 문구(없으면 키)를 쓴다.
 */

// biome-ignore lint/suspicious/noExplicitAny: i18next t의 여러 겹친 모양을 그대로 받는다
type Translate = (key: string, options?: any) => string;

let current: { t: Translate; translateError: ReturnType<typeof createTranslateError>; i18n: I18n | null } = {
    t: (key, fallback) => (typeof fallback === 'string' && fallback) || key,
    translateError: (msg) => (msg ? String(msg) : ''),
    i18n: null,
};

export const setServerI18n = (t: TFunction | Translate, i18n: I18n) => {
    current = { t: t as Translate, translateError: createTranslateError(t), i18n };
};

// 화면의 useTranslation t와 같은 모양으로 내보낸다 (helpers의 문구 함수들이 TFunction을 받는다)
export const t = ((key: string, options?: unknown) => current.t(key, options)) as unknown as TFunction;
export const translateError = (message?: unknown) => current.translateError(message);
/** 모듈 locales를 붙일 i18n (넣기 전에는 null) */
export const serverI18n = () => current.i18n;
