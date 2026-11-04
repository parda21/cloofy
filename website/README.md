# cloofy.app

cloofy's website: a landing page and the docs, built with [Astro](https://astro.build) and [Starlight](https://starlight.astro.build), and served by [Cloudflare Workers](https://developers.cloudflare.com/workers/static-assets/) from the `cloofy.app` zone.

```sh
npm install
npm run dev        # http://localhost:4321, reloads on changes
npm run build      # the site in dist/
npm run preview    # dist/ through the Worker, as Cloudflare serves it
npm run deploy     # build, and deploy to cloofy.app (after `npx wrangler login`)
```

- `src/pages/index.astro` is the landing page; `src/scripts/field.ts` draws its pixel equalizer.
- `src/content/docs/docs/` holds the docs, served under `/docs/`. They follow the README; change both together.
- `public/screenshots/` are the README's pictures, recorded from `cloofy demo`.
- `worker/index.ts` runs only for `/install.sh`: it serves the installer from `main`, so `curl -fsSL https://cloofy.app/install.sh | bash` always gets the current one. Everything else is static.
- The [Website workflow](../.github/workflows/website.yml) builds every change, and deploys from `main` once the repository has a `CLOUDFLARE_API_TOKEN` secret.
