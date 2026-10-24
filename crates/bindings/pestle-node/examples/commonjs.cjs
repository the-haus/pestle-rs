// The same package from CommonJS: `require` gives the same API as `import`.
//
//   node commonjs.cjs page.html [https://venue.example/page]

const { readFileSync } = require("node:fs");

const { extractPage, extractStructured, strip } = require("@pestle-rs/node");

const [input, url] = process.argv.slice(2);
if (!input) throw new Error("usage: node commonjs.cjs <file> [url]");
const html = readFileSync(input);

const page = extractPage(html, { url });
const { events } = extractStructured(html, { url });
// festimap's `cleanForLearn`: the page cut down to a model's budget.
const { tokens, truncated } = strip(html, { maxChars: 12_000 });

console.log(JSON.stringify({ title: page.title, headings: page.headings, events: events.length, tokens, truncated }));
