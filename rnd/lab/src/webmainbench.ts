// WebMainBench's HTML was saved from its annotation tool: every element the annotators selected as main content carries
// cc-select="true" and the class token mark-selected, and the tool injected <style id="cc-extraStyle"> to highlight them.
// These are the answer key; they are removed so that no candidate can read them. The tool's other traces are kept, since
// they carry no selection: data-anno-uid (on selected and unselected elements alike) and the class cc-unloaded.
export function stripAnnotationMarkers(html: string): string {
  return html
    .replace(/<style\b[^>]*\bid="cc-extraStyle"[^>]*>[\s\S]*?<\/style>/g, '')
    .replace(/ cc-select="true"/g, '')
    .replace(/ class="([^"]*)"/g, (attribute, value: string) => {
      const tokens = value.split(/\s+/).filter(token => token && token !== 'mark-selected');
      if (tokens.length === value.split(/\s+/).filter(Boolean).length) return attribute;
      return tokens.length ? ` class="${tokens.join(' ')}"` : '';
    });
}

// Markers left after stripping; the prepare tool refuses to write a page that still has any.
export const annotationMarkers = (html: string): string[] =>
  ['cc-select', 'mark-selected', 'cc-extraStyle'].filter(marker => html.includes(marker));
