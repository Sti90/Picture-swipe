import { convertFileSrc, invoke } from "@tauri-apps/api/core";

export type Decision = "undecided" | "keep" | "discard";
export type MenuContext = "empty" | "sweep" | "review" | "kept";
export type LibraryStatus = "notDetermined" | "restricted" | "denied" | "limited" | "authorized";

/** Passed in place of a folder path to open the macOS Photos library. */
export const LIBRARY_PATH = "bilder:library";

export interface Fingerprint {
  len: number;
  modifiedMs: number;
  inode: number;
}

export interface Photo {
  name: string;
  path: string;
  size: number;
  modified: number;
  decision: Decision;
  webSafe: boolean;
  fingerprint: Fingerprint;
}

export interface FolderState {
  source: "folder" | "library";
  folder: string;
  folderName: string;
  photos: Photo[];
  stateReset: boolean;
}

export interface TrashResult {
  trashed: string[];
  failed: { name: string; error: string }[];
  bytesFreed: number;
  stateError: string | null;
}

export const api = {
  openFolder: (path: string) => invoke<FolderState>("open_folder", { path }),
  lastFolder: () => invoke<string | null>("last_folder"),
  libraryStatus: () => invoke<LibraryStatus>("library_status"),
  openPhotosPrivacySettings: () => invoke<void>("open_photos_privacy_settings"),
  setDecisions: (folder: string, names: string[], versions: Fingerprint[], decision: Decision) =>
    invoke<void>("set_decisions", { folder, names, versions, decision }),
  thumbnail: (path: string) => invoke<string>("thumbnail", { path }),
  preview: (path: string) => invoke<string>("preview", { path }),
  trashPhotos: (folder: string, names: string[]) =>
    invoke<TrashResult>("trash_photos", { folder, names }),
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  setMenuContext: (args: {
    context: MenuContext;
    canUndo: boolean;
    hasSelection: boolean;
    canReveal: boolean;
    discardCount: number;
  }) => invoke<void>("set_menu_context", args),
};

export const fileUrl = (path: string) => convertFileSrc(path);

export const errorText = (e: unknown) =>
  typeof e === "string" ? e : e instanceof Error ? e.message : "Noe gikk galt.";
