// Serve the repository's files over HTTP on this machine, for trying the
// web page: a page that loads WebAssembly has to come from a server, not
// straight from a file.
//
// usage: node tools/web-serve.js [port]
//
// Then open http://127.0.0.1:8765/web/index.html in a browser.

const fs = require("fs");
const http = require("http");
const path = require("path");

const root = path.resolve(__dirname, "..");
const port = Number(process.argv[2] ?? 8765);
const types = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
};

http
  .createServer((request, response) => {
    const name = decodeURIComponent(
      new URL(request.url, "http://localhost").pathname,
    );
    const file = path.join(root, name);
    // Nothing outside the repository is served.
    if (!file.startsWith(root + path.sep)) {
      response.writeHead(403).end();
      return;
    }
    fs.readFile(file, (error, contents) => {
      if (error) {
        response.writeHead(404).end("Not found");
        return;
      }
      const type = types[path.extname(file)] ?? "application/octet-stream";
      response.writeHead(200, {
        "Content-Type": type,
        "Cache-Control": "no-store",
      });
      response.end(contents);
    });
  })
  .listen(port, "127.0.0.1", () => {
    console.log(`http://127.0.0.1:${port}/web/index.html`);
  });
