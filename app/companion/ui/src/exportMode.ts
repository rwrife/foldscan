export interface ExportCommands {
  preview: "preview_pdf_export_plan" | "preview_export_plan";
  execute: "execute_pdf_export_plan" | "execute_export_plan";
}

export function exportCommands(pdf: boolean): ExportCommands {
  return pdf
    ? { preview: "preview_pdf_export_plan", execute: "execute_pdf_export_plan" }
    : { preview: "preview_export_plan", execute: "execute_export_plan" };
}
