/**
 * 익스텐션 slot 표. 호스트가 그리는 자리의 이름·종류·창은 여기 한 곳에만 둔다
 * (docs/architecture/03-extensions.md §4.2와 같은 표).
 *
 * - surface: 그 자리가 있는 창. 번들은 자기가 선언한 slot이 있는 창에서만 읽는다
 * - kind: inline(그 자리에 그대로) · page(탭·목록 한 줄 + 화면) · button(아이콘 버튼 + 작은 창) · card(선택 카드 + 화면)
 */
import type { ExtensionListItem } from '../types/daemon';

export type SlotSurface = 'main' | 'tray' | 'desktop';
export type SlotKind = 'inline' | 'page' | 'button' | 'card';

export const SLOT_TABLE = {
    'AppBar.status': { surface: 'main', kind: 'inline' },
    'AppBar.actions': { surface: 'main', kind: 'button' },
    'App.page': { surface: 'main', kind: 'page' },
    'Settings.tab': { surface: 'main', kind: 'page' },
    'Discord.nav': { surface: 'main', kind: 'page' },
    'ServerCard.badge': { surface: 'main', kind: 'inline' },
    'ServerCard.headerGauge': { surface: 'main', kind: 'inline' },
    'ServerCard.provision': { surface: 'main', kind: 'inline' },
    'ServerCard.expandedStats': { surface: 'main', kind: 'inline' },
    'ServerDetail.section': { surface: 'main', kind: 'inline' },
    'ServerDetail.tab': { surface: 'main', kind: 'page' },
    'ServerSettings.tab': { surface: 'main', kind: 'inline' },
    'AddServer.options': { surface: 'main', kind: 'inline' },
    'AddServer.method': { surface: 'main', kind: 'card' },
    'TrayPopup.section': { surface: 'tray', kind: 'inline' },
    // 바탕화면 오버레이 창 (셸 overlay.rs): 이 slot을 선언한 익스텐션이 켜져 있으면 셸이 투명 창을 띄운다
    'Desktop.overlay': { surface: 'desktop', kind: 'inline' },
} as const satisfies Record<string, { surface: SlotSurface; kind: SlotKind }>;

export type SlotId = keyof typeof SLOT_TABLE;

export const isKnownSlot = (id: string): id is SlotId => Object.hasOwn(SLOT_TABLE, id);

/** manifest `gui.slots.<slotId>`: 탭·버튼·카드에 보일 이름(익스텐션 i18n 키 또는 글자)·아이콘·설명 */
export interface SlotMeta {
    label?: string;
    icon?: string;
    description?: string;
}

/** manifest `gui.slots`를 slotId → 표시 정보로 읽는다. 옛 모양(slotId → 컴포넌트 이름 문자열)은 빈 표시 정보다 */
export function declaredSlots(ext: Pick<ExtensionListItem, 'gui'>): Record<string, SlotMeta> {
    const raw = (ext.gui?.slots ?? {}) as Record<string, unknown>;
    const out: Record<string, SlotMeta> = {};
    for (const [id, value] of Object.entries(raw)) {
        const meta: SlotMeta = {};
        if (value && typeof value === 'object' && !Array.isArray(value)) {
            for (const key of ['label', 'icon', 'description'] as const) {
                const v = (value as Record<string, unknown>)[key];
                if (typeof v === 'string' && v) meta[key] = v;
            }
        }
        out[id] = meta;
    }
    return out;
}

/**
 * 이 창에서 번들을 읽을 익스텐션인가. 트레이 팝업·바탕화면 오버레이는 그 창의 slot을 선언한 것만 읽는다(작은 창이라 가벼워야 한다).
 * 메인 창은 메인 slot을 선언했거나 아무 slot도 선언하지 않은 것을 읽는다(선언 없는 번들도 예전처럼 읽고 경고만 남긴다)
 */
export function loadsOn(ext: Pick<ExtensionListItem, 'gui'>, surface: SlotSurface): boolean {
    const ids = Object.keys(declaredSlots(ext));
    const here = ids.some((id) => isKnownSlot(id) && SLOT_TABLE[id].surface === surface);
    return surface === 'main' ? here || ids.length === 0 : here;
}

/** 선언과 등록이 어긋난 곳: 모르는 slot 이름, 선언하지 않고 등록한 slot */
export function slotProblems(ext: Pick<ExtensionListItem, 'id' | 'gui'>, registered: string[]): string[] {
    const declared = declaredSlots(ext);
    const problems: string[] = [];
    for (const id of new Set([...Object.keys(declared), ...registered])) {
        if (!isKnownSlot(id)) problems.push(`unknown slot '${id}'`);
        else if (!Object.hasOwn(declared, id)) problems.push(`slot '${id}' is registered but not declared in manifest gui.slots`);
    }
    return problems;
}
