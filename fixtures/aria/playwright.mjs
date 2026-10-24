// Playwright's ariaSnapshot of each page: node ref.mjs <mode default|ai> <out dir> <files…>
// Each page is served at https://pestle.test/<name> with JavaScript off; images get a 1×1 PNG,
// every other request is refused.
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);
const [mode, out, ...files] = process.argv.slice(2);
fs.mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
const ctx = await browser.newContext({ javaScriptEnabled: false });
let current = "";
await ctx.route("**/*", (r) => {
  const req = r.request();
  if (req.isNavigationRequest() && req.frame() === req.frame().page().mainFrame())
    return r.fulfill({ status: 200, contentType: "text/html; charset=utf-8", body: current });
  if (req.resourceType() === "image") return r.fulfill({ status: 200, contentType: "image/png", body: PNG });
  return r.abort();
});
const times = [];
for (const f of files) {
  // A fresh page each time: Playwright numbers refs per page.
  const page = await ctx.newPage();
  current = fs.readFileSync(f, "utf8");
  const name = path.basename(f, ".html");
  const t0 = performance.now();
  await page.goto("https://pestle.test/" + encodeURIComponent(name), { waitUntil: "load" });
  const t1 = performance.now();
  const yaml = await page.ariaSnapshot({ mode });
  const t2 = performance.now();
  fs.writeFileSync(path.join(out, name + ".yaml"), yaml);
  times.push(`${name}\t${current.length}\t${(t1 - t0).toFixed(2)}\t${(t2 - t1).toFixed(2)}`);
  await page.close();
}
fs.writeFileSync(path.join(out, "_times.tsv"), times.join("\n") + "\n");
await browser.close();
