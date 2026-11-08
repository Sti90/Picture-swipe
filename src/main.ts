import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText, fileUrl, type MenuContext } from "./api";
import { bytes, count, esc, images, photoTitle, when } from "./format";
import { createGrid, type Grid } from "./grid";
import { icons } from "./icons";
import {
  askTrash,
  clearSelection,
  closeSheet,
  confirmTrash,
  counts,
  currentPhoto,
  decide,
  isLibrary,
  notify,
  openFolder,
  openLibrary,
  photoByName,
  photos,
  restoreSelected,
  setView,
  skip,
  state,
  subscribe,
  undo,
  withDecision,
  type View,
} from "./store";
import { createSweep } from "./sweep";

const root = document.querySelector<HTMLElement>("#app")!;
root.innerHTML = `
  <div class="app">
    <aside class="sidebar">
      <div class="sidebar-top" data-tauri-drag-region></div>
      <div class="sidebar-body"></div>
    </aside>
    <main class="main">
      <header class="toolbar" data-tauri-drag-region></header>
      <div class="view"></div>
      <footer class="statusbar"></footer>
    </main>
    <div class="empty" hidden data-tauri-drag-region></div>
    <div class="lightbox" hidden></div>
    <div class="sheet-backdrop" hidden></div>
    <div class="drop-ring" hidden></div>
  </div>`;

const $ = <T extends HTMLElement>(s: string) => root.querySelector<T>(s)!;
const appEl = $(".app");
const sidebarEl = $(".sidebar-body");
const toolbarEl = $(".toolbar");
const viewEl = $(".view");
const statusEl = $(".statusbar");
const emptyEl = $(".empty");
const lightboxEl = $(".lightbox");
const sheetEl = $(".sheet-backdrop");
const dropEl = $(".drop-ring");

const rendered = new WeakMap<HTMLElement, string>();
const setHtml = (el: HTMLElement, html: string) => {
  if (rendered.get(el) !== html) {
    rendered.set(el, html);
    el.innerHTML = html;
  }
};

// Views

let mounted: View | null = null;
let mountedFolder: unknown = null;
let sweep: ReturnType<typeof createSweep> | null = null;
let grid: Grid | null = null;

function mountView() {
  const view = state.folder ? state.view : null;
  // A new folder gets fresh views, or a card could outlive the photo it showed.
  if (view !== mounted || state.folder !== mountedFolder) {
    mountedFolder = state.folder;
    grid?.destroy();
    grid = null;
    sweep = null;
    viewEl.replaceChildren();
    mounted = view;
    if (view === "sweep") {
      sweep = createSweep();
      viewEl.append(sweep.el);
    } else if (view) {
      grid = createGrid();
      viewEl.append(grid.el);
    }
  }
  sweep?.update();
  grid?.update();
}

const sizeOf = (bytesTotal: number) => (isLibrary() ? "" : `, ${bytes(bytesTotal)}`);

function canReveal() {
  if (!state.folder || isLibrary()) return false;
  return state.view === "sweep" ? !!currentPhoto() : state.selection.size > 0;
}

function renderSidebar() {
  if (!state.folder) return;
  const c = counts();
  const row = (view: View, label: string, icon: string, n: number) =>
    `<button class="row${state.view === view ? " active" : ""}" data-action="view-${view}">
       ${icon}<span class="row-label">${label}</span><span class="row-count">${count(n)}</span>
     </button>`;
  setHtml(
    sidebarEl,
    `<div class="section"${isLibrary() ? "" : ` title="${esc(state.folder.folder)}"`}>${esc(state.folder.folderName)}</div>
     ${row("sweep", "Sveip", icons.sweep, c.undecided)}
     ${row("review", "Til sletting", icons.trash, c.discard)}
     ${row("kept", "Beholdt", icons.check, c.keep)}
     <div class="grow" data-tauri-drag-region></div>
     <button class="row" data-action="open-library">${icons.library}<span class="row-label">Åpne Bilder-biblioteket</span></button>
     <button class="row" data-action="open-folder">${icons.folder}<span class="row-label">Åpne mappe…</span></button>`,
  );
}

function renderToolbar() {
  if (!state.folder) return;
  const c = counts();
  let title = "Sveip";
  let subtitle = "";
  let trailing = "";
  const undoButton = `<button class="tool" data-action="undo" title="Angre (⌘Z)" ${state.undo.length ? "" : "disabled"}>${icons.undo}</button>`;
  const reveal = isLibrary()
    ? ""
    : `<button class="tool" data-action="reveal" title="Vis i Finder" ${canReveal() ? "" : "disabled"}>${icons.reveal}</button>`;
  if (state.view === "sweep") {
    const total = photos().length;
    subtitle = state.queue.length ? `${count(total - state.queue.length + 1)} av ${count(total)}` : "";
    trailing = `<div class="tools">${state.queue.length ? reveal : ""}${undoButton}</div>`;
  } else if (state.view === "review") {
    title = "Til sletting";
    subtitle = c.discard ? `${images(c.discard)}${sizeOf(c.discardBytes)}` : "";
    trailing = `<div class="tools">${reveal}${undoButton}</div>
      <button class="btn prominent" data-action="trash-all" ${c.discard ? "" : "disabled"}>${isLibrary() ? "Slett i Bilder…" : "Flytt til papirkurven…"}</button>`;
  } else {
    title = "Beholdt";
    subtitle = c.keep ? `${images(c.keep)}${sizeOf(c.keepBytes)}` : "";
    trailing = `<div class="tools">${reveal}${undoButton}</div>`;
  }
  setHtml(
    toolbarEl,
    `<div class="title"><div class="title-main">${title}</div><div class="title-sub">${subtitle}</div></div>
     <div class="toolbar-trailing">${trailing}</div>`,
  );
}

const accessHelp = () =>
  state.libraryAccess === "restricted"
    ? "Tilgangen til Bilder er begrenset på denne Mac-en, for eksempel av en administrator."
    : "Bildesveip har ikke tilgang til Bilder. Slå det på i Systeminnstillinger, under Personvern og sikkerhet.";

function renderStatusbar() {
  let body = "";
  if (state.libraryAccess) {
    body = `<span class="status error">${icons.warning}<span>${esc(accessHelp())}</span></span>
      <button class="btn" data-action="open-privacy">Åpne Systeminnstillinger</button>
      <button class="tool small" data-action="dismiss" title="Lukk">${icons.close}</button>`;
  } else if (state.error) {
    body = `<span class="status error">${icons.warning}<span>${esc(state.error)}</span></span>
      <button class="tool small" data-action="dismiss" title="Lukk">${icons.close}</button>`;
  } else if (state.status) {
    body = `<span class="status">${icons.done}<span>${esc(state.status)}</span></span>
      <button class="tool small" data-action="dismiss" title="Lukk">${icons.close}</button>`;
  } else if (state.view === "sweep") {
    const p = currentPhoto();
    if (p) {
      body = isLibrary()
        ? `<span class="meta name">${esc(photoTitle(p))}</span>`
        : `<span class="meta name">${esc(photoTitle(p))}</span><span class="meta">${bytes(p.size)}</span><span class="meta">${when(p.modified)}</span>`;
    }
  } else if (state.selection.size) {
    let size = 0;
    for (const n of state.selection) size += photoByName(n)?.size ?? 0;
    body = `<span class="meta">${count(state.selection.size)} valgt${sizeOf(size)}</span>`;
    if (state.selection.size === 1) {
      const p = photoByName([...state.selection][0]);
      if (p) {
        body += `<span class="meta name">${esc(photoTitle(p))}</span>`;
        if (!isLibrary()) body += `<span class="meta">${when(p.modified)}</span>`;
      }
    }
  }
  setHtml(statusEl, body);
}

function renderEmpty() {
  emptyEl.hidden = !!state.folder;
  appEl.classList.toggle("no-folder", !state.folder);
  if (state.folder) return;
  setHtml(
    emptyEl,
    `<div class="message" data-tauri-drag-region>
       <h2>Sorter bildene dine</h2>
       <p>Velg en mappe, eller slipp den her. Ingenting slettes før du har gått gjennom bildene.</p>
       <div class="actions">
         <button class="btn" data-action="open-library" ${state.loading ? "disabled" : ""}>Åpne Bilder-biblioteket</button>
         <button class="btn prominent" data-action="open-folder" ${state.loading ? "disabled" : ""}>Velg mappe…</button>
       </div>
       ${state.error ? `<p class="inline-error">${esc(state.error)}</p>` : ""}
       ${state.libraryAccess ? `<p class="inline-error">${esc(accessHelp())}</p><button class="btn" data-action="open-privacy">Åpne Systeminnstillinger</button>` : ""}
     </div>`,
  );
}

let lightboxName: string | null = null;

function zoomTarget() {
  if (state.view === "sweep") return currentPhoto();
  const name = state.cursor ?? [...state.selection][0];
  return name ? photoByName(name) : undefined;
}

function renderLightbox() {
  const photo = state.zoom ? zoomTarget() : undefined;
  lightboxEl.hidden = !photo;
  if (!photo) {
    if (lightboxName) lightboxEl.replaceChildren();
    lightboxName = null;
    if (state.zoom) state.zoom = false;
    return;
  }
  if (lightboxName === photo.name) return;
  lightboxName = photo.name;
  const fail = () => {
    if (lightboxName !== photo.name) return;
    lightboxEl.innerHTML = `<div class="failed-large">${icons.photo}<span>${esc(photoTitle(photo))}</span></div>`;
  };
  const drag = document.createElement("div");
  drag.className = "lightbox-drag";
  drag.dataset.tauriDragRegion = "";
  const img = new Image();
  img.alt = "";
  img.draggable = false;
  img.onerror = fail;
  lightboxEl.replaceChildren(img, drag);
  api.preview(photo.path).then((file) => {
    if (lightboxName === photo.name) img.src = fileUrl(file);
  }, fail);
}

function failureTitle(name: string) {
  const photo = photoByName(name);
  return photo ? photoTitle(photo) : isLibrary() ? "Ukjent bilde" : name;
}

let sheetKey = "";

function renderSheet() {
  const sheet = state.sheet;
  sheetEl.hidden = !sheet;
  if (!sheet) {
    sheetKey = "";
    sheetEl.replaceChildren();
    return;
  }
  const key = sheet.kind === "confirm" ? `confirm:${sheet.busy}:${sheet.names.length}` : JSON.stringify(sheet);
  if (key === sheetKey) return;
  sheetKey = key;
  if (sheet.kind === "confirm") {
    const copy = isLibrary()
      ? {
          title: `Flytte ${images(sheet.names.length)} til Nylig slettet?`,
          text: "De fjernes fra Bilder på alle enhetene dine med iCloud-bilder, også for andre hvis de ligger i et delt bibliotek. De kan hentes tilbake i 30 dager. macOS ber deg bekrefte én gang til.",
          action: "Flytt til Nylig slettet",
        }
      : {
          title: `Flytte ${images(sheet.names.length)} til papirkurven?`,
          text: `Dette frigjør ${bytes(sheet.bytes)} når papirkurven tømmes.`,
          action: "Flytt til papirkurven",
        };
    sheetEl.innerHTML = `
      <div class="sheet" role="alertdialog" aria-modal="true" aria-labelledby="sheet-title" tabindex="-1">
        <h3 id="sheet-title">${copy.title}</h3>
        <p>${copy.text}</p>
        <div class="sheet-buttons">
          <button class="btn" data-action="sheet-cancel" ${sheet.busy ? "disabled" : ""}>Avbryt</button>
          <button class="btn" data-action="sheet-confirm" ${sheet.busy ? "disabled" : ""}>${sheet.busy ? "Flytter…" : copy.action}</button>
        </div>
      </div>`;
  } else {
    const n = sheet.failed.length;
    const moved = sheet.moved ? ` ${images(sheet.moved)} ble flyttet.` : "";
    const rows = sheet.failed
      .map((f) => `<li><span class="fail-name">${esc(failureTitle(f.name))}</span><span class="fail-why">${esc(f.error)}</span></li>`)
      .join("");
    sheetEl.innerHTML = `
      <div class="sheet" role="alertdialog" aria-modal="true" aria-labelledby="sheet-title" tabindex="-1">
        <h3 id="sheet-title">${images(n)} kunne ikke flyttes</h3>
        <p>De ligger fortsatt i Til sletting.${moved}</p>
        <ul class="fail-list">${rows}</ul>
        <div class="sheet-buttons one"><button class="btn" data-action="sheet-cancel">OK</button></div>
      </div>`;
  }
  // Focus the sheet itself so Return never lands on a button by accident.
  sheetEl.querySelector<HTMLElement>(".sheet")?.focus();
}

let lastTitle = "";
function syncTitle() {
  const title = state.folder?.folderName ?? "Bildesveip";
  if (title !== lastTitle) {
    lastTitle = title;
    getCurrentWindow().setTitle(title).catch(() => {});
  }
}

let lastMenu = "";
function syncMenu() {
  const context: MenuContext = state.folder ? state.view : "empty";
  const args = {
    context,
    canUndo: state.undo.length > 0,
    hasSelection: state.selection.size > 0,
    canReveal: canReveal(),
    discardCount: withDecision("discard").length,
  };
  const key = JSON.stringify(args);
  if (key === lastMenu) return;
  lastMenu = key;
  api.setMenuContext(args).catch(() => {});
}

function render() {
  renderEmpty();
  renderSidebar();
  renderToolbar();
  mountView();
  renderStatusbar();
  renderLightbox();
  renderSheet();
  syncTitle();
  syncMenu();
}

subscribe(render);

// Commands

async function chooseFolder() {
  try {
    const picked = await open({ directory: true, multiple: false, title: "Velg mappe" });
    if (typeof picked === "string") await openFolder(picked);
  } catch (e) {
    state.error = errorText(e);
    notify();
  }
}

function reveal() {
  if (isLibrary()) return;
  const photo = state.view === "sweep" ? currentPhoto() : zoomTarget();
  if (!photo) return;
  api.revealInFinder(photo.path).catch((e) => {
    state.error = errorText(e);
    notify();
  });
}

function run(action: string) {
  if (state.sheet) {
    if (action === "sheet-cancel") closeSheet();
    else if (action === "sheet-confirm") void confirmTrash();
    return;
  }
  const hasFolder = !!state.folder;
  const sweeping = hasFolder && state.view === "sweep";
  switch (action) {
    case "open-folder":
      void chooseFolder();
      break;
    case "open-library":
      void openLibrary();
      break;
    case "open-privacy":
      api.openPhotosPrivacySettings().catch((e) => {
        state.error = errorText(e);
        notify();
      });
      break;
    case "keep":
    case "discard":
      if (sweeping) decide(action);
      break;
    case "skip":
      if (sweeping) skip();
      break;
    case "undo":
      undo();
      break;
    case "restore":
      restoreSelected();
      break;
    case "retry-preview":
      sweep?.retry();
      break;
    case "select-all":
      grid?.selectAll();
      break;
    case "trash-all":
      askTrash();
      break;
    case "view-sweep":
    case "view-review":
    case "view-kept":
      setView(action.slice(5) as View);
      break;
    case "toggle-zoom":
      if (hasFolder) {
        state.zoom = !state.zoom;
        notify();
      }
      break;
    case "reveal":
      reveal();
      break;
    case "dismiss":
      state.error = null;
      state.libraryAccess = null;
      state.status = null;
      notify();
      break;
  }
}

// The same keypress can reach us from JS and from the native menu.
const lastRun = new Map<string, number>();
function dispatch(action: string) {
  const now = performance.now();
  const previous = lastRun.get(action) ?? -Infinity;
  lastRun.set(action, now);
  if (now - previous < 100) return;
  run(action);
}

root.addEventListener("click", (e) => {
  const button = (e.target as Element).closest<HTMLElement>("[data-action]");
  if (button && !(button as HTMLButtonElement).disabled) run(button.dataset.action!);
  else if ((e.target as Element).closest(".lightbox") && !state.sheet) {
    state.zoom = false;
    notify();
  }
});

// Clicking must not leave focus on a button, or Space would press it instead of toggling zoom.
root.addEventListener("mousedown", (e) => {
  if ((e.target as Element).closest("button")) e.preventDefault();
});

window.addEventListener("keydown", (e) => {
  const cmd = e.metaKey;
  if (state.sheet) {
    if (e.key === "Escape" || (cmd && e.key === ".")) {
      e.preventDefault();
      closeSheet();
    } else if (e.key === "Enter") {
      // No default button: Return must not confirm or dismiss by reflex.
      if (document.activeElement?.tagName !== "BUTTON") e.preventDefault();
    } else if (e.key === "Tab") {
      const buttons = [...sheetEl.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
      const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = buttons[(i + (e.shiftKey ? buttons.length - 1 : 1)) % buttons.length];
      e.preventDefault();
      next?.focus();
    }
    return;
  }

  if (e.repeat && !e.key.startsWith("Arrow")) return;
  const focusedButton = document.activeElement?.tagName === "BUTTON";
  if (focusedButton && (e.key === " " || e.key === "Enter")) return;

  const act = (action: string) => {
    e.preventDefault();
    dispatch(action);
  };

  if (cmd) {
    const key = e.key.toLowerCase();
    if (key === "o") act(e.shiftKey ? "open-library" : "open-folder");
    else if (key === "z" && !e.shiftKey) act("undo");
    else if (key === "1") act("view-sweep");
    else if (key === "2") act("view-review");
    else if (key === "3") act("view-kept");
    else if (key === "r" && e.shiftKey) act("reveal");
    else if (e.key === "Backspace" && e.shiftKey) act("trash-all");
    else if (key === "a" && grid) act("select-all");
    return;
  }
  if (!state.folder) return;

  if (e.key === "Escape") {
    e.preventDefault();
    if (state.zoom) {
      state.zoom = false;
      notify();
    } else if (grid && state.selection.size) {
      clearSelection();
      notify();
    }
    return;
  }
  if (e.key === " ") return act("toggle-zoom");

  if (state.view === "sweep") {
    if (e.repeat) return;
    if (e.key === "ArrowRight") act("keep");
    else if (e.key === "ArrowLeft") act("discard");
    else if (e.key === "ArrowDown") act("skip");
  } else if (grid) {
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, -1],
      ArrowDown: [0, 1],
    };
    const move = moves[e.key];
    if (move) {
      e.preventDefault();
      grid.moveCursor(move[0], move[1], e.shiftKey);
    } else if (e.key.toLowerCase() === "b") {
      act("restore");
    }
  }
});

void listen<string>("menu-action", (e) => dispatch(e.payload));

void getCurrentWebview().onDragDropEvent((e) => {
  const type = e.payload.type;
  if (type === "enter" || type === "over") {
    dropEl.hidden = false;
  } else {
    dropEl.hidden = true;
    if (type === "drop" && e.payload.paths[0]) void openFolder(e.payload.paths[0]);
  }
});

render();

api
  .lastFolder()
  .then((path) => (path ? openFolder(path) : undefined))
  .catch(() => {});
