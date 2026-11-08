import type { Photo } from "./api";

const number = new Intl.NumberFormat("nb-NO", { maximumFractionDigits: 1 });
const whole = new Intl.NumberFormat("nb-NO");
const date = new Intl.DateTimeFormat("nb-NO", {
  day: "numeric",
  month: "short",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

/** Decimal units, the same way Finder counts. */
export function bytes(n: number): string {
  if (n < 1000) return `${n} byte`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = n;
  let unit = -1;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${number.format(value)} ${units[unit]}`;
}

export const count = (n: number) => whole.format(n);
export const images = (n: number) => `${whole.format(n)} ${n === 1 ? "bilde" : "bilder"}`;
export const when = (ms: number) => date.format(new Date(ms));

export const isLibraryPhoto = (photo: Photo) => photo.path.startsWith("bilder:asset:");

/** What to call a photo on screen. A library photo's `name` is an identifier and is never shown. */
export const photoTitle = (photo: Photo) =>
  !isLibraryPhoto(photo) ? photo.name : photo.modified ? when(photo.modified) : "Ukjent dato";

export function esc(text: string): string {
  return text.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}
