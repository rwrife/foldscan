import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { renderCapturePreview } from "../src/capturePreview.ts";

const valid = { status: "ok", preview: {
  schema: "foldscan.companion.capture_preview/0.1", session_id: "sess-001",
  capture_id: "cap-1", width_px: 3, height_px: 2, bytes: 72,
  data_uri: "data:image/png;base64,iVBORw0KGgo=",
} };

test("renders one accessible local image; errors and failures clear prior image", () => {
  const { window } = new JSDOM("<div id='preview'></div>");
  const root = window.document.getElementById("preview");
  assert.equal(renderCapturePreview(root, "sess-001", "cap-1", valid), true);
  const image = root.querySelector("img");
  assert.equal(image.width, 3);
  assert.equal(image.height, 2);
  assert.match(image.alt, /cap-1.*3.*2/);
  assert.equal(image.src, valid.preview.data_uri);
  assert.match(root.textContent, /72 source bytes/);
  image.dispatchEvent(new window.Event("error"));
  assert.equal(root.querySelector("img"), null);
  assert.match(root.textContent, /could not be displayed/);
  assert.equal(renderCapturePreview(root, "sess-001", "cap-1", valid), true);
  assert.equal(renderCapturePreview(root, "sess-001", "cap-1", { status: "err", error: { category: "checksum_mismatch", message: "changed" } }), false);
  assert.equal(root.querySelector("img"), null);
  assert.match(root.textContent, /checksum_mismatch.*changed/);
  window.close();
});

test("rejects mismatched IDs, bad bounds, oversized/unsafe image URIs", () => {
  const { window } = new JSDOM("<div id='preview'></div>");
  const root = window.document.getElementById("preview");
  for (const preview of [
    { ...valid.preview, capture_id: "cap-2" },
    { ...valid.preview, session_id: "other" },
    { ...valid.preview, width_px: 0 },
    { ...valid.preview, width_px: 4096, height_px: 4096 },
    { ...valid.preview, bytes: 9 * 1024 * 1024 },
    { ...valid.preview, data_uri: "https://example.test/image.png" },
    { ...valid.preview, data_uri: "data:image/svg+xml;base64,PHN2Zz4=" },
    { ...valid.preview, data_uri: "data:image/png;base64," + "A".repeat(12 * 1024 * 1024) },
  ]) {
    assert.equal(renderCapturePreview(root, "sess-001", "cap-1", { status: "ok", preview }), false);
    assert.equal(root.querySelector("img"), null);
    assert.match(root.textContent, /Invalid preview/);
  }
  window.close();
});
