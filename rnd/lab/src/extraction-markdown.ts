// Markdown structure oracle (D-markdown). A Markdown document is reduced to structural elements, and a prediction is scored
// against the ground truth by multiset matching of those elements, per type. The same parser (marked 18, GFM) reads both
// sides, so a converter's choice of syntax (ATX or setext headings, '*' or '-' bullets, pipe or HTML tables, fenced or
// indented code) does not matter; only the structure and the text it carries do.
//
// Element keys, after normalization (entities decoded, inline markup dropped, lowercase, whitespace collapsed):
//   heading  text of each heading, any level            list   text of each list item, nested lists excluded
//   table    text of each table cell (pipe or HTML table) code   content of each code block (fenced or indented)
//   link     "text -> href" of each inline link
// The structure score of a page is the micro F1 over heading, list, table and code elements. Links are scored apart: the
// WebMainBench ground truth drops nearly all of them, so they are only meaningful on the curated fixtures.
import { Lexer, type Token, type Tokens } from 'marked';

export const structureTypes = ['heading', 'list', 'table', 'code'] as const;
export const elementTypes = [...structureTypes, 'link'] as const;
export type ElementType = (typeof elementTypes)[number];
export type Elements = Record<ElementType, string[]>;

const named: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', ndash: '–', mdash: '—', hellip: '…', copy: '©', reg: '®', laquo: '«', raquo: '»', rsquo: '’', lsquo: '‘', rdquo: '”', ldquo: '“', middot: '·', bull: '•', times: '×' };
export function decodeEntities(text: string) {
  return text.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z]+);/gi, (whole, body: string) => {
    if (body[0] === '#') { const code = body[1] === 'x' || body[1] === 'X' ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10); return Number.isFinite(code) && code > 0 && code < 0x110000 ? String.fromCodePoint(code) : whole; }
    return named[body.toLowerCase()] ?? whole;
  });
}
export const normalize = (text: string) => decodeEntities(text).normalize('NFKC').replace(/\\([\\`*_{}[\]()#+\-.!|<>~])/g, '$1').toLowerCase().replace(/\s+/g, ' ').trim();

// Plain text of inline tokens: markup dropped, link text kept, images and raw HTML tags dropped.
function inlineText(tokens: Token[] | undefined, raw = ''): string {
  if (!tokens) return raw;
  return tokens.map(token => {
    switch (token.type) {
      case 'image': return '';
      case 'html': return '';
      case 'br': return ' ';
      case 'codespan': return (token as Tokens.Codespan).text;
      case 'escape': return (token as Tokens.Escape).text;
      default: { const t = token as { tokens?: Token[]; text?: string }; return t.tokens ? inlineText(t.tokens) : t.text ?? ''; }
    }
  }).join('');
}
function inlineLinks(tokens: Token[] | undefined, out: string[]) {
  for (const token of tokens ?? []) {
    if (token.type === 'link') { const link = token as Tokens.Link; out.push(`${normalize(inlineText(link.tokens))} -> ${decodeEntities(link.href).trim()}`); }
    const nested = (token as { tokens?: Token[] }).tokens; if (nested && token.type !== 'link') inlineLinks(nested, out);
  }
}
// Cells of HTML tables written inside Markdown (the WebMainBench ground truth keeps some tables as HTML).
function htmlTableCells(html: string, out: string[]) {
  for (const table of html.match(/<table[\s\S]*?<\/table>/gi) ?? []) for (const cell of table.match(/<t[dh](?=[\s>])[^>]*>[\s\S]*?(?=<t[dh][\s>]|<\/t[dh]>|<\/tr>|<\/table>)/gi) ?? []) {
    const text = normalize(cell.replace(/<[^>]*>/g, ' ').replace(/[*_`]/g, '')); if (text) out.push(text);
  }
}

export function markdownElements(markdown: string): Elements {
  const elements: Elements = { heading: [], list: [], table: [], code: [], link: [] };
  const push = (type: ElementType, text: string) => { const value = type === 'code' ? text : normalize(text); if (value) elements[type].push(value); };
  const walk = (tokens: Token[]) => {
    for (const token of tokens) {
      switch (token.type) {
        case 'heading': { const h = token as Tokens.Heading; push('heading', inlineText(h.tokens, h.text)); inlineLinks(h.tokens, elements.link); break; }
        case 'list': for (const item of (token as Tokens.List).items) {
          const own = item.tokens.filter(child => child.type !== 'list');
          push('list', own.map(child => child.type === 'text' || child.type === 'paragraph' ? inlineText((child as Tokens.Text).tokens, (child as Tokens.Text).text) : '').join(' '));
          walk(item.tokens.filter(child => child.type === 'list'));
          for (const child of own) inlineLinks((child as { tokens?: Token[] }).tokens, elements.link);
          // Code blocks, tables and HTML inside an item are still structure.
          walk(own.filter(child => child.type !== 'text' && child.type !== 'paragraph'));
        } break;
        case 'table': { const t = token as Tokens.Table; for (const cell of [...t.header, ...t.rows.flat()]) { push('table', inlineText(cell.tokens, cell.text)); inlineLinks(cell.tokens, elements.link); } break; }
        // Code content keeps its lines: trailing spaces and surrounding blank lines dropped, nothing else.
        case 'code': { const text = (token as Tokens.Code).text.split('\n').map(line => line.trimEnd()).join('\n').trim(); if (text) elements.code.push(text); break; }
        case 'html': htmlTableCells((token as Tokens.HTML).text, elements.table); break;
        case 'paragraph': case 'text': { const tokens = (token as Tokens.Paragraph).tokens; inlineLinks(tokens, elements.link); for (const inner of tokens ?? []) if (inner.type === 'html') htmlTableCells((inner as Tokens.HTML).text, elements.table); break; }
        case 'blockquote': walk((token as Tokens.Blockquote).tokens); break;
        default: break;
      }
    }
  };
  // marked can throw on pathological input; an unparsable document has no structure.
  try { walk(new Lexer({ gfm: true }).lex(markdown)); } catch { /* left empty */ }
  return elements;
}

export type Counts = { matched: number; predicted: number; expected: number };
export function matchCounts(predicted: string[], expected: string[]): Counts {
  const left = new Map<string, number>(); for (const key of expected) left.set(key, (left.get(key) ?? 0) + 1);
  let matched = 0; for (const key of predicted) { const n = left.get(key) ?? 0; if (n > 0) { matched++; left.set(key, n - 1); } }
  return { matched, predicted: predicted.length, expected: expected.length };
}
export const f1Of = ({ matched, predicted, expected }: Counts) => { const p = predicted ? matched / predicted : 0, r = expected ? matched / expected : 0; return p + r > 0 ? 2 * p * r / (p + r) : 0; };

export type PageScore = { counts: Record<ElementType, Counts>; structure: Counts; structureF1: number | null };
// structureF1 is null when the ground truth has no structural element: the page says nothing about structure.
export function scoreMarkdown(predictedMarkdown: string, truthMarkdown: string): PageScore {
  const predicted = markdownElements(predictedMarkdown), truth = markdownElements(truthMarkdown);
  const counts = Object.fromEntries(elementTypes.map(type => [type, matchCounts(predicted[type], truth[type])])) as Record<ElementType, Counts>;
  const structure = structureTypes.reduce((sum, type) => ({ matched: sum.matched + counts[type].matched, predicted: sum.predicted + counts[type].predicted, expected: sum.expected + counts[type].expected }), { matched: 0, predicted: 0, expected: 0 });
  return { counts, structure, structureF1: structure.expected ? f1Of(structure) : null };
}
