export const initialOf = (name: string | null | undefined) => (name || '?').trim().charAt(0).toUpperCase() || '?';

interface AvatarProps {
    name?: string | null;
    url?: unknown;
    size?: number;
}

export function Avatar({ name, url, size = 36 }: AvatarProps) {
    return (
        <span className="dc-avatar" style={{ width: size, height: size }}>
            {typeof url === 'string' && url.startsWith('http') ? (
                // 사진은 끌리지 않는다: 칩을 끌 때 브라우저가 사진만 따로 끌지 않게 (사용자 지적 2026-11-07)
                <img src={url} alt="" loading="lazy" referrerPolicy="no-referrer" draggable={false} />
            ) : (
                initialOf(name)
            )}
        </span>
    );
}
