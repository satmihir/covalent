import type { Action } from "./api.ts";

/** LARGEST_TEXT is the biggest file the panel shows as text; larger ones are downloads. */
export const LARGEST_TEXT = 1024 * 1024;

const PICTURES = ["png", "jpg", "jpeg", "webp", "gif"];

/** Kind says how the panel shows a file: as a picture, as text it still has to decode, or
 *  only as a download. The server serves exactly these pictures inline. */
export function kind(path: string, size: number): "picture" | "text" | "download" {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  if (path.includes(".") && PICTURES.includes(extension)) return "picture";
  return size <= LARGEST_TEXT ? "text" : "download";
}

/** Decode reads a file as UTF-8 text, or returns null for one that is not text. */
export function decode(bytes: ArrayBuffer): string | null {
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return text.includes("\u0000") ? null : text;
  } catch {
    return null;
  }
}

/** Size writes a byte count the way a file list does. */
export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** Touched lists the files a reply created or changed, in the order it first touched them. */
export function touched(activity: Action[]): string[] {
  const writers = ["write_file", "edit_file", "generate_image"];
  const paths = activity
    .filter((a) => a.state === "done" && a.target && writers.includes(a.tool))
    .map((a) => a.target as string);
  return [...new Set(paths)];
}
