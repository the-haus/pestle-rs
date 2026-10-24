/**
 * The package as consumers load it: the built ESM and CommonJS entry points
 * (dist/esm, dist), each with its own native loader. The rest of the suite
 * runs the TypeScript sources, so only this catches a loader that leaves a
 * native export out (the ESM one did: `parseRobots` and the sitemap readers
 * threw on `import`).
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";

import { describe, expect, it } from "vitest";

const pkg = resolve(__dirname, "..");
const built = existsSync(join(pkg, "dist", "index.js")) && existsSync(join(pkg, "dist", "esm", "index.js"));

/** Every public export, and every native one, with a check that each works. */
const probe = (load: string) => `
  ${load}
  const html = '<html lang=en><title>T</title><a href="/a">A</a><script type="application/ld+json">{"@type":"Event","name":"E"}</script>';
  const robots = api.parseRobots("User-agent: *\\nDisallow: /x\\n", "bot");
  const sitemap = api.readSitemap('<urlset><url><loc>https://s.example/p</loc></url></urlset>');
  (async () => {
    const entries = [];
    for await (const e of api.parseSitemap('<urlset><url><loc>/q</loc></url></urlset>', { url: "https://s.example/" })) entries.push(e.url);
    const async = await api.readSitemapAsync('<urlset><url><loc>https://s.example/r</loc></url></urlset>');
    const page = api.extractPage(html, { url: "https://s.example/" });
    const scraped = await api.scrape(html, { url: "https://s.example/" });
    console.log(JSON.stringify({
      api: Object.keys(api).filter((k) => k !== "default" && k !== "__esModule").sort(),
      native: Object.keys(native).filter((k) => k !== "default" && native[k] !== undefined).sort(),
      robots: [robots.allows("/x/1"), robots.allows("/y"), api.Robots.allowAll().allows("/x"), api.Robots.disallowAll().allows("/y")],
      sitemap: [sitemap.entries[0].url, entries[0], async.entries[0].url],
      page: [page.title, page.links[0].href],
      scraped: scraped.structured.events[0].name,
      aria: api.ariaSnapshot(html).snapshot.split("\\n")[0],
      grammars: api.hasGrammars(),
    }));
  })();
`;

function run(file: string, script: string) {
  const r = spawnSync(process.execPath, ["--input-type=" + (file === "esm" ? "module" : "commonjs"), "-e", script], {
    cwd: pkg,
    encoding: "utf8",
  });
  if (r.status !== 0) throw new Error(`${file}: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

describe.skipIf(!built)("the built package (yarn build:ts)", () => {
  const esm = () =>
    run("esm", probe(`import * as api from "./dist/esm/index.js"; import * as native from "./dist/esm/native.js";`));
  const cjs = () => run("cjs", probe(`const api = require("./dist/index.js"); const native = require("./dist/native.js");`));

  it("works the same from ESM and CommonJS", () => {
    const [e, c] = [esm(), cjs()];
    expect(e).toEqual(c);
    expect(e).toMatchObject({
      robots: [false, true, true, false],
      sitemap: ["https://s.example/p", "https://s.example/q", "https://s.example/r"],
      page: ["T", "https://s.example/a"],
      scraped: "E",
      aria: '- link "A":',
    });
  });

  it("exports every native function and class from both loaders", () => {
    const { native } = esm();
    for (const name of ["NativeExtractor", "NativeSession", "NativeRobots", "NativeSitemapReader", "readSitemapAsync", "templateHash", "hasGrammars"]) {
      expect(native).toContain(name);
    }
    expect(native).toEqual(cjs().native);
  });

  it("resolves by its name, with ./package.json exported", () => {
    const r = spawnSync(
      process.execPath,
      ["-e", 'console.log(require.resolve("@pestle-rs/node"), require.resolve("@pestle-rs/node/package.json"))'],
      { cwd: pkg, encoding: "utf8" },
    );
    expect(r.status, r.stderr).toBe(0);
    expect(r.stdout).toContain(join("dist", "index.js"));
  });
});
