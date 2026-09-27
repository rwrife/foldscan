import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import {
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

const statusElement = document.getElementById("status");
const refreshButton = document.getElementById("refresh");
const importForm = document.getElementById("import-form");
const importPath = document.getElementById("import-path");
const chooseFolderButton = document.getElementById("choose-folder");
const importStatus = document.getElementById("import-status");
const importResults = document.getElementById("import-results");
const reviewPlanElement = document.getElementById("review-plan");

let currentSummary: ImportSummary | null = null;
let currentPlan: ReviewPlan | null = null;

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

    reviewPlanElement.append(section);
  }
}

function clearImportDetails(): void {
  currentSummary = null;
  currentPlan = null;
  if (importResults) {
    importResults.replaceChildren();
  }
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

if (reviewPlanElement) {
  reviewPlanElement.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLButtonElement) || !currentPlan) {
      return;
    }

    const { action, sessionId, captureId } = target.dataset;
    if (!action || !sessionId || !captureId) {
      return;
    }

    switch (action) {
      case "move-up":
        currentPlan = moveCapture(currentPlan, sessionId, captureId, -1);
        break;
      case "move-down":
        currentPlan = moveCapture(currentPlan, sessionId, captureId, 1);
        break;
      case "remove":
        currentPlan = removeCapture(currentPlan, sessionId, captureId);
        break;
      case "restore":
        currentPlan = restoreCapture(currentPlan, sessionId, captureId);
        break;
      default:
        return;
    }

    renderReviewPlan();
  });
}

renderReviewPlan();
void refreshStatus();
