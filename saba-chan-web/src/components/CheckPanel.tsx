import { type CheckIssue, checkIssueText } from '@gui/utils/settingIssue';
import type { SettingField } from '@gui/types/daemon';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Icon } from './icons';
import { LogsCard } from './LogsCard';
import { PortCheck } from './PortCheck';
import { Button, Card, errorText, failed } from './ui';
import type { Module, ServerPanelProps } from '../types';

/** 점검 결과 한 건 (모듈 validate·diagnose_log, 데몬이 더한 setting_invalid) */
type Issue = CheckIssue;

/** 점검 결과 화면 */
interface CheckResult {
    kind: 'validate' | 'diagnose';
    error?: string;
    issues?: Issue[];
}

/** 업데이트 확인 결과 */
interface UpdateInfo {
    error?: string;
    update_available?: boolean;
    reason?: string;
}

const SEVERITY_ORDER: Record<string, number> = { critical: 0, error: 1, warning: 2, info: 3 };
// 모듈이 구현한 함수만 (목록이 없는 예전 모듈은 다 된다고 본다)
const supports = (moduleData: Partial<Module> | null | undefined, fn: string) => !Array.isArray(moduleData?.functions) || moduleData.functions.includes(fn);

/** 서버 점검: 켜기 전 점검·로그 진단(모듈 validate·diagnose_log) */
function ServerCheck({ api, server, moduleData }: Omit<ServerPanelProps, 'running'>) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [running, setRunning] = useState<CheckResult['kind'] | null>(null);
    const [result, setResult] = useState<CheckResult | null>(null);
    const canValidate = supports(moduleData, 'validate');
    const canDiagnose = supports(moduleData, 'diagnose_log');

    // biome-ignore lint/correctness/useExhaustiveDependencies: 다른 서버로 가면 결과를 지운다
    useEffect(() => setResult(null), [server.id]);
    if (!canValidate && !canDiagnose) return null;

    const run = async (kind: CheckResult['kind']) => {
        setRunning(kind);
        const res = await (kind === 'validate' ? api.instanceValidate(server.id) : api.instanceDiagnose(server.id));
        setRunning(null);
        if (res?.error_code === 'not_supported') return setResult({ kind, error: tg('errors.not_supported') });
        if (failed(res) && !Array.isArray(res?.issues)) return setResult({ kind, error: errorText(t, res) });
        const issues = [...(res?.issues || [])].sort(
            (a, b) => (SEVERITY_ORDER[a.severity] ?? 9) - (SEVERITY_ORDER[b.severity] ?? 9),
        );
        setResult({ kind, issues });
    };

    return (
        <Card icon="checkCircle" tone="3" title={t('check.server')}>
            <div className="row-actions">
                {canValidate && (
                    <Button busy={running === 'validate'} disabled={!!running} onClick={() => run('validate')}>
                        <Icon name="checkCircle" size={16} /> {tg('shell.check_validate')}
                    </Button>
                )}
                {canDiagnose && (
                    <Button busy={running === 'diagnose'} disabled={!!running} onClick={() => run('diagnose')}>
                        <Icon name="tool" size={16} /> {tg('shell.check_diagnose')}
                    </Button>
                )}
            </div>
            {result?.error && (
                <p className="banner banner-error">{tg('shell.check_failed', { error: result.error })}</p>
            )}
            {result?.issues?.length === 0 && (
                <p className="banner banner-ok">
                    {result.kind === 'validate' ? tg('shell.check_ok') : tg('shell.check_log_ok')}
                </p>
            )}
            {(result?.issues?.length ?? 0) > 0 && (
                <ul className="issues">
                    {result?.issues?.map((issue, i) => (
                        // biome-ignore lint/suspicious/noArrayIndexKey: 같은 코드의 문제가 여러 번 올 수 있어 순번을 붙인다
                        <li key={`${issue.code}-${i}`} className={`issue sev-${issue.severity}`}>
                            {(() => {
                                // 데몬이 더한 설정 값 문제는 칸 이름으로 번역한다 (앱 서버 점검과 같다)
                                const fields = (moduleData?.settings as { fields?: SettingField[] } | undefined)?.fields;
                                const text = checkIssueText(tg, issue, server.module, fields);
                                return (
                                    <>
                                        <p>{text.message}</p>
                                        {text.solution && <p className="hint">{text.solution}</p>}
                                    </>
                                );
                            })()}
                        </li>
                    ))}
                </ul>
            )}
        </Card>
    );
}

/** 게임 서버 업데이트: 물어볼 때만 확인한다 (설치 방식이 업데이트를 모르면 데몬이 알려 준다) */
function UpdateCheck({ api, server, running }: Omit<ServerPanelProps, 'moduleData'>) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const ask = useApp((s) => s.ask);
    const [info, setInfo] = useState<UpdateInfo | null>(null);
    const [busy, setBusy] = useState<'check' | 'apply' | null>(null);

    const check = async () => {
        setBusy('check');
        const res = await api.instanceCheckUpdate(server.id);
        setBusy(null);
        setInfo(res?.error ? { error: errorText(t, res) } : res || {});
    };
    const apply = async () => {
        const ok = await ask({ title: tg('shell.update'), message: t('check.update_confirm', { name: server.name }) });
        if (!ok) return;
        setBusy('apply');
        const res = await api.instanceApplyUpdate(server.id);
        setBusy(null);
        if (failed(res)) return toast(errorText(t, res), 'error');
        toast(t('check.update_started'), 'ok');
        setInfo(null);
    };

    return (
        <Card icon="refresh" tone="4" title={t('check.update')}>
            <div className="row-actions">
                <Button busy={busy === 'check'} disabled={!!busy} onClick={check}>
                    <Icon name="refresh" size={16} /> {t('check.update_check')}
                </Button>
                {info?.update_available && (
                    <Button tone="accent" busy={busy === 'apply'} disabled={!!busy || running} onClick={apply}>
                        {tg('shell.update')}
                    </Button>
                )}
            </div>
            {info?.error && <p className="banner banner-error">{info.error}</p>}
            {info && !info.error && (
                <p className={info.update_available ? 'banner banner-info' : 'hint'}>
                    {info.update_available
                        ? tg('shell.update_available')
                        : info.reason === 'update_unsupported'
                          ? t('check.update_unsupported')
                          : t('check.update_none')}
                </p>
            )}
            {info?.update_available && running && <p className="hint">{t('settings.stop_first')}</p>}
        </Card>
    );
}

/** 서버 초기화: 월드·저장 데이터를 지운다 (무엇이 지워지는지는 모듈 locales reset.description) */
function ResetServer({ api, server, running }: Omit<ServerPanelProps, 'moduleData'>) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const ask = useApp((s) => s.ask);
    const [busy, setBusy] = useState(false);
    const reset = async () => {
        const ok = await ask({
            title: tg('server_settings.reset_confirm_title'),
            message: [
                tg('server_settings.reset_confirm', { name: server.name }),
                t(`mod_${server.module}:reset.description`, { defaultValue: '' }),
            ]
                .filter(Boolean)
                .join('\n\n'),
            confirmLabel: tg('server_settings.reset_button'),
            danger: true,
        });
        if (!ok) return;
        setBusy(true);
        const res = await api.instanceResetServer(server.id);
        setBusy(false);
        if (failed(res)) return toast(tg('server_settings.reset_failed', { error: errorText(t, res) }), 'error');
        toast(tg('server_settings.reset_success', { name: server.name, count: res?.deleted?.length || 0 }), 'ok');
    };
    return (
        <Card icon="alertTriangle" tone="2" title={tg('server_settings.reset_server_label')} desc={tg('server_settings.reset_server_desc')}>
            <Button tone="ghost-danger" busy={busy} disabled={running} onClick={reset}>
                {tg('server_settings.reset_button')}
            </Button>
            {running && <p className="hint">{tg('server_settings.reset_server_running')}</p>}
        </Card>
    );
}

export function CheckPanel({ api, server, moduleData, running }: ServerPanelProps) {
    const { t } = useTranslation();
    return (
        <div className="panel">
            <Card icon="network" tone="1" title={t('detail.port_check')}>
                <PortCheck api={api} server={server} />
            </Card>
            <LogsCard api={api} server={server} />
            <ServerCheck api={api} server={server} moduleData={moduleData} />
            <UpdateCheck api={api} server={server} running={running} />
            <ResetServer api={api} server={server} running={running} />
        </div>
    );
}
