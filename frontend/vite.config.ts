import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [tailwindcss(), sveltekit()],
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:9514',
        changeOrigin: true
      },
      // Kobo protocol routes: /kobo/<token>/... — proxy to backend.
      // The bare /kobo path is the device-management SPA page, so we only
      // match paths that have at least one segment after the token.
      '^/kobo/[^/]+/.+$': {
        target: 'http://localhost:9514',
        changeOrigin: true
      }
    }
  }
});
