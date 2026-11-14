import type { TFunction } from 'i18next';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import type { ModalConfig } from '../stores/useUIStore';
import { isManaged } from './serverView';

/** 창 닫기 요청(셸 `onCloseRequest`)에 답한다: 닫기 버튼 설정대로 트레이로·모두 끄기, 아니면 묻는 창 (App.tsx에서 옮김) */
export function answerCloseRequest(t: TFunction, setModal: (modal: ModalConfig | null) => void) {
    // 받았다고 먼저 답한다. 답이 없으면 셸은 렌더러가 멈춘 것으로 보고 다음 닫기에 인터페이스만 끝낸다
    window.api.closeResponse('ack');
    // managed 모드로 실행 중인 인스턴스 검출
    const currentServers = useServerStore.getState().servers;
    const runningManaged = currentServers.filter((srv) => srv.status === 'running' && isManaged(srv));

    const doQuit = () => {
        window.api.closeResponse('quit');
        setModal(null);
    };

    // 모두 끄기: managed 서버가 켜져 있으면 한 번 더 묻는다
    const quitAll = () => {
        if (runningManaged.length > 0) {
            // managed 인스턴스가 실행 중 → 2차 경고
            const names = runningManaged.map((s) => s.name).join(', ');
            setModal({
                type: 'warning',
                title: t('app_exit.managed_warning_title'),
                message: t('app_exit.managed_warning_message', {
                    names,
                    count: runningManaged.length,
                }),
                detail: t('app_exit.managed_warning_detail'),
                buttons: [
                    {
                        label: t('app_exit.quit_all_label'),
                        action: doQuit,
                    },
                    {
                        label: t('modals.cancel'),
                        action: () => {
                            window.api.closeResponse('cancel');
                            setModal(null);
                        },
                    },
                ],
            });
        } else {
            doQuit();
        }
    };

    const showExitModal = () => {
        setModal({
            type: 'question',
            title: t('app_exit.confirm_title'),
            message: t('app_exit.confirm_message'),
            detail: t('app_exit.confirm_detail'),
            buttons: [
                {
                    label: t('app_exit.exit_interface_label'),
                    action: () => {
                        window.api.closeResponse('exit-interface');
                        setModal(null);
                    },
                },
                {
                    label: t('app_exit.quit_all_label'),
                    action: quitAll,
                },
                {
                    label: t('modals.cancel'),
                    action: () => {
                        window.api.closeResponse('cancel');
                        setModal(null);
                    },
                },
            ],
        });
    };

    // 창 닫기 버튼 동작 설정 (사용자 10-27): 매번 묻기 / 트레이로 / 모두 끄기
    const closeAction = useSettingsStore.getState().closeAction;
    if (closeAction === 'tray') window.api.closeResponse('exit-interface');
    else if (closeAction === 'quit') quitAll();
    else showExitModal();
}
