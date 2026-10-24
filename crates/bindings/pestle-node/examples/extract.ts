// Extract what a spec names: CSS selectors pick the elements, each value is
// read as text, an attribute or markup (then as a URL, a number or JSON).
// The spec is a JSON file (see agenda.spec.json); prints the data as JSON.
//
//   npx tsx extract.ts agenda.spec.json https://venue.example/agenda
//   npx tsx extract.ts agenda.spec.json page.html [https://venue.example/agenda]

import { readFileSync } from "node:fs";

import { Extractor, SpecError, type Spec } from "@pestle-rs/node";

const [specFile, input, base] = process.argv.slice(2);
if (!specFile || !input) throw new Error("usage: extract.ts <spec.json> <url | file [url]>");

let extractor: Extractor;
try {
  // Compile once; an Extractor is safe to share between any number of pages at once.
  extractor = new Extractor(JSON.parse(readFileSync(specFile, "utf8")) as Spec);
} catch (e) {
  if (e instanceof SpecError) {
    console.error(`the spec does not compile:\n  ${e.errors.join("\n  ")}`);
    process.exit(1);
  }
  throw e;
}

const { data, errors } = /^https?:\/\//.test(input)
  ? await extractor.stream(await fetch(input))
  : extractor.extract(readFileSync(input), { url: base });

for (const e of errors) console.error(`warning: ${e}`);
console.log(JSON.stringify(data, null, 2));
