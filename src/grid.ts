import type { Photo } from "./api";
import { esc, photoTitle } from "./format";
import { icons } from "./icons";
import { requestThumb } from "./media";
import { notify, state, viewDecision, withDecision } from "./store";

const MIN_CELL = 148;
const GAP = 10;
const PADDING = 16;
const CAPTION = 26;
const OVERSCAN_ROWS = 2;

interface Cell {
  el: HTMLDivElement;
  cancel: () => void;
}

export function createGrid() {
  const scroller = document.createElement("div");
  scroller.className = "grid-scroll";
  scroller.tabIndex = -1;
  const sizer = document.createElement("div");
  sizer.className = "grid-sizer";
  const empty = document.createElement("div");
  empty.className = "message";
  empty.hidden = true;
  scroller.append(sizer, empty);

  const cells = new Map<string, Cell>();
  let list: Photo[] = [];
  let columns = 1;
  let cellSize = MIN_CELL;

  const rowHeight = () => cellSize + CAPTION + GAP;

  function measure() {
    const width = scroller.clientWidth - PADDING * 2;
    columns = Math.max(1, Math.floor((width + GAP) / (MIN_CELL + GAP)));
    cellSize = (width - GAP * (columns - 1)) / columns;
    const rows = Math.ceil(list.length / columns);
    sizer.style.height = rows ? `${rows * rowHeight() - GAP + PADDING * 2}px` : "0";
  }

  function removeCell(name: string) {
    const cell = cells.get(name);
    if (!cell) return;
    cell.cancel();
    cell.el.remove();
    cells.delete(name);
  }

  function makeCell(photo: Photo): Cell {
    const el = document.createElement("div");
    el.className = "cell";
    el.dataset.name = photo.name;
    el.innerHTML = `<div class="thumb"></div><div class="caption"><span>${esc(photoTitle(photo))}</span></div>`;
    const box = el.querySelector<HTMLElement>(".thumb")!;
    const placeholder = () => {
      box.classList.add("failed");
      box.innerHTML = icons.photo;
    };
    const { url, cancel } = requestThumb(photo.path);
    url.then((src) => {
      const img = new Image();
      img.alt = "";
      img.draggable = false;
      img.onerror = placeholder;
      img.src = src;
      box.append(img);
    }, placeholder);
    return { el, cancel };
  }

  function layout() {
    const top = scroller.scrollTop;
    const height = scroller.clientHeight;
    const first = Math.max(0, Math.floor((top - PADDING) / rowHeight()) - OVERSCAN_ROWS);
    const last = Math.ceil((top + height) / rowHeight()) + OVERSCAN_ROWS;
    const start = first * columns;
    const end = Math.min(list.length, (last + 1) * columns);
    const wanted = new Set<string>();
    for (let i = start; i < end; i += 1) {
      const photo = list[i];
      wanted.add(photo.name);
      let cell = cells.get(photo.name);
      if (!cell) {
        cell = makeCell(photo);
        cells.set(photo.name, cell);
        sizer.append(cell.el);
      }
      const row = Math.floor(i / columns);
      const col = i % columns;
      const s = cell.el.style;
      s.width = `${cellSize}px`;
      s.height = `${cellSize + CAPTION}px`;
      s.transform = `translate(${PADDING + col * (cellSize + GAP)}px, ${PADDING + row * rowHeight()}px)`;
      cell.el.classList.toggle("selected", state.selection.has(photo.name));
    }
    for (const name of [...cells.keys()]) if (!wanted.has(name)) removeCell(name);
  }

  function renderEmpty() {
    const text =
      viewDecision() === "discard"
        ? "<h2>Ingen bilder til sletting</h2><p>Bilder du kaster i Sveip, havner her først.</p>"
        : "<h2>Ingen beholdte bilder</h2><p>Bilder du beholder i Sveip, vises her.</p>";
    empty.hidden = list.length > 0;
    if (!list.length && empty.innerHTML !== text) empty.innerHTML = text;
  }

  function update() {
    list = withDecision(viewDecision());
    measure();
    layout();
    renderEmpty();
  }

  function scrollIntoView(name: string) {
    const index = list.findIndex((p) => p.name === name);
    if (index < 0) return;
    const y = PADDING + Math.floor(index / columns) * rowHeight();
    const bottom = y + cellSize + CAPTION;
    if (y < scroller.scrollTop + PADDING) scroller.scrollTop = y - PADDING;
    else if (bottom > scroller.scrollTop + scroller.clientHeight - PADDING)
      scroller.scrollTop = bottom - scroller.clientHeight + PADDING;
  }

  function selectRange(from: string, to: string) {
    const a = list.findIndex((p) => p.name === from);
    const b = list.findIndex((p) => p.name === to);
    if (a < 0 || b < 0) return;
    state.selection = new Set(list.slice(Math.min(a, b), Math.max(a, b) + 1).map((p) => p.name));
  }

  function moveCursor(dx: number, dy: number, extend: boolean) {
    if (!list.length) return;
    const current = list.findIndex((p) => p.name === state.cursor);
    let index: number;
    if (current < 0) {
      index = 0;
    } else if (dy) {
      index = current + dy * columns;
      if (index < 0 || index >= list.length) return;
    } else {
      index = current + dx;
      if (index < 0 || index >= list.length) return;
    }
    const name = list[index].name;
    if (extend && state.anchor) {
      selectRange(state.anchor, name);
    } else {
      state.selection = new Set([name]);
      state.anchor = name;
    }
    state.cursor = name;
    scrollIntoView(name);
    notify();
  }

  function selectAll() {
    state.selection = new Set(list.map((p) => p.name));
    state.anchor = list[0]?.name ?? null;
    state.cursor = list[list.length - 1]?.name ?? null;
    notify();
  }

  scroller.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const cell = (e.target as Element).closest<HTMLElement>(".cell");
    if (!cell) {
      state.selection = new Set();
      state.anchor = null;
      state.cursor = null;
      notify();
      return;
    }
    const name = cell.dataset.name!;
    if (e.shiftKey && state.anchor) {
      selectRange(state.anchor, name);
      state.cursor = name;
    } else if (e.metaKey) {
      const next = new Set(state.selection);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      state.selection = next;
      state.anchor = name;
      state.cursor = next.has(name) ? name : null;
    } else {
      state.selection = new Set([name]);
      state.anchor = name;
      state.cursor = name;
    }
    notify();
  });

  scroller.addEventListener("dblclick", (e) => {
    if ((e.target as Element).closest(".cell") && !state.zoom) {
      state.zoom = true;
      notify();
    }
  });

  scroller.addEventListener("scroll", layout, { passive: true });
  const resize = new ResizeObserver(() => {
    measure();
    layout();
  });
  resize.observe(scroller);

  const destroy = () => {
    resize.disconnect();
    for (const name of [...cells.keys()]) removeCell(name);
  };

  return { el: scroller as HTMLElement, update, moveCursor, selectAll, destroy };
}

export type Grid = ReturnType<typeof createGrid>;
