/**
 * 언어별 설명 고르기: `descriptions: { ko, "zh-CN", ... }` → GUI 언어, 없으면 기본 언어("pt-BR" → "pt"),
 * 그래도 없으면 `description`(영어). 데몬 목록은 이미 골라서 주고, 스토어 매니페스트는 여기서 고른다.
 */
export function localizedDescription(item?: { descriptions?: Record<string, string>; description?: string } | null, language?: string | null) {
    return pick(item?.descriptions, language) || item?.description || '';
}

/** 언어별 이름 고르기: 카탈로그 모듈의 `display_names`(모듈 locales `module.display_name`) → `display_name` */
export function localizedName(item?: { display_names?: Record<string, string>; display_name?: string } | null, language?: string | null) {
    return pick(item?.display_names, language) || item?.display_name || '';
}

function pick(map: Record<string, string> | undefined, language?: string | null) {
    if (!map || typeof map !== 'object' || !language) return undefined;
    const base = language.split(/[-_]/)[0];
    return map[language] || map[base] || Object.entries(map).find(([k]) => k.split(/[-_]/)[0] === base)?.[1];
}
