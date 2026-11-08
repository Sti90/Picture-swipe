import { esc, images, isLibraryPhoto, photoTitle } from "./format";
import type { Photo } from "./api";
import { icons } from "./icons";
import { forgetPreview, keepPreviews, previewImage, previewReady } from "./media";
import { canDecide, counts, decide, isLibrary, photoByName, photos, state, type SweepHint } from "./store";

const DURATION = 220;
const EASE_OUT = "cubic-bezier(0.25, 0.7, 0.35, 1)";
const SPRING = "cubic-bezier(0.3, 1.35, 0.5, 1)";
const PRELOAD = 3;

const motionOff = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

interface Card {
  name: string;
  el: HTMLDivElement;
}

export function createSweep() {
  const root = document.createElement("div");
  root.className = "stage";
  root.innerHTML = `
    <div class="swipe-label left">${icons.trash}<span>Kast</span></div>
    <div class="swipe-label right">${icons.check}<span>Behold</span></div>
    <div class="sweep-controls glass">
      <button data-action="discard" title="Kast (venstre pil)">${icons.trash}<span>Kast</span></button>
      <span class="divider"></span>
      <button data-action="keep" title="Behold (høyre pil)">${icons.check}<span>Behold</span></button>
    </div>
    <div class="message" hidden></div>`;
  const leftLabel = root.querySelector<HTMLElement>(".swipe-label.left")!;
  const rightLabel = root.querySelector<HTMLElement>(".swipe-label.right")!;
  const controls = root.querySelector<HTMLElement>(".sweep-controls")!;
  const message = root.querySelector<HTMLElement>(".message")!;

  let card: Card | null = null;
  let messageHtml = "";

  const threshold = () => Math.min(180, root.clientWidth * 0.22);

  function showOffset(dx: number) {
    if (!card || !canDecide()) return;
    card.el.style.transform = `translateX(${dx}px) rotate(${dx * 0.012}deg)`;
    const progress = Math.min(1, Math.abs(dx) / threshold());
    leftLabel.style.opacity = dx < 0 ? String(progress) : "0";
    rightLabel.style.opacity = dx > 0 ? String(progress) : "0";
  }

  function hideLabels() {
    leftLabel.style.opacity = "0";
    rightLabel.style.opacity = "0";
  }

  function springBack() {
    if (!card) return;
    const el = card.el;
    const from = el.style.transform;
    el.style.transform = "";
    hideLabels();
    if (from && !motionOff()) {
      el.animate([{ transform: from }, { transform: "none" }], { duration: 260, easing: SPRING });
    }
  }

  function dismiss(old: Card, hint: SweepHint | null) {
    if (hint?.mode !== "out" || motionOff()) {
      old.el.remove();
      return;
    }
    const from = old.el.style.transform || "none";
    const distance = root.clientWidth * 0.75;
    const to =
      hint.direction === "down"
        ? `translateY(${root.clientHeight * 0.5}px) scale(0.92)`
        : `translateX(${hint.direction === "right" ? distance : -distance}px) rotate(${hint.direction === "right" ? 7 : -7}deg)`;
    old.el.classList.add("leaving");
    old.el
      .animate([{ transform: from, opacity: 1 }, { transform: to, opacity: 0 }], {
        duration: DURATION,
        easing: EASE_OUT,
        fill: "forwards",
      })
      .finished.finally(() => old.el.remove());
  }

  function fail(el: HTMLElement, photo: Photo) {
    el.classList.add("failed");
    const retry = isLibraryPhoto(photo) ? `<button class="btn" data-action="retry-preview">Prøv igjen</button>` : "";
    el.innerHTML = `${icons.photo}<span>${esc(photoTitle(photo))}</span>${retry}`;
  }

  function syncControls() {
    const blocked = !!card && !canDecide();
    controls.querySelectorAll("button").forEach((button) => {
      button.disabled = blocked;
    });
  }

  function mount(name: string, hint: SweepHint | null): Card {
    const el = document.createElement("div");
    el.className = "card";
    const photo = photoByName(name)!;
    const img = previewImage(photo);
    img.addEventListener("error", () => el.isConnected && fail(el, photo), { once: true });
    if (img.dataset.failed) fail(el, photo);
    else el.append(img);
    if (isLibraryPhoto(photo) && !img.dataset.failed) {
      if (!previewReady(name)) {
        const loading = document.createElement("div");
        loading.className = "card-loading";
        loading.innerHTML = '<span class="spinner"></span><span>Henter bildet…</span>';
        el.append(loading);
      }
      const settled = () => {
        el.querySelector(".card-loading")?.remove();
        syncControls();
      };
      img.addEventListener("load", settled, { once: true });
      img.addEventListener("error", settled, { once: true });
    }
    root.prepend(el);
    if (!motionOff()) {
      if (hint?.mode === "in" && hint.direction !== "down") {
        const distance = root.clientWidth * 0.75;
        const x = hint.direction === "right" ? distance : -distance;
        el.animate([{ transform: `translateX(${x}px)`, opacity: 0 }, { transform: "none", opacity: 1 }], {
          duration: DURATION,
          easing: EASE_OUT,
        });
      } else {
        el.animate([{ transform: "scale(0.985)", opacity: 0.6 }, { transform: "none", opacity: 1 }], {
          duration: 160,
          easing: EASE_OUT,
        });
      }
    }
    return { name, el };
  }

  function renderMessage() {
    let html = "";
    if (!state.queue.length) {
      const c = counts();
      if (!photos().length) {
        html = `<h2>Ingen bilder her</h2><p>${isLibrary() ? "Biblioteket" : "Mappen"} inneholder ingen bilder som kan sorteres.</p>
          <button class="btn" data-action="open-folder">Velg mappe…</button>`;
      } else if (c.discard) {
        html = `<h2>Alle bildene er sortert</h2><p>${images(c.discard)} venter på å bli gjennomgått.</p>
          <button class="btn prominent" data-action="view-review">Gå gjennom til sletting</button>`;
      } else {
        html = `<h2>Alle bildene er sortert</h2><p>Ingen bilder er merket for sletting.</p>
          <button class="btn" data-action="open-folder">Velg mappe…</button>`;
      }
    }
    if (html !== messageHtml) {
      messageHtml = html;
      message.innerHTML = html;
    }
    message.hidden = !html;
    controls.hidden = !!html;
  }

  function update() {
    const name = state.queue[0] ?? null;
    const hint = state.sweepHint;
    state.sweepHint = null;
    if (card?.name !== name) {
      pointer = null;
      resetWheel();
      if (card) dismiss(card, hint);
      card = name ? mount(name, hint) : null;
      hideLabels();
    }
    const upcoming = state.queue.slice(0, PRELOAD + 1);
    upcoming.forEach((n) => {
      const photo = photoByName(n);
      if (photo) previewImage(photo);
    });
    keepPreviews(upcoming);
    renderMessage();
    syncControls();
  }

  // Dragging with a mouse or a pressed trackpad.
  let pointer: { id: number; x: number; lastX: number; lastT: number; v: number } | null = null;

  root.addEventListener("pointerdown", (e) => {
    if (!card || e.button !== 0 || (e.target as Element).closest("button")) return;
    root.setPointerCapture(e.pointerId);
    pointer = { id: e.pointerId, x: e.clientX, lastX: e.clientX, lastT: e.timeStamp, v: 0 };
    root.classList.add("dragging");
  });

  root.addEventListener("pointermove", (e) => {
    if (!pointer || e.pointerId !== pointer.id) return;
    const dt = Math.max(1, e.timeStamp - pointer.lastT);
    pointer.v = (e.clientX - pointer.lastX) / dt;
    pointer.lastX = e.clientX;
    pointer.lastT = e.timeStamp;
    showOffset(e.clientX - pointer.x);
  });

  function endPointer(e: PointerEvent) {
    if (!pointer || e.pointerId !== pointer.id) return;
    const dx = e.clientX - pointer.x;
    // A pause before letting go means the speed from earlier no longer counts.
    const moving = e.timeStamp - pointer.lastT < 60;
    const flick = moving && Math.abs(pointer.v) > 0.7 && Math.abs(dx) > 40;
    const passed = Math.abs(dx) >= threshold() || flick;
    pointer = null;
    root.classList.remove("dragging");
    if (e.type !== "pointerup" || !passed || !decide(dx > 0 ? "keep" : "discard")) springBack();
  }
  root.addEventListener("pointerup", endPointer);
  root.addEventListener("pointercancel", endPointer);

  // Two-finger swipes arrive as wheel events. The photo follows the fingers.
  let wheelDx = 0;
  let wheelTimer = 0;
  let wheelActive = false;
  let ignoreUntil = 0;

  function resetWheel() {
    wheelDx = 0;
    wheelActive = false;
    clearTimeout(wheelTimer);
  }

  root.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      if (!card || e.ctrlKey) return;
      // Trackpad momentum keeps firing after a decision; wait for it to stop.
      if (e.timeStamp < ignoreUntil) {
        ignoreUntil = e.timeStamp + 180;
        return;
      }
      if (!wheelActive && Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
      wheelActive = true;
      wheelDx -= e.deltaX;
      showOffset(wheelDx);
      clearTimeout(wheelTimer);
      if (Math.abs(wheelDx) >= threshold()) {
        const direction = wheelDx > 0 ? "keep" : "discard";
        ignoreUntil = e.timeStamp + 180;
        resetWheel();
        if (!decide(direction)) springBack();
        return;
      }
      wheelTimer = window.setTimeout(() => {
        resetWheel();
        springBack();
      }, 120);
    },
    { passive: false },
  );

  function retry() {
    if (!card) return;
    const name = card.name;
    forgetPreview(name);
    card.el.remove();
    card = mount(name, null);
    syncControls();
  }

  return { el: root as HTMLElement, update, retry };
}
