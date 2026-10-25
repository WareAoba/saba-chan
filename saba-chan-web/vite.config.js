import path from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// 사바쨩 웹: nginx 가 /remote/ 아래에 정적으로 둔다 (server-chan deploy/nginx/nginx.conf)
export default defineConfig({
    plugins: [react()],
    // GUI 아이콘(@gui/components/Icon)은 .js 안에 JSX 를 쓴다
    esbuild: { loader: 'jsx', include: [/src[\\/].*\.jsx?$/], exclude: [] },
    base: '/remote/',
    build: { outDir: 'dist', sourcemap: false },
    server: { port: 5174, strictPort: true },
    resolve: {
        alias: {
            // GUI 에서 가져다 쓰는 것은 src/test/guiImports.test.js 의 목록만
            '@gui': path.resolve(__dirname, '../saba-chan-gui/src'),
            '@locales': path.resolve(__dirname, '../locales'),
            // 로고·사바쨩 그림 (src/art.js 의 목록만)
            '@art': path.resolve(__dirname, '../saba-chan-gui/public'),
        },
        // GUI 파일이 부르는 라이브러리도 이 폴더의 것 하나만 쓴다
        dedupe: ['react', 'react-dom', 'i18next', 'react-i18next', 'zustand'],
    },
    test: {
        globals: true,
        environment: 'jsdom',
        include: ['src/test/**/*.test.{js,jsx}'],
    },
});
