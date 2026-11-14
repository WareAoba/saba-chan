import { gameNameOf, protocolLabel } from '@gui/utils/serverView';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Section } from './ui';
import type { ServerPanelProps } from '../types';

/** 정보: 게임, 버전(모듈에 물어 확인), 포트, 인스턴스 id */
export function InfoPanel({ api, server, moduleData }: Omit<ServerPanelProps, 'running'>) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [version, setVersion] = useState(server.server_version || null);

    useEffect(() => {
        let alive = true;
        api.instanceGetInstalledVersion(server.id).then((r) => {
            if (alive && typeof r?.version === 'string' && r.version) setVersion(r.version);
        });
        return () => {
            alive = false;
        };
    }, [api, server.id]);

    // 사용자 11-06 #24·#26: 모르는 값은 줄째 숨기고, 개발 용어(통신 방식)는 '자세히' 안에 (앱 정보 탭과 같다)
    const rows = [
        [tg('shell.info_module'), gameNameOf(t, server.module, moduleData)],
        version && [tg('shell.info_version'), version],
        server.port && [tg('shell.info_port'), String(server.port)],
        [tg('shell.info_id'), server.id],
    ].filter(Boolean) as [string, string][];
    const moreRows = [moduleData && [tg('shell.info_protocol'), protocolLabel(server)]].filter(
        Boolean,
    ) as [string, string][];
    const list = (items: [string, string][]) => (
        <dl className="info-list">
            {items.map(([k, v]) => (
                <div key={k} className="info-row">
                    <dt>{k}</dt>
                    <dd>{v}</dd>
                </div>
            ))}
        </dl>
    );

    return (
        <div className="panel">
            <Section title={tg('shell.tab_info')}>
                {list(rows)}
                {moreRows.length > 0 && (
                    <details className="info-more">
                        <summary>{tg('shell.info_more')}</summary>
                        {list(moreRows)}
                    </details>
                )}
            </Section>
        </div>
    );
}
