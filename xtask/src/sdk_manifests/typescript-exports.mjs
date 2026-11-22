// The members the TypeScript SDK's packages export, for `xtask
// sdk-exports` (issue 2920). Run from the workspace's folder with the
// SDK's folder as its argument, it prints one JSON array of the members of
// each package's `exports` entries, read through the package's own
// `tsconfig.json` by TypeScript's checker:
//
// - every name an entry exports, on `(module)`;
// - each public property, method and accessor of an exported class, its
//   static ones included, and its constructor, declared or implicit;
// - each method of an exported interface, and its properties too when it
//   has a method: an interface of properties alone is a shape of data, which
//   its name's line describes.
//
// A member declared outside the SDK's folder, such as `Error`'s `message`
// or `Disposable`'s `[Symbol.dispose]`, is the language library's: it
// carries `library`, a line may name it and none has to. A member is
// generated when every declaration of it is under a `generated/` folder.
// Each member carries `id`, the same for one declaration however many
// entries export it, `home`, the folder of the package whose entry exports
// it, `package`, the folder of the package declaring it, `doc`, its TSDoc
// text without its tags, and `markers`, its tags' names, such as
// `experimental` and `deprecated`.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { API, SymbolFlags } from "typescript/unstable/sync";

// SyntaxKind's numbers of the nodes read here.
const PRIVATE_KEYWORD = 122;
const PROTECTED_KEYWORD = 123;
const PRIVATE_IDENTIFIER = 80;
const METHOD_SIGNATURE = 174;
const CONSTRUCTOR = "__constructor";
const MODULE = "(module)";

const sdk = resolve(process.argv[process.argv.length - 1]);
const inSdk = (path) => path.startsWith(sdk + sep) && !path.split(sep).includes("node_modules");
const generated = (paths) => paths.every((path) => path.split(sep).includes("generated"));
const idOf = (declarations) => declarations.map((d) => `${d.path}#${d.index}`).join(",");
const packageOf = (path) => relative(sdk, path).split(sep)[0];
const isPublic = (node) =>
  node.name?.kind !== PRIVATE_IDENTIFIER &&
  ![...(node.modifiers ?? [])].some(
    (modifier) => modifier.kind === PRIVATE_KEYWORD || modifier.kind === PROTECTED_KEYWORD,
  );

const api = new API({ cwd: process.cwd() });
const members = [];
try {
  const folders = readdirSync(sdk, { withFileTypes: true }).toSorted((a, b) =>
    a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
  );
  for (const folder of folders) {
    const dir = join(sdk, folder.name);
    const manifest = join(dir, "package.json");
    if (!folder.isDirectory() || !existsSync(manifest)) continue;
    const pkg = JSON.parse(readFileSync(manifest, "utf8"));
    const config = join(dir, "tsconfig.json");
    const project = api.updateSnapshot({ openProjects: [config] }).getProject(config);
    const { checker, emitter, program } = project;
    const nameOf = (node) => node.name.text ?? emitter.printNode(node.name);
    for (const [subpath, target] of Object.entries(pkg.exports ?? {})) {
      const entry = subpath === "." ? pkg.name : `${pkg.name}/${subpath.replace(/^\.\//, "")}`;
      const file = program.getSourceFile(resolve(dir, target));
      if (file === undefined) throw new Error(`${entry}: ${target} is not in ${config}`);
      for (const exported of checker.getExportsOfModule(checker.getSymbolAtLocation(file))) {
        const symbol =
          exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported;
        const paths = symbol.declarations.map((declaration) => declaration.path);
        const id = idOf(symbol.declarations);
        const add = (receiver, member, memberPaths, memberId, documented, library = false) =>
          members.push({
            entry,
            receiver,
            member,
            id: memberId,
            home: folder.name,
            package: library ? undefined : packageOf(memberPaths[0]),
            generated: !library && generated(memberPaths),
            library,
            doc: documented?.getDocumentationComment(checker) ?? "",
            markers: (documented?.getJsDocTags(checker) ?? []).map((tag) => tag.name),
          });
        add(MODULE, exported.name, paths, id, symbol);
        const isClass = (symbol.flags & SymbolFlags.Class) !== 0;
        if (!isClass && !(symbol.flags & SymbolFlags.Interface)) continue;
        const own = [];
        const library = [];
        for (const property of checker.getPropertiesOfType(
          checker.getDeclaredTypeOfSymbol(symbol),
        )) {
          const declarations = property.declarations.filter((d) => inSdk(d.path));
          const node = (declarations[0] ?? property.declarations[0])?.resolve();
          if (node === undefined || !isPublic(node)) continue;
          if (declarations.length === 0) library.push(nameOf(node));
          else own.push({ name: nameOf(node), node, declarations, symbol: property });
        }
        if (isClass) {
          for (const member of symbol.getExports().values()) {
            const node = member.declarations[0]?.resolve();
            if (member.flags & SymbolFlags.Prototype || node === undefined || !isPublic(node))
              continue;
            own.push({
              name: nameOf(node),
              node,
              declarations: member.declarations,
              symbol: member,
            });
          }
          const constructor = symbol.getMembers().get(CONSTRUCTOR);
          const node = constructor?.declarations[0]?.resolve();
          if (node === undefined || isPublic(node)) {
            own.push({
              name: "constructor",
              declarations: constructor?.declarations,
              symbol: constructor,
            });
          }
        } else if (!own.some(({ node }) => node.kind === METHOD_SIGNATURE)) {
          continue;
        }
        for (const name of library) add(exported.name, name, [], `${id}.${name}`, undefined, true);
        // An inherited member keeps the id of its declaration, so it is
        // listed once, on the receiver declaring it.
        for (const { name, declarations, symbol: documented } of own) {
          if (declarations === undefined)
            add(exported.name, name, paths, `${id}.${name}`, documented);
          else
            add(
              exported.name,
              name,
              declarations.map((d) => d.path),
              idOf(declarations),
              documented,
            );
        }
      }
    }
  }
} finally {
  api.close();
}
process.stdout.write(JSON.stringify(members));
