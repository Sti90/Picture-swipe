// Original line glyphs drawn for this app, sized and weighted to sit well next to system text.
const glyph = (body: string) =>
  `<svg class="icon" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const icons = {
  sweep: glyph(
    '<rect x="4.5" y="6" width="11" height="10.5" rx="2"/><path d="M6.5 3.5h7"/><path d="m4.5 13.5 3-2.75 2.25 2 2.25-1.75 3.5 3"/>',
  ),
  trash: glyph(
    '<path d="M3.75 5.5h12.5"/><path d="M8 5.5V4a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v1.5"/><path d="m5.25 5.5.7 10a1.5 1.5 0 0 0 1.5 1.4h5.1a1.5 1.5 0 0 0 1.5-1.4l.7-10"/><path d="M8.5 9v4.5M11.5 9v4.5"/>',
  ),
  check: glyph('<path d="m4 10.5 4 4 8-9"/>'),
  folder: glyph(
    '<path d="M2.75 6a1.5 1.5 0 0 1 1.5-1.5h3.2l1.8 2h6.5a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5Z"/>',
  ),
  library: glyph(
    '<rect x="5.5" y="3.5" width="11.5" height="9.5" rx="1.75"/><path d="M3 6.75v7.5A2.25 2.25 0 0 0 5.25 16.5h8.5"/><path d="m8 11 2.25-2 1.75 1.5 1.5-1 2 1.75"/>',
  ),
  undo: glyph('<path d="M7.5 4 3.5 8l4 4"/><path d="M3.5 8h8.25a4.25 4.25 0 0 1 0 8.5H8"/>'),
  reveal: glyph('<circle cx="10" cy="10" r="7"/><path d="M6.75 10h6.5"/><path d="m10.75 7.25 2.75 2.75-2.75 2.75"/>'),
  close: glyph('<path d="m5.5 5.5 9 9M14.5 5.5l-9 9"/>'),
  photo: glyph(
    '<rect x="3" y="4" width="14" height="12" rx="2"/><circle cx="7.25" cy="8" r="1.25"/><path d="m3.5 14 3.75-3.25 2.5 2.25 2.5-2 4.25 3.5"/>',
  ),
  warning: glyph('<circle cx="10" cy="10" r="7"/><path d="M10 6.25v4.5"/><path d="M10 13.6v.05"/>'),
  done: glyph('<circle cx="10" cy="10" r="7"/><path d="m6.75 10.25 2.25 2.25 4.25-4.75"/>'),
};
