import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
    plugins: [react()],
    // Keep relative base so it works when served from a subpath.
    base: './',
});
