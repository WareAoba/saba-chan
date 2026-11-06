import i18n from '../i18n';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';
import { parseScreen } from './screenAddress';

/** 가장 가까운 위 화면을 열었을 때 그 이유 */
export function screenFallbackNotice(key: 'server_missing' | 'guild_missing' | 'pick_guild' | 'tab_missing') {
    window.showToast?.(i18n.t(`gui:links.${key}`), 'info', 5000);
}

/**
 * 화면 주소(utils/screenAddress)의 화면을 연다 (사용자 11-06 #19).
 * 없어진 서버를 가리키면 서버 목록을 열고 이유를 알린다.
 * 디스코드 서버·탭은 디스코드 페이지가, 서버 탭은 서버 상세가 확인하고 없으면 같은 식으로 알린다.
 */
export function openScreen(address: string) {
    const screen = parseScreen(address);
    if (!screen) return;
    const ui = useUIStore.getState();
    switch (screen.page) {
        case 'server': {
            if (!useServerStore.getState().servers.some((s) => s.id === screen.id)) {
                ui.setActiveTab('servers');
                screenFallbackNotice('server_missing');
                return;
            }
            // 좁은 창에서도 목록 대신 그 서버 상세를 보인다
            ui.revealServer(screen.id);
            if (screen.tab) ui.showServerTab(screen.id, screen.tab);
            return;
        }
        case 'discord':
            ui.showDiscordView(
                screen.view === 'node'
                    ? { kind: 'node', id: screen.id, tab: screen.tab, sub: screen.sub }
                    : { kind: screen.view },
            );
            return;
        case 'settings':
            ui.openSettings(screen.tab ?? null);
            return;
        case 'activity':
            ui.setActiveTab('activity');
            return;
    }
}
