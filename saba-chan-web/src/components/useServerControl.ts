import { controlOutcome } from '@gui/utils/serverControl';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { errorText, failed, timedOut } from './ui';
import type { ApiBody, Server, WebApi } from '../types';

/** 켜기·끄기·다시 켜기·지우기. 확인 창과 약관 동의는 GUI 와 같다 */
export function useServerControl(api: WebApi) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const { toast, ask } = useApp.getState();
    const [busy, setBusy] = useState(false);

    const run = async <T>(fn: () => Promise<T>) => {
        setBusy(true);
        try {
            return await fn();
        } finally {
            setBusy(false);
        }
    };

    /**
     * 콘솔이 없어 저장하고 정지하라고 요청할 길이 없으면 데몬은 서버를 죽이지 않고 `graceful_stop_unavailable`로 답한다.
     * 앱과 같은 문구로 저장 없이 정지할지 묻고, 그러겠다면 같은 요청을 `force`로 다시 보낸다
     */
    const orForce = async <T extends ApiBody | null | undefined>(
        server: Server,
        send: (opts: { force?: boolean }) => Promise<T>,
    ): Promise<T | null> => {
        const res = await send({});
        if (controlOutcome(res).kind !== 'forceStop') return res;
        const ok = await ask({
            title: tg('servers.force_stop_title'),
            message: tg('servers.force_stop_message', { name: server.name }),
            confirmLabel: tg('servers.force_stop_confirm'),
            danger: true,
        });
        return ok ? send({ force: true }) : null;
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
            // 이미 켜져 있었다 / 켜는 동안 정지했다: GUI 와 같이 "켰어요"로 알리지 않는다
            if (outcome.kind === 'alreadyRunning') {
                return toast(t('servers.already_running', { name: server.name }), 'info');
            }
            if (outcome.kind === 'cancelled') return toast(t('errors.start_cancelled'), 'info');
            if (timedOut(res)) return toast(t('errors.still_working'), 'info');
            if (failed(res)) return toast(errorText(t, res), 'error');
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
            const res = await orForce(server, (opts) => api.instanceStop(server.id, opts));
            if (res === null) return;
            if (timedOut(res)) return toast(t('errors.still_working'), 'info');
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
            const res = await orForce(server, (opts) => api.instanceRestart(server.id, opts));
            if (res === null) return;
            if (timedOut(res)) return toast(t('errors.still_working'), 'info');
            if (controlOutcome(res).kind === 'stillStopping') return toast(t('errors.still_stopping'), 'error');
            if (failed(res)) return toast(errorText(t, res), 'error');
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
                toast(timedOut(res) ? t('errors.still_working') : errorText(t, res), timedOut(res) ? 'info' : 'error');
                return false;
            }
            toast(t('servers.deleted', { name: server.name }), 'ok');
            return true;
        });

    return { busy, start, stop, restart, remove };
}
