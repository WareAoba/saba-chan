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
                <img src={url} alt="" loading="lazy" referrerPolicy="no-referrer" />
            ) : (
                initialOf(name)
            )}
        </span>
    );
}
