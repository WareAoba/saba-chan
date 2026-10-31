import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Dropdown } from './Dropdown';
import { Button, errorText, failed, Section } from './ui';

const GAME_CHAT_MODES = ['off', 'all', 'keyword'];
/** 'true'·'false' 글자로 저장된 값도 있다. 기본값은 데몬과 같다 (크래시 재시작 켜짐, 원인 모를 종료 꺼짐) */
const asFlag = (v, dflt) => (v === true || v === 'true' ? true : v === false || v === 'false' ? false : dflt);

/** 저장된 사바쨩 칸 값 (GUI useServerSettings 와 같은 자리에서 읽는다) */
export function sabaValues(server) {
    const ms = server?.module_settings || {};
    return {
        memory_limit_mb: server?.memory_limit_mb == null ? '' : String(server.memory_limit_mb),
        stop_timeout: ms.stop_timeout == null ? '' : String(ms.stop_timeout),
        game_chat: server?.game_chat || 'off',
        game_chat_keyword: server?.game_chat_keyword || '',
        auto_restart: asFlag(ms.auto_restart, true),
        restart_on_unknown_exit: asFlag(ms.restart_on_unknown_exit, false),
    };
}

/** 보낼 값: 폼을 연(또는 마지막으로 맞춘) 때의 값 `opened`에서 바꾼 칸만.
 * 비운 숫자 칸은 null(제한 없음·모듈 값), 형식과 범위 검사는 데몬이 한다 */
export function sabaPatch(opened, values) {
    const patch = {};
    for (const key of ['memory_limit_mb', 'stop_timeout']) {
        const v = String(values[key] ?? '').trim();
        if (v !== opened[key]) patch[key] = v === '' ? null : v;
    }
    for (const key of ['game_chat', 'game_chat_keyword']) {
        const v = String(values[key] ?? '').trim();
        if (v !== opened[key]) patch[key] = v;
    }
    for (const key of ['auto_restart', 'restart_on_unknown_exit']) {
        if (values[key] !== opened[key]) patch[key] = values[key];
    }
    return patch;
}

function Toggle({ label, hint, checked, onChange }) {
    return (
        <label className="field field-row">
            <span>
                {label}
                {hint && <small className="hint">{hint}</small>}
            </span>
            <input type="checkbox" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        </label>
    );
}

function NumberField({ label, hint, placeholder, value, onChange, note }) {
    return (
        <label className="field">
            <span>{label}</span>
            <input
                type="number"
                inputMode="numeric"
                min="1"
                value={value}
                placeholder={placeholder}
                onChange={(e) => onChange(e.target.value)}
            />
            {hint && <small className="hint">{hint}</small>}
            {note && <small className="hint">{note}</small>}
        </label>
    );
}

/** 사바쨩이 서버를 다루는 방식: 자동 재시작, 램 상한, 게임 채팅 전달, 정지 대기 */
export function SabaSettings({ api, server, running }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const ask = useApp((s) => s.ask);
    // 기준값: 폼을 연 때(또는 다른 곳의 변경을 따라간 때·저장한 때)의 값. 고쳤는지는 이것과 비교한다
    // (폴링으로 온 최신 값과 비교하면 다른 곳에서 바꾼 값을 사용자가 고친 것으로 본다)
    const [opened, setOpened] = useState(() => sabaValues(server));
    const [values, setValues] = useState(opened);
    const [saving, setSaving] = useState(false);
    // 고치기 시작한 때의 revision 을 싣는다 (폴링의 최신 값을 실으면 다른 곳의 변경을 덮는다)
    const revision = useRef(server.revision || null);
    const patch = sabaPatch(opened, values);
    const dirty = Object.keys(patch).length > 0;

    // 고치는 칸이 없으면 다른 곳에서 바뀐 값을 따라간다
    // biome-ignore lint/correctness/useExhaustiveDependencies: revision 이 바뀔 때만 본다
    useEffect(() => {
        if (dirty) return;
        const fresh = sabaValues(server);
        setOpened(fresh);
        setValues(fresh);
        revision.current = server.revision || null;
    }, [server.revision]);

    const set = (key, v) => setValues((prev) => ({ ...prev, [key]: v }));

    const save = async () => {
        if (!dirty) return;
        setSaving(true);
        const res = await api.instanceUpdateSettings(server.id, {
            ...patch,
            ...(revision.current ? { revision: revision.current } : {}),
        });
        setSaving(false);
        if (res?.error_code === 'settings_changed') {
            const ok = await ask({
                title: t('settings.save_failed'),
                message: t('settings.changed_elsewhere'),
                confirmLabel: t('settings.reload'),
            });
            if (ok) {
                const fresh = await api.instanceGet(server.id);
                if (!fresh?.id || failed(fresh)) return toast(errorText(t, fresh), 'error');
                const next = sabaValues(fresh);
                setOpened(next);
                setValues(next);
                revision.current = fresh.revision || res.revision || null;
            }
            return;
        }
        if (failed(res)) return toast(errorText(t, res), 'error');
        if (res?.revision) revision.current = res.revision;
        setOpened(values);
        toast(running ? t('settings.saved_restart') : t('settings.saved'), 'ok');
    };

    return (
        <form
            className="panel"
            onSubmit={(e) => {
                e.preventDefault();
                save();
            }}
        >
            <Section title={tg('shell.restart_title')}>
                <p className="hint">{tg('shell.restart_desc')}</p>
                <div className="form">
                    <Toggle
                        label={tg('shell.restart_on_crash')}
                        hint={tg('shell.restart_on_crash_hint')}
                        checked={values.auto_restart}
                        onChange={(v) => set('auto_restart', v)}
                    />
                    <Toggle
                        label={tg('shell.restart_on_unknown')}
                        hint={tg('shell.restart_on_unknown_hint')}
                        checked={values.restart_on_unknown_exit}
                        onChange={(v) => set('restart_on_unknown_exit', v)}
                    />
                </div>
            </Section>
            <Section title={tg('memory_limit.title')}>
                <p className="hint">{tg('memory_limit.desc')}</p>
                <div className="form">
                    <NumberField
                        label={tg('memory_limit.label')}
                        hint={tg('memory_limit.hint')}
                        placeholder={tg('memory_limit.placeholder')}
                        value={values.memory_limit_mb}
                        onChange={(v) => set('memory_limit_mb', v)}
                        note={running ? tg('memory_limit.restart_note') : null}
                    />
                </div>
            </Section>
            {server.game_chat_supported && (
                <Section title={tg('game_chat.title')}>
                    <p className="hint">{tg('game_chat.desc')}</p>
                    <div className="form">
                        <div className="field">
                            <span>{tg('game_chat.mode_label')}</span>
                            <Dropdown
                                value={values.game_chat}
                                onChange={(v) => set('game_chat', v)}
                                options={GAME_CHAT_MODES.map((m) => ({ value: m, label: tg(`game_chat.mode_${m}`) }))}
                            />
                        </div>
                        {values.game_chat === 'keyword' && (
                            <label className="field">
                                <span>{tg('game_chat.keyword_label')}</span>
                                <input
                                    value={values.game_chat_keyword}
                                    placeholder={tg('game_chat.keyword_placeholder')}
                                    onChange={(e) => set('game_chat_keyword', e.target.value)}
                                />
                                <small className="hint">{tg('game_chat.keyword_hint')}</small>
                            </label>
                        )}
                        <small className="hint">{tg('game_chat.channel_note')}</small>
                    </div>
                </Section>
            )}
            {server.managed !== false && (
                <Section title={tg('stop_timeout.title')}>
                    <p className="hint">{tg('stop_timeout.desc')}</p>
                    <div className="form">
                        <NumberField
                            label={tg('stop_timeout.label')}
                            hint={tg('stop_timeout.hint')}
                            placeholder={tg('stop_timeout.placeholder')}
                            value={values.stop_timeout}
                            onChange={(v) => set('stop_timeout', v)}
                        />
                    </div>
                </Section>
            )}
            <div className="savebar">
                {dirty && (
                    <Button onClick={() => setValues(opened)} disabled={saving}>
                        {tg('discord.revert')}
                    </Button>
                )}
                <Button tone="accent" type="submit" busy={saving} disabled={!dirty}>
                    {t('settings.save')}
                </Button>
            </div>
        </form>
    );
}
