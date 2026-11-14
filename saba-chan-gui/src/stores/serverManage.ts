import type { ApiBody } from '../types/daemon';
import { safeShowToast } from '../utils/helpers';
import { blockedText, showTrustWarnings } from '../utils/trustNotice';
import type { AddServerPayload } from './serverActions';
import { ui } from './serverActionsState';
import { t, translateError } from './serverI18n';
import type { Server, ServerState } from './useServerStore';
import { setModal } from './useUIStore';

/** 이름 바꾸기, 만들기(설치), 지우기 */
export function serverManageActions(get: () => ServerState, fetchServers: () => Promise<void>) {
    // 되돌리지 못한 설정 사본: 개수와 함께 데몬이 알려준 보관 위치를 보인다
    const unrestoredNote = (r: ApiBody) =>
        r.settings_kept_at
            ? t('server_actions.unlink_restore_failed_kept', { n: r.settings_restore_failed, path: r.settings_kept_at })
            : t('server_actions.unlink_restore_failed', { n: r.settings_restore_failed });

    // ── renameServer: 꺼져 있을 때만 (데몬이 server_running으로 거절) ──
    const renameServer = async (id: string, newName: string) => {
        const name = (newName || '').trim();
        const srv = get().servers.find((s) => s.id === id);
        if (!srv || !name || name === srv.name || !window.api.instanceRename) return false;
        try {
            const result = await window.api.instanceRename(id, name);
            if (result?.success) {
                get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, name: result.name || name } : s)));
                fetchServers();
                return true;
            }
            const code = result?.error_code;
            const msg =
                code === 'name_taken' || code === 'server_running' || code === 'instance_busy'
                    ? t(`servers.rename_${code}`, { name })
                    : code === 'invalid_name'
                      ? t('servers.name_comma')
                      : translateError(result?.error || '');
            safeShowToast(t('servers.rename_failed_toast', { error: msg }), 'error', 4000);
        } catch (error) {
            safeShowToast(t('servers.rename_failed_toast', { error: translateError((error as Error).message) }), 'error', 4000);
        }
        return false;
    };

    // ── addServer ─────────────────────────────────────
    const addServer = async (payload: AddServerPayload, legacyModuleName?: string) => {
        // payload = { name, module_name, accept_agreements?, preset?, extension_data?, migration_source?, migration_mode?, module_settings?, version?, working_dir?, managed_start?, port? }
        const serverName = typeof payload === 'string' ? payload : payload?.name;
        const moduleName = typeof payload === 'string' ? legacyModuleName : payload?.module_name;

        if (!serverName || !serverName.trim()) {
            setModal({
                type: 'failure',
                title: t('servers.add_server_name_empty_title'),
                message: t('servers.add_server_name_empty_message'),
            });
            return false;
        }
        if (!moduleName) {
            setModal({
                type: 'failure',
                title: t('servers.add_module_empty_title'),
                message: t('servers.add_module_empty_message'),
            });
            return false;
        }

        try {
            const instanceData: ApiBody = {
                name: serverName.trim(),
                module_name: moduleName,
                // executable_path는 모듈의 server_executable + working_dir에서 자동 해석됨
            };

            // 만들 때 고른 모듈 설정 (예: 서버 종류)
            if (payload?.module_settings && Object.keys(payload.module_settings).length > 0) {
                instanceData.module_settings = payload.module_settings;
            }

            // 만들 때 동의한 약관 id (module.toml [[agreements]])
            if (payload?.accept_agreements?.length) {
                instanceData.accept_agreements = payload.accept_agreements;
            }

            // 익스텐션이 물은 칸 (AddServer.options → extension_data)
            if (payload?.extension_data) {
                instanceData.extension_data = payload.extension_data;
            }

            // 모듈 추천 프리셋 id (값은 데몬이 푼다)
            if (payload?.preset) {
                instanceData.preset = payload.preset;
            }

            // 마법사에서 고른 버전·폴더·실행 방식·포트 (installFields.createBody가 고른 것만 싣는다)
            for (const key of ['version', 'working_dir', 'managed_start', 'port', 'branch_of'] as const) {
                if (payload?.[key] !== undefined) instanceData[key] = payload[key];
            }

            // 마이그레이션: 기존 서버 디렉토리 직접 연결
            if (payload?.migration_source) {
                instanceData.migration_source = payload.migration_source;
                // 그 자리에 연결(link) / 사바쨩으로 옮기기(move)
                if (payload.migration_mode) {
                    instanceData.migration_mode = payload.migration_mode;
                }
            }

            console.log('Adding instance:', instanceData);
            const result = await window.api.instanceCreate(instanceData);

            // ── 켜진 익스텐션 파일이 바뀌어 확인되지 않음 (다시 설치·되돌리기 안내) ──
            const blocked = blockedText(result, t);
            if (blocked) {
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: blocked });
                return false;
            }

            // ── extension_required 처리 ──
            if (result.error_code === 'extension_required') {
                setModal({
                    type: 'question',
                    title: t('servers.extension_required_title', { defaultValue: 'Extension Required' }),
                    message: result.missing_extensions
                        ? t('servers.extension_required_message_detail', {
                              name: serverName,
                              extensions: result.missing_extensions.join(', '),
                              defaultValue: `Server '${serverName}' requires the following extension(s): ${result.missing_extensions.join(', ')}. Enable them in Settings → Extensions first.`,
                          })
                        : t('servers.extension_required_message', {
                              name: serverName,
                              defaultValue: `Server '${serverName}' requires an extension that is not enabled.`,
                          }),
                    buttons: [
                        {
                            label: t('servers.extension_open_settings', { defaultValue: 'Open Extension Settings' }),
                            action: () => {
                                setModal(null);
                                ui().openSettings('extensions');
                            },
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => setModal(null),
                        },
                    ],
                });
                return false;
            }

            // 데몬은 같은 이름의 서버를 만들지 않는다 (디스코드·CLI는 서버를 이름으로 부른다)
            if (result.error_code === 'name_taken') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('servers.rename_name_taken', { name: serverName }),
                });
                return false;
            }

            // 쉼표가 든 이름 (디스코드에서 부르는 말을 쉼표로 나눈다)
            if (result.error_code === 'invalid_name') {
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: t('servers.name_comma') });
                return false;
            }

            if (result.error_code === 'working_dir_in_use') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('servers.working_dir_in_use', { instance: result.instance || '?' }),
                });
                return false;
            }

            // 가져오기 "옮기기"가 폴더를 옮기지 못함 (다른 드라이브, 켜진 서버·열린 파일 등): 아무것도 만들지 않았다
            if (['move_cross_volume', 'move_in_use', 'move_failed', 'source_missing', 'destination_exists'].includes(result.error_code as string)) {
                const key = ['move_cross_volume', 'move_in_use'].includes(result.error_code as string) ? result.error_code : 'move_failed';
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: t(`servers.${key}`) });
                return false;
            }

            // 고른 설정 값을 사바쨩이 설치하지 못한다 (모듈 `install_options` 밖)
            // 새 서버를 설치할 폴더: 그 사이 파일이 생겼거나 폴더를 만들지 못함 (만들기 마법사의 파일 위치)
            if (['folder_not_empty', 'folder_unavailable'].includes(result.error_code as string)) {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t(`add_server_modal.${result.error_code}`),
                });
                return false;
            }

            if (result.error_code === 'not_installable') {
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: t('servers.not_installable') });
                return false;
            }

            if (result.error_code === 'preset_not_found') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('add_server_modal.preset_not_found'),
                });
                return false;
            }

            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: errorMsg });
            } else {
                showTrustWarnings(result, t);
                if (result.provisioning) {
                    // 프로비저닝 시작 (컨테이너 또는 네이티브 설치) — 모달 닫고 서버 리스트에서 진행률 표시
                    ui().setShowModuleManager(false);
                    fetchServers();
                } else {
                    // 마이그레이션: 모듈이 판정한 서버 종류와 플러그인·모드
                    const detected = result.detected;
                    const components = (detected?.components || []).filter((c: { enabled?: boolean }) => c.enabled !== false);
                    const detectedLine = detected?.server_type
                        ? '\n' +
                          t('server_actions.server_detected', {
                              type: detected.server_type,
                              version: detected.version || '',
                              count: components.length,
                              names: components.length ? ` (${components.map((c: { name: string }) => c.name).join(', ')})` : '',
                          })
                        : '';
                    // 가져온 폴더에서 미리 알릴 것 (데몬 `notices`: 쓸 수 없는 폴더, 밖에서 켜 둔 서버)
                    const noticeLines = (result.notices || [])
                        .map((code: string) => '\n' + t(`server_actions.migrate_notice_${code}`, { defaultValue: '' }))
                        .filter((line: string) => line.trim())
                        .join('');
                    setModal({
                        type: 'success',
                        title: t('command_modal.success'),
                        message: t('server_actions.server_added', { name: serverName }) + detectedLine + noticeLines,
                    });
                    ui().setShowModuleManager(false);
                    fetchServers();
                }
            }
            return !result.error;
        } catch (error) {
            const errorMsg = translateError((error as Error).message);
            setModal({ type: 'failure', title: t('servers.add_error_title'), message: errorMsg });
            return false;
        }
    };

    // ── deleteServer ──────────────────────────────────
    const deleteServer = async (server: Server) => {
        // 가져온 서버(인스턴스 폴더 밖의 서버 폴더): 연결만 끊고 폴더는 남긴다. 보관한 원래 설정으로 되돌릴지 고른다
        if (server.linked_folder) {
            const remove = (keepSettings: boolean) => () => performDeleteServer(server, { keepSettings });
            setModal({
                type: 'question',
                title: t('server_actions.unlink_confirm_title'),
                message: t('server_actions.unlink_confirm_message', { name: server.name }),
                detail: t(
                    server.restorable_settings
                        ? 'server_actions.unlink_detail_restorable'
                        : 'server_actions.unlink_detail',
                    { path: server.linked_folder },
                ),
                buttons: server.restorable_settings
                    ? [
                          { label: t('server_actions.unlink_restore'), action: remove(false) },
                          { label: t('server_actions.unlink_keep_settings'), action: remove(true) },
                          { label: t('modals.cancel'), action: () => setModal(null) },
                      ]
                    : undefined,
                onConfirm: remove(false),
            });
            return;
        }
        // 옮겨 온 서버: 서버 파일은 함께 지워지지만, 서버 폴더 밖(데이터 폴더)에 남는 설정은 되돌릴지 고른다
        if (server.restorable_settings) {
            const remove = (keepSettings: boolean) => () => performDeleteServer(server, { keepSettings });
            setModal({
                type: 'question',
                title: t('server_actions.delete_confirm_title'),
                message: t('server_actions.delete_confirm_message', { name: server.name }),
                detail: t('server_actions.delete_detail_restorable'),
                buttons: [
                    { label: t('server_actions.delete_restore'), action: remove(false) },
                    { label: t('server_actions.delete_keep_settings'), action: remove(true) },
                    { label: t('modals.cancel'), action: () => setModal(null) },
                ],
                onConfirm: remove(false),
            });
            return;
        }
        setModal({
            type: 'question',
            title: t('server_actions.delete_confirm_title'),
            message: t('server_actions.delete_confirm_message', { name: server.name }),
            detail: t('server_actions.delete_detail'),
            onConfirm: () => performDeleteServer(server),
        });
    };

    const performDeleteServer = async (server: Server, options?: { keepSettings?: boolean }) => {
        setModal(null);
        try {
            const result = await window.api.instanceDelete(server.id, options);
            if (result.error_code === 'ETIMEDOUT') {
                // 답이 늦었을 뿐 데몬은 계속 지우고 있을 수 있다: 목록에 남았는지로 판단한다
                const list = await window.api.serverList().catch(() => null);
                const checked = Array.isArray(list?.servers);
                const gone = checked && !list?.servers?.some((s) => s.id === server.id);
                setModal(
                    !checked
                        ? {
                              type: 'failure',
                              title: t('server_actions.delete_confirm_title'),
                              message: t('server_actions.delete_unconfirmed', { name: server.name }),
                          }
                        : gone
                          ? {
                                type: 'success',
                                title: t('command_modal.success'),
                                message: t('server_actions.server_deleted', { name: server.name }),
                            }
                          : {
                                type: 'notification',
                                title: t('server_actions.delete_confirm_title'),
                                message: t('server_actions.delete_still_running', { name: server.name }),
                            },
                );
                fetchServers();
                return;
            }
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.delete_failed_title'), message: errorMsg });
            } else {
                console.log(`Instance "${server.name}" (ID: ${server.id}) deleted`);
                const message = result.linked_folder
                    ? t('server_actions.server_unlinked', { name: server.name, path: result.linked_folder }) +
                      (result.settings_restore_failed ? '\n' + unrestoredNote(result) : '')
                    : t('server_actions.server_deleted', { name: server.name }) +
                      (result.settings_restore_failed ? '\n' + unrestoredNote(result) : '');
                setModal({ type: 'success', title: t('command_modal.success'), message });
                fetchServers();
            }
        } catch (error) {
            const errorMsg = translateError((error as Error).message);
            setModal({ type: 'failure', title: t('servers.delete_error_title'), message: errorMsg });
        }
    };
    return { renameServer, addServer, deleteServer };
}
