import path from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig, transformWithEsbuild } from 'vite';

// GUI 파일(@gui)은 TypeScript로 옮기는 중이라 남은 .js 안에 JSX가 있다. .ts·.tsx·.jsx는 Vite 기본 변환이 맡는다
const jsxInJs = {
    name: 'saba-jsx-in-js',
    enforce: 'pre',
    transform(code, id) {
        if (!/[\\/]src[\\/].*\.js$/.test(id.split('?')[0])) return null;
        return transformWithEsbuild(code, id, { loader: 'jsx', jsx: 'automatic' });
    },
};

// 사바쨩 웹: nginx 가 /remote/ 아래에 정적으로 둔다 (server-chan deploy/nginx/nginx.conf)
export default defineConfig({
    plugins: [jsxInJs, react()],
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
        dedupe: ['react', 'react-dom', 'i18next', 'react-i18next', 'zustand', 'clsx'],
    },
    test: {
        globals: true,
        environment: 'jsdom',
        include: ['src/test/**/*.test.{js,jsx}'],
    },
});
