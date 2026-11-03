# wadspaces.com

The WadSpaces landing page: one static page that says what WadSpaces is, with no sign-in and no backend. React, Vite, TypeScript and Tailwind, on **Cloudflare Pages**.

```bash
npm install
npm run dev          # http://localhost:8081
npm run build        # typecheck, then dist/
npx wrangler pages deploy dist
```

## The domain

Every record lives in the `wadspaces.com` zone on Cloudflare:

| Name | Points at | Made by |
|---|---|---|
| `wadspaces.com`, `www` | this site (a Pages custom domain) | by hand, once |
| `<id>.wadspaces.com` | that machine's own tunnel (`<tunnel-id>.cfargotunnel.com`, proxied) | the installer stick, when it installs the machine (`apps/wadd/src/provision.rs`) |

- **There's no wildcard.** A name nobody made gets a clean "not found", never some machine's tunnel.
- **A machine's id** is chosen at install. It's DNS-safe and never one of the site's names (`www`, `api`, `mail`, `docs`, `status`, `app`, `admin`). Renaming the machine later doesn't touch DNS.
- **Streams** live under their machine's name: `https://<id>.wadspaces.com/ws-<guid>/`. They're served by the machine itself (wadd's proxy behind its tunnel), not by this site.
- **Machines never hold a DNS token.** Only the stick does, and it uses it once, at install. `wadd tunnel remove <id>`, run on the build computer with the secrets, retires a machine's tunnel and record.

## What machines need from here

Nothing. Machines sign in to GitHub through GitHub's device flow, and they find each other over the tailnet.
