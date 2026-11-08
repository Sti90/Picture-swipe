import { api, errorText, LIBRARY_PATH, type Decision, type FolderState, type Photo } from "./api";
import { bytes, images } from "./format";
import { forgetMedia, previewReady } from "./media";

export type View = "sweep" | "review" | "kept";

interface Change {
  name: string;
  prev: Decision;
}
interface UndoEntry {
  changes: Change[];
  next: Decision;
}

export type Sheet =
  | { kind: "confirm"; busy: boolean; folder: string; names: string[]; bytes: number }
  | { kind: "failures"; failed: { name: string; error: string }[]; moved: number };

/** One-shot instruction telling the sweep view how the next photo change should animate. */
export type SweepHint = { mode: "out" | "in"; direction: "left" | "right" | "down" };

export const state = {
  folder: null as FolderState | null,
  loading: false,
  view: "sweep" as View,
  queue: [] as string[],
  undo: [] as UndoEntry[],
  selection: new Set<string>(),
  anchor: null as string | null,
  cursor: null as string | null,
  zoom: false,
  status: null as string | null,
  error: null as string | null,
  libraryAccess: null as "denied" | "restricted" | null,
  sheet: null as Sheet | null,
  sweepHint: null as SweepHint | null,
};

let byName = new Map<string, Photo>();
let writes: Promise<unknown> = Promise.resolve();
// What is known to be on disk, and how many writes are still queued, per photo.
let confirmed = new Map<string, Decision>();
const pending = new Map<string, number>();
let openRequest = 0;
const listeners = new Set<() => void>();

export const subscribe = (fn: () => void) => listeners.add(fn);
export const notify = () => listeners.forEach((fn) => fn());
export const photoByName = (name: string) => byName.get(name);
export const photos = () => state.folder?.photos ?? [];
export const withDecision = (d: Decision) => photos().filter((p) => p.decision === d);
export const currentPhoto = () => (state.queue[0] ? byName.get(state.queue[0]) : undefined);
export const viewDecision = (): Decision => (state.view === "kept" ? "keep" : "discard");
export const isLibrary = () => state.folder?.source === "library";

/** A library photo can't be judged until it is visible, which may take an iCloud download. */
export function canDecide() {
  const name = state.queue[0];
  return !!name && (!isLibrary() || previewReady(name));
}

export function counts() {
  const c = { undecided: 0, keep: 0, discard: 0, discardBytes: 0, keepBytes: 0 };
  for (const p of photos()) {
    c[p.decision] += 1;
    if (p.decision === "discard") c.discardBytes += p.size;
    if (p.decision === "keep") c.keepBytes += p.size;
  }
  return c;
}

// Nothing may change while a folder is loading or a confirmation is showing.
const locked = () => state.loading || !!state.sheet;

export async function openFolder(path: string) {
  if (state.sheet) return;
  const request = ++openRequest;
  state.loading = true;
  state.error = null;
  state.libraryAccess = null;
  state.status = null;
  notify();
  try {
    // Finish saving first, so the state read back includes every earlier decision.
    await writes;
    const folder = await api.openFolder(path);
    if (request !== openRequest) return;
    forgetMedia();
    state.folder = folder;
    byName = new Map(folder.photos.map((p) => [p.name, p]));
    confirmed = new Map(folder.photos.map((p) => [p.name, p.decision]));
    pending.clear();
    state.queue = folder.photos.filter((p) => p.decision === "undecided").map((p) => p.name);
    state.undo = [];
    state.view = "sweep";
    state.zoom = false;
    clearSelection();
    if (folder.stateReset) {
      state.error = "De lagrede valgene for denne mappen var skadet. Alle bildene vises derfor som usortert.";
    }
  } catch (e) {
    if (request !== openRequest) return;
    const access = path === LIBRARY_PATH ? await api.libraryStatus().catch(() => null) : null;
    if (request !== openRequest) return;
    if (access === "denied" || access === "restricted") state.libraryAccess = access;
    else state.error = errorText(e);
  } finally {
    if (request === openRequest) {
      state.loading = false;
      notify();
    }
  }
}

export const openLibrary = () => openFolder(LIBRARY_PATH);

export function clearSelection() {
  state.selection = new Set();
  state.anchor = null;
  state.cursor = null;
}

export function setView(view: View) {
  if (!state.folder || state.view === view) return;
  state.view = view;
  state.zoom = false;
  state.status = null;
  clearSelection();
  notify();
}

/** Applies a change locally. Newly undecided photos go to the front when coming back from undo. */
function apply(names: string[], decision: Decision, placeAtFront: boolean) {
  for (const name of names) {
    const photo = byName.get(name);
    if (photo) photo.decision = decision;
  }
  if (decision === "undecided") {
    const present = new Set(state.queue);
    const incoming = names.filter((n) => !present.has(n));
    state.queue = placeAtFront ? [...incoming, ...state.queue] : [...state.queue, ...incoming];
  } else {
    const leaving = new Set(names);
    state.queue = state.queue.filter((n) => !leaving.has(n));
  }
}

// Writes run strictly in order, so a quick undo can never land before the decision it reverses.
function persist(folder: FolderState, names: string[], decision: Decision, onFail: () => void) {
  for (const name of names) pending.set(name, (pending.get(name) ?? 0) + 1);
  const versions = names.map((name) => byName.get(name)!.fingerprint);
  writes = writes.then(async () => {
    let failure: unknown = null;
    let failed = false;
    try {
      await api.setDecisions(folder.folder, names, versions, decision);
      for (const name of names) confirmed.set(name, decision);
    } catch (e) {
      failed = true;
      failure = e;
    }
    for (const name of names) {
      const left = (pending.get(name) ?? 1) - 1;
      if (left > 0) {
        pending.set(name, left);
        continue;
      }
      pending.delete(name);
      // Only the last queued write decides what the photo should show. A failure
      // with newer writes behind it must not overwrite them.
      if (failed && state.folder === folder) {
        const onDisk = confirmed.get(name);
        if (onDisk && byName.get(name)?.decision !== onDisk) apply([name], onDisk, true);
      }
    }
    if (failed && state.folder === folder) {
      onFail();
      pruneSelection();
      state.status = null;
      state.error = `Kunne ikke lagre valget. ${errorText(failure)}`;
      notify();
    }
  });
}

function commit(names: string[], decision: Decision, recordUndo: boolean, placeAtFront: boolean) {
  const folder = state.folder;
  if (!folder || !names.length) return;
  const changes: Change[] = names.map((name) => ({ name, prev: byName.get(name)!.decision }));
  apply(names, decision, placeAtFront);
  const entry: UndoEntry = { changes, next: decision };
  if (recordUndo) state.undo.push(entry);
  state.error = null;
  state.libraryAccess = null;
  state.status = null;
  persist(folder, names, decision, () => {
    const i = state.undo.indexOf(entry);
    if (i >= 0) state.undo.splice(i, 1);
  });
}

/** Returns false if the decision was not made, so a dragged photo can return to its place. */
export function decide(decision: "keep" | "discard") {
  const name = state.queue[0];
  if (!name || locked() || !canDecide()) return false;
  state.sweepHint = { mode: "out", direction: decision === "keep" ? "right" : "left" };
  commit([name], decision, true, true);
  notify();
  return true;
}

export function skip() {
  if (state.queue.length < 2 || locked()) return;
  state.sweepHint = { mode: "out", direction: "down" };
  state.queue = [...state.queue.slice(1), state.queue[0]];
  state.status = null;
  notify();
}

export function undo() {
  if (locked()) return;
  const entry = state.undo.pop();
  if (!entry) return;
  const groups = new Map<Decision, string[]>();
  for (const { name, prev } of entry.changes) {
    if (!byName.has(name)) continue;
    groups.set(prev, [...(groups.get(prev) ?? []), name]);
  }
  if (state.view === "sweep" && entry.next !== "undecided") {
    state.sweepHint = { mode: "in", direction: entry.next === "keep" ? "right" : "left" };
  }
  groups.forEach((names, prev) => commit(names, prev, false, true));
  pruneSelection();
  notify();
}

export function restoreSelected() {
  if (state.view === "sweep" || !state.selection.size || locked()) return;
  const before = withDecision(viewDecision());
  const firstIndex = before.findIndex((p) => state.selection.has(p.name));
  const names = before.filter((p) => state.selection.has(p.name)).map((p) => p.name);
  commit(names, "undecided", true, false);
  // Move on to the neighbouring photo, so B can be pressed again without touching the mouse.
  const after = withDecision(viewDecision());
  const next = after[Math.min(firstIndex, after.length - 1)];
  clearSelection();
  if (next) {
    state.selection = new Set([next.name]);
    state.anchor = next.name;
    state.cursor = next.name;
  } else {
    state.zoom = false;
  }
  state.status = `${images(names.length)} lagt tilbake i Sveip.`;
  notify();
}

function pruneSelection() {
  state.selection = new Set(
    [...state.selection].filter((n) => byName.get(n)?.decision === viewDecision()),
  );
  if (state.cursor && !state.selection.has(state.cursor)) state.cursor = null;
  if (state.anchor && !state.selection.has(state.anchor)) state.anchor = null;
}

export function askTrash() {
  const marked = withDecision("discard");
  if (state.view !== "review" || !marked.length || locked() || !state.folder) return;
  state.zoom = false;
  state.sheet = {
    kind: "confirm",
    busy: false,
    folder: state.folder.folder,
    names: marked.map((p) => p.name),
    bytes: marked.reduce((sum, p) => sum + p.size, 0),
  };
  notify();
}

export async function confirmTrash() {
  const folder = state.folder;
  const sheet = state.sheet;
  if (!folder || sheet?.kind !== "confirm" || sheet.busy || sheet.folder !== folder.folder) return;
  state.sheet = { ...sheet, busy: true };
  notify();
  await writes;
  // Exactly the photos the dialog described, never a recomputed list.
  const names = sheet.names;
  try {
    const result = await api.trashPhotos(folder.folder, names);
    const gone = new Set(result.trashed);
    folder.photos = folder.photos.filter((p) => !gone.has(p.name));
    for (const name of gone) byName.delete(name);
    state.undo = state.undo
      .map((e) => ({ ...e, changes: e.changes.filter((c) => !gone.has(c.name)) }))
      .filter((e) => e.changes.length);
    clearSelection();
    state.error = result.stateError
      ? `Bildene ble flyttet, men lagringen av valgene feilet. ${result.stateError}`
      : null;
    const moved = images(result.trashed.length);
    state.status = !result.trashed.length
      ? null
      : folder.source === "library"
        ? `${moved} flyttet til Nylig slettet.`
        : `${moved} flyttet til papirkurven. ${bytes(result.bytesFreed)} frigjort.`;
    state.sheet = result.failed.length
      ? { kind: "failures", failed: result.failed, moved: result.trashed.length }
      : null;
  } catch (e) {
    state.sheet = null;
    state.status = null;
    state.error = `Bildene ble ikke flyttet. ${errorText(e)}`;
  }
  notify();
}

export function closeSheet() {
  if (state.sheet?.kind === "confirm" && state.sheet.busy) return;
  state.sheet = null;
  notify();
}
