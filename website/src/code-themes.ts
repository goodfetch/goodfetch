// The code blocks' colours (ADR 0073): Prism's GitHub theme in light mode and
// Dracula in dark mode, with every token colour under WCAG 2.2 AA's 4.5:1
// on the block's background moved toward the text colour until it meets it.
import { themes, type PrismTheme } from "prism-react-renderer";

/** WCAG 2.2 AA's contrast ratio for normal text. */
const minimum = 4.5;

type Rgb = readonly [number, number, number];

/** Reads a `#rgb`, `#rrggbb` or `rgb(r, g, b)` colour. */
function rgb(colour: string): Rgb {
  const channels = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/.exec(colour.trim());
  if (channels !== null) {
    return [Number(channels[1]), Number(channels[2]), Number(channels[3])];
  }
  const digits = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(colour.trim())?.[1];
  if (digits === undefined) {
    throw new Error(`${colour} is no #hex or rgb() colour`);
  }
  const full = digits.length === 3 ? digits.replaceAll(/./g, "$&$&") : digits;
  const channel = (at: number) => Number.parseInt(full.slice(at, at + 2), 16);
  return [channel(0), channel(2), channel(4)];
}

/** WCAG 2.2's linear value of an sRGB channel. */
function linear(channel: number): number {
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2.2's relative luminance. */
function luminance([red, green, blue]: Rgb): number {
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

/** WCAG 2.2's contrast ratio between two colours. */
export function contrast(foreground: string, background: string): number {
  const [first, second] = [luminance(rgb(foreground)), luminance(rgb(background))];
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

function hex(colour: Rgb): string {
  return `#${colour.map((channel) => Math.round(channel).toString(16).padStart(2, "0")).join("")}`;
}

/** `colour` moved toward `text` by the least whole percent that meets the minimum on `background`. */
function meeting(colour: string, text: string, background: string): string {
  if (contrast(colour, background) >= minimum) {
    return colour;
  }
  const [from, to] = [rgb(colour), rgb(text)];
  const mix = (index: 0 | 1 | 2, share: number) => from[index] + (to[index] - from[index]) * share;
  for (let percent = 1; percent <= 100; percent++) {
    const share = percent / 100;
    const mixed = hex([mix(0, share), mix(1, share), mix(2, share)]);
    if (contrast(mixed, background) >= minimum) {
      return mixed;
    }
  }
  return text;
}

/** `theme`, with each token colour under 4.5:1 on its background moved toward its text colour. */
export function legible(theme: PrismTheme): PrismTheme {
  const { color: text, backgroundColor: background } = theme.plain;
  if (text === undefined || background === undefined) {
    throw new Error("a code theme names its text and background colours");
  }
  return {
    plain: theme.plain,
    styles: theme.styles.map(({ types, style }) => ({
      types,
      style:
        style.color === undefined
          ? style
          : { ...style, color: meeting(style.color, text, background) },
    })),
  };
}

/** The code blocks' themes, for `themeConfig.prism`. */
export const codeThemes: { readonly theme: PrismTheme; readonly darkTheme: PrismTheme } = {
  theme: legible(themes.github),
  darkTheme: legible(themes.dracula),
};
