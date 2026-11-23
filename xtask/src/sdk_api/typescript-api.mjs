// The public API of the TypeScript SDK's packages, for `xtask sdk-api`
// (issue 148). Run from the workspace's folder, whose `node_modules` holds
// TypeScript and API Extractor, with one JSON argument: `tree`, the folder
// of the tree to read (the workspace's or the previous release's), `out`,
// the folder to write in, and `packages`, the folders of the packages under
// `tree`. A missing package folder is read as a package with no API.
//
// For each package, TypeScript writes the declarations of its sources
// without checking them (each source declares its own types, under
// `isolatedDeclarations`, so no dependency is read); then API Extractor
// reads each entry point of its `exports`, testing entry points included,
// and writes its API report, `<entry>.api.md`, and its model,
// `<entry>.api.json`. The items of every model go to `<out>/items.json`, one
// object per declaration and member:
//
// - `entry`, the entry point's import name, such as `@goodfetch/collie/testing`;
// - `top`, the name of the entry's declaration it is or belongs to;
// - `key`, its canonical reference within the entry (`TestEngine#close:member(1)`);
// - `signature`, its declaration's text, whitespace folded;
// - for a function, method or constructor, `parameters`, each one's type
//   text and whether it is optional, and `returns`, its return type's text;
// - `experimental`, whether its doc comment or an enclosing declaration's
//   has the `@experimental` tag;
// - `required_member`, whether it is a property an interface requires.
//
// The generated layer is read as the rest is: its entry points, such as
// `@goodfetch/collie-client/generated`, and the declarations an entry
// re-exports from a generated file are public API a program imports.
import { Extractor, ExtractorConfig } from "@microsoft/api-extractor";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, relative, resolve } from "node:path";

const { tree, out, packages } = JSON.parse(process.argv[process.argv.length - 1]);
const workspace = process.cwd();
const require = createRequire(join(workspace, "package.json"));
const tsc = join(dirname(require.resolve("typescript/package.json")), "bin", "tsc");
const typeRoots = [join(workspace, "node_modules", "@types")];

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
const items = [];
// Each entry point of the packages read so far, to its declaration file: a
// package that re-exports another's entries, as Collie's does the Collie
// client package's, finds them there.
const paths = {};
for (const folder of packages) {
  const source = resolve(tree, folder);
  if (!existsSync(join(source, "package.json"))) continue;
  const manifest = JSON.parse(readFileSync(join(source, "package.json"), "utf8"));
  const declarations = join(out, folder, "declarations");
  declare(source, declarations);
  writeFileSync(join(declarations, "package.json"), JSON.stringify({ name: manifest.name }));
  const entries = Object.entries(manifest.exports ?? {}).map(([subpath, file]) => {
    const entry = subpath === "." ? manifest.name : manifest.name + subpath.slice(1);
    const declared = join(
      declarations,
      relative(join(source, "src"), resolve(source, file)),
    ).replace(/\.ts$/, ".d.ts");
    if (!existsSync(declared)) throw new Error(`${entry}: TypeScript wrote no ${declared}`);
    paths[entry] = [declared];
    return [entry, declared];
  });
  for (const [entry, declared] of entries) {
    const slug = entry.replace(/[@/]/g, "_");
    const model = join(out, folder, `${slug}.api.json`);
    extract(declarations, declared, join(out, folder), slug, model);
    const json = JSON.parse(readFileSync(model, "utf8"));
    for (const point of json.members) {
      for (const member of point.members) walk(member, entry, member.name, false, undefined);
    }
  }
}
writeFileSync(join(out, "items.json"), JSON.stringify(items));

// Writes the declarations of the package at `source` under `declarations`.
// tsc reports the errors of an unchecked program whose dependencies are
// missing, and writes the declarations all the same, so its status is not
// read; a missing entry's declaration is.
function declare(source, declarations) {
  try {
    execFileSync(
      process.execPath,
      [
        tsc,
        "-p",
        join(source, "tsconfig.json"),
        "--noEmit",
        "false",
        "--emitDeclarationOnly",
        "--noCheck",
        "--noEmitOnError",
        "false",
        "--rootDir",
        join(source, "src"),
        // The workspace's, which the previous release's tree lacks: without
        // `@types/node`, `Symbol.asyncDispose` would be unknown there.
        "--typeRoots",
        typeRoots.join(","),
        "--outDir",
        declarations,
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
  } catch {
    // Read through the entries' declarations below.
  }
}

// Runs API Extractor on one entry point's declaration file.
function extract(projectFolder, entryFile, reportFolder, slug, model) {
  const config = ExtractorConfig.prepare({
    configObject: {
      projectFolder,
      mainEntryPointFilePath: entryFile,
      compiler: {
        overrideTsconfig: {
          compilerOptions: {
            target: "es2024",
            lib: ["esnext"],
            module: "nodenext",
            moduleResolution: "nodenext",
            allowImportingTsExtensions: true,
            noEmit: true,
            skipLibCheck: true,
            types: ["node"],
            typeRoots,
            paths,
          },
          files: [entryFile],
        },
      },
      apiReport: {
        enabled: true,
        reportFolder,
        reportTempFolder: reportFolder,
        reportFileName: slug,
      },
      docModel: { enabled: true, apiJsonFilePath: model },
      dtsRollup: { enabled: false },
      tsdocMetadata: { enabled: false },
    },
    configObjectFullPath: undefined,
    packageJsonFullPath: join(projectFolder, "package.json"),
  });
  const result = Extractor.invoke(config, {
    localBuild: true,
    messageCallback: (message) => {
      message.handled = true;
    },
  });
  if (!existsSync(model)) {
    throw new Error(`API Extractor wrote no model for ${entryFile} (${result.errorCount} errors)`);
  }
}

// Adds `item` and its members to `items`.
function walk(item, entry, top, enclosingExperimental, parentKind) {
  const experimental = enclosingExperimental || /@experimental\b/.test(item.docComment ?? "");
  const reference = item.canonicalReference ?? item.name;
  const tokens = item.excerptTokens ?? [];
  items.push({
    entry,
    top,
    key: reference.slice(reference.indexOf("!") + 1),
    signature: text(tokens, { startIndex: 0, endIndex: tokens.length }),
    parameters: item.parameters?.map((parameter) => ({
      type: text(tokens, parameter.parameterTypeTokenRange),
      optional: parameter.isOptional,
    })),
    returns: item.returnTypeTokenRange && text(tokens, item.returnTypeTokenRange),
    experimental,
    required_member:
      parentKind === "Interface" && item.kind === "PropertySignature" && !item.isOptional,
  });
  for (const member of item.members ?? []) walk(member, entry, top, experimental, item.kind);
}

// The text of a range of a declaration's tokens, whitespace folded.
function text(tokens, range) {
  return tokens
    .slice(range.startIndex, range.endIndex)
    .map((token) => token.text)
    .join("")
    .replace(/\s+/g, " ")
    .trim();
}
