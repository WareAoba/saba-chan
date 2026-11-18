import { controlOutcome } from '@gui/utils/serverControl';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { errorText, failed, refusalText, unsureText } from './ui';
import type { ApiBody, Server, WebApi } from '../types';

/** 켜기·끄기·다시 켜기·지우기. 확인 창과 약관 동의는 GUI 와 같다 */
export function useServerControl(api: WebApi) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const { toast, ask } = useApp.getState();
    const [busy, setBusy] = useState(false);

    // 데몬이 Windows 예약 범위를 피해 포트를 옮겨 저장했다 (켜기·다시 켜기 답, 성공·실패 모두): 앱과 같은 문구로 알린다
    const tellMovedPorts = (server: Server, res: ApiBody) => {
        for (const m of Array.isArray(res?.port_moved) ? res.port_moved : []) {
            toast(tg('servers.port_moved', { name: server.name, from: m.from, to: m.to }), 'info');
        }
    };

    const run = async <T>(fn: () => Promise<T>) => {
        setBusy(true);
        try {
            return await fn();
        } finally {
            setBusy(false);
        }
    };

    const start = (server: Server) =>
        run(async () => {
            let res = await api.instanceStart(server.id);
            let outcome = controlOutcome(res);
            // 모듈이 선언한 약관(EULA 등)에 아직 동의하지 않았다: 원문 링크를 보이고 동의하면 다시 켠다
            if (outcome.kind === 'agreements' && outcome.agreements.length) {
                const { agreements } = outcome;
                const ok = await ask({
                    title: t('agreements.title'),
                    message: t('agreements.message', {
                        name: server.name,
                        list: agreements.map((a) => `${a.label || a.id}: ${a.url}`).join('\n'),
                    }),
                    confirmLabel: t('agreements.accept'),
                });
                if (!ok) return;
                const accepted = await api.instanceAgreements(
                    server.id,
                    agreements.map((a) => a.id),
                );
                if (failed(accepted)) return toast(errorText(t, accepted), 'error');
                res = await api.instanceStart(server.id);
                outcome = controlOutcome(res);
            }
            tellMovedPorts(server, res);
            // 이미 켜져 있었다 / 켜는 동안 정지했다: GUI 와 같이 "켰어요"로 알리지 않는다
            if (outcome.kind === 'alreadyRunning') {
                return toast(t('servers.already_running', { name: server.name }), 'info');
            }
            if (outcome.kind === 'cancelled') return toast(t('errors.start_cancelled'), 'info');
            const unsure = unsureText(t, res);
            if (unsure) return toast(unsure, 'info');
            if (failed(res)) return toast(refusalText(t, res), 'error');
            toast(t('servers.started', { name: server.name }), 'ok');
        });

    const stop = (server: Server) =>
        run(async () => {
            const ok = await ask({
                title: t('servers.stop_title', { name: server.name }),
                message: t('servers.stop_message'),
                confirmLabel: t('servers.stop'),
                danger: true,
            });
            if (!ok) return;
            const res = await api.instanceStop(server.id);
            const unsure = unsureText(t, res);
            if (unsure) return toast(unsure, 'info');
            if (failed(res)) return toast(errorText(t, res), 'error');
            toast(t('servers.stopped', { name: server.name }), 'ok');
        });

    const restart = (server: Server) =>
        run(async () => {
            const ok = await ask({
                title: t('servers.restart_title', { name: server.name }),
                message: t('servers.restart_message'),
                confirmLabel: t('servers.restart'),
                danger: true,
            });
            if (!ok) return;
            const res = await api.instanceRestart(server.id);
            const unsure = unsureText(t, res);
            if (unsure) return toast(unsure, 'info');
            tellMovedPorts(server, res);
            const outcome = controlOutcome(res);
            // 그사이 다른 곳에서 정지·시작·삭제했다: 실패가 아니라 그만둔 것 (문구는 앱 errors.restart_cancelled)
            if (outcome.kind === 'cancelled') return toast(refusalText(t, res), 'info');
            if (outcome.kind === 'stillStopping') return toast(t('errors.still_stopping'), 'error');
            if (failed(res)) return toast(refusalText(t, res), 'error');
            toast(t('servers.restarted', { name: server.name }), 'ok');
        });

    const remove = (server: Server) =>
        run(async () => {
            const ok = await ask({
                title: t('servers.delete_title', { name: server.name }),
                message: t('servers.delete_message'),
                confirmLabel: t('servers.delete'),
                danger: true,
            });
            if (!ok) return false;
            const res = await api.instanceDelete(server.id);
            if (failed(res)) {
                const unsure = unsureText(t, res);
                toast(unsure ?? errorText(t, res), unsure ? 'info' : 'error');
                return false;
            }
            toast(t('servers.deleted', { name: server.name }), 'ok');
            return true;
        });

    return { busy, start, stop, restart, remove };
}
