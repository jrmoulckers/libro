/**
 * Pure chapter-HTML rewriter for the reader shell.
 *
 * A chapter's XHTML references its images/styles/fonts by relative path *inside
 * the zip*. Before we inject the chapter into a sandboxed iframe, those
 * references must point at something the browser can actually load — object URLs
 * the shell mints from the extracted zip entries.
 *
 * {@link rewriteChapterHtml} does that rewriting with `DOMParser` only. It takes a
 * `resolve` callback — `(inZipPath) => url | undefined` — so it stays pure and
 * fully unit-testable with a fake resolver: no zip, no `URL.createObjectURL`, no
 * `File`. The lazy {@link ./render} shell supplies the real resolver.
 *
 * It also strips `<script>` elements: the reader iframe is sandboxed without
 * `allow-scripts`, so book scripts never run anyway, but removing them keeps the
 * injected markup clean and avoids console noise.
 */

import { resolveRelativePath } from '../epub/opf';

/** Resolve an in-zip resource path to a loadable URL, or `undefined` to leave it. */
export type ResourceResolver = (inZipPath: string) => string | undefined;

/** Attributes that can reference a resource, by element (local) name. */
const REFERENCING_ATTRS: Record<string, string> = {
  img: 'src',
  image: 'href', // SVG <image>
  link: 'href', // stylesheets
  source: 'src',
  audio: 'src',
  video: 'src',
};

/**
 * Rewrite a chapter document's resource references to resolver-provided URLs and
 * strip scripts. Returns serialized HTML ready for an iframe `srcdoc`. Pure.
 *
 * @param html the chapter XHTML/HTML source
 * @param chapterPath the chapter's own in-zip path (used to resolve relatives)
 * @param resolve maps an in-zip path to a URL; unresolved refs are left as-is
 */
export function rewriteChapterHtml(
  html: string,
  chapterPath: string,
  resolve: ResourceResolver,
): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');

  for (const script of Array.from(doc.getElementsByTagName('script'))) {
    script.remove();
  }

  for (const [tag, attr] of Object.entries(REFERENCING_ATTRS)) {
    for (const el of Array.from(doc.getElementsByTagName(tag))) {
      rewriteAttr(el, attr, chapterPath, resolve);
      // SVG <image> may use the legacy xlink:href.
      if (tag === 'image' && !el.getAttribute('href')) {
        rewriteAttr(el, 'xlink:href', chapterPath, resolve);
      }
    }
  }

  return `<!doctype html>\n${doc.documentElement.outerHTML}`;
}

function rewriteAttr(
  el: Element,
  attr: string,
  chapterPath: string,
  resolve: ResourceResolver,
): void {
  const raw = el.getAttribute(attr);
  if (!raw || !isRelative(raw)) return;

  const [pathPart, fragment] = splitFragment(raw);
  const inZip = resolveRelativePath(chapterPath, pathPart);
  const url = resolve(inZip);
  if (url) el.setAttribute(attr, fragment ? `${url}${fragment}` : url);
}

/** Only rewrite document-relative refs — never absolute URLs or data/blob URIs. */
function isRelative(href: string): boolean {
  return !/^([a-z][a-z0-9+.-]*:|\/\/|#|data:|blob:)/i.test(href.trim());
}

function splitFragment(href: string): [string, string] {
  const hash = href.indexOf('#');
  return hash >= 0 ? [href.slice(0, hash), href.slice(hash)] : [href, ''];
}
