// Runs protoc with protoc-gen-es over the project's .proto files: the
// frontend's Connect client, generated as one Bazel action
// (//frontend/react:proto_es).
// Usage: protoc.mjs <protoc> <include dir> <out dir> <protos...>, every path
// relative to the execroot (js_binary itself runs from the bin directory).
import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { delimiter, dirname, resolve } from "node:path";

const execroot = process.env.JS_BINARY__EXECROOT ?? process.cwd();
const [protoc, include, outDir, ...protos] = process.argv.slice(2).map((p) => resolve(execroot, p));
const plugin = createRequire(import.meta.url).resolve("@bufbuild/protoc-gen-es/bin/protoc-gen-es");
mkdirSync(outDir, { recursive: true });
const run = spawnSync(
  protoc,
  [`--plugin=protoc-gen-es=${plugin}`, `--es_out=${outDir}`, "--es_opt=target=ts", `-I${include}`, ...protos],
  {
    stdio: "inherit",
    // The plugin starts with `#!/usr/bin/env node`: this node, not the host's.
    env: { ...process.env, PATH: dirname(process.execPath) + delimiter + (process.env.PATH ?? "") },
  },
);
process.exit(run.status ?? 1);
