// The W3C RDFa 1.1 test suite (HTML5 host language): pestle's statements against each test's
// expected graph, both canonicalized (URDNA2015), with rdfa-streaming-parser scored for scale.
// node bench/rdfa-suite.mjs [test number] [-v]
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { extractStructured } = require("../dist/index.js");
const N3 = require("n3");
const canonize = require("rdf-canonize");
const { RdfaParser } = require("rdfa-streaming-parser");

const dir = new URL("../../../../bench/.work/suites/rdfa/test-suite/test-cases/rdfa1.1/html5/", import.meta.url).pathname;
const only = process.argv[2] && !process.argv[2].startsWith("-") ? process.argv[2] : null;
const verbose = process.argv.includes("-v");

const term = (t) =>
  t.termType === "NamedNode" ? `<${t.value}>`
  : t.termType === "BlankNode" ? `_:${t.value.replace(/^_:/, "").replace(/[^A-Za-z0-9]/g, "x")}`
  : `"${t.value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\r/g, "\\r")}"` +
    (t.language ? `@${t.language}` : t.datatype && t.datatype.value !== "http://www.w3.org/2001/XMLSchema#string" ? `^^<${t.datatype.value}>` : "");
const nquads = (quads) => quads.map((q) => `${term(q.subject)} ${term(q.predicate)} ${term(q.object)} .\n`).join("");
const canon = async (quads) => canonize.canonize(nquads(quads), { algorithm: "RDFC-1.0", inputFormat: "application/n-quads", format: "application/n-quads" });

const ours = (html, base) => {
  const triples = extractStructured(html, { url: base, raw: true, lenient: false, jsonLd: false, microdata: false, microformats: false }).raw.rdfa;
  const node = (v) => (v.startsWith("_:") ? { termType: "BlankNode", value: v.slice(2) } : { termType: "NamedNode", value: v });
  return triples.map((t) => ({
    subject: node(t.subject),
    predicate: node(t.predicate),
    object:
      t.object.type === "literal"
        ? { termType: "Literal", value: t.object.value, language: t.object.language ?? "", datatype: t.object.datatype ? { value: t.object.datatype } : null }
        : node(t.object.value),
  }));
};

const reference = (html, base) =>
  new Promise((resolve, reject) => {
    const out = [];
    const p = new RdfaParser({ baseIRI: base, profile: "html", contentType: "text/html" });
    p.on("data", (q) => out.push(q)).on("error", reject).on("end", () => resolve(out));
    p.write(html);
    p.end();
  });

let pass = 0, refPass = 0, total = 0;
const fails = [];
for (const f of readdirSync(dir).filter((f) => f.endsWith(".html")).sort()) {
  const num = f.slice(0, -5);
  if (only && num !== only) continue;
  let ttl;
  try { ttl = readFileSync(dir + num + ".ttl", "utf8"); } catch { continue; }
  const base = `http://rdfa.info/test-suite/test-cases/rdfa1.1/html5/${f}`;
  const html = readFileSync(dir + f, "utf8");
  const expected = new N3.Parser({ baseIRI: base }).parse(ttl);
  total++;
  const want = await canon(expected);
  const got = await canon(ours(html, base));
  if (got === want) pass++;
  else {
    fails.push(num);
    if (verbose) console.log(`== ${num}\n-- want\n${want}-- got\n${got}`);
  }
  try { if ((await canon(await reference(html, base))) === want) refPass++; } catch {}
}
console.log(`RDFa 1.1 (HTML5) suite: pestle ${pass}/${total}, rdfa-streaming-parser ${refPass}/${total}`);
if (!verbose && fails.length) console.log("failing:", fails.join(" "));
if (fails.length) process.exitCode = 1;
