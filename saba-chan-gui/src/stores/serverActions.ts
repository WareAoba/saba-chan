import type { StoreApi } from 'zustand';
import type { Json } from '../types/daemon';
import { serverListActions } from './serverList';
import { serverManageActions } from './serverManage';
import { serverPowerActions } from './serverPower';
import type { Server, ServerState } from './useServerStore';

/** 만들기 요청 본문 (AddServer 모달 → 데몬 POST /api/instances). 예전 호출은 이름 문자열만 넘겼다 */
export interface AddServerPayload {
    name: string;
    module_name: string;
    accept_agreements?: string[];
    preset?: string;
    extension_data?: Record<string, Json>;
    migration_source?: string;
    migration_mode?: string;
    module_settings?: Record<string, unknown>;
    version?: string;
    working_dir?: string;
    managed_start?: boolean;
    port?: number;
    /** 브랜치 서버: 상위 서버 id (모듈·설정은 데몬이 그 서버에서 가져온다) */
    branch_of?: string;
}

/** 서버 동작: 목록 받기, 켜기·끄기·다시 시작, 이름 바꾸기, 만들기, 지우기 (useServerStore의 액션) */
export interface ServerActions {
    fetchServers: () => Promise<void>;
    startServer: (id: string) => Promise<void>;
    stopServer: (id: string) => Promise<void>;
    /** confirmed: 부른 곳에서 이미 재시작을 골랐다 (설정 저장 뒤 '지금 다시 시작') → 다시 묻지 않는다 */
    restartServer: (id: string, options?: { confirmed?: boolean }) => Promise<void>;
    renameServer: (id: string, newName: string) => Promise<boolean>;
    addServer: (payload: AddServerPayload, legacyModuleName?: string) => Promise<boolean>;
    deleteServer: (server: Server) => void;
}

/** useServerStore의 액션: 목록(serverList)·켜고 끄기(serverPower)·이름·만들기·지우기(serverManage)를 묶는다 */
export function serverActions(set: StoreApi<ServerState>['setState'], get: () => ServerState): ServerActions {
    const { fetchServers } = serverListActions(set, get);
    return {
        fetchServers,
        ...serverPowerActions(get, fetchServers),
        ...serverManageActions(get, fetchServers),
    };
}
