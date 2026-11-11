import { fireEvent, screen } from '@testing-library/react';
import type { createRelay } from '../relay';
import type { Module, Server, WebApi, WebSession } from '../types';

/** 테스트가 쓰는 칸만 채운 가짜를 화면이 받는 타입으로 (나머지 칸은 쓰이지 않는다) */
export const mockApi = (partial: object) => partial as unknown as WebApi;
export const mockSession = (partial: object) => partial as unknown as WebSession;
export const mockRelay = (partial: object) => partial as unknown as ReturnType<typeof createRelay>;
export const asServers = (list: object[]) => list as unknown as Server[];
export const asServer = (server: object) => server as unknown as Server;
export const asModules = (list: object[]) => list as unknown as Module[];

// jsdom에 없는 요소 메서드 (GUI 드롭다운의 Radix Select가 연 목록에서 고른 항목으로 스크롤하고 포인터를 잡는다)
Element.prototype.scrollIntoView ??= () => {};
Element.prototype.hasPointerCapture ??= () => false;
Element.prototype.releasePointerCapture ??= () => {};

/** 지금 보이는 값으로 드롭다운(GUI CustomDropdown)을 찾아 연다 */
export function openDropdown(current: string) {
    const trigger = screen.getAllByRole('combobox').find((c) => c.textContent?.includes(current));
    if (!trigger) throw new Error(`no dropdown showing "${current}"`);
    fireEvent.click(trigger);
}

/** 드롭다운을 열어 이름으로 항목을 고른다 */
export function pickOption(current: string, name: string | RegExp) {
    openDropdown(current);
    fireEvent.click(screen.getByRole('option', { name }));
}
