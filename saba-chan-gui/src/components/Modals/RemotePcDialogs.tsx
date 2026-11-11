import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaNumberInput, SabaSpinner } from '../ui/SabaUI';
import { failed } from '../../utils/apiResult';

/** 리눅스 한 줄 설치 (install.sh 맨 위와 같다). SSH가 안 될 때 그 PC에 붙여 넣는다 */
export const INSTALL_ONE_LINER = 'curl -fsSL https://raw.githubusercontent.com/WareAoba/saba-chan/main/install.sh | sh -s -- --pair';

interface RemoveDialogProps {
    name: string;
    /** 이 PC가 연결한(저장한) 원격 데몬인가. 아니면 서버쨩 목록에서 빼기만 고른다 */
    saved: boolean;
    /** 그 PC에서 사바쨩을 지울 수 있는가 (온라인인 리눅스·맥) */
    canUninstall: boolean;
    summary: string;
    busy: boolean;
    onCancel: () => void;
    onConfirm: (choice: { uninstall: boolean; keepSettings: boolean; keepInstances: boolean; removePc: boolean }) => void;
}

/** 내 PC 카드의 지우기: 무엇까지 지울지 한 창에서 고른다 */
export function RemoveDialog({ name, saved, canUninstall, summary, busy, onCancel, onConfirm }: RemoveDialogProps) {
    const { t } = useTranslation('gui');
    const [uninstall, setUninstall] = useState(false);
    const [keepSettings, setKeepSettings] = useState(true);
    const [keepInstances, setKeepInstances] = useState(true);
    const [removePc, setRemovePc] = useState(!saved);
    const nothing = !saved && !removePc;
    return (
        <div className="modal-overlay" role="presentation">
            <div className="modal remote-pc-dialog" role="dialog" aria-modal="true" aria-label={t('remote_daemon.remove_title', { name })}>
                <h3 className="remote-pc-dialog-title">{t('remote_daemon.remove_title', { name })}</h3>
                <p className="setting-description">{summary}</p>
                {saved && (
                    <div className="remote-pc-choice">
                        <SabaCheckbox checked disabled onChange={() => {}}>
                            {t('remote_daemon.remove_forget')}
                        </SabaCheckbox>
                        <span className="setting-description">{t('remote_daemon.remove_forget_desc')}</span>
                    </div>
                )}
                {saved && (
                    <div className="remote-pc-choice">
                        <SabaCheckbox
                            checked={uninstall}
                            disabled={!canUninstall || busy}
                            onChange={setUninstall}
                            title={canUninstall ? undefined : t('remote_daemon.remove_uninstall_unavailable')}
                        >
                            {t('remote_daemon.remove_uninstall')}
                        </SabaCheckbox>
                        <span className="setting-description">
                            {canUninstall ? t('remote_daemon.remove_uninstall_desc') : t('remote_daemon.remove_uninstall_unavailable')}
                        </span>
                        {uninstall && (
                            <div className="remote-pc-subchoice">
                                <SabaCheckbox checked={keepSettings} disabled={busy} onChange={setKeepSettings}>
                                    {t('remote_daemon.remove_keep_settings')}
                                </SabaCheckbox>
                                <SabaCheckbox checked={keepInstances} disabled={busy} onChange={setKeepInstances}>
                                    {t('remote_daemon.remove_keep_instances')}
                                </SabaCheckbox>
                            </div>
                        )}
                    </div>
                )}
                <div className="remote-pc-choice">
                    <SabaCheckbox checked={removePc} disabled={busy} onChange={setRemovePc}>
                        {t('remote_daemon.remove_relay')}
                    </SabaCheckbox>
                    <span className="setting-description">{t('remote_daemon.remove_relay_desc')}</span>
                </div>
                <div className="remote-pc-dialog-actions">
                    <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy}>
                        {t('remote_daemon.cancel')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-danger"
                        disabled={busy || nothing}
                        onClick={() => onConfirm({ uninstall: saved && uninstall, keepSettings, keepInstances, removePc })}
                    >
                        {busy ? <SabaSpinner size="xs" /> : <Icon name="trash" size="sm" />} {t('remote_daemon.remove_go')}
                    </button>
                </div>
            </div>
        </div>
    );
}

const POLL_MS = 700;
const PROMPT_LABELS: Record<string, string> = {
    password: 'remote_daemon.prompt_password',
    sudo: 'remote_daemon.prompt_sudo',
    passphrase: 'remote_daemon.prompt_passphrase',
};

interface InstallDialogProps {
    onClose: () => void;
    /** 새 PC를 붙였다 (목록을 다시 읽는다) */
    onPaired: () => void;
    errorText: (res: ApiBody) => string;
}

/**
 * 리눅스에 설치: 이 PC의 데몬이 ssh로 그 PC에 붙어 설치하고 짝을 맺는다 (`/api/remote/install`).
 * 처음 접속 확인·비밀번호·sudo 물음은 카드로 묻고 답은 그 물음에만 보낸다. SSH가 안 되면 한 줄 설치 뒤 코드를 넣는다
 */
export function InstallDialog({ onClose, onPaired, errorText }: InstallDialogProps) {
    const { t } = useTranslation('gui');
    const [form, setForm] = useState({ host: '', user: '', port: '22', linger: true, steamLibs: false });
    const [id, setId] = useState<string | null>(null);
    const [view, setView] = useState<ApiBody | null>(null);
    const [output, setOutput] = useState('');
    const [answer, setAnswer] = useState('');
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const [fallback, setFallback] = useState(false);
    const [code, setCode] = useState('');
    const [copied, setCopied] = useState(false);
    const offset = useRef(0);
    const paired = useRef(false);
    const consoleRef = useRef<HTMLPreElement>(null);

    const phase = String(view?.phase || '');
    const running = Boolean(id) && !['done', 'failed', 'cancelled'].includes(phase);

    useEffect(() => {
        if (!id) return;
        let stop = false;
        const tick = async () => {
            const res = await window.api.remoteInstall(id, offset.current);
            if (stop || failed(res)) return;
            offset.current = Number(res.offset ?? offset.current);
            if (res.output) setOutput((o) => o + res.output);
            setView(res);
            if (res.phase === 'done' && !paired.current) {
                paired.current = true;
                onPaired();
            }
            if (['done', 'failed', 'cancelled'].includes(res.phase)) return;
            timer = setTimeout(tick, POLL_MS);
        };
        let timer = setTimeout(tick, 0);
        return () => {
            stop = true;
            clearTimeout(timer);
        };
    }, [id, onPaired]);

    useEffect(() => {
        const el = consoleRef.current;
        if (el && typeof el.scrollTo === 'function') el.scrollTo({ top: el.scrollHeight });
    }, [output]);

    const start = async () => {
        const port = Number(form.port);
        if (!form.host.trim() || !form.user.trim() || !Number.isInteger(port) || port < 1 || port > 65535) {
            setError(t('remote_daemon.install_need_fields'));
            return;
        }
        setBusy(true);
        setError(null);
        const res = await window.api.remoteInstallStart({
            host: form.host.trim(),
            user: form.user.trim(),
            port,
            linger: form.linger,
            steamLibs: form.steamLibs,
        });
        setBusy(false);
        if (failed(res) || !res.id) {
            setError(errorText(res));
            if (res?.error_code === 'ssh_missing') setFallback(true);
            return;
        }
        offset.current = 0;
        setOutput('');
        setView({ phase: 'connecting' });
        setId(res.id);
    };

    const reply = async (text: string) => {
        if (!id) return;
        setAnswer('');
        await window.api.remoteInstallInput(id, text);
        setView((v) => (v ? { ...v, prompt: null } : v));
    };

    const close = async () => {
        if (id) await window.api.remoteInstallCancel(id).catch(() => null);
        onClose();
    };

    const claim = async () => {
        setBusy(true);
        setError(null);
        const res = await window.api.remoteClaim(code.trim());
        setBusy(false);
        if (failed(res)) {
            setError(errorText(res));
            return;
        }
        setView({ phase: 'done', name: res.label || res.name });
        if (!paired.current) {
            paired.current = true;
            onPaired();
        }
    };

    const copy = async () => {
        await navigator.clipboard?.writeText(INSTALL_ONE_LINER).catch(() => null);
        setCopied(true);
    };

    const prompt = view?.prompt as { kind: string; text: string } | null | undefined;
    const failText = phase === 'failed' ? errorText({ error_code: view?.error_code || 'install_failed' }) : null;

    return (
        <div className="modal-overlay" role="presentation">
            <div className="modal remote-pc-dialog remote-install" role="dialog" aria-modal="true" aria-label={t('remote_daemon.install_title')}>
                <h3 className="remote-pc-dialog-title">
                    <Icon name="linux" size="sm" /> {t('remote_daemon.install_title')}
                </h3>
                {!id && !fallback && (
                    <>
                        <p className="setting-description">{t('remote_daemon.install_description')}</p>
                        <div className="remote-install-grid">
                            <label>
                                <span>{t('remote_daemon.install_host')}</span>
                                <input
                                    type="text"
                                    className="input setting-field"
                                    aria-label={t('remote_daemon.install_host')}
                                    placeholder={t('remote_daemon.install_host_placeholder')}
                                    value={form.host}
                                    onChange={(e) => setForm({ ...form, host: e.target.value })}
                                />
                            </label>
                            <label>
                                <span>{t('remote_daemon.install_user')}</span>
                                <input
                                    type="text"
                                    className="input setting-field"
                                    aria-label={t('remote_daemon.install_user')}
                                    value={form.user}
                                    onChange={(e) => setForm({ ...form, user: e.target.value })}
                                />
                            </label>
                            <label>
                                <span>{t('remote_daemon.install_port')}</span>
                                <SabaNumberInput
                                    className="input setting-num"
                                    aria-label={t('remote_daemon.install_port')}
                                    min={1}
                                    max={65535}
                                    value={form.port}
                                    onChange={(e) => setForm({ ...form, port: e.target.value })}
                                />
                            </label>
                        </div>
                        <SabaCheckbox checked={form.linger} onChange={(linger) => setForm({ ...form, linger })}>
                            {t('remote_daemon.install_linger')}
                        </SabaCheckbox>
                        <SabaCheckbox checked={form.steamLibs} onChange={(steamLibs) => setForm({ ...form, steamLibs })}>
                            {t('remote_daemon.install_steam')}
                        </SabaCheckbox>
                    </>
                )}

                {id && (
                    <>
                        <p className="remote-install-phase" role="status">
                            {running && <SabaSpinner size="xs" />}{' '}
                            {phase === 'done'
                                ? t('remote_daemon.install_phase_done', { name: view?.name || '' })
                                : t(`remote_daemon.install_phase_${phase || 'connecting'}`)}
                        </p>
                        <pre className="remote-install-console" ref={consoleRef} aria-label={t('remote_daemon.install_console')}>
                            {output}
                        </pre>
                        {prompt && running && (
                            <div className="remote-install-prompt" role="group" aria-label={t('remote_daemon.install_prompt')}>
                                {prompt.kind === 'host_key' ? (
                                    <>
                                        <p>{t('remote_daemon.prompt_host_key')}</p>
                                        <pre className="remote-install-fingerprint">{prompt.text}</pre>
                                        <div className="remote-pc-dialog-actions">
                                            <button type="button" className="btn btn-ghost" onClick={() => reply('no')}>
                                                {t('remote_daemon.prompt_no')}
                                            </button>
                                            <button type="button" className="btn btn-primary" onClick={() => reply('yes')}>
                                                {t('remote_daemon.prompt_yes')}
                                            </button>
                                        </div>
                                    </>
                                ) : (
                                    <form
                                        onSubmit={(e) => {
                                            e.preventDefault();
                                            reply(answer);
                                        }}
                                    >
                                        <label>
                                            <span>{t(PROMPT_LABELS[prompt.kind] || 'remote_daemon.prompt_password')}</span>
                                            <span className="muted">{prompt.text}</span>
                                            <input
                                                type="password"
                                                className="input setting-field"
                                                aria-label={t(PROMPT_LABELS[prompt.kind] || 'remote_daemon.prompt_password')}
                                                autoComplete="off"
                                                value={answer}
                                                onChange={(e) => setAnswer(e.target.value)}
                                            />
                                        </label>
                                        <button type="submit" className="btn btn-primary">
                                            {t('remote_daemon.prompt_send')}
                                        </button>
                                    </form>
                                )}
                            </div>
                        )}
                    </>
                )}

                {!id && phase === 'done' && (
                    <p className="remote-install-phase" role="status">
                        {t('remote_daemon.install_phase_done', { name: view?.name || '' })}
                    </p>
                )}

                {(fallback || phase === 'failed') && phase !== 'done' && (
                    <div className="remote-install-fallback">
                        <p className="setting-description">{t('remote_daemon.install_fallback')}</p>
                        <div className="remote-install-oneliner">
                            <code>{INSTALL_ONE_LINER}</code>
                            <button type="button" className="btn btn-ghost btn-sm" onClick={copy}>
                                <Icon name={copied ? 'check' : 'copy'} size="sm" /> {copied ? t('remote_daemon.copied') : t('remote_daemon.copy')}
                            </button>
                        </div>
                        <label className="remote-install-claim">
                            <span>{t('remote_daemon.claim_code')}</span>
                            <input
                                type="text"
                                className="input setting-field"
                                aria-label={t('remote_daemon.claim_code')}
                                value={code}
                                onChange={(e) => setCode(e.target.value)}
                            />
                            <button type="button" className="btn btn-primary" disabled={busy || !code.trim()} onClick={claim}>
                                {busy ? <SabaSpinner size="xs" /> : <Icon name="plug" size="sm" />} {t('remote_daemon.claim_go')}
                            </button>
                        </label>
                    </div>
                )}

                {(error || failText) && (
                    <div className="setting-validation-error" role="alert">
                        <Icon name="alertCircle" size="sm" /> {error || failText}
                    </div>
                )}

                <div className="remote-pc-dialog-actions">
                    {!id && !fallback && (
                        <button type="button" className="btn btn-ghost" onClick={() => setFallback(true)}>
                            {t('remote_daemon.install_no_ssh')}
                        </button>
                    )}
                    <button type="button" className="btn btn-ghost" onClick={close}>
                        {running ? t('remote_daemon.install_cancel') : t('remote_daemon.install_close')}
                    </button>
                    {!id && !fallback && (
                        <button type="button" className="btn btn-primary" onClick={start} disabled={busy}>
                            {busy ? <SabaSpinner size="xs" /> : <Icon name="download" size="sm" />} {t('remote_daemon.install_start')}
                        </button>
                    )}
                </div>
            </div>
        </div>
    );
}
