/** MAX_FILES and MAX_BYTES are the server's limits: files per message, and bytes per file. */
export const MAX_FILES = 10;
export const MAX_BYTES = 20 * 1024 * 1024;

/** Accept adds files to the ones already attached, within the limits, and returns a sentence
 *  about any it left out. A file already attached is not added twice. */
export function accept(current: File[], incoming: File[]): { files: File[]; refused: string | null } {
  const files = [...current];
  const reasons: string[] = [];
  for (const file of incoming) {
    if (files.some((f) => f.name === file.name && f.size === file.size && f.lastModified === file.lastModified)) continue;
    if (file.name.startsWith(".")) {
      reasons.push(`${file.name} can't be uploaded because its name starts with a dot.`);
    } else if (file.size > MAX_BYTES) {
      reasons.push(`${file.name} is over the 20 MB limit.`);
    } else if (files.length >= MAX_FILES) {
      reasons.push(`A message can carry at most ${MAX_FILES} files.`);
      break;
    } else {
      files.push(file);
    }
  }
  return { files, refused: reasons.length ? reasons.join(" ") : null };
}

/** Named gives a pasted picture, which browsers call image.png, a name with the time it was
 *  pasted, so several in one chat stay apart. Other files keep their names. */
export function named(file: File, now: Date): File {
  if (file.name !== "image.png") return file;
  const pad = (n: number) => String(n).padStart(2, "0");
  const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}`;
  return new File([file], `Pasted ${stamp}.png`, { type: file.type, lastModified: file.lastModified });
}
