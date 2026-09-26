import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

/** Must match `CompanionStatus` in `src-tauri/src/main.rs`. */
interface CompanionStatus {
  schema: string;
  app_version: string;
  domain_protocol: string;
}

interface ImportedSessionSummary {
  session_id: string;
  capture_count: number;
  total_bytes: number;
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

function renderImportResult(result: ImportResult): void {
  if (!importStatus || !importResults) {
    return;
  }

  importResults.replaceChildren();
  if (result.status === "err") {
    importStatus.textContent =
      `Import failed (${result.error.category}): ${result.error.message}`;
    return;
  }

  const summary = result.summary;
  importStatus.textContent =
    `Import ready for device ${summary.device_id}, firmware ${summary.firmware_version}: ` +
    `${summary.session_count} ${plural(summary.session_count, "session")}, ` +
    `${summary.total_captures} ${plural(summary.total_captures, "capture")}, ` +
    `${formatBytes(summary.total_bytes)} bytes. No source files were changed.`;

  const heading = document.createElement("h3");
  heading.textContent = "Sessions";
  importResults.append(heading);

  if (summary.sessions.length === 0) {
    const empty = document.createElement("p");
    empty.textContent = "No sessions found on this volume.";
    importResults.append(empty);
    return;
  }

  const list = document.createElement("ul");
  for (const session of summary.sessions) {
    const item = document.createElement("li");
    item.textContent =
      `${session.session_id}: ${session.capture_count} ` +
      `${plural(session.capture_count, "capture")}, ${formatBytes(session.total_bytes)} bytes`;
    list.append(item);
  }
  importResults.append(list);
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
  try {
    const result = await invoke<ImportResult>("import_volume_summary", {
      volumePath: path,
    });
    renderImportResult(result);
  } catch (error) {
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

void refreshStatus();
