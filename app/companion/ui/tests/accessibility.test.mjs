import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import axe from "axe-core";
import { JSDOM, VirtualConsole } from "jsdom";

const indexUrl = new URL("../index.html", import.meta.url);
const wcagRunOptions = {
  runOnly: {
    type: "tag",
    values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"],
  },
  // jsdom has no rendered pixels/canvas. Contrast stays a documented GUI
  // acceptance gap rather than silently passing an inapplicable browser rule.
  rules: {
    "color-contrast": { enabled: false },
  },
};

async function loadCompanionDocument() {
  const html = await readFile(indexUrl, "utf8");
  const virtualConsole = new VirtualConsole();
  virtualConsole.on("jsdomError", (error) => {
    throw error;
  });
  const dom = new JSDOM(html, {
    runScripts: "dangerously",
    url: "https://foldscan.local/",
    virtualConsole,
  });
  dom.window.eval(axe.source);
  return dom;
}

function violationReport(violations) {
  return violations
    .map((violation) => {
      const nodes = violation.nodes
        .map((node) => `  ${node.target.join(" ")}: ${node.failureSummary}`)
        .join("\n");
      return `${violation.id} (${violation.impact ?? "unknown impact"}): ${violation.help}\n${nodes}\n  ${violation.helpUrl}`;
    })
    .join("\n\n");
}

async function assertNoWcagViolations(dom, state) {
  const results = await dom.window.axe.run(dom.window.document, wcagRunOptions);
  assert.equal(
    results.violations.length,
    0,
    `${state} has WCAG A/AA violations:\n\n${violationReport(results.violations)}`,
  );
}

function renderRepresentativeImportAndReview(document) {
  const importStatus = document.getElementById("import-status");
  const importResults = document.getElementById("import-results");
  const reviewPlan = document.getElementById("review-plan");
  const exportPreviewStatus = document.getElementById("export-preview-status");
  const exportPreviewResults = document.getElementById("export-preview-results");
  const exportStatus = document.getElementById("export-status");
  const exportResults = document.getElementById("export-results");
  assert.ok(importStatus && importResults && reviewPlan);
  assert.ok(exportPreviewStatus && exportPreviewResults);
  assert.ok(exportStatus && exportResults);

  importStatus.textContent =
    "Import ready for device fixture-device: 1 session, 2 captures. No source files were changed.";

  const importHeading = document.createElement("h3");
  importHeading.textContent = "Import summary";
  const importList = document.createElement("ul");
  const importItem = document.createElement("li");
  importItem.textContent = "fixture-session: 2 captures, 3,072 bytes";
  importList.append(importItem);
  importResults.replaceChildren(importHeading, importList);

  const note = document.createElement("p");
  note.textContent =
    "Review plan is in memory only. These controls do not modify source files.";

  const session = document.createElement("section");
  session.className = "review-session";
  const sessionHeading = document.createElement("h3");
  sessionHeading.textContent = "Session fixture-session";
  const orderedCaptures = document.createElement("ol");

  for (const [index, captureId] of ["capture-001", "capture-002"].entries()) {
    const item = document.createElement("li");
    const summary = document.createElement("span");
    summary.textContent = `${captureId} — ${index + 1},024 bytes`;
    const controls = document.createElement("div");
    controls.className = "capture-controls";

    for (const label of ["Move up", "Move down", "Remove"]) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      if ((label === "Move up" && index === 0) || (label === "Move down" && index === 1)) {
        button.disabled = true;
      }
      controls.append(button);
    }

    item.append(summary, controls);
    orderedCaptures.append(item);
  }

  session.append(sessionHeading, orderedCaptures);

  const removedHeading = document.createElement("h4");
  removedHeading.textContent = "Removed from export";
  const removed = document.createElement("ul");
  const removedItem = document.createElement("li");
  removedItem.textContent = "capture-003 — 4,096 bytes ";
  const restore = document.createElement("button");
  restore.type = "button";
  restore.textContent = "Restore";
  removedItem.append(restore);
  removed.append(removedItem);
  session.append(removedHeading, removed);

  const pdfLabel = document.createElement("label");
  const pdfCheckbox = document.createElement("input");
  pdfCheckbox.type = "checkbox";
  pdfLabel.append(pdfCheckbox, " Include ordered PDF (8-bit grayscale PNG captures only)");
  session.append(pdfLabel);

  const previewButton = document.createElement("button");
  previewButton.type = "button";
  previewButton.textContent = "Preview export for fixture-session";
  session.append(previewButton);

  const exportButton = document.createElement("button");
  exportButton.type = "button";
  exportButton.textContent = "Export fixture-session to folder…";
  session.append(exportButton);

  reviewPlan.replaceChildren(note, session);

  exportPreviewStatus.textContent =
    "Export preview ready for session fixture-session: 2 captures, 3 planned files. Manifest digest 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef. No files were written.";
  const previewHeading = document.createElement("h3");
  previewHeading.textContent = "Canonical files for fixture-session";
  const previewList = document.createElement("ul");
  const file1 = document.createElement("li");
  file1.textContent = "originals/fixture-session/capture-001.jpg (original, capture capture-001)";
  const file2 = document.createElement("li");
  file2.textContent = "export.json (manifest)";
  previewList.append(file1, file2);
  exportPreviewResults.replaceChildren(previewHeading, previewList);

  exportStatus.textContent =
    "Export complete for session fixture-session: 3 files written to /home/user/exports/foldscan-export-fixture-session-0123456789ab. Manifest digest 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef. Source files were not modified.";
  const exportHeading = document.createElement("h3");
  exportHeading.textContent = "Exported pages for fixture-session";
  const exportList = document.createElement("ol");
  for (const captureId of ["capture-001", "capture-002"]) {
    const item = document.createElement("li");
    item.textContent = captureId;
    exportList.append(item);
  }
  exportResults.replaceChildren(exportHeading, exportList);
}

test("the initial companion document has no detectable WCAG A/AA violations", async () => {
  const dom = await loadCompanionDocument();
  try {
    await assertNoWcagViolations(dom, "initial companion document");
  } finally {
    dom.window.close();
  }
});

test("a populated import and review state has no detectable WCAG A/AA violations", async () => {
  const dom = await loadCompanionDocument();
  try {
    renderRepresentativeImportAndReview(dom.window.document);
    await assertNoWcagViolations(dom, "populated import and review state");
  } finally {
    dom.window.close();
  }
});

test("the audit reports actionable evidence for an accessible-name regression", async () => {
  const dom = await loadCompanionDocument();
  try {
    const unlabeled = dom.window.document.createElement("button");
    unlabeled.id = "regression-canary";
    dom.window.document.querySelector("main")?.append(unlabeled);

    const results = await dom.window.axe.run(dom.window.document, wcagRunOptions);
    const violation = results.violations.find((entry) => entry.id === "button-name");
    assert.ok(violation, "axe must detect a button without an accessible name");
    assert.match(violationReport([violation]), /#regression-canary/);
  } finally {
    dom.window.close();
  }
});
