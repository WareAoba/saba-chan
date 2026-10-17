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

/** 타입 오류 한 건 → 한 줄 ("port에는 숫자가 들어가야 해요") */
export function argErrorText(e, t, labelOf) {
    return t('launch_preview.arg_error', {
        arg: e.arg,
        field: labelOf(e.field),
        expected: t(`launch_preview.type_${e.expected}`, { defaultValue: e.expected }),
        got: e.got,
    });
}

/**
 * 서버 설정 맨 위: 데몬이 계산한 최종 실행 인자 (읽기 전용) + 직접 덧붙일 인자.
 * 필드나 추가 인자를 바꾸면 잠깐 뒤 저장하지 않은 값으로 다시 묻는다. 비밀번호는 데몬이 가려서 준다.
 * 추가 인자 중 설정 필드에 해당하는 것은 그 필드에 바로 반영하고(onFieldsFromArgs), 나머지는 그대로 넘긴다.
 */
export function LaunchPreview({
    serverId,
    fields,
    values,
    extraArgs = '',
    onExtraArgsChange,
    onFieldsFromArgs,
    labelOf,
}) {
    const { t } = useTranslation('gui');
    const draft = useMemo(() => launchDraft(fields, values), [fields, values]);
    const key = JSON.stringify([draft, extraArgs]);
    const [res, setRes] = useState(null);
    const [loading, setLoading] = useState(true);
    const seq = useRef(0);
    // 필드를 직접 고친 뒤에도 추가 인자가 다시 덮어쓰지 않게, 추가 인자가 바뀐 응답만 필드에 반영한다
    const appliedExtra = useRef(extraArgs);
    const name = labelOf || ((f) => f);

    // biome-ignore lint/correctness/useExhaustiveDependencies: key가 draft·추가 인자 내용을 대신한다
    useEffect(() => {
        const mine = ++seq.current;
        setLoading(true);
        const timer = setTimeout(async () => {
            let next;
            try {
                next = await window.api.instanceLaunchPreview(serverId, { settings: draft, extra_args: extraArgs });
            } catch (e) {
                next = { error: e.message };
            }
            if (mine !== seq.current) return;
            setRes(next);
            setLoading(false);
            if (extraArgs !== appliedExtra.current) {
                appliedExtra.current = extraArgs;
                const found = next?.fields_from_args;
                if (found && Object.keys(found).length && onFieldsFromArgs) onFieldsFromArgs(found);
            }
        }, DEBOUNCE_MS);
        return () => clearTimeout(timer);
    }, [serverId, key]);

    const line = previewLine(res);
    const problem =
        res && line === null
            ? res.action_required
                ? t('launch_preview.action_required', { reason: res.message || res.action_required })
                : res.error || res.message || t('launch_preview.failed')
            : null;
    const errors = Array.isArray(res?.errors) ? res.errors : [];
    const unknown = Array.isArray(res?.unknown_args) ? res.unknown_args : [];

    return (
        <div className="launch-preview">
            <div className="launch-preview-head">
                <Icon name="terminal" size="sm" /> {t('launch_preview.title')}
                {loading && <SabaSpinner size="xs" />}
            </div>
            <div aria-live="polite">
                {problem ? (
                    <p className="launch-preview-problem">
                        <Icon name="alertTriangle" size="xs" /> {problem}
                    </p>
                ) : (
                    <code className="launch-preview-line">{line ?? '…'}</code>
                )}
            </div>
            {onExtraArgsChange && (
                <label className="launch-preview-extra">
                    <span>{t('launch_preview.extra_label')}</span>
                    <input
                        type="text"
                        className="setting-input-text launch-preview-input"
                        value={extraArgs}
                        spellCheck={false}
                        placeholder={t('launch_preview.extra_placeholder')}
                        onChange={(e) => onExtraArgsChange(e.target.value)}
                    />
                </label>
            )}
            {errors.map((e) => (
                <p key={`${e.arg}-${e.field}`} className="launch-preview-problem" role="alert">
                    <Icon name="alertCircle" size="xs" /> {argErrorText(e, t, name)}
                </p>
            ))}
            {unknown.length > 0 && (
                <p className="launch-preview-note">{t('launch_preview.unknown_args', { args: unknown.join(' ') })}</p>
            )}
        </div>
    );
}
