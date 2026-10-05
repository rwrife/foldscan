import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { renderOcrReview, type OcrReviewResult } from "./reviewOcr";

import {
  createExportExecutionRequest,
  createExportPreviewRequest,
  createReviewPlan,
  moveCapture,
  removeCapture,
  restoreCapture,
  type ReviewPlan,
} from "./reviewPlan";

/** Must match `CompanionStatus` in `src-tauri/src/main.rs`. */
interface CompanionStatus {
  schema: string;
  app_version: string;
  domain_protocol: string;
}

interface ImportedCaptureSummary {
  capture_id: string;
  bytes: number;
}

interface ImportedSessionSummary {
  session_id: string;
  capture_count: number;
  total_bytes: number;
  captures: ImportedCaptureSummary[];
}

interface ImportSummary {
  schema: string;
  device_id: string;
  firmware_version: string;
  session_count: number;
  total_captures: number;
  total_bytes: number;
  sessions: ImportedSessionSummary[];
}

type ImportResult =
  | { status: "ok"; summary: ImportSummary }
  | { status: "err"; error: { category: string; message: string } };

interface ExportPreviewFile {
  relative_path: string;
  capture_id: string | null;
  content_kind: string;
}

interface ExportPreview {
  schema: string;
  manifest_schema: string;
  session_id: string;
  capture_ids: string[];
  files: ExportPreviewFile[];
  manifest_digest: string;
}

type ExportPreviewResult =
  | { status: "ok"; preview: ExportPreview }
  | { status: "err"; error: { category: string; message: string } };

interface ExportExecutionSummary {
  schema: string;
  session_id: string;
  capture_ids: string[];
  root: string;
  files_written: number;
  manifest_sha256: string;
  manifest_digest: string;
}

type ExportExecutionResult =
  | { status: "ok"; execution: ExportExecutionSummary }
  | { status: "err"; error: { category: string; message: string } };

const statusElement = document.getElementById("status");
const refreshButton = document.getElementById("refresh");
const importForm = document.getElementById("import-form");
const importPath = document.getElementById("import-path");
const chooseFolderButton = document.getElementById("choose-folder");
const importStatus = document.getElementById("import-status");
const importResults = document.getElementById("import-results");
const reviewPlanElement = document.getElementById("review-plan");
const exportPreviewStatus = document.getElementById("export-preview-status");
const exportPreviewResults = document.getElementById("export-preview-results");
const exportStatus = document.getElementById("export-status");
const exportResults = document.getElementById("export-results");
const ocrForm = document.getElementById("ocr-form");
const ocrPath = document.getElementById("ocr-export-path");
const ocrDigest = document.getElementById("ocr-digest");
const ocrChooseFolder = document.getElementById("ocr-choose-folder");
const ocrStatus = document.getElementById("ocr-status");
const ocrResults = document.getElementById("ocr-results");
let ocrGeneration = 0;

let currentSummary: ImportSummary | null = null;
let currentPlan: ReviewPlan | null = null;
let currentVolumePath = "";

function renderStatus(text: string): void {
  if (statusElement) {
    statusElement.textContent = text;
  }
}

function describe(status: CompanionStatus): string {
  return (
    `Companion ${status.app_version} is ready. ` +
    `Domain protocol ${status.domain_protocol}. All operations are local.`
  );
}

function plural(count: number, singular: string): string {
  return count === 1 ? singular : `${singular}s`;
}

function formatBytes(bytes: number): string {
  return new Intl.NumberFormat().format(bytes);
}

function createActionButton(
  label: string,
  action: string,
  sessionId: string,
  captureId: string,
  disabled = false,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  button.dataset.action = action;
  button.dataset.sessionId = sessionId;
  button.dataset.captureId = captureId;
  button.disabled = disabled;
  return button;
}

function renderReviewPlan(): void {
  if (!reviewPlanElement) {
    return;
  }

  reviewPlanElement.replaceChildren();

  if (!currentPlan || !currentSummary) {
    const placeholder = document.createElement("p");
    placeholder.textContent =
      "Run an import check to review capture order. Review state is in memory only.";
    reviewPlanElement.append(placeholder);
    return;
  }

  const note = document.createElement("p");
  note.textContent =
    "Review plan is in memory only. Reorder/remove/restore does not modify source files and does not run export.";
  reviewPlanElement.append(note);

  for (const session of currentPlan.sessions) {
    const source = currentSummary.sessions.find((item) => item.session_id === session.sessionId);

    const section = document.createElement("section");
    section.className = "review-session";

    const heading = document.createElement("h4");
    heading.textContent = `Session ${session.sessionId}`;
    section.append(heading);

    const counts = document.createElement("p");
    counts.textContent =
      `${session.active.length} of ${source?.capture_count ?? session.active.length} ` +
      `${plural(source?.capture_count ?? session.active.length, "capture")} in export plan.`;
    section.append(counts);

    const activeList = document.createElement("ol");
    activeList.className = "capture-list";
    for (const [index, capture] of session.active.entries()) {
      const item = document.createElement("li");
      item.className = "capture-item";

      const summary = document.createElement("span");
      summary.textContent = `${capture.captureId} — ${formatBytes(capture.bytes)} bytes`;
      item.append(summary);

      const controls = document.createElement("div");
      controls.className = "capture-controls";
      controls.append(
        createActionButton(
          "Move up",
          "move-up",
          session.sessionId,
          capture.captureId,
          index === 0,
        ),
        createActionButton(
          "Move down",
          "move-down",
          session.sessionId,
          capture.captureId,
          index === session.active.length - 1,
        ),
        createActionButton("Remove", "remove", session.sessionId, capture.captureId),
      );
      item.append(controls);
      activeList.append(item);
    }

    if (session.active.length === 0) {
      const empty = document.createElement("p");
      empty.textContent = "No active captures remain in this export plan.";
      section.append(empty);
    } else {
      section.append(activeList);
    }

    if (session.removed.length > 0) {
      const removedHeading = document.createElement("h5");
      removedHeading.textContent = "Removed from export";
      section.append(removedHeading);

      const removedList = document.createElement("ul");
      removedList.className = "removed-list";
      for (const entry of session.removed) {
        const item = document.createElement("li");
        const summary = document.createElement("span");
        summary.textContent = `${entry.capture.captureId} — ${formatBytes(entry.capture.bytes)} bytes`;
        item.append(summary);
        item.append(
          createActionButton(
            "Restore",
            "restore",
            session.sessionId,
            entry.capture.captureId,
          ),
        );
        removedList.append(item);
      }
      section.append(removedList);
    }

    const previewButton = document.createElement("button");
    previewButton.type = "button";
    previewButton.textContent = `Preview export for ${session.sessionId}`;
    previewButton.dataset.action = "preview-export";
    previewButton.dataset.sessionId = session.sessionId;
    previewButton.disabled = session.active.length === 0;
    section.append(previewButton);

    const exportButton = document.createElement("button");
    exportButton.type = "button";
    exportButton.textContent = `Export ${session.sessionId} to folder…`;
    exportButton.dataset.action = "export-to-folder";
    exportButton.dataset.sessionId = session.sessionId;
    exportButton.disabled = session.active.length === 0;
    section.append(exportButton);

    reviewPlanElement.append(section);
  }
}

function clearExportPreview(): void {
  if (exportPreviewStatus) {
    exportPreviewStatus.textContent =
      "Run an import check, then preview a session's export layout.";
  }
  if (exportPreviewResults) {
    exportPreviewResults.replaceChildren();
  }
}

function renderExportPreview(result: ExportPreviewResult): void {
  if (!exportPreviewStatus || !exportPreviewResults) {
    return;
  }
  exportPreviewResults.replaceChildren();

  if (result.status === "err") {
    exportPreviewStatus.textContent = `Export preview failed (${result.error.category}): ${result.error.message}`;
    return;
  }

  const preview = result.preview;
  exportPreviewStatus.textContent =
    `Export preview ready for session ${preview.session_id}: ` +
    `${preview.capture_ids.length} ${plural(preview.capture_ids.length, "capture")}, ` +
    `${preview.files.length} planned files. Manifest digest ${preview.manifest_digest}. No files were written.`;

  const heading = document.createElement("h3");
  heading.textContent = `Canonical files for ${preview.session_id}`;
  const list = document.createElement("ul");
  for (const file of preview.files) {
    const item = document.createElement("li");
    const label = file.capture_id ? `${file.relative_path} (${file.content_kind}, capture ${file.capture_id})` : `${file.relative_path} (${file.content_kind})`;
    item.textContent = label;
    list.append(item);
  }
  exportPreviewResults.append(heading, list);
}

async function previewExport(sessionId: string): Promise<void> {
  if (!exportPreviewStatus || !exportPreviewResults) {
    return;
  }
  if (!currentPlan) {
    exportPreviewStatus.textContent = "Run an import check before previewing export.";
    return;
  }
  const session = currentPlan.sessions.find((s) => s.sessionId === sessionId);
  if (!session) {
    exportPreviewStatus.textContent = `Session ${sessionId} not found in review plan.`;
    return;
  }
  if (session.active.length === 0) {
    exportPreviewStatus.textContent = `Session ${sessionId} has no active captures in its review plan.`;
    return;
  }
  if (!currentVolumePath) {
    exportPreviewStatus.textContent = "Volume path is missing. Run an import check first.";
    return;
  }

  exportPreviewStatus.textContent = `Previewing canonical export layout for session ${sessionId}…`;
  exportPreviewResults.replaceChildren();

  const req = createExportPreviewRequest(currentVolumePath, session);
  try {
    const result = await invoke<ExportPreviewResult>("preview_export_plan", {
      volumePath: req.volumePath,
      sessionId: req.sessionId,
      captureIds: req.captureIds,
    });
    renderExportPreview(result);
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    exportPreviewStatus.textContent = `Export preview command unavailable: ${reason}`;
  }
}

function clearExportExecution(): void {
  if (exportStatus) {
    exportStatus.textContent =
      "Run an import check, review a session, then choose a destination folder to export it.";
  }
  if (exportResults) {
    exportResults.replaceChildren();
  }
}

function renderExportExecution(result: ExportExecutionResult): void {
  if (!exportStatus || !exportResults) {
    return;
  }
  exportResults.replaceChildren();

  if (result.status === "err") {
    exportStatus.textContent = `Export failed (${result.error.category}): ${result.error.message}`;
    return;
  }

  const execution = result.execution;
  exportStatus.textContent =
    `Export complete for session ${execution.session_id}: ` +
    `${execution.files_written} ${plural(execution.files_written, "file")} written to ${execution.root}. ` +
    `Manifest digest ${execution.manifest_digest}. Source files were not modified.`;

  const heading = document.createElement("h3");
  heading.textContent = `Exported pages for ${execution.session_id}`;
  const list = document.createElement("ol");
  for (const captureId of execution.capture_ids) {
    const item = document.createElement("li");
    item.textContent = captureId;
    list.append(item);
  }
  exportResults.append(heading, list);
}

async function exportSessionToFolder(sessionId: string): Promise<void> {
  if (!exportStatus || !exportResults) {
    return;
  }
  if (!currentPlan) {
    exportStatus.textContent = "Run an import check before exporting.";
    return;
  }
  const session = currentPlan.sessions.find((s) => s.sessionId === sessionId);
  if (!session) {
    exportStatus.textContent = `Session ${sessionId} not found in review plan.`;
    return;
  }
  if (session.active.length === 0) {
    exportStatus.textContent = `Session ${sessionId} has no active captures in its review plan.`;
    return;
  }
  if (!currentVolumePath) {
    exportStatus.textContent = "Volume path is missing. Run an import check first.";
    return;
  }

  exportStatus.textContent = "Opening the system destination folder picker…";
  exportResults.replaceChildren();
  let destinationPath: string;
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: `Choose the destination folder for session ${sessionId}`,
    });
    if (selected === null) {
      exportStatus.textContent = "Export cancelled. Nothing was written.";
      return;
    }
    destinationPath = selected;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    exportStatus.textContent = `Folder picker unavailable: ${reason}`;
    return;
  }

  exportStatus.textContent = `Exporting session ${sessionId} into ${destinationPath}…`;
  const req = createExportExecutionRequest(currentVolumePath, destinationPath, session);
  try {
    const result = await invoke<ExportExecutionResult>("execute_export_plan", {
      volumePath: req.volumePath,
      destinationPath: req.destinationPath,
      sessionId: req.sessionId,
      captureIds: req.captureIds,
    });
    renderExportExecution(result);
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    exportStatus.textContent = `Export command unavailable: ${reason}`;
  }
}

function clearImportDetails(): void {
  currentSummary = null;
  currentPlan = null;
  currentVolumePath = "";
  if (importResults) {
    importResults.replaceChildren();
  }
  clearExportPreview();
  clearExportExecution();
  renderReviewPlan();
}

function renderImportResult(result: ImportResult): void {
  if (!importStatus || !importResults) {
    return;
  }

  importResults.replaceChildren();
  if (result.status === "err") {
    clearImportDetails();
    importStatus.textContent =
      `Import failed (${result.error.category}): ${result.error.message}`;
    return;
  }

  const summary = result.summary;
  currentSummary = summary;
  currentPlan = createReviewPlan(summary);
  clearExportPreview();
  if (exportPreviewStatus) {
    exportPreviewStatus.textContent =
      summary.session_count === 0
        ? "The imported volume has no sessions to preview."
        : "Review a session, then choose its Preview export button.";
  }

  importStatus.textContent =
    `Import ready for device ${summary.device_id}, firmware ${summary.firmware_version}: ` +
    `${summary.session_count} ${plural(summary.session_count, "session")}, ` +
    `${summary.total_captures} ${plural(summary.total_captures, "capture")}, ` +
    `${formatBytes(summary.total_bytes)} bytes. No source files were changed.`;

  const heading = document.createElement("h3");
  heading.textContent = "Import summary";
  importResults.append(heading);

  const summaryList = document.createElement("ul");
  for (const session of summary.sessions) {
    const item = document.createElement("li");
    item.textContent =
      `${session.session_id}: ${session.capture_count} ` +
      `${plural(session.capture_count, "capture")}, ${formatBytes(session.total_bytes)} bytes`;
    summaryList.append(item);
  }
  importResults.append(summaryList);

  renderReviewPlan();
}

async function refreshStatus(): Promise<void> {
  renderStatus("Checking companion status…");
  try {
    const status = await invoke<CompanionStatus>("companion_status");
    renderStatus(describe(status));
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    renderStatus(`Companion status unavailable: ${reason}`);
  }
}

async function probeImport(path: string): Promise<void> {
  if (!importStatus || !importResults) {
    return;
  }

  importStatus.textContent = "Checking the selected removable-media path…";
  importResults.replaceChildren();
  clearImportDetails();
  currentVolumePath = path;
  try {
    const result = await invoke<ImportResult>("import_volume_summary", {
      volumePath: path,
    });
    renderImportResult(result);
  } catch (error) {
    clearImportDetails();
    const reason = error instanceof Error ? error.message : String(error);
    importStatus.textContent = `Import command unavailable: ${reason}`;
  }
}

if (refreshButton instanceof HTMLButtonElement) {
  refreshButton.addEventListener("click", () => {
    void refreshStatus();
  });
}

if (importForm instanceof HTMLFormElement && importPath instanceof HTMLInputElement) {
  importForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const path = importPath.value.trim();
    if (path.length === 0) {
      importPath.setCustomValidity("Enter the mounted volume path.");
      importPath.reportValidity();
      return;
    }
    importPath.setCustomValidity("");
    void probeImport(path);
  });
  importPath.addEventListener("input", () => importPath.setCustomValidity(""));
}

async function chooseImportFolder(): Promise<void> {
  if (
    !(importPath instanceof HTMLInputElement) ||
    !(chooseFolderButton instanceof HTMLButtonElement) ||
    !importStatus
  ) {
    return;
  }

  chooseFolderButton.disabled = true;
  importStatus.textContent = "Opening the system folder picker…";
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Choose the mounted FoldScan volume",
    });
    if (selected === null) {
      importStatus.textContent =
        "Folder selection cancelled. No import was started.";
      return;
    }

    importPath.value = selected;
    importPath.setCustomValidity("");
    importStatus.textContent =
      "Folder selected. Choose Check import path to validate it; no import has started.";
    importPath.focus();
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    importStatus.textContent = `Folder picker unavailable: ${reason}`;
  } finally {
    chooseFolderButton.disabled = false;
  }
}

if (chooseFolderButton instanceof HTMLButtonElement) {
  chooseFolderButton.addEventListener("click", () => {
    void chooseImportFolder();
  });
}

function clearOcrReview(): number {
  ocrGeneration += 1;
  ocrResults?.replaceChildren();
  if (ocrStatus) ocrStatus.textContent = "No export reviewed.";
  return ocrGeneration;
}

if (ocrForm instanceof HTMLFormElement &&
    ocrPath instanceof HTMLInputElement &&
    ocrDigest instanceof HTMLInputElement && ocrStatus && ocrResults) {
  ocrPath.addEventListener("input", clearOcrReview);
  ocrDigest.addEventListener("input", clearOcrReview);
  ocrForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const generation = clearOcrReview();
    const exportPath = ocrPath.value.trim();
    const reviewedDigest = ocrDigest.value.trim();
    if (!exportPath || !/^[0-9a-f]{64}$/.test(reviewedDigest)) {
      ocrStatus.textContent = "Enter an export directory and a reviewed lowercase SHA-256 digest.";
      return;
    }
    ocrStatus.textContent = "Verifying exported OCR locally…";
    void invoke<OcrReviewResult>("review_exported_ocr", { exportPath, reviewedDigest })
      .then((result) => {
        if (generation !== ocrGeneration) return;
        if (result.status === "err") {
          ocrStatus.textContent = `OCR review failed (${result.error.category}): ${result.error.message}`;
          return;
        }
        renderOcrReview(ocrResults, result.review);
        ocrStatus.textContent = `OCR review ready: ${result.review.documents.length} completed documents. Text is in memory only.`;
      })
      .catch(() => {
        if (generation === ocrGeneration) ocrStatus.textContent = "OCR review command unavailable.";
      });
  });
}

if (ocrChooseFolder instanceof HTMLButtonElement && ocrPath instanceof HTMLInputElement && ocrStatus) {
  ocrChooseFolder.addEventListener("click", () => {
    clearOcrReview();
    ocrChooseFolder.disabled = true;
    ocrStatus.textContent = "Opening the system export folder picker…";
    void open({ directory: true, multiple: false, title: "Choose a local FoldScan export" })
      .then((selected) => {
        if (selected === null) {
          ocrStatus.textContent = "Folder selection cancelled. No OCR was reviewed.";
        } else {
          ocrPath.value = selected;
          ocrStatus.textContent = "Folder selected. Enter the independent reviewed digest, then review.";
          ocrPath.focus();
        }
      })
      .catch(() => { ocrStatus.textContent = "Export folder picker unavailable."; })
      .finally(() => { ocrChooseFolder.disabled = false; });
  });
}

if (reviewPlanElement) {
  reviewPlanElement.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLButtonElement) || !currentPlan) {
      return;
    }

    const { action, sessionId, captureId } = target.dataset;
    if (!action || !sessionId) {
      return;
    }

    switch (action) {
      case "preview-export":
        void previewExport(sessionId);
        return;
      case "export-to-folder":
        void exportSessionToFolder(sessionId);
        return;
      case "move-up":
        if (!captureId) {
          return;
        }
        currentPlan = moveCapture(currentPlan, sessionId, captureId, -1);
        break;
      case "move-down":
        if (!captureId) {
          return;
        }
        currentPlan = moveCapture(currentPlan, sessionId, captureId, 1);
        break;
      case "remove":
        if (!captureId) {
          return;
        }
        currentPlan = removeCapture(currentPlan, sessionId, captureId);
        break;
      case "restore":
        if (!captureId) {
          return;
        }
        currentPlan = restoreCapture(currentPlan, sessionId, captureId);
        break;
      default:
        return;
    }

    clearExportPreview();
    if (exportPreviewStatus) {
      exportPreviewStatus.textContent =
        "Review plan changed. Preview the session again to refresh the canonical layout.";
    }
    renderReviewPlan();
  });
}

renderReviewPlan();
void refreshStatus();
