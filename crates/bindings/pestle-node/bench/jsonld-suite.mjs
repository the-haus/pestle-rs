// The W3C JSON-LD 1.1 toRdf suite: pestle's JSON-LD 1.1 processor (examples/jsonld_suite.rs)
// against each test's expected N-Quads, both canonicalized (RDFC-1.0); negative tests must fail
// with the expected error code. jsonld.js is scored for scale. Tests for JSON-LD 1.0 processing, rdfDirection, generalized RDF
// and expandContext are out of scope.
// node bench/jsonld-suite.mjs [test id] [-v]
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const canonize = require("rdf-canonize");
const jsonld = require("jsonld");

const root = new URL("../../../../", import.meta.url).pathname;
const dir = root + "bench/.work/suites/json-ld-api/tests/";
const only = process.argv[2] && !process.argv[2].startsWith("-") ? process.argv[2] : null;
const verbose = process.argv.includes("-v");

const out = execFileSync("cargo", ["run", "-q", "--release", "-p", "pestle-html", "--example", "jsonld_suite", "--", dir], {
  cwd: root,
  maxBuffer: 1 << 28,
  encoding: "utf8",
});
const results = new Map(out.trim().split("\n").map((l) => { const r = JSON.parse(l); return [r.id, r]; }));
const manifest = JSON.parse(readFileSync(dir + "toRdf-manifest.jsonld", "utf8"));
const canon = (nq) => canonize.canonize(nq, { algorithm: "RDFC-1.0", inputFormat: "application/n-quads", format: "application/n-quads", maxDeepIterations: Infinity });

const skipped = (o = {}) =>
  o.specVersion === "json-ld-1.0" || o.processingMode === "json-ld-1.0" || o.rdfDirection || o.produceGeneralizedRdf || o.expandContext;

const BASE = "https://w3c.github.io/json-ld-api/tests/";
const documentLoader = async (url) => {
  if (!url.startsWith(BASE)) throw new Error(`no ${url}`);
  return { documentUrl: url, document: JSON.parse(readFileSync(dir + url.slice(BASE.length), "utf8")) };
};
const reference = async (t) => {
  try {
    const doc = JSON.parse(readFileSync(dir + t.input, "utf8"));
    return { nq: await jsonld.toRDF(doc, { base: t.option?.base ?? BASE + t.input, documentLoader, format: "application/n-quads", safe: false }) };
  } catch (e) {
    return { error: e.details?.code ?? e.name };
  }
};
const verdict = async (t, r) => {
  if (t["@type"].includes("jld:NegativeEvaluationTest")) return r.error === t.expectErrorCode;
  if (r.error) return false;
  if (t["@type"].includes("jld:PositiveSyntaxTest")) return true;
  try { return (await canon(readFileSync(dir + t.expect, "utf8"))) === (await canon(r.nq)); } catch { return false; }
};

let pass = 0, refPass = 0, total = 0, skip = 0;
const fails = [];
for (const t of manifest.sequence) {
  if (only && t["@id"] !== only && t["@id"] !== "#t" + only) continue;
  if (skipped(t.option)) { skip++; continue; }
  total++;
  const r = results.get(t["@id"]);
  let ok = false, want = "", got = "";
  if (t["@type"].includes("jld:NegativeEvaluationTest")) {
    want = `error: ${t.expectErrorCode}`;
    got = r.error ? `error: ${r.error}` : r.nq;
    ok = r.error === t.expectErrorCode;
  } else if (r.error) {
    got = `error: ${r.error}`;
    want = t.expect ? readFileSync(dir + t.expect, "utf8") : "(no error)";
  } else if (t["@type"].includes("jld:PositiveSyntaxTest")) {
    ok = true;
  } else {
    try {
      want = await canon(readFileSync(dir + t.expect, "utf8"));
      got = await canon(r.nq);
      ok = want === got;
    } catch (e) {
      got = `canonicalize: ${e.message}\n${r.nq}`;
    }
  }
  if (await verdict(t, await reference(t))) refPass++;
  if (ok) pass++;
  else {
    fails.push(t["@id"]);
    if (verbose) console.log(`== ${t["@id"]} ${t.name}\n-- want\n${want}\n-- got\n${got}`);
  }
}
console.log(`JSON-LD 1.1 toRdf suite: pestle ${pass}/${total}, jsonld.js ${refPass}/${total} (${skip} out of scope)`);
if (!verbose && fails.length) console.log("failing:", fails.join(" "));
