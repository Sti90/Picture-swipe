# Bildesveip: contract between Rust and the UI

macOS-first Tauri 2 app. The user opens a folder of images and sorts them one at a
time: keep or discard. Discarded images are only *marked*. They stay exactly where
they are on disk and appear in a separate "Til sletting" view, where the user can
review them and change their mind. Files leave their folder only when the user
confirms from that view, and then they go to the macOS Trash, never a permanent delete.

UI language: Norwegian bokmål.

## Ownership

| Area | Files | Owner |
|---|---|---|
| Backend | `src-tauri/**` | Codex |
| Frontend | `index.html`, `src/**`, `vite.config.ts`, `package.json` | Claude subagent |

Do not edit the other side's files. If the contract is wrong, report it in your final
message rather than changing it unilaterally.

## Types

Rust uses `#[serde(rename_all = "camelCase")]`.

```ts
type Decision = "undecided" | "keep" | "discard";

interface Photo {
  name: string;       // file name only, unique within the folder
  path: string;       // absolute path
  size: number;       // bytes
  modified: number;   // ms since the Unix epoch
  decision: Decision;
  webSafe: boolean;   // true if WebKit can show the original (jpg, jpeg, png, gif, webp, heic, heif, avif, bmp, tif, tiff)
  fingerprint: { len: number; modifiedMs: number; inode: number }; // the file version that was scanned
}

interface FolderState {
  folder: string;     // absolute path
  folderName: string;
  photos: Photo[];    // sorted by modified ascending, then name
  stateReset: boolean; // true if saved decisions were damaged and set aside
}

interface TrashResult {
  trashed: string[];                          // names
  failed: { name: string; error: string }[];  // error text in Norwegian
  bytesFreed: number;
  stateError: string | null; // files moved, but updating saved decisions failed
}

type MenuContext = "empty" | "sweep" | "review" | "kept";
```

## Commands (`invoke`)

Arguments are camelCase. Failures reject with a Norwegian, human-readable string.

| Command | Args | Returns | Notes |
|---|---|---|---|
| `open_folder` | `{ path }` | `FolderState` | Top level only, skips hidden files and non-images. Adds the folder to the asset protocol scope. Loads saved decisions, ignores entries for files that no longer exist. Remembers it as the last folder. |
| `last_folder` | none | `string \| null` | Null if none, or if it no longer exists. |
| `set_decisions` | `{ folder, names, versions, decision }` | `void` | `versions` lines up with `names` and holds each photo's `fingerprint`. Rejected, with nothing changed, if a file differs from that version. Saved durably before returning. |
| `thumbnail` | `{ path }` | `string` | Absolute path to a cached JPEG, longest edge 512 px. |
| `preview` | `{ path }` | `string` | Original path if `webSafe`, otherwise a cached JPEG, longest edge 2400 px. |
| `trash_photos` | `{ folder, names }` | `TrashResult` | Moves files to the macOS Trash. Per-file failures go in `failed`; the rest still proceed. Forgets saved decisions for files that were trashed. |
| `reveal_in_finder` | `{ path }` | `void` | Runs `open -R`. |
| `set_menu_context` | `{ context, canUndo, hasSelection, discardCount }` | `void` | Enables and disables menu items. |

Display files in the UI with `convertFileSrc(path)`.

### Safety rules (backend must enforce)

- `names` must be plain file names: reject any with `/`, or equal to `.` or `..`.
- `thumbnail`, `preview`, `reveal_in_finder` must only accept paths directly inside a
  folder opened in this session.
- `trash_photos` must refuse any name not currently marked `discard` in saved state,
  and must never call a permanent delete.
- Each saved decision stores a fingerprint of the file (size, mtime, inode). A file
  whose fingerprint differs is a different file: its decision is dropped on
  `open_folder`, and `trash_photos` refuses it. Entries for missing files are dropped
  on `open_folder` as well, and a `discard` with no fingerprint is never honoured.

### Accepted limitations

- A compromised page could call `set_decisions` and `trash_photos` itself, with the
  macOS dialog as the last check for the library and none for folders. The page loads
  no remote content, escapes file names, and can't navigate away, so this needs a
  separate injection bug first.

- An external process replacing a file in the instant between the fingerprint check
  and the move to the Trash is not detected. The result is one file in the Trash,
  which is recoverable.
- A decision still in the write queue when the app quits may not be saved. An unsaved
  undo can therefore leave an earlier discard in place, but moving that file still
  requires a new confirmation from the review view.
- If the app crashes between moving files and saving, stale entries are dropped the
  next time the folder is opened.

## Native menu

Every command lives in the menu bar (Apple HIG). Clicking an item, or pressing its
accelerator, emits the Tauri event `menu-action` with one of these string ids as payload.

| Menu | Item | Id | Accelerator | Enabled in |
|---|---|---|---|---|
| Bildesveip | standard About, Services, Hide, Quit | | | |
| Arkiv | Åpne mappe… | `open-folder` | Cmd+O | always |
| Arkiv | Vis i Finder | `reveal` | Cmd+Shift+R | sweep; review/kept when `hasSelection` |
| Rediger | Angre | `undo` | Cmd+Z | when `canUndo` |
| Rediger | Marker alt | `select-all` | Cmd+A | review, kept |
| Rediger | standard Cut, Copy, Paste | | | |
| Bilde | Behold | `keep` | Right | sweep |
| Bilde | Kast | `discard` | Left | sweep |
| Bilde | Hopp over | `skip` | Down | sweep |
| Bilde | Legg tilbake | `restore` | B | review/kept when `hasSelection` |
| Bilde | Flytt til papirkurven… | `trash-all` | Cmd+Shift+Backspace | review when `discardCount > 0` |
| Vis | Sveip | `view-sweep` | Cmd+1 | not empty |
| Vis | Til sletting | `view-review` | Cmd+2 | not empty |
| Vis | Beholdt | `view-kept` | Cmd+3 | not empty |
| Vis | Stor visning | `toggle-zoom` | Space | not empty |
| Vis | standard Enter Full Screen | | | |
| Vindu | standard Minimize, Zoom | | | |

Frontend note: WKWebView may deliver an unmodified key to JS, to the menu, or both.
Handle these keys in JS too, ignore `event.repeat`, and drop a repeat of the same
action arriving within 100 ms.

## Window

`titleBarStyle: Overlay`, `hiddenTitle: true`, 1180x780, min 820x560, native `sidebar`
window vibrancy (transparent window, `macOSPrivateApi: true`), product name `Bildesveip`.
Traffic lights at x 19, y 20. The frontend paints the main content opaque and leaves
the sidebar transparent so the native material shows through.

## Capabilities

`core:default`, `dialog:allow-open`, `core:window:allow-start-dragging`,
`core:window:allow-set-title`, and permission to use the asset protocol. The dialog
is opened from the frontend with `@tauri-apps/plugin-dialog`.

## Bilder-biblioteket (Photos library)

Besides folders, the app can open the user's macOS Photos library. Everything above
applies unless stated here. The library is one more source behind the same commands,
so the frontend mostly doesn't care which one is open.

The constant `LIBRARY_PATH = "bilder:library"` is passed as `path` / `folder` wherever a
folder would be. It can never be a real folder: real paths are absolute.

### Type changes

```ts
interface FolderState { source: "folder" | "library"; /* rest unchanged */ }
```

`FolderState` for the library: `folder` is `LIBRARY_PATH`, `folderName` is `"Bilder"`.

`Photo` for the library:

| Field | Value |
|---|---|
| `name` | The PHAsset `localIdentifier` with every `/` replaced by `:`. Opaque to the UI, never shown. |
| `path` | `bilder:asset:<name>`, passed to `thumbnail` and `preview`, never shown. |
| `size` | `0`, meaning unknown. The UI must not show a size or bytes freed for the library. |
| `modified` | `creationDate` in ms (`0` if absent). |
| `webSafe` | `false` |
| `fingerprint` | All zeros. A `localIdentifier` is never reused for another photo, and `modificationDate` changes on its own when Photos analyses a picture, so it can't serve as identity. |

Photos only: `PHAssetMediaType.image`, source type user library, hidden excluded, bursts
excluded (one row stands in for many photos), sorted by `creationDate` ascending. Videos are out of scope. Display in the UI: the date, via
`photoTitle(p)`, never `name`.

### Commands

| Command | Args | Returns | Notes |
|---|---|---|---|
| `library_status` | none | `"notDetermined" \| "restricted" \| "denied" \| "limited" \| "authorized"` | Must never trigger a prompt. |
| `open_folder` | `{ path: LIBRARY_PATH }` | `FolderState` | Requests access if `notDetermined`, which shows the system prompt. Rejects with a Norwegian message if denied or restricted. Remembered as last folder. |
| `last_folder` | none | `string \| null` | Returns `LIBRARY_PATH` only if it was last and `library_status` is already authorized or limited, so launching never prompts. |
| `set_decisions` | as before | | Library: `names` must decode to well-formed identifiers. Versions must be all zeros. |
| `thumbnail` | `{ path }` | absolute JPEG path | `bilder:asset:<name>`: no network, opportunistic local derivative, 512 px. |
| `preview` | `{ path }` | absolute JPEG path | `bilder:asset:<name>`: network allowed so iCloud-only originals download, 2400 px. |
| `trash_photos` | as before | `TrashResult` | Library: see below. `bytesFreed` is 0. |
| `open_photos_privacy_settings` | none | `void` | Opens only the fixed Privacy & Security > Photos URL in System Settings. |

`thumbnail`, `preview`, `set_decisions` and `trash_photos` reject library input unless the
library was opened in this session. `reveal_in_finder` rejects library paths.

### Deleting

`trash_photos` on the library must:

1. Refuse every name not saved as `discard`, exactly as for folders.
2. Delete with one `PHAssetChangeRequest.deleteAssets` in `performChangesAndWait`. Photos
   itself shows a system confirmation and moves them to Recently Deleted. Nothing may
   bypass that, and there is no permanent delete anywhere.
3. Report identifiers that no longer exist in `failed`, and still process the rest.
4. Treat the user cancelling the system dialog as nothing moved: `trashed` and `failed`
   both empty, decisions untouched, no error.
5. Treat any other PhotoKit error as nothing moved: every requested name goes in `failed`
   with a Norwegian message, decisions untouched.
6. On success remove those decisions, reporting a save failure in `stateError`.

`set_decisions` / `trash_photos` logic must sit behind a trait so tests use a fake
library. Tests must never touch PhotoKit.

### Native bridge

PhotoKit is reached through a small Objective-C file compiled with the `cc` crate
(`src-tauri/src/photokit/bridge.m`, ARC) and a thin Rust FFI layer. Blocking calls only,
always on the blocking pool, never the main thread. Results cross the boundary as
malloc'd UTF-8 JSON that Rust copies and frees through one bridge function.

- Thumbnails use `isSynchronous = true`, `networkAccessAllowed = false`.
- Previews use `isSynchronous = true`, `networkAccessAllowed = true`, at most 3 at once,
  on a separate limit from thumbnails, so a slow iCloud download can't starve the grid.
- JPEG output goes through the same temp-file-then-rename approach as `thumbs.rs`.
- Previews are private photos at full screen size: keep at most 64 library previews in
  the cache, removing the oldest. Only ever remove files from that one cache subfolder.
- Library thumbnails have their own subfolder, capped at 3000 when the library opens.
  Temp files older than an hour are removed from both. Cache keys include
  `modificationDate`, so edited photos render again. It is never used for identity.

`src-tauri/Info.plist` holds `NSPhotoLibraryUsageDescription` in Norwegian.
`bundle.macOS.signingIdentity` is `"-"` (ad-hoc). The hardened runtime needs
`Entitlements.plist` with `com.apple.security.personal-information.photos-library`.
An ad-hoc signature changes with each build, so macOS may ask for access again after a
rebuild.

### Menu and context

`Arkiv > Åpne Bilder-biblioteket`, id `open-library`, Cmd+Shift+O, always enabled.
`set_menu_context` gains `canReveal: boolean`, which replaces the view-based enabled rule
for `reveal`.

### UI

- Empty state: a second action, "Åpne Bilder-biblioteket". Sidebar: a row for it next to
  "Åpne mappe…". Never prompt at launch.
- If access is denied or restricted: an inline explanation with a button that calls
  `open_photos_privacy_settings`, not a dead end.
- The review toolbar action reads "Slett i Bilder…" for the library.
- Confirmation sheet for the library, title `Flytte 23 bilder til Nylig slettet?`, text:
  `De fjernes fra Bilder på alle enhetene dine med iCloud-bilder, også for andre hvis de ligger i et delt bibliotek. De kan hentes tilbake i 30 dager. macOS ber deg bekrefte én gang til.`
  Buttons `Avbryt` and `Flytt til Nylig slettet`.
- Afterwards: `23 bilder flyttet til Nylig slettet.` No bytes. Cancelling the system
  dialog just closes the sheet.
- Reveal in Finder is hidden and disabled for the library.
- A failed library preview offers "Prøv igjen", so the last photo can't get stuck.
- Folders inside a `.photoslibrary` package are refused by `open_folder`.
