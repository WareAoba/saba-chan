import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CliText } from './CliLine';
import { Icon } from './Icon';

/**
 * 에이전트별 설치 줄과 덧붙일 안내. 저장소 주소는 데몬(GET /api/maker)이 준 것 하나만 쓴다.
 * Claude Code는 받은 저장소의 `.claude/skills/`로 스킬을 찾는다 (플러그인 마켓은 아직 열지 않았다)
 */
function installLines(agent, repo, t) {
    if (agent === 'codex') {
        return { lines: [t('saba_storage.maker_codex_line', { repo: repo.replace(/^https:\/\/github\.com\//, '') })] };
    }
    const note = agent === 'claude' ? 'saba_storage.maker_claude_note' : 'saba_storage.maker_other_note';
    return { lines: [`git clone ${repo}`], note: t(note) };
}

/**
 * 사바 스토리지 › 모듈 탭의 "직접 만들기": 목록에 없는 게임을 내 AI 에이전트로 만드는 방법.
 * 메이커 스킬·도구는 모듈 저장소에 있고, 여기서는 데몬이 알려 준 저장소로 설치 줄과 첫 부탁만 보여 준다.
 */
export function ModuleMakerCard() {
    const { t } = useTranslation('gui');
    const [open, setOpen] = useState(false);
    const [info, setInfo] = useState(null);
    const [agent, setAgent] = useState('claude');
    const [game, setGame] = useState('');

    useEffect(() => {
        if (!open || info) return;
        let alive = true;
        Promise.resolve(window.api.makerInfo()).then((r) => {
            if (alive) setInfo(r?.ok ? r : { error: r?.error || t('saba_storage.maker_unavailable') });
        });
        return () => {
            alive = false;
        };
    }, [open, info, t]);

    const install = info?.repo ? installLines(agent, info.repo, t) : null;
    const agents = [
        ['claude', 'Claude Code'],
        ['codex', 'Codex'],
        ['other', t('saba_storage.maker_other')],
    ];

    return (
        <div className="ss-maker">
            <div className="ss-card">
                <div className="ss-card-body">
                    <span className="ss-card-name">{t('saba_storage.maker_title')}</span>
                    <span className="ss-card-desc ss-maker-desc">{t('saba_storage.maker_desc')}</span>
                </div>
                <div className="ss-card-actions">
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        aria-expanded={open}
                        onClick={() => setOpen((v) => !v)}
                    >
                        {t('saba_storage.maker_open')}
                    </button>
                </div>
            </div>
            {open && info?.error && <p className="ss-empty">{info.error}</p>}
            {open && info?.repo && (
                <div className="ss-maker-panel">
                    <div className="ss-maker-agents" role="tablist">
                        {agents.map(([id, label]) => (
                            <button
                                key={id}
                                type="button"
                                role="tab"
                                aria-selected={agent === id}
                                className={clsx('ss-maker-agent', { active: agent === id })}
                                onClick={() => setAgent(id)}
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                    <p className="ss-maker-step">{t('saba_storage.maker_step_install')}</p>
                    {install.lines.map((line) => (
                        <CliText key={line} line={line} />
                    ))}
                    {install.note && <p className="ss-maker-note">{install.note}</p>}
                    <label className="ss-maker-step" htmlFor="ss-maker-game">
                        {t('saba_storage.maker_game_label')}
                    </label>
                    <input
                        id="ss-maker-game"
                        className="ss-maker-game"
                        value={game}
                        placeholder={t('saba_storage.maker_game_placeholder')}
                        onChange={(e) => setGame(e.target.value)}
                    />
                    <CliText
                        line={t('saba_storage.maker_prompt_text', {
                            game: game.trim() || t('saba_storage.maker_game_placeholder'),
                        })}
                    />
                    <p className="ss-maker-note">{t('saba_storage.maker_unverified')}</p>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        onClick={() => window.electron?.shell?.openExternal?.(info.guide)}
                    >
                        <Icon name="externalLink" size="sm" />
                        {t('saba_storage.maker_guide')}
                    </button>
                </div>
            )}
        </div>
    );
}
