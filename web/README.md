# Flowdren Next.js frontend

From the repository root:

- `npm --prefix web install`
- `npm --prefix web run dev`
- Open http://localhost:3000 (landing) or /demo (dashboard).
- `npm --prefix web run build` validates the production build and TypeScript.
- `npm --prefix web run start` serves the built app.

App Router pages live in app/; React components live in components/. The treasury dashboard uses React state. Phantom connection requests public-key access only, handles account changes and disconnects, and never signs transactions. An installed Phantom extension or Phantom browser is required. Other wallets are not yet supported.

All balances, approvals, stream changes and withdrawals are simulated and reset on reload. Contract integration is still blocked by the issues in ../READINESS.md. Legacy static HTML/JS files are archived in legacy/ and are not served by Next.js.
