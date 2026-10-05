// Browser preview server: serves desktop/ui/ as-is, adds the mock backend to index.html, nothing else changes.
// usage: node desktop/preview/serve.mjs [port]
import http from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const ui = path.join(here, "..", "ui");
const port = Number(process.argv[2]) || 4173;
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".png": "image/png", ".svg": "image/svg+xml", ".md": "text/plain" };

http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  let root = ui, rel = decodeURIComponent(url.pathname);
  if (rel.startsWith("/__preview/")) { root = here; rel = rel.slice("/__preview".length); }
  if (rel === "/") rel = "/index.html";
  const file = path.join(root, path.normalize(rel));
  if (!file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try {
    let body = await readFile(file);
    // String.replace with a string pattern changes the first match only
    if (root === ui && rel === "/index.html")
      body = Buffer.from(body.toString().replace("<script", '<script src="/__preview/mock.js"></script>\n<script'));
    res.writeHead(200, { "content-type": TYPES[path.extname(file)] || "application/octet-stream", "cache-control": "no-store" }).end(body);
  } catch { res.writeHead(404).end("not found"); }
}).listen(port, () => console.log(`preview on http://localhost:${port}/?as=owner&page=team`));
