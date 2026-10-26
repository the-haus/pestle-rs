// Native module loader of the ESM build: native.js (see there) with
// `createRequire`, which bundlers leave alone, so a bundle loads the
// binary at run time from the platform package or PESTLE_RS_NODE_BINARY.

import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const { platform, arch } = process;
// `require` is not used by name: bundlers replace it in ESM output with a
// stub that throws. In a bundle converted to CommonJS `import.meta.url` is
// empty, and `__filename` is the bundle.
const here = import.meta.url ? dirname(fileURLToPath(import.meta.url)) : undefined;
const load = createRequire(import.meta.url || __filename);

function isMuslLibc() {
  try {
    const header = process.report?.getReport()?.header;
    if (header) return !header.glibcVersionRuntime;
  } catch {
    /* ignore */
  }
  try {
    return readFileSync("/usr/bin/ldd", "utf8").includes("musl");
  } catch {
    return false;
  }
}

function candidateTriples() {
  if (platform === "linux") {
    const libc = isMuslLibc() ? "musl" : "gnu";
    return [`${platform}-${arch}-${libc}`, `${platform}-${arch}`];
  }
  if (platform === "win32") {
    return [`${platform}-${arch}-msvc`, `${platform}-${arch}`];
  }
  return [`${platform}-${arch}`];
}

function loadNative() {
  const override = process.env.PESTLE_RS_NODE_BINARY;
  if (override) return load(override);
  const triples = candidateTriples();
  const localCandidates = [...triples.map((t) => `pestle-node.${t}.node`), "pestle-node.node"];
  // dist/esm/native.js: the binaries are two levels up.
  for (const name of here ? localCandidates : []) {
    const candidate = join(here, "..", "..", name);
    if (existsSync(candidate)) {
      return load(candidate);
    }
  }
  const subpackageNames = triples.map((t) => `@pestle-rs/node-${t}`);
  for (const name of subpackageNames) {
    try {
      return load(name);
    } catch (err) {
      if (err && err.code !== "MODULE_NOT_FOUND") throw err;
    }
  }
  throw new Error(
    `@pestle-rs/node: no native binary found for ${platform}-${arch}. ` +
      `Tried local: ${localCandidates.join(", ")}. ` +
      `Tried npm: ${subpackageNames.join(", ")}. ` +
      "Set PESTLE_RS_NODE_BINARY to the binary's path, or, for local development, run `yarn build` in crates/bindings/pestle-node.",
  );
}

const native = loadNative();

// Every value ts/native.d.ts declares (test/package.test.ts holds the two loaders to the same names).
export const {
  NativeGrammar,
  NativeExtractor,
  NativeSession,
  NativeShaper,
  NativeRobots,
  NativeRouteBuilder,
  NativeRoutes,
  NativeSitemapReader,
  hasGrammars,
  publicSuffix,
  publicSuffixListVersion,
  readSitemapAsync,
  registrableDomain,
  setCallLimit,
  templateHash,
} = native;
