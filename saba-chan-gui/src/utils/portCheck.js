/**
 * 포트포워딩 확인 결과 보기 (GUI·사바쨩 웹이 같이 쓴다). 판정·상태 이름은 데몬 값 그대로 (`port_check::Report`):
 * verdict open | firewall | closed | cgnat | unknown | no_ports | not_checked, 포트 status open | closed | unknown | in_use | firewall
 */

/** 웹사이트의 "친구들이 접속하게 하기" 안내 (영어는 앞에 언어가 없다) */
const SITE = 'https://saba-chan.online';
const SITE_LOCALES = ['ko', 'ja', 'de', 'es', 'fr', 'pt-BR', 'ru', 'zh-CN', 'zh-TW'];

export function portHelpUrl(language) {
    const tag = String(language || '').toLowerCase();
    const lang =
        SITE_LOCALES.find((l) => l.toLowerCase() === tag) || SITE_LOCALES.find((l) => l === tag.split('-')[0]);
    return `${SITE}${lang ? `/${lang}` : ''}/docs/server-management`;
}

/** 닫혔거나 공유기에 공인 주소가 없어 알려야 하는 결과인가 (자동 확인 결과를 상세 위에 띄운다) */
export const portCheckNeedsAttention = (report) => ['closed', 'cgnat', 'firewall'].includes(report?.verdict);

/** 확인한 결과가 있는가 (확인 전·포트 없음은 보일 것이 없다) */
export const portCheckShown = (report) =>
    !!report?.verdict && report.verdict !== 'not_checked' && report.verdict !== 'no_ports';

export const portLabel = (p) => `${p.port}/${String(p.protocol || '').toUpperCase()}`;
