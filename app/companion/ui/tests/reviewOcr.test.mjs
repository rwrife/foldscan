import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { renderOcrReview } from "../src/reviewOcr.ts";

test("recognized text is inert and confidence/coordinates have text equivalents", () => {
  const dom = new JSDOM("<main id='results'></main>");
  globalThis.document = dom.window.document;
  const root = document.getElementById("results");
  renderOcrReview(root, {
    schema: "foldscan.companion.ocr_review/0.1",
    documents: [{
      capture_id: "p1", requested_languages: ["eng"], frame_width: 40, frame_height: 50,
      blocks: [{ text: "<img src=x onerror=alert(1)>", confidence: 753,
        x: 2, y: 3, width: 20, height: 10 }],
    }],
  });
  assert.equal(root.querySelector("img"), null);
  assert.match(root.textContent, /<img src=x onerror=alert\(1\)>/);
  assert.match(root.textContent, /Confidence 753 per 1000; box x 2, y 3, width 20, height 10/);
  renderOcrReview(root, { schema: "foldscan.companion.ocr_review/0.1", documents: [] });
  assert.equal(root.textContent, "");
});
