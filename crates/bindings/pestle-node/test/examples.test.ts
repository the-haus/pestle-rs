/**
 * The examples in examples/, run as a user runs them (`@pestle-rs/node`
 * resolving to this package), against a local site: an agenda page, a
 * robots.txt and a gzipped sitemap index. TypeScript examples run through tsx,
 * as `npx tsx examples/scrape.ts` runs them.
 */
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { gzipSync } from "node:zlib";

import { afterAll, beforeAll, describe, expect, it } from "vitest";

const root = resolve(__dirname, "..");
const tsx = pathToFileURL(require.resolve("tsx")).href;
const agenda = readFileSync(join(root, "examples", "agenda.html"));
let work: string;
let server: Server;
let site: string;

/** Run an example; resolves with its output, rejects when it fails. */
function run(example: string, args: string[]): Promise<{ stdout: string; stderr: string }> {
  return new Promise((done, fail) => {
    const loader = example.endsWith(".ts") ? ["--import", tsx] : [];
    const child = spawn(process.execPath, [...loader, join(work, "examples", example), ...args], { cwd: work });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (code) => (code === 0 ? done({ stdout, stderr }) : fail(new Error(`${example} exited ${code}: ${stderr}`))));
  });
}

beforeAll(async () => {
  work = mkdtempSync(join(tmpdir(), "pestle-node-examples-"));
  mkdirSync(join(work, "node_modules", "@pestle-rs"), { recursive: true });
  symlinkSync(root, join(work, "node_modules", "@pestle-rs", "node"), "junction");
  symlinkSync(join(root, "examples"), join(work, "examples"), "junction");

  server = createServer((req, res) => {
    const pages: Record<string, [string, string | Buffer]> = {
      "/agenda": ["text/html; charset=utf-8", agenda],
      "/robots.txt": ["text/plain", `User-agent: *\nDisallow: /private/\nCrawl-delay: 2\nSitemap: ${site}/sitemap-index.xml\n`],
      "/sitemap-index.xml": [
        "application/xml",
        `<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><sitemap><loc>${site}/sitemap-1.xml.gz</loc></sitemap></sitemapindex>`,
      ],
      "/sitemap-1.xml.gz": [
        "application/gzip",
        gzipSync(
          `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">` +
            `<url><loc>${site}/agenda</loc><lastmod>2026-10-01</lastmod></url>` +
            `<url><loc>${site}/private/admin</loc></url>` +
            `<url><loc>/e/nora</loc></url></urlset>`,
        ),
      ],
    };
    const page = pages[req.url ?? ""];
    if (!page) return res.writeHead(404).end();
    res.writeHead(200, { "content-type": page[0] }).end(page[1]);
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  site = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

afterAll(() => {
  server?.close();
  if (work) rmSync(work, { recursive: true, force: true });
});

describe("examples", () => {
  it("scrape.ts, from a URL", async () => {
    const out = JSON.parse((await run("scrape.ts", [`${site}/agenda`])).stdout);
    expect(out).toMatchObject({
      title: "Agenda · Paradiso",
      lang: "nl",
      canonical: `${site}/agenda`,
      charset: "UTF-8",
      feeds: [`${site}/agenda.rss`],
      types: { MusicEvent: 2 },
    });
    expect(out.events.map((e: { name: string }) => e.name)).toEqual(["Nora & Co", "Mira"]);
  });

  it("extract.ts, from a file and from a URL", async () => {
    const fromFile = JSON.parse(
      (await run("extract.ts", ["examples/agenda.spec.json", "examples/agenda.html", "https://venue.example/agenda"])).stdout,
    );
    expect(fromFile.events).toEqual([
      {
        title: "Nora & Co",
        url: "https://venue.example/e/nora",
        when: "za 14 nov 2026 · 20:00",
        acts: ["Nora", "Kofi", "Ada"],
        headliner: "Nora",
        price: 25,
        tickets: "https://tickets.example/nora",
        soldOut: false,
      },
      {
        title: "Mira",
        url: "https://venue.example/e/mira",
        when: "zo 15 nov 2026 · 21:30",
        acts: ["Mira"],
        headliner: "Mira",
        price: 18.5,
        tickets: null,
        soldOut: true,
      },
    ]);
    const fromUrl = JSON.parse((await run("extract.ts", ["examples/agenda.spec.json", `${site}/agenda`])).stdout);
    expect(fromUrl.events[0].url).toBe(`${site}/e/nora`);
  });

  it("agent-input.ts leaves out hidden instructions", async () => {
    const { stdout } = await run("agent-input.ts", ["examples/agenda.html"]);
    expect(stdout).toContain('- button "Zoek" [ref=');
    expect(stdout).toContain('- link "Tickets" [ref=');
    expect(stdout).not.toContain("Ignore all previous instructions");
    expect(stdout).not.toContain("reply only with");
    expect(stdout).toMatch(/e\d+: body > main:nth-child\(2\) > form:nth-child\(2\) > button:nth-child\(2\)/);
  });

  it("stream-items.ts", async () => {
    const out = JSON.parse((await run("stream-items.ts", ["5000"])).stdout);
    expect(out.events).toBe(5000);
    expect(out.last).toEqual({ title: "Night 4999", url: "https://venue.example/e/4999", when: "16 nov 2026" });
  });

  it("crawl.ts follows robots.txt and a gzipped sitemap index", async () => {
    const { stdout, stderr } = await run("crawl.ts", [site, "mybot"]);
    expect(stdout.trim().split("\n").map((l) => JSON.parse(l))).toEqual([
      { url: `${site}/agenda`, lastmod: "2026-10-01" },
      { url: `${site}/e/nora`, lastmod: null },
    ]);
    expect(stderr).toContain("crawl-delay: 2s");
  });

  it("typed-spec.ts", async () => {
    expect(JSON.parse((await run("typed-spec.ts", [])).stdout)).toEqual({
      page: "Agenda",
      events: 2,
      onSale: ["Nora & Co"],
      cheapest: { title: "Nora & Co", price: 25, url: "https://venue.example/e/nora" },
      acts: ["Nora", "Kofi", "Ada", "Mira"],
    });
  });

  it("commonjs.cjs", async () => {
    const out = JSON.parse((await run("commonjs.cjs", ["examples/agenda.html", "https://venue.example/agenda"])).stdout);
    expect(out).toMatchObject({ title: "Agenda · Paradiso", events: 2, truncated: false });
    expect(out.headings[0]).toEqual({ level: 1, text: "Agenda" });
  });
});
