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
    (_, name) => `const ${name ?? "open"} = (...args) => globalThis.fixtureOpen(...args);`);
assert.doesNotMatch(code, /@tauri-apps\//);
let fixtureId = 0;
const settle = () => new Promise((resolve) => setImmediate(resolve));
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
  const pickers = [];
  globalThis.fixtureInvoke = (command, args) => {
    if (command === "companion_status") return Promise.resolve({ app_version: "test", domain_protocol: "test" });
    const pending = deferred();
    calls.push({ command, args, ...pending });
    return pending.promise;
  };
  globalThis.fixtureOpen = () => {
    const pending = deferred();
    pickers.push(pending);
    return pending.promise;
  };
  await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}#${++fixtureId}`);
  return { dom, document: dom.window.document, calls, pickers };
}
function importPath(document, path) {
  document.getElementById("import-path").value = path;
  document.getElementById("import-form").dispatchEvent(
    new document.defaultView.Event("submit", { bubbles: true, cancelable: true }),
  );
}
const summary = { status: "ok", summary: {
  device_id: "device", firmware_version: "fixture", session_count: 1,
  total_captures: 2, total_bytes: 144,
  sessions: [{ session_id: "session", capture_count: 2, total_bytes: 144,
    captures: [{ capture_id: "first", bytes: 72 }, { capture_id: "second", bytes: 72 }] }],
} };
function action(document, name) {
  const button = document.querySelector(`button[data-action="${name}"]`);
  assert.ok(button, name);
  button.click();
}
const preview = (digest) => ({ status: "ok", preview: {
  session_id: "session", capture_ids: ["first", "second"],
  files: [{ relative_path: `${digest}.png`, content_kind: "original", capture_id: "first" }],
  manifest_digest: digest,
} });

// ponytail: bundled UI and mocked Tauri boundary; add native WebView/device checks when hardware exists.
test("export preview ignores reordered, mode-changed, re-imported and superseded responses", async () => {
  const { dom, document, calls } = await ready();
  try {
    importPath(document, "/one");
    calls[0].resolve(summary);
    await settle();
    action(document, "preview-export");
    const old = calls.at(-1);
    action(document, "move-down");
    old.resolve(preview("stale-order"));
    await settle();
    assert.doesNotMatch(document.getElementById("export-preview-results").textContent, /stale-order/);

    action(document, "preview-export");
    const beforeMode = calls.at(-1);
    const pdf = document.querySelector('input[type="checkbox"][data-pdf-session-id]');
    pdf.click();
    beforeMode.reject(Error("obsolete failure"));
    await settle();
    assert.doesNotMatch(document.getElementById("export-preview-status").textContent, /obsolete failure/);

    action(document, "preview-export");
    const prior = calls.at(-1);
    assert.equal(prior.command, "preview_pdf_export_plan");
    action(document, "preview-export");
    const latest = calls.at(-1);
    latest.resolve(preview("latest"));
    await settle();
    prior.resolve(preview("older"));
    await settle();
    assert.match(document.getElementById("export-preview-results").textContent, /latest/);
    assert.doesNotMatch(document.getElementById("export-preview-results").textContent, /older/);

    action(document, "preview-export");
    const beforeImport = calls.at(-1);
    importPath(document, "/two");
    beforeImport.resolve(preview("stale-import"));
    await settle();
    assert.doesNotMatch(document.getElementById("export-preview-results").textContent, /stale-import/);
  } finally { dom.window.close(); }
});

test("changed intent cancels pending folder selection, but an already-started export reports receipt", async () => {
  const { dom, document, calls, pickers } = await ready();
  try {
    importPath(document, "/one");
    calls[0].resolve(summary);
    await settle();
    action(document, "export-to-folder");
    action(document, "remove");
    pickers[0].resolve("/destination/old");
    await settle();
    assert.equal(calls.length, 1);
    assert.match(document.getElementById("export-status").textContent, /Choose Export again/);

    action(document, "export-to-folder");
    document.querySelector('input[data-pdf-session-id]').click();
    pickers[1].reject(Error("old picker"));
    await settle();
    assert.equal(calls.length, 1);
    assert.doesNotMatch(document.getElementById("export-status").textContent, /old picker/);

    action(document, "export-to-folder");
    action(document, "export-to-folder");
    pickers[2].resolve("/destination/old");
    await settle();
    assert.equal(calls.length, 1);
    importPath(document, "/two");
    pickers[3].resolve("/destination/old");
    await settle();
    assert.equal(calls.length, 2); // second call is import, not export
    assert.equal(calls[1].command, "import_volume_summary");
    calls[1].resolve(summary);
    await settle();

    action(document, "export-to-folder");
    pickers[4].resolve("/destination/new");
    await settle();
    const execution = calls.at(-1);
    assert.equal(execution.command, "execute_export_plan");
    assert.equal(execution.args.volumePath, "/two");
    assert.deepEqual(execution.args.captureIds, ["first", "second"]);
    importPath(document, "/three");
    execution.resolve({ status: "ok", execution: {
      session_id: "session", files_written: 3, root: "/destination/new/session",
      capture_ids: ["first", "second"], manifest_digest: "receipt",
    } });
    await settle();
    assert.match(document.getElementById("export-status").textContent, /Export complete.*receipt/);
  } finally { dom.window.close(); }
});
