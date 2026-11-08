import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
    plugins: [svelte()],
    base: '/ui/',
    build: { outDir: '../app/static/ui', emptyOutDir: true },
    server: { proxy: { '/api': 'http://127.0.0.1:5001' } },
});
