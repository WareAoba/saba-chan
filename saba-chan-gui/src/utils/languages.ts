/** GUI가 지원하는 언어 (설정 > 언어, 디스코드 봇 언어) */
export const LANGUAGE_OPTIONS = [
    { value: 'en', label: 'English' },
    { value: 'ko', label: '한국어 (Korean)' },
    { value: 'ja', label: '日本語 (Japanese)' },
    { value: 'zh-CN', label: '简体中文 (Simplified Chinese)' },
    { value: 'zh-TW', label: '繁體中文 (Traditional Chinese)' },
    { value: 'es', label: 'Español (Spanish)' },
    { value: 'pt-BR', label: 'Português (Portuguese - Brazil)' },
    { value: 'ru', label: 'Русский (Russian)' },
    { value: 'de', label: 'Deutsch (German)' },
    { value: 'fr', label: 'Français (French)' },
];

/** 지원 언어 코드. Rust `constants::SUPPORTED_LANGUAGES`와 같다 (업데이터 테스트가 대조한다) */
export const SUPPORTED_LANGUAGES = LANGUAGE_OPTIONS.map((o) => o.value);

/**
 * 브라우저·OS 언어 후보 → 지원 언어 (ko-KR → ko, pt → pt-BR, zh-HK → zh-TW). 맞는 것이 없으면 en.
 * 데몬·CLI·셸의 `constants::resolve_locale`과 같은 규칙: 중국어는 스크립트 태그(Hans·Hant)가 지역보다 먼저다
 */
export function pickLanguage(tags: readonly string[]): string {
    for (const raw of tags) {
        const tag = String(raw).trim().split('.')[0].replace(/_/g, '-').toLowerCase();
        const exact = SUPPORTED_LANGUAGES.find((l) => l.toLowerCase() === tag);
        if (exact) return exact;
        const [base, ...subtags] = tag.split('-');
        if (base === 'zh') {
            const traditional = !subtags.includes('hans') && subtags.some((t) => ['hant', 'tw', 'hk', 'mo'].includes(t));
            return traditional ? 'zh-TW' : 'zh-CN';
        }
        const hit = SUPPORTED_LANGUAGES.find((l) => l === base || l.startsWith(`${base}-`));
        if (hit) return hit;
    }
    return 'en';
}
