/**
 * 언어별 설명 고르기: `descriptions: { ko, "zh-CN", ... }` → GUI 언어, 없으면 기본 언어("pt-BR" → "pt"),
 * 그래도 없으면 `description`(영어). 데몬 목록은 이미 골라서 주고, 스토어 매니페스트는 여기서 고른다.
 */
export function localizedDescription(item, language) {
    const map = item?.descriptions;
    if (map && typeof map === 'object' && language) {
        const base = language.split(/[-_]/)[0];
        const hit = map[language] || map[base] || Object.entries(map).find(([k]) => k.split(/[-_]/)[0] === base)?.[1];
        if (hit) return hit;
    }
    return item?.description || '';
}
