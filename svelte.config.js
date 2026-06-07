// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({
      fallback: "index.html",
    }),
    // CSP via SvelteKit so its inline bootstrap script is hashed per-build.
    // Tauri's own csp is left null in tauri.conf.json to defer to this meta tag.
    csp: {
      mode: "hash",
      directives: {
        "default-src": ["self"],
        "script-src": ["self"],
        "style-src": ["self", "unsafe-inline"],
        "img-src": ["self", "data:", "asset:", "https://asset.localhost"],
        "connect-src": ["self", "ipc:", "http://ipc.localhost"],
        "object-src": ["none"],
        "base-uri": ["self"],
      },
    },
  },
};

export default config;
