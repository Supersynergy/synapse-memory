import { defineConfig } from 'astro/config';

export default defineConfig({
  site: 'https://synapse.sh',
  base: '/bench',
  build: {
    format: 'directory',
    inlineStylesheets: 'always',
  },
  output: 'static',
});
