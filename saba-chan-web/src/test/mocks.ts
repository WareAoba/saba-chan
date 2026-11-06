import type { createRelay } from '../relay';
import type { Module, Server, WebApi, WebSession } from '../types';

/** 테스트가 쓰는 칸만 채운 가짜를 화면이 받는 타입으로 (나머지 칸은 쓰이지 않는다) */
export const mockApi = (partial: object) => partial as unknown as WebApi;
export const mockSession = (partial: object) => partial as unknown as WebSession;
export const mockRelay = (partial: object) => partial as unknown as ReturnType<typeof createRelay>;
export const asServers = (list: object[]) => list as unknown as Server[];
export const asServer = (server: object) => server as unknown as Server;
export const asModules = (list: object[]) => list as unknown as Module[];
