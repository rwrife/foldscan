export interface CapturePreview {
  schema: string;
  session_id: string;
  capture_id: string;
  width_px: number;
  height_px: number;
  bytes: number;
  data_uri: string;
}

export type CapturePreviewResult =
  | { status: "ok"; preview: CapturePreview }
  | { status: "err"; error: { category: string; message: string } };

function isBase64PngDataUri(uri: string): boolean {
  if (!uri.startsWith("data:image/png;base64,") || uri.length > 12 * 1024 * 1024) {
    return false;
  }
  const payload = uri.slice("data:image/png;base64,".length);
  if (payload.length === 0 || payload.length % 4 !== 0) {
    return false;
  }
  for (let i = 0; i < payload.length; i += 1) {
    const ch = payload.charCodeAt(i);
    const isAlphaNum =
      (ch >= 65 && ch <= 90) || // A-Z
      (ch >= 97 && ch <= 122) || // a-z
      (ch >= 48 && ch <= 57); // 0-9
    const isSymbol = ch === 43 || ch === 47; // + or /
    const isPad = ch === 61; // =
    if (isPad) {
      if (i < payload.length - 2) return false;
      if (i === payload.length - 2 && payload.charCodeAt(i + 1) !== 61) return false;
      continue;
    }
    if (!isAlphaNum && !isSymbol) {
      return false;
    }
  }
  return true;
}

export function renderCapturePreview(
  root: HTMLElement, sessionId: string, captureId: string, result: CapturePreviewResult,
): boolean {
  root.replaceChildren();
  const message = root.ownerDocument.createElement("p");
  if (result.status === "err") {
    message.textContent = `Image preview failed (${result.error.category}): ${result.error.message}`;
  } else {
    const p = result.preview;
    if (p.schema !== "foldscan.companion.capture_preview/0.1" ||
        p.session_id !== sessionId || p.capture_id !== captureId ||
        !Number.isSafeInteger(p.width_px) || !Number.isSafeInteger(p.height_px) ||
        p.width_px < 1 || p.height_px < 1 || p.width_px * p.height_px > 4 * 1024 * 1024 ||
        !Number.isSafeInteger(p.bytes) || p.bytes < 1 || p.bytes > 8 * 1024 * 1024 ||
        !isBase64PngDataUri(p.data_uri)) {
      message.textContent = "Invalid preview response. No image displayed.";
    } else {
      message.textContent = `Session ${sessionId}, capture ${captureId}: ${p.width_px} × ${p.height_px} pixels, ${p.bytes} source bytes.`;
      const image = root.ownerDocument.createElement("img");
      image.alt = `Capture ${captureId}, ${p.width_px} by ${p.height_px} pixel grayscale image`;
      image.width = p.width_px;
      image.height = p.height_px;
      image.src = p.data_uri;
      image.addEventListener("error", () => {
        if (!root.contains(image)) return;
        root.replaceChildren();
        message.textContent = "Image could not be displayed.";
        root.append(message);
      }, { once: true });
      root.append(message, image);
      return true;
    }
  }
  root.append(message);
  return false;
}
