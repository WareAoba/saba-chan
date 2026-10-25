import '@gui/styles/tokens.css';
import './styles.css';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { createWebApi } from './api';
import { art } from './art';
import { initI18n } from './i18n';
import { createSession } from './session';
import { useApp } from './store';

/** 링크 비밀값은 주소의 # 뒤에 온다. 읽자마자 주소에서 지운다 (새로 고침·공유로 다시 쓰이지 않게) */
function takeLink() {
    const raw = window.location.hash.slice(1);
    if (raw) window.history.replaceState(null, '', window.location.pathname);
    try {
        return decodeURIComponent(raw);
    } catch {
        return ''; // 깨진 주소는 링크가 없는 것과 같다 (빈 화면 대신 안내)
    }
}

const theme = window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
document.documentElement.dataset.theme = theme;

const link = takeLink();

const icon = document.createElement('link');
icon.rel = 'icon';
icon.href = art.favicon;
document.head.append(icon);

// 번역을 읽은 뒤 연결한다 (오래된 브라우저 대상이라 최상위 await 를 쓰지 않는다)
initI18n().then(() => {
    const url = `${window.location.protocol === 'http:' ? 'ws' : 'wss'}://${window.location.host}/remote/ws`;
    let bridge = null;
    const session = link
        ? createSession({
              url,
              link,
              onState: (s) => useApp.getState().setSession(s),
              onEvent: (e) => bridge?.emit(e),
          })
        : null;
    if (!session) useApp.getState().setSession({ ...useApp.getState().session, phase: 'closed', reason: 'no_link' });
    bridge = session ? createWebApi(session) : null;

    // 페이지를 떠나면 세션도 끝난다 (릴레이는 1001 로 받아도 끝낸다. bye 는 그보다 먼저 닿게)
    window.addEventListener('pagehide', () => session?.leave());

    createRoot(document.getElementById('root')).render(<App api={bridge?.api} session={session} />);
});
