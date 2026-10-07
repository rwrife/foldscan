import assert from "node:assert/strict";
import test from "node:test";
import { exportCommands } from "../src/exportMode.ts";

test("PDF mode uses explicit commands and originals mode stays unchanged", () => {
  assert.deepEqual(exportCommands(true), { preview: "preview_pdf_export_plan", execute: "execute_pdf_export_plan" });
  assert.deepEqual(exportCommands(false), { preview: "preview_export_plan", execute: "execute_export_plan" });
});
