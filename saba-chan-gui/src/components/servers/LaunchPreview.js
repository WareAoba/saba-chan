import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';

export const DEBOUNCE_MS = 400;

/** 미리보기를 띄울지: IPC가 있고, 모듈에 실제 실행 인자로 들어가는 필드가 있을 때만 */
export const launchPreviewAvailable = (fields) =>
    Boolean(window.api?.instanceLaunchPreview) && (fields || []).some((f) => f.launch_arg);

/** 실행 인자 필드의 아직 저장하지 않은 값 (저장할 때와 같은 변환) */
export function launchDraft(fields, values) {
    const out = {};
    for (const f of fields || []) {
        if (!f.launch_arg) continue;
        const v = values?.[f.name];
        if (f.field_type === 'boolean') out[f.name] = v === true || v === 'true';
        else if (v === '' || v === null || v === undefined) continue;
        else out[f.name] = f.field_type === 'number' ? Number(v) : v;
    }
    return out;
}

/** 데몬 응답 → 한 줄 (실패면 null) */
export function previewLine(res) {
    if (!res || res.success === false || res.error || res.action_required) return null;
    if (typeof res.command_line === 'string') return res.command_line;
    if (Array.isArray(res.args)) return [res.program || res.command, ...res.args].filter(Boolean).join(' ');
    return null;
}

/**
 * 서버 설정 맨 위: 데몬이 계산한 최종 실행 인자 (읽기 전용).
 * 실행 인자 필드를 바꾸면 잠깐 뒤 저장하지 않은 값으로 다시 묻는다. 비밀번호는 데몬이 가려서 준다.
 */
export function LaunchPreview({ serverId, fields, values }) {
    const { t } = useTranslation('gui');
    const draft = useMemo(() => launchDraft(fields, values), [fields, values]);
    const key = JSON.stringify(draft);
    const [line, setLine] = useState(null);
    const [problem, setProblem] = useState(null);
    const [loading, setLoading] = useState(true);
    const seq = useRef(0);

    // biome-ignore lint/correctness/useExhaustiveDependencies: key가 draft 내용을 대신한다
    useEffect(() => {
        const mine = ++seq.current;
        setLoading(true);
        const timer = setTimeout(async () => {
            let res;
            try {
                res = await window.api.instanceLaunchPreview(serverId, draft);
            } catch (e) {
                res = { error: e.message };
            }
            if (mine !== seq.current) return;
            const text = previewLine(res);
            setLine(text);
            setProblem(
                text === null
                    ? res?.action_required
                        ? t('launch_preview.action_required', { reason: res.message || res.action_required })
                        : res?.error || res?.message || t('launch_preview.failed')
                    : null,
            );
            setLoading(false);
        }, DEBOUNCE_MS);
        return () => clearTimeout(timer);
    }, [serverId, key]);

    return (
        <div className="launch-preview" aria-live="polite">
            <div className="launch-preview-head">
                <Icon name="terminal" size="sm" /> {t('launch_preview.title')}
                {loading && <SabaSpinner size="xs" />}
            </div>
            {problem ? (
                <p className="launch-preview-problem">
                    <Icon name="alertTriangle" size="xs" /> {problem}
                </p>
            ) : (
                <code className="launch-preview-line">{line ?? '…'}</code>
            )}
        </div>
    );
}
