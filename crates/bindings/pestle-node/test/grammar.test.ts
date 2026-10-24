import { describe, expect, it } from "vitest";

import { Grammar, GrammarError, hasGrammars, ParseError, setCallLimit } from "../ts/index.js";

// pest grammars are in builds with the `grammar` feature (`yarn build:grammar`).
const enabled = hasGrammars();
const grammar = (source: string) => (enabled ? new Grammar(source) : (null as unknown as Grammar));

const time = grammar(`
  time = { #hour = num ~ ":" ~ #minute = num }
  num  = @{ ASCII_DIGIT{2} }
`);

it.skipIf(enabled)("says so when this build has no grammars", () => {
  expect(() => new Grammar("a = { \"a\" }")).toThrow(/no pest grammars/);
});

describe.skipIf(!enabled)("Grammar", () => {
  it("knows its rules and tags", () => {
    expect(time.rules).toEqual([
      { name: "time", kind: "normal" },
      { name: "num", kind: "atomic" },
    ]);
    expect(time.tags).toEqual(["hour", "minute"]);
    expect(time.defaultRule).toBe("time");
  });

  it("parses into pairs like pest's", () => {
    const tree = time.parse("20:30 tail");
    expect(tree.end).toBe(5);
    const [root] = tree.pairs;
    expect(root!.rule).toBe("time");
    expect(root!.children.map((p) => [p.rule, p.tag, p.text])).toEqual([
      ["num", "hour", "20"],
      ["num", "minute", "30"],
    ]);
    expect(tree.find("minute")!.text).toBe("30");
    expect(tree.flatten()).toHaveLength(3);
    expect(JSON.parse(JSON.stringify(tree))[0].children[0]).toEqual({ rule: "num", tag: "hour", text: "20", start: 0, end: 2 });
  });

  it("shapes values by tags", () => {
    expect(time.shape("20:30")).toEqual({ hour: "20", minute: "30" });
    expect(time.shape("20:30", { numbers: ["hour"], lists: ["minute"] })).toEqual({ hour: 20, minute: ["30"] });
    expect(time.shape("20:30", { byRule: true })).toEqual({ num: ["20", "30"] });
  });

  it("finds matches in text, offsets in UTF-16 units", () => {
    const text = "Deuren 🎸 19:30 · aanvang 20:15";
    const m = time.find(text)!;
    expect(m.text).toBe("19:30");
    expect(text.slice(m.start, m.end)).toBe("19:30");
    expect(m.tree.pairs[0]!.start).toBe(m.start);
    expect(time.findAll(text).map((f) => f.text)).toEqual(["19:30", "20:15"]);
    expect(time.findAll(text, { limit: 1 })).toHaveLength(1);
    expect(time.find("geen tijd")).toBeNull();
  });

  it("reports parse errors with positions", () => {
    expect(time.matches("20:30")).toBe(true);
    expect(time.matches("20-30")).toBe(false);
    try {
      time.parse("é 20:3x".slice(2));
      expect.unreachable();
    } catch (e) {
      expect(e).toBeInstanceOf(ParseError);
      const p = e as ParseError;
      expect([p.line, p.col, p.offset]).toEqual([1, 4, 3]);
      expect(p.expected).toEqual(["num"]);
      expect(p.message).toContain("expected num");
    }
  });

  it("reports grammar errors", () => {
    expect(() => new Grammar("a = { b ~ }")).toThrow(GrammarError);
    try {
      new Grammar("a = { nope }");
    } catch (e) {
      expect((e as GrammarError).errors[0]!.message).toContain("nope");
    }
  });

  it("recovers from errors when asked", () => {
    const lineup = new Grammar(`
      WHITESPACE = _{ " " | "\\n" }
      lineup = { SOI ~ act* ~ EOI }
      act    = { #name = name ~ "@" ~ #time = time ~ ";" }
      name   = @{ (ASCII_ALPHA | " ")+ }
      time   = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }
    `);
    const input = "Nora @ 20:00; Mira @ 2l:30; Lu @ 23:00;";
    expect(() => lineup.parse(input)).toThrow(ParseError);
    const tree = lineup.parse(input, { recover: { rules: ["act"], sync: [";"], consume: true } });
    expect(tree.recovered).toHaveLength(1);
    const [r] = tree.recovered;
    expect(input.slice(r!.start, r!.end)).toBe("Mira @ 2l:30;");
    expect(r!.error.message).toContain("act broke here");
    expect(tree.flatten().map((p) => p.rule)).toContain("ERROR");
    expect(tree.findAll("name").map((p) => p.text.trim())).toEqual(["Nora", "Lu"]);
  });

  it("stops a runaway parse instead of hanging or crashing", () => {
    const g = new Grammar(`a = { ("x" | "x")* ~ "y" }`);
    setCallLimit(1000);
    try {
      expect(() => g.parse("x".repeat(10_000))).toThrow(/call limit/);
    } finally {
      setCallLimit(null);
    }
    const deep = new Grammar(`a = { "(" ~ a? ~ ")" }`);
    expect(() => deep.parse("(".repeat(200_000) + ")".repeat(200_000))).toThrow(ParseError);
  });
});
