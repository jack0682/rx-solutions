import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
const apiPort = Number(process.env.RX_DEV_API_PORT ?? '8080');
if (!Number.isInteger(apiPort) || apiPort < 1 || apiPort > 65535)
  throw new Error('RX_DEV_API_PORT must be a valid loopback port');
export default defineConfig({
  plugins: [react()],
  build: { assetsInlineLimit: 0 },
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: { '/api': { target: `http://127.0.0.1:${apiPort}`, changeOrigin: false } },
  },
});
