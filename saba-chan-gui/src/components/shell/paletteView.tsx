import clsx from 'clsx';
import type { CommandInput } from '../../types/daemon';
import { Icon } from '../Icon';
import type { Control, PaletteCommand, PaletteItem, PaletteServer, Translate } from './paletteModel';

/** 명령 팔레트의 그리기 조각: 줄 아이콘, 오른쪽 미리 보기, 아래 단축키 안내. 상태 없이 받은 값만 그린다 */

export type Mode = 'list' | 'entities' | 'args' | 'confirm' | 'controlConfirm' | 'busy' | 'done';

const CONTROL_ICON: Record<Control, string> = { start: 'play', stop: 'stop', restart: 'refresh', backup: 'save' };

export const optional = (i: CommandInput) => !i.required || i.default != null;

/** 입력 칸 후보 (고르는 칸만) */
export function inputChoices(i: CommandInput, t: Translate) {
    if (i.type === 'boolean') {
        return [
            { value: 'true', label: t('palette.bool_on') },
            { value: 'false', label: t('palette.bool_off') },
        ];
    }
    if (Array.isArray(i.options) && i.options.length) return i.options.map((o) => ({ value: o, label: o }));
    return null;
}

/** 서버 상태 한 낱말 */
export const statusText = (tr: Translate, status?: string) =>
    tr(
        status === 'running'
            ? 'palette.status_running'
            : status === 'stopped'
              ? 'palette.status_stopped'
              : 'palette.status_busy',
    );

/** 줄·미리 보기 아이콘. status: 그 항목 서버의 상태 (게임 서버 줄의 점 색) */
export const iconOf = (it: PaletteItem, status?: string, big = false) => {
    const cls = (...c: (string | false | undefined)[]) => clsx('palette-ico', { big }, ...c);
    switch (it.group) {
        case 'ctl':
            return (
                <span className={cls('ctl', it.control === 'stop' && 'stop')}>
                    <Icon name={CONTROL_ICON[it.control as Control]} size="sm" />
                </span>
            );
        case 'cmd':
        case 'xcmd':
            return (
                <span className={cls('cmd')}>
                    <Icon name="terminal" size="sm" />
                </span>
            );
        case 'setting':
            return (
                <span className={cls('set')}>
                    <Icon name="settings" size="sm" />
                </span>
            );
        case 'esrv':
            return (
                <span className={cls('srv')}>
                    <span className={clsx('palette-dot', status)} />
                </span>
            );
        case 'eguild':
            return <span className={cls('gld')}>{Array.from(it.title)[0]}</span>;
        default:
            return (
                <span className={cls(it.serverId || it.guildId ? 'jump' : 'app')}>
                    <Icon name={it.icon ?? (it.action === 'newServer' ? 'plus' : 'externalLink')} size="sm" />
                </span>
            );
    }
};

/** 게임 명령 미리 보기 */
export const commandPreview = (c: PaletteCommand, serverName: string | undefined, tr: Translate) => (
    <>
        <span className="palette-ico big cmd">
            <Icon name="terminal" size="sm" />
        </span>
        <h3>{c.label}</h3>
        {c.description && <p>{c.description}</p>}
        <div className="palette-cmdline">
            {c.name}
            {c.inputs.map((i) => (
                <span key={i.name} className="ph">
                    {` <${i.label}>`}
                </span>
            ))}
        </div>
        {c.inputs.length ? (
            <dl>
                {c.inputs.map((i) => (
                    <div key={i.name} className="palette-dl-row">
                        <dt>{i.label}</dt>
                        <dd>
                            {inputChoices(i, tr)
                                ?.map((o) => o.label)
                                .join(' · ') ||
                                (i.placeholder
                                    ? tr('palette.example', { value: i.placeholder })
                                    : tr('palette.free_input'))}
                            {optional(i) ? ` · ${tr('palette.optional')}` : ''}
                        </dd>
                    </div>
                ))}
            </dl>
        ) : (
            <p>{tr('palette.no_inputs_hint')}</p>
        )}
        {serverName && (
            <div className="palette-why">
                <Icon name="check" size="xs" /> {tr('palette.pv_ready', { name: serverName })}
            </div>
        )}
    </>
);

/** 고른 줄의 미리 보기. s: 그 항목의 서버 */
export const itemPreview = (it: PaletteItem, s: PaletteServer | undefined, tr: Translate) => {
    if (it.group === 'eguild' || it.group === 'esrv') {
        const cmds = s ? (s.commands || []).filter((c) => c.ready).length : 0;
        return (
            <>
                {iconOf(it, s?.status, true)}
                <h3>{it.title}</h3>
                {s && (
                    <dl>
                        <div className="palette-dl-row">
                            <dt>{tr('palette.status')}</dt>
                            <dd>
                                <span className={clsx('palette-pill', { ok: s.status === 'running' })}>
                                    {statusText(tr, s.status)}
                                </span>
                            </dd>
                        </div>
                        <div className="palette-dl-row">
                            <dt>{tr('palette.group.cmd')}</dt>
                            <dd>
                                {cmds ? tr('palette.commands_count', { count: cmds }) : tr('palette.commands_none')}
                            </dd>
                        </div>
                    </dl>
                )}
                <p>{tr('palette.pv_scope')}</p>
            </>
        );
    }
    if (it.group === 'xcmd' && it.command && s) {
        return (
            <>
                {iconOf(it, s?.status, true)}
                <h3>{it.title}</h3>
                {it.command.description && <p>{it.command.description}</p>}
                <p>{tr('palette.pv_switch', { name: s.name })}</p>
                <div className="palette-why">
                    <Icon name="check" size="xs" /> {tr('palette.pv_ready', { name: s.name })}
                </div>
            </>
        );
    }
    if (it.group === 'cmd' && it.command) return commandPreview(it.command, s?.name, tr);
    if (it.group === 'ctl') {
        return (
            <>
                {iconOf(it, s?.status, true)}
                <h3>
                    {s?.name} {it.title}
                </h3>
                <p>{tr(`palette.control_desc.${it.control}`)}</p>
                <dl>
                    <div className="palette-dl-row">
                        <dt>{tr('palette.status')}</dt>
                        <dd>
                            <span className={clsx('palette-pill', { ok: s?.status === 'running' })}>
                                {statusText(tr, s?.status)}
                            </span>
                        </dd>
                    </div>
                </dl>
            </>
        );
    }
    return (
        <>
            {iconOf(it, s?.status, true)}
            <h3>{it.title}</h3>
            {it.path && (
                <div className="palette-crumb">
                    {it.path.map((p, i) => (
                        // biome-ignore lint/suspicious/noArrayIndexKey: 고정된 경로 조각
                        <span key={i}>{p}</span>
                    ))}
                </div>
            )}
            <p>{tr(it.group === 'setting' ? 'palette.pv_setting' : 'palette.pv_jump')}</p>
        </>
    );
};

/** 아래 줄의 단축키 안내 */
export const paletteFoot = (mode: Mode, hasScope: boolean, tr: Translate) => {
    const k = (key: string, label: string) => (
        <span key={key + label}>
            <kbd>{key}</kbd>
            {label}
        </span>
    );
    if (mode === 'args')
        return [
            k('↵', tr('palette.foot.next_field')),
            k('⌫', tr('palette.foot.prev_field')),
            k('Esc', tr('palette.foot.to_list')),
        ];
    if (mode === 'confirm' || mode === 'controlConfirm')
        return [k('↵', tr('palette.foot.run')), k('Esc', tr('palette.foot.back'))];
    if (mode === 'busy' || mode === 'done') return [];
    if (mode === 'entities')
        return [
            k('↑↓', tr('palette.foot.move')),
            k('↵', tr('palette.foot.to_scope')),
            k('Esc', tr('palette.foot.keep')),
        ];
    return [
        k('↑↓', tr('palette.foot.move')),
        k('↵', tr('palette.foot.open')),
        k('Tab', tr('palette.foot.pick_scope')),
        ...(hasScope ? [k('⌫', tr('palette.foot.clear_scope'))] : []),
        k('Ctrl 1·2·3', tr('palette.foot.categories')),
        k('Esc', tr('palette.foot.close')),
        <span key="cho" className="r">
            {tr('palette.foot.choseong')}
        </span>,
    ];
};
