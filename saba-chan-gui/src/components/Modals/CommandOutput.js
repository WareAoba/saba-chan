import { useState } from 'react';
import { useTranslation } from 'react-i18next';

// 처음에 보이는 줄 수. 넘으면 "N개 더 보기"로 펼친다
const PREVIEW_ROWS = 8;

// 데몬이 정리한 명령 결과 (`output`: { text } | { fields } | { table }, 데몬 commands::output)
export function CommandOutput({ output }) {
    const { t } = useTranslation('gui');
    const [all, setAll] = useState(false);
    if (typeof output.text === 'string') return <pre className="cmd-entry-out">{output.text}</pre>;

    const rows = output.fields || output.table?.rows || [];
    const shown = all ? rows : rows.slice(0, PREVIEW_ROWS);
    const more = rows.length - shown.length;
    const table = output.table;
    return (
        <div className="cmd-entry-out cmd-output">
            {table?.label && (
                <div className="cmd-output-title">
                    {table.label} ({rows.length})
                </div>
            )}
            {output.fields && (
                <dl className="cmd-output-fields">
                    {shown.map((f) => (
                        <div key={f.key} className="cmd-output-field">
                            <dt>{f.label}</dt>
                            <dd>{f.value}</dd>
                        </div>
                    ))}
                </dl>
            )}
            {table && shown.length > 0 && (
                <table className="cmd-output-table">
                    <thead>
                        <tr>
                            {table.columns.map((c) => (
                                <th key={c.key}>{c.label}</th>
                            ))}
                        </tr>
                    </thead>
                    <tbody>
                        {shown.map((row, i) => (
                            // biome-ignore lint/suspicious/noArrayIndexKey: 서버가 준 순서 그대로인 결과 줄 (고유 키가 없다)
                            <tr key={i}>
                                {row.map((cell, j) => (
                                    // biome-ignore lint/suspicious/noArrayIndexKey: 열 순서 고정
                                    <td key={j}>{cell}</td>
                                ))}
                            </tr>
                        ))}
                    </tbody>
                </table>
            )}
            {more > 0 && (
                <button type="button" className="cmd-output-more" onClick={() => setAll(true)}>
                    {t('command_modal.show_more', { count: more })}
                </button>
            )}
        </div>
    );
}
