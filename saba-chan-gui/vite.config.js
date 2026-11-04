import { defineConfig, transformWithEsbuild } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

// TypeScript로 옮기는 동안 남은 .js 파일에도 JSX가 있다. .ts·.tsx는 Vite 기본 변환(esbuild)이 맡는다
const jsxInJs = {
    name: 'saba-jsx-in-js',
    enforce: 'pre',
    transform(code, id) {
        if (!/\/src\/.*\.js$/.test(id.split('?')[0])) return null;
        return transformWithEsbuild(code, id, { loader: 'jsx', jsx: 'automatic' });
    },
};

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [jsxInJs, react()],
  base: './',
  build: {
    outDir: 'build',
    sourcemap: false, // 프로덕션: sourcemap 비활성화
    minify: 'terser',
    rollupOptions: {
      output: {
        manualChunks: {
          // Split vendor libraries into separate chunks
          'vendor-react': ['react', 'react-dom'],
          'vendor-i18n': ['i18next', 'react-i18next'],
        },
      },
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  optimizeDeps: {
    esbuildOptions: {
      loader: {
        '.js': 'jsx',
      },
    },
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: './src/setupTests.ts',
    css: true,
    include: ['src/test/**/*.test.{js,ts,tsx}'],
    testTimeout: 20000,
  },
});
