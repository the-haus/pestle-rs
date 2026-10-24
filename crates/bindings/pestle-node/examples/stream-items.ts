// A listing read as it arrives, each item handed over the moment it closes:
// a field with `emit: true` never keeps its items, so a page of any length
// reads in the memory of one item. Here a generated agenda of 100,000 events
// is streamed in small chunks; the items are counted, never held.
//
//   npx tsx stream-items.ts [events]

import { Extractor, type Item } from "@pestle-rs/node";

interface Event {
  title: string | null;
  url: string | null;
  when: string | null;
}

const n = Number(process.argv[2] ?? 100_000);

async function* agenda(): AsyncGenerator<string> {
  yield "<!doctype html><html><body><main>";
  for (let i = 0; i < n; i++) {
    yield `<article class="event"><h2><a href="/e/${i}">Night ${i}</a></h2><p class="date">${1 + (i % 28)} nov 2026</p></article>`;
  }
  yield "</main></body></html>";
}

const x = new Extractor({
  events: {
    select: "article.event",
    all: true,
    emit: true,
    fields: { title: "h2", url: { select: "a", attr: "href", url: "http" }, when: ".date" },
  },
});

let count = 0;
let last: Event | undefined;
const t = performance.now();
const { stats } = await x.stream(agenda(), {
  url: "https://venue.example/agenda",
  onItem: ({ field, value }: Item) => {
    if (field === "events") {
      count++;
      last = value as Event;
    }
  },
});
const ms = performance.now() - t;

console.log(
  JSON.stringify({
    events: count,
    last,
    bytes: stats.bytes,
    ms: Math.round(ms),
    heapMB: Math.round(process.memoryUsage().heapUsed / 1048576),
  }),
);
