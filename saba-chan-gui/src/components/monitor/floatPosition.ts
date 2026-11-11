/**
 * 떠 있는 시스템 모니터의 자리 (사용자 11-11).
 * 자리는 가까운 모서리와 그 모서리 두 변에서의 거리로 기억한다(데몬 GuiSettings.systemMonitorFloat).
 * 창 크기가 바뀌면 그 모서리를 따라가고, 밖으로 밀리면 안쪽 가장자리로 돌아온다.
 * 모서리에서 SNAP 안쪽에 놓으면 그 모서리에 붙는다.
 */

export type MonitorCorner = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
export interface MonitorFloat {
    corner: MonitorCorner;
    x: number;
    y: number;
}
interface Size {
    width: number;
    height: number;
}

/** 가장자리에서 띄우는 간격 */
export const FLOAT_EDGE = 12;
/** 이 거리 안에 놓으면 모서리에 붙는다 */
export const FLOAT_SNAP = 64;
export const DEFAULT_MONITOR_FLOAT: Readonly<MonitorFloat> = Object.freeze({ corner: 'top-right', x: FLOAT_EDGE, y: FLOAT_EDGE });

const CORNERS: readonly MonitorCorner[] = ['top-left', 'top-right', 'bottom-left', 'bottom-right'];
export const isMonitorFloat = (v: unknown): v is MonitorFloat => {
    const f = v as MonitorFloat | null;
    return !!f && CORNERS.includes(f.corner) && Number.isFinite(f.x) && Number.isFinite(f.y) && f.x >= 0 && f.y >= 0;
};

/** 한 축: 시작 변에서의 위치를 안쪽 가장자리 사이로 묶는다. 들어갈 자리가 없으면 0 */
function clampAxis(start: number, room: number) {
    if (room < 2 * FLOAT_EDGE) return 0;
    return Math.round(Math.min(room - FLOAT_EDGE, Math.max(FLOAT_EDGE, start)));
}

/** 기억한 자리 → 판 안의 left·top */
export function placeFloat(pos: MonitorFloat, area: Size, box: Size): { left: number; top: number } {
    const roomX = area.width - box.width;
    const roomY = area.height - box.height;
    const left = pos.corner.endsWith('left') ? pos.x : roomX - pos.x;
    const top = pos.corner.startsWith('top') ? pos.y : roomY - pos.y;
    return { left: clampAxis(left, roomX), top: clampAxis(top, roomY) };
}

/** 끌어 놓은 left·top → 기억할 자리 (가운데가 가까운 모서리 기준, 가까우면 붙는다) */
export function floatFromPoint(left: number, top: number, area: Size, box: Size): MonitorFloat {
    const isLeft = left + box.width / 2 < area.width / 2;
    const isTop = top + box.height / 2 < area.height / 2;
    const corner = `${isTop ? 'top' : 'bottom'}-${isLeft ? 'left' : 'right'}` as MonitorCorner;
    const x = Math.max(0, Math.round(isLeft ? left : area.width - box.width - left));
    const y = Math.max(0, Math.round(isTop ? top : area.height - box.height - top));
    if (x < FLOAT_SNAP && y < FLOAT_SNAP) return { corner, x: FLOAT_EDGE, y: FLOAT_EDGE };
    return { corner, x, y };
}
