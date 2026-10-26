// `npm start`: vite 개발 서버를 띄우고, 5173이 응답하면 Tauri 셸을 띄운다. 한쪽이 끝나면 다른 쪽도 끝낸다.
import { spawn } from 'node:child_process';

const DEV_URL = 'http://localhost:5173';
const children = [];
let exiting = false;

function run(command, args) {
    // Unix는 자식마다 프로세스 그룹을 따로 둬서 npm이 띄운 셸·vite까지 한 번에 끝낸다
    const child = spawn(command, args, { stdio: 'inherit', shell: process.platform === 'win32', detached: process.platform !== 'win32' });
    children.push(child);
    child.on('exit', (code) => stopAll(code ?? 1));
    return child;
}

function stopAll(code) {
    if (exiting) return;
    exiting = true;
    for (const child of children) {
        if (child.exitCode !== null) continue;
        // Windows는 셸(cmd) 아래 자식까지 끝내야 vite가 남지 않는다
        if (process.platform === 'win32') spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
        else {
            try {
                process.kill(-child.pid, 'SIGTERM');
            } catch {}
        }
    }
    process.exitCode = code;
}

async function waitForDevServer() {
    while (!exiting) {
        try {
            if ((await fetch(DEV_URL)).ok) return true;
        } catch {}
        await new Promise((resolve) => setTimeout(resolve, 300));
    }
    return false;
}

process.on('SIGINT', () => stopAll(130));
run('npm', ['run', 'dev']);
if (await waitForDevServer()) run('cargo', ['run', '-p', 'saba-chan-gui', '--no-default-features']);
