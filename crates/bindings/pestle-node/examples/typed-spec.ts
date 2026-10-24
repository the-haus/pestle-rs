// A spec with a typed result: describe the shape once, and `data` is typed
// everywhere it goes. Values that may be missing are `| null`; `all` fields
// are arrays.
//
//   npx tsx typed-spec.ts [page.html] [https://venue.example/agenda]

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { Extractor, type ExtractResult } from "@pestle-rs/node";

interface Event {
  title: string | null;
  url: string | null;
  when: string | null;
  acts: string[];
  price: number | null;
  soldOut: boolean;
}

interface Agenda {
  title: string | null;
  events: Event[];
}

const agenda = new Extractor<Agenda>({
  title: "h1",
  events: {
    select: "article.event",
    all: true,
    fields: {
      title: "h2",
      url: { select: "h2 a", attr: "href", url: "http" },
      when: ".date",
      acts: ["ul.acts > li"],
      price: { select: ".price", number: true },
      soldOut: { select: ".sold-out", exists: true },
    },
  },
});

const [file = fileURLToPath(new URL("agenda.html", import.meta.url)), url = "https://venue.example/agenda"] = process.argv.slice(2);
const result: ExtractResult<Agenda> = agenda.extract(readFileSync(file), { url });

const upcoming = result.data.events.filter((e) => !e.soldOut);
const cheapest = upcoming.reduce<Event | undefined>((a, e) => (e.price !== null && (a?.price == null || e.price < a.price) ? e : a), undefined);

console.log(
  JSON.stringify({
    page: result.data.title,
    events: result.data.events.length,
    onSale: upcoming.map((e) => e.title),
    cheapest: cheapest && { title: cheapest.title, price: cheapest.price, url: cheapest.url },
    acts: [...new Set(result.data.events.flatMap((e) => e.acts))],
  }),
);
