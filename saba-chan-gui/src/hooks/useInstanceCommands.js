import { useEffect, useState } from 'react';

/**
 * 인스턴스에서 쓸 수 있는 명령 목록. 데몬 카탈로그(GET /api/instance/:id/commands)가 있으면
 * 모듈 명령 + 켜진 익스텐션의 명령(source: 'extension')을 함께 받고, 없으면 모듈 명령만 쓴다.
 * 카탈로그는 risk를 안 적은 명령도 'caution'으로 채우므로, 표시는 module.toml 값을 우선한다.
 */
export function useInstanceCommands(server, moduleCommands) {
    const [catalog, setCatalog] = useState(null);

    useEffect(() => {
        setCatalog(null);
        if (!server?.id || !window.api?.instanceCommands) return;
        let alive = true;
        window.api
            .instanceCommands(server.id)
            .then((r) => {
                if (alive && Array.isArray(r?.commands)) setCatalog(r.commands);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [server?.id]);

    if (!catalog) return moduleCommands;
    const byName = Object.fromEntries(moduleCommands.map((c) => [c.name, c]));
    // module.toml에 없는 명령(익스텐션·동적 명령)은 데몬이 보낸 danger만 믿는다
    return catalog.map((c) =>
        byName[c.name] && c.source !== 'extension'
            ? { ...c, ...byName[c.name], risk: byName[c.name].risk }
            : { ...c, risk: c.risk === 'danger' ? 'danger' : undefined },
    );
}
