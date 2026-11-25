// The docs build's remark plugins that fail what Docusaurus only warns about
// (TESTING.md, Docs site and Docs code samples; ADR 0068):
// - `fenceLanguage`, a fenced block with no language: `text` names output
//   and prose;
// - `directivesUsed`, a directive no plugin rendered, such as a mistyped
//   admonition, which Docusaurus 3.10 logs and prints as text. It runs among
//   the `remarkPlugins`, after the admonitions plugin.

interface MarkdownNode {
  type: string;
  lang?: string | null | undefined;
  name?: string | undefined;
  attributes?: Record<string, unknown> | null | undefined;
  data?: unknown;
  position?: { start: { line: number } } | undefined;
  children?: MarkdownNode[] | undefined;
}

interface MarkdownFile {
  path: string;
}

type Transformer = (tree: MarkdownNode, file: MarkdownFile) => void;

function visit(node: MarkdownNode, each: (node: MarkdownNode) => void): void {
  each(node);
  for (const child of node.children ?? []) {
    visit(child, each);
  }
}

function where(file: MarkdownFile, node: MarkdownNode): string {
  return `${file.path}:${node.position?.start.line ?? 0}`;
}

/** Throws on the first fenced block that names no language. */
export function fenceLanguage(): Transformer {
  return (tree, file) => {
    visit(tree, (node) => {
      if (node.type === "code" && (node.lang ?? "") === "") {
        throw new Error(
          `${where(file, node)}: a fenced block names no language; use \`text\` for output and prose`,
        );
      }
    });
  };
}

const markers: Record<string, string> = {
  containerDirective: ":::",
  leafDirective: "::",
  textDirective: ":",
};

/**
 * Throws on the first directive no plugin rendered. A text directive with no
 * label or attributes, such as the `:30` of `10:30`, is text, as Docusaurus
 * reads it.
 */
export function directivesUsed(): Transformer {
  return (tree, file) => {
    visit(tree, (node) => {
      const marker = markers[node.type];
      if (marker === undefined || node.data !== undefined) {
        return;
      }
      const bare =
        node.type === "textDirective" &&
        Object.keys(node.attributes ?? {}).length === 0 &&
        (node.children ?? []).length === 0;
      if (!bare) {
        throw new Error(
          `${where(file, node)}: the directive \`${marker}${node.name ?? ""}\` is unused; no plugin renders it`,
        );
      }
    });
  };
}
