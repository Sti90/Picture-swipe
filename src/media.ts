import { api, fileUrl, type Photo } from "./api";
import { isLibraryPhoto } from "./format";

// Generating a thumbnail costs real work, so only a few run at once. Newest requests go
// first: whatever the user just scrolled to is what they are looking at.
const MAX_IN_FLIGHT = 4;

interface Job {
  path: string;
  resolve: (url: string) => void;
  reject: (reason: unknown) => void;
}

const thumbs = new Map<string, string>();
const pending: Job[] = [];
let inFlight = 0;

function pump() {
  while (inFlight < MAX_IN_FLIGHT && pending.length) {
    const job = pending.pop()!;
    inFlight += 1;
    api
      .thumbnail(job.path)
      .then((file) => {
        const url = fileUrl(file);
        thumbs.set(job.path, url);
        job.resolve(url);
      }, job.reject)
      .finally(() => {
        inFlight -= 1;
        pump();
      });
  }
}

/** Returns a promise for the thumbnail URL and a function that withdraws an unstarted request. */
export function requestThumb(path: string): { url: Promise<string>; cancel: () => void } {
  const cached = thumbs.get(path);
  if (cached) return { url: Promise.resolve(cached), cancel: () => {} };
  let job!: Job;
  const url = new Promise<string>((resolve, reject) => {
    job = { path, resolve, reject };
  });
  pending.push(job);
  pump();
  return {
    url,
    cancel: () => {
      const i = pending.indexOf(job);
      if (i >= 0) pending.splice(i, 1);
    },
  };
}

export function forgetMedia() {
  thumbs.clear();
  pending.length = 0;
  previews.clear();
  previewQueue.length = 0;
}

// Large images for the sweep view. The element is built ahead of time so the next photo
// is already decoded when the current one leaves.
const previews = new Map<string, HTMLImageElement>();

// A library preview can sit waiting on an iCloud download, so only a few run at once and
// photos the user has already skipped past are dropped before they start.
const MAX_LIBRARY_PREVIEWS = 3;
const previewQueue: { name: string; start: () => Promise<void> }[] = [];
let libraryPreviewsInFlight = 0;

function pumpLibraryPreviews() {
  while (libraryPreviewsInFlight < MAX_LIBRARY_PREVIEWS && previewQueue.length) {
    const job = previewQueue.shift()!;
    libraryPreviewsInFlight += 1;
    void job.start().finally(() => {
      libraryPreviewsInFlight -= 1;
      pumpLibraryPreviews();
    });
  }
}

export function previewImage(photo: Photo): HTMLImageElement {
  const existing = previews.get(photo.name);
  if (existing) return existing;
  const img = new Image();
  img.alt = "";
  img.draggable = false;
  img.decoding = "async";
  previews.set(photo.name, img);
  img.addEventListener("error", () => {
    img.dataset.failed = "1";
  });
  const start = () =>
    api.preview(photo.path).then(
      (file) => {
        img.src = fileUrl(file);
      },
      () => {
        img.dispatchEvent(new Event("error"));
      },
    );
  if (isLibraryPhoto(photo)) {
    previewQueue.push({ name: photo.name, start });
    pumpLibraryPreviews();
  } else {
    void start();
  }
  return img;
}

/** Drops a failed preview so the next request for it starts over. */
export function forgetPreview(name: string) {
  previews.delete(name);
}

/** True once the photo is actually on screen, as opposed to requested or failed. */
export function previewReady(name: string) {
  const img = previews.get(name);
  return !!img && !img.dataset.failed && !!img.getAttribute("src") && img.complete && img.naturalWidth > 0;
}

export function keepPreviews(names: string[]) {
  const order = new Map(names.map((name, i) => [name, i]));
  for (const name of previews.keys()) if (!order.has(name)) previews.delete(name);
  for (let i = previewQueue.length - 1; i >= 0; i -= 1) {
    if (!order.has(previewQueue[i].name)) previewQueue.splice(i, 1);
  }
  previewQueue.sort((a, b) => order.get(a.name)! - order.get(b.name)!);
}
