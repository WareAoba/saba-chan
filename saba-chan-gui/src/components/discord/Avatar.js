export const initialOf = (name) => (name || '?').trim().charAt(0).toUpperCase() || '?';

export function Avatar({ name, url, size = 36 }) {
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
