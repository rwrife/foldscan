import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { build } from "vite";
import { JSDOM } from "jsdom";

const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
const bundle = await build({
  logLevel: "silent",
  build: {
    write: false,
    lib: { entry: new URL("../src/main.ts", import.meta.url).pathname, formats: ["es"] },
    rollupOptions: { external: ["@tauri-apps/api/core", "@tauri-apps/plugin-dialog"] },
  },
});
const code = (Array.isArray(bundle) ? bundle[0] : bundle).output[0].code
  .replace(/import\s*\{\s*invoke(?:\s+as\s+(\w+))?\s*\}\s*from\s*["']@tauri-apps\/api\/core["'];?/,
    (_, name) => `const ${name ?? "invoke"} = (...args) => globalThis.fixtureInvoke(...args);`)
  .replace(/import\s*\{\s*open(?:\s+as\s+(\w+))?\s*\}\s*from\s*["']@tauri-apps\/plugin-dialog["'];?/,
    (_, name) => `const ${name ?? "open"} = async () => null;`);
assert.doesNotMatch(code, /@tauri-apps\//, "Tauri imports must be replaced for fixture");
let bundleNumber = 0;

function deferred() {
  let resolve, reject;
  const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
  return { promise, resolve, reject };
}

async function ready() {
  const dom = new JSDOM(html, { url: "https://foldscan.local/" });
  for (const name of ["document", "HTMLButtonElement", "HTMLInputElement", "HTMLFormElement"]) {
    globalThis[name] = dom.window[name];
  }
  const calls = [];
  globalThis.fixtureInvoke = (command, args) => {
    if (command === "companion_status") return Promise.resolve({ app_version: "test", domain_protocol: "test" });
    const pending = deferred();
    calls.push({ command, args, ...pending });
    return pending.promise;
  };
  const url = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}#${++bundleNumber}`;
  await import(url);
  return { dom, calls, document: dom.window.document };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));
const summary = (session_id) => ({ status: "ok", summary: {
  device_id: "fixture", firmware_version: "fixture", session_count: 1,
  total_captures: 1, total_bytes: 72,
  sessions: [{ session_id, capture_count: 1, total_bytes: 72,
    captures: [{ capture_id: "cap-1", bytes: 72 }] }],
} });
const image = (session_id) => ({ status: "ok", preview: {
  schema: "foldscan.companion.capture_preview/0.1", session_id,
  capture_id: "cap-1", width_px: 3, height_px: 2, bytes: 72,
  data_uri: "data:image/png;base64,iVBORw0KGgo=",
} });

function importPath(document, path) {
  document.getElementById("import-path").value = path;
  document.getElementById("import-form").dispatchEvent(
    new document.defaultView.Event("submit", { bubbles: true, cancelable: true }),
  );
}
function preview(document, sessionId) {
  const button = [...document.querySelectorAll('button[data-action="preview-image"]')]
    .find((element) => element.dataset.sessionId === sessionId);
  assert.ok(button, `preview control for ${sessionId}`);
  button.click();
}

// ponytail: headless DOM and controlled Tauri responses; real WebView and mounted media need GUI/device acceptance.
test("only latest import wins; stale preview, errors and detached image errors stay inert", async () => {
  const { dom, calls, document } = await ready();
  try {
    importPath(document, "/old");
    importPath(document, "/new");
    const [oldImport, newImport] = calls;
    assert.deepEqual(calls.map((call) => call.command), ["import_volume_summary", "import_volume_summary"]);
    newImport.resolve(summary("new"));
    await settle();
    oldImport.reject(Error("obsolete failure"));
    await settle();
    assert.match(document.getElementById("import-status").textContent, /Import ready/);
    assert.match(document.getElementById("review-plan").textContent, /Session new/);
    assert.doesNotMatch(document.getElementById("review-plan").textContent, /Session old/);

    preview(document, "new");
    assert.equal(calls[2].args.volumePath, "/new");
    importPath(document, "/third");
    calls[2].resolve(image("new"));
    await settle();
    assert.equal(document.querySelector("#image-preview-results img"), null);
    calls[3].resolve(summary("third"));
    await settle();
    preview(document, "third");
    const first = calls[4];
    preview(document, "third");
    const second = calls[5];
    assert.equal(second.args.volumePath, "/third");
    second.resolve(image("third"));
    await settle();
    const shown = document.querySelector("#image-preview-results img");
    assert.ok(shown);
    first.reject(Error("old preview failed"));
    await settle();
    assert.equal(document.querySelector("#image-preview-results img"), shown);
    preview(document, "third");
    const newest = calls[6];
    newest.resolve(image("third"));
    await settle();
    shown.dispatchEvent(new dom.window.Event("error"));
    assert.ok(document.querySelector("#image-preview-results img"));
    assert.match(document.getElementById("image-preview-status").textContent, /ready/);
    document.querySelector("#image-preview-results img").dispatchEvent(new dom.window.Event("error"));
    assert.equal(document.querySelector("#image-preview-results img"), null);
    assert.match(document.getElementById("image-preview-status").textContent, /could not be displayed/);
  } finally {
    dom.window.close();
  }
});

test("out-of-order successful import cannot pair old plan with new path; changes invalidate previews", async () => {
  const { dom, calls, document } = await ready();
  try {
    importPath(document, "/old");
    importPath(document, "/new");
    calls[1].resolve(summary("new"));
    await settle();
    calls[0].resolve(summary("old"));
    await settle();
    preview(document, "new");
    assert.equal(calls[2].args.volumePath, "/new");
    document.querySelector('button[data-action="remove"]').click();
    calls[2].resolve(image("new"));
    await settle();
    assert.equal(document.querySelector("#image-preview-results img"), null);
    assert.match(document.getElementById("review-plan").textContent, /No active captures/);
  } finally {
    dom.window.close();
  }
});
