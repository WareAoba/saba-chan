import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { safeShowToast } from '../../utils/helpers';
import { SabaSpinner } from '../ui/SabaUI';

export const DEBOUNCE_MS = 400;

/** 미리보기를 물어볼 IPC가 있는지. 실제로 띄울지는 데몬 응답의 has_launch_args가 정한다 */
export const launchPreviewAvailable = () => Boolean(window.api?.instanceLaunchPreview);

/** 아직 저장하지 않은 설정값 (저장할 때와 같은 변환) */
export function launchDraft(fields, values) {
    const out = {};
    for (const f of fields || []) {
        const v = values?.[f.name];
        if (f.field_type === 'boolean') out[f.name] = v === true || v === 'true';
        else if (v === '' || v === null || v === undefined) continue;
        else out[f.name] = f.field_type === 'number' ? Number(v) : v;
    }
    return out;
}

/** 남는 인자 배열 → 안내 글자 (공백이 든 인자는 따옴표로) */
export const joinArgs = (args) =>
    Array.isArray(args) ? args.map((a) => (/\s/.test(a) ? JSON.stringify(a) : a)).join(' ') : '';

/** 데몬 응답 → 한 줄 (실패면 null) */
export function previewLine(res) {
    if (!res || res.success === false || res.error || res.action_required) return null;
    if (typeof res.command_line === 'string') return res.command_line;
    if (Array.isArray(res.args)) return [res.program, ...res.args].filter(Boolean).join(' ');
    return null;
}

/**
 * 서버 설정 맨 위: 데몬이 계산한 최종 실행 인자 (읽기 전용) + 직접 덧붙일 인자.
 * extraArgs가 undefined면 저장된 추가 인자로 계산하고, 응답의 extra_args_text로 입력칸을 채운다(onExtraArgsLoaded).
 * 설정이나 추가 인자를 바꾸면 잠깐 뒤 저장하지 않은 값으로 다시 묻는다. 비밀번호는 데몬이 가려서 준다.
 * 추가 인자 중 설정에 해당하는 것은 그 칸에 바로 반영한다(onSettingsFromArgs). 남는 인자는 저장할 때 다시 나눈다.
 * 형식이 틀리면 칸마다 따로 알리지 않고 토스트 하나로 알린다: 틀린 칸의 이름표만 싣는다 (모듈마다 형식이 달라서).
 * `modNs`는 모듈 번역 이름공간(`mod_<모듈>`)이다.
 */
export function LaunchPreview({
    serverId,
    modNs,
    fields,
    values,
    extraArgs,
    onExtraArgsChange,
    onExtraArgsLoaded,
    onSettingsFromArgs,
}) {
    const { t } = useTranslation('gui');
    const draft = useMemo(() => launchDraft(fields, values), [fields, values]);
    const key = JSON.stringify([draft, extraArgs]);
    const [res, setRes] = useState(null);
    const [loading, setLoading] = useState(true);
    const seq = useRef(0);
    // 칸을 직접 고친 뒤에도 추가 인자가 다시 덮어쓰지 않게, 추가 인자가 바뀐 응답만 칸에 반영한다
    const appliedExtra = useRef(extraArgs);
    // 같은 오류로 토스트를 되풀이하지 않는다
    const toasted = useRef('');

    // biome-ignore lint/correctness/useExhaustiveDependencies: key가 draft·추가 인자 내용을 대신한다
    useEffect(() => {
        const mine = ++seq.current;
        setLoading(true);
        const timer = setTimeout(async () => {
            let next;
            try {
                next = await window.api.instanceLaunchPreview(serverId, draft, extraArgs);
            } catch (e) {
                next = { success: false, error: e.message };
            }
            if (mine !== seq.current) return;
            setRes(next);
            setLoading(false);
            if (extraArgs === undefined) {
                // 저장된 추가 인자를 데몬이 글자로 돌려준다: 입력칸만 채우고 설정 칸은 이미 같은 값이다
                if (typeof next?.extra_args_text === 'string') {
                    appliedExtra.current = next.extra_args_text;
                    onExtraArgsLoaded?.(next.extra_args_text);
                }
            } else if (extraArgs !== appliedExtra.current) {
                appliedExtra.current = extraArgs;
                const found = next?.settings_from_args;
                if (found && Object.keys(found).length) onSettingsFromArgs?.(found);
            }
            const errors = Array.isArray(next?.errors) ? next.errors : [];
            const errorKey = errors.length ? JSON.stringify(errors) : '';
            if (errorKey && errorKey !== toasted.current) {
                // 어느 칸인지는 이름표로만 알린다 (모듈마다 형식이 달라 형식 설명은 붙이지 않는다)
                const names = [...new Set(errors.map((e) => e?.field).filter(Boolean))];
                const labels = names.map((name) => {
                    const field = (fields || []).find((f) => f.name === name);
                    return t(`${modNs}:settings.${name}.label`, { defaultValue: field?.label || name });
                });
                safeShowToast(
                    labels.length
                        ? t('launch_preview.invalid_fields', { fields: labels.join(', ') })
                        : t('launch_preview.invalid_format'),
                    'warning',
                    4000,
                );
            }
            toasted.current = errorKey;
        }, DEBOUNCE_MS);
        return () => clearTimeout(timer);
    }, [serverId, key]);

    // 첫 응답 전이거나 실행 인자를 쓰지 않는 모듈이면 띄우지 않는다
    if (!res?.has_launch_args) return null;

    const line = previewLine(res);
    const problem =
        line === null
            ? res.action_required
                ? t('launch_preview.action_required', { reason: res.message || res.action_required })
                : res.message || res.error || t('launch_preview.failed')
            : null;
    const leftover = Array.isArray(res.extra_args) ? res.extra_args : [];

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
                    <code className="launch-preview-line">{line}</code>
                )}
            </div>
            {onExtraArgsChange && (
                <label className="launch-preview-extra">
                    <span>{t('launch_preview.extra_label')}</span>
                    <input
                        type="text"
                        className="setting-input-text launch-preview-input"
                        value={extraArgs ?? ''}
                        spellCheck={false}
                        placeholder={t('launch_preview.extra_placeholder')}
                        onChange={(e) => onExtraArgsChange(e.target.value)}
                    />
                </label>
            )}
            {leftover.length > 0 && (
                <p className="launch-preview-note">{t('launch_preview.unknown_args', { args: joinArgs(leftover) })}</p>
            )}
        </div>
    );
}
