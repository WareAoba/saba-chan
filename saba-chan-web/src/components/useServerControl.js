import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { errorText, failed } from './ui';

/** 켜기·끄기·다시 켜기·지우기. 확인 창과 약관 동의는 GUI 와 같다 */
export function useServerControl(api) {
    const { t } = useTranslation();
    const { toast, ask } = useApp.getState();
    const [busy, setBusy] = useState(false);

    const run = async (fn) => {
        setBusy(true);
        try {
            return await fn();
        } finally {
            setBusy(false);
        }
    };

    const start = (server) =>
        run(async () => {
            let res = await api.instanceStart(server.id);
            // 모듈이 선언한 약관(EULA 등)에 아직 동의하지 않았다: 원문 링크를 보이고 동의하면 다시 켠다
            if (res?.error_code === 'agreement_required' && Array.isArray(res.agreements) && res.agreements.length) {
                const ok = await ask({
                    title: t('agreements.title'),
                    message: t('agreements.message', {
                        name: server.name,
                        list: res.agreements.map((a) => `${a.label || a.id}: ${a.url}`).join('\n'),
                    }),
                    confirmLabel: t('agreements.accept'),
                });
                if (!ok) return;
                const accepted = await api.instanceAgreements(
                    server.id,
                    res.agreements.map((a) => a.id),
                );
                if (failed(accepted)) return toast(errorText(t, accepted), 'error');
                res = await api.instanceStart(server.id);
            }
            if (failed(res)) return toast(errorText(t, res), 'error');
            toast(t('servers.started', { name: server.name }), 'ok');
        });

    const stop = (server) =>
        run(async () => {
            const ok = await ask({
                title: t('servers.stop_title', { name: server.name }),
                message: t('servers.stop_message'),
                confirmLabel: t('servers.stop'),
                danger: true,
            });
            if (!ok) return;
            const res = await api.instanceStop(server.id, {});
            if (failed(res)) return toast(errorText(t, res), 'error');
            toast(t('servers.stopped', { name: server.name }), 'ok');
        });

    const restart = (server) =>
        run(async () => {
            const ok = await ask({
                title: t('servers.restart_title', { name: server.name }),
                message: t('servers.restart_message'),
                confirmLabel: t('servers.restart'),
                danger: true,
            });
            if (!ok) return;
            const res = await api.instanceRestart(server.id, {});
            if (failed(res)) return toast(errorText(t, res), 'error');
            toast(t('servers.restarted', { name: server.name }), 'ok');
        });

    const remove = (server) =>
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
                toast(errorText(t, res), 'error');
                return false;
            }
            toast(t('servers.deleted', { name: server.name }), 'ok');
            return true;
        });

    return { busy, start, stop, restart, remove };
}
