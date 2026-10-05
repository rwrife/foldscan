export interface OcrReviewBlock {
  text: string;
  confidence: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface OcrReviewDocument {
  capture_id: string;
  requested_languages: string[];
  frame_width: number;
  frame_height: number;
  blocks: OcrReviewBlock[];
}

export interface OcrReview {
  schema: string;
  documents: OcrReviewDocument[];
}

export type OcrReviewResult =
  | { status: "ok"; review: OcrReview }
  | { status: "err"; error: { category: string; message: string } };

/** Only append inert text nodes. Never interpolate recognized text as markup. */
export function renderOcrReview(container: HTMLElement, review: OcrReview): void {
  container.replaceChildren();
  for (const doc of review.documents) {
    const section = document.createElement("section");
    const heading = document.createElement("h3");
    heading.textContent = `Capture ${doc.capture_id}`;
    const meta = document.createElement("p");
    meta.textContent = `Languages: ${doc.requested_languages.join(", ")}. Frame: ${doc.frame_width} × ${doc.frame_height} pixels.`;
    section.append(heading, meta);
    if (doc.blocks.length === 0) {
      const blank = document.createElement("p");
      blank.textContent = "No recognized text blocks.";
      section.append(blank);
    }
    const list = document.createElement("ol");
    for (const block of doc.blocks) {
      const item = document.createElement("li");
      const text = document.createElement("p");
      text.textContent = block.text;
      const details = document.createElement("p");
      details.textContent = `Confidence ${block.confidence} per 1000; box x ${block.x}, y ${block.y}, width ${block.width}, height ${block.height} pixels.`;
      item.append(text, details);
      list.append(item);
    }
    if (doc.blocks.length > 0) section.append(list);
    container.append(section);
  }
}
