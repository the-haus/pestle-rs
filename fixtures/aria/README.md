# Accessibility snapshots

Each page here has the snapshot Playwright 1.64.1 (Chromium) takes of it: `name.yaml` is
`page.ariaSnapshot()`, `name.ai.yaml` is `page.ariaSnapshot({ mode: "ai" })`.
`crates/pestle-html/tests/aria.rs` holds pestle to them.

Playwright loaded each page from `https://pestle.test/<name>` with JavaScript off. Images were
answered with a 1×1 PNG and every other request was refused. `<noscript>` was taken out
beforehand, since a page with scripts on hides it. To regenerate them (`npm i playwright@1.64.1`
and `npx playwright install chromium` first):

```sh
node fixtures/aria/playwright.mjs default <out dir> fixtures/aria/*.html
node fixtures/aria/playwright.mjs ai <out dir> fixtures/aria/*.html
```

The small pages are written to cover the rules: names, roles, states, forms, tables, hidden
content, style sheets, YAML quoting. The rest are copies of `fixtures/sites`.
