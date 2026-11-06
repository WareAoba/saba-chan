import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';


// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    outDir: 'build',
    sourcemap: false, // 프로덕션: sourcemap 비활성화
    minify: 'terser',
    rolldownOptions: {
      output: {
        // Split vendor libraries into separate chunks (rolldown은 manualChunks 객체 형식을 받지 않는다)
        codeSplitting: {
          groups: [
            { name: 'vendor-react', test: /[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/ },
            { name: 'vendor-i18n', test: /[\\/]node_modules[\\/](i18next|react-i18next)[\\/]/ },
          ],
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
    include: ['src/test/**/*.test.{ts,tsx}'],
    testTimeout: 20000,
  },
});
