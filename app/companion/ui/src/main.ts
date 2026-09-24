import { invoke } from "@tauri-apps/api/core";

/** Must match `CompanionStatus` in `src-tauri/src/main.rs`. */
interface CompanionStatus {
  schema: string;
  app_version: string;
  domain_protocol: string;
}

const statusElement = document.getElementById("status");
const refreshButton = document.getElementById("refresh");

function render(text: string): void {
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

async function refreshStatus(): Promise<void> {
  render("Checking companion status…");
  try {
    const status = await invoke<CompanionStatus>("companion_status");
    render(describe(status));
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    render(`Companion status unavailable: ${reason}`);
  }
}

if (refreshButton instanceof HTMLButtonElement) {
  refreshButton.addEventListener("click", () => {
    void refreshStatus();
  });
}

void refreshStatus();
