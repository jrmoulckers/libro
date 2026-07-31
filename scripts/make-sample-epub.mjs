// Generates a small, multi-chapter, public-domain sample EPUB into public/sample.epub
// so the in-app reader is demoable in a plain browser (`npm run dev`) without the
// Tauri backend. Text is Aesop's Fables — public domain. Run: `node scripts/make-sample-epub.mjs`.
import JSZip from "jszip";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const outDir = join(__dirname, "..", "public");
const outFile = join(outDir, "sample.epub");

const chapters = [
  {
    id: "ch1",
    title: "The Fox and the Grapes",
    body: `<p>A famished Fox saw some clusters of ripe black grapes hanging from a
      trellised vine. She resorted to all her tricks to get at them, but wearied
      herself in vain, for she could not reach them. At last she turned away,
      hiding her disappointment and saying: “The Grapes are sour, and not
      ripe as I thought.”</p>`,
  },
  {
    id: "ch2",
    title: "The Tortoise and the Hare",
    body: `<p>A Hare one day ridiculed the short feet and slow pace of the Tortoise,
      who replied, laughing: “Though you be swift as the wind, I will beat
      you in a race.” The Hare, believing her assertion to be simply
      impossible, assented to the proposal; and they agreed that the Fox should
      choose the course and fix the goal. On the day appointed for the race the
      two started together. The Tortoise never for a moment stopped, but went on
      with a slow but steady pace straight to the end of the course. The Hare,
      lying down by the wayside, fell fast asleep. At last waking up, and moving
      as fast as he could, he saw the Tortoise had reached the goal, and was
      comfortably dozing after her fatigue.</p>`,
  },
  {
    id: "ch3",
    title: "The Ant and the Grasshopper",
    body: `<p>In a field one summer’s day a Grasshopper was hopping about,
      chirping and singing to its heart’s content. An Ant passed by,
      bearing along with great toil an ear of corn he was taking to the nest.
      “Why not come and chat with me,” said the Grasshopper,
      “instead of toiling and moiling in that way?” “I am
      helping to lay up food for the winter,” said the Ant, “and
      recommend you to do the same.” When the winter came the Grasshopper
      had no food, and found itself dying of hunger, while it saw the ants
      distributing every day corn from the stores they had collected in the
      summer.</p>`,
  },
];

const xhtml = (title, inner) =>
  `<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="en" lang="en">
<head><meta charset="utf-8"/><title>${title}</title></head>
<body><h1>${title}</h1>${inner}</body>
</html>`;

const zip = new JSZip();

// mimetype MUST be first and stored (uncompressed).
zip.file("mimetype", "application/epub+zip", { compression: "STORE" });

zip.file(
  "META-INF/container.xml",
  `<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>`,
);

for (const c of chapters) {
  zip.file(`OEBPS/${c.id}.xhtml`, xhtml(c.title, c.body));
}

const manifestItems = chapters
  .map(
    (c) =>
      `    <item id="${c.id}" href="${c.id}.xhtml" media-type="application/xhtml+xml"/>`,
  )
  .join("\n");
const spineItems = chapters
  .map((c) => `    <itemref idref="${c.id}"/>`)
  .join("\n");

const navList = chapters
  .map((c) => `        <li><a href="${c.id}.xhtml">${c.title}</a></li>`)
  .join("\n");

zip.file(
  "OEBPS/nav.xhtml",
  `<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="en">
<head><meta charset="utf-8"/><title>Contents</title></head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Contents</h1>
    <ol>
${navList}
    </ol>
  </nav>
</body>
</html>`,
);

zip.file(
  "OEBPS/content.opf",
  `<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:uuid:libro-sample-aesop-0001</dc:identifier>
    <dc:title>The Fables of Aesop (sample)</dc:title>
    <dc:creator>Aesop</dc:creator>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">2024-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
${manifestItems}
  </manifest>
  <spine>
${spineItems}
  </spine>
</package>`,
);

const buf = await zip.generateAsync({
  type: "nodebuffer",
  mimeType: "application/epub+zip",
});
mkdirSync(outDir, { recursive: true });
writeFileSync(outFile, buf);
console.log(`Wrote ${outFile} (${buf.length} bytes)`);
