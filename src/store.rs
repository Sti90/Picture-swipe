//! What the user has decided, and what is known to be on disk. It only asks for work via
//! `outbox`, so all of it is tested without threads, files or PhotoKit.
use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    backend::Command,
    decisions::Decision,
    format::{bytes, images},
    library::Status,
    scan::{FolderState, Photo},
    trash::TrashResult,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Sweep,
    Review,
    Kept,
}

impl View {
    pub fn decision(self) -> Decision {
        match self {
            View::Kept => Decision::Keep,
            _ => Decision::Discard,
        }
    }
}

struct UndoEntry {
    id: u64,
    changes: Vec<(usize, Decision)>,
    next: Decision,
}

struct PendingWrite {
    photos: Vec<usize>,
    decision: Decision,
    undo: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    Confirm {
        busy: bool,
        folder: String,
        photos: Vec<usize>,
        bytes: u64,
    },
    Failures {
        failed: Vec<(String, String)>,
        moved: usize,
    },
}

/// Which side an undone photo comes back from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    InFromLeft,
    InFromRight,
}

#[derive(Default)]
struct Lists {
    revision: u64,
    keep: Vec<usize>,
    discard: Vec<usize>,
    keep_bytes: u64,
    discard_bytes: u64,
    undecided: usize,
}

pub struct Store {
    pub folder: Option<FolderState>,
    pub loading: bool,
    pub view: View,
    pub queue: VecDeque<usize>,
    undo: Vec<UndoEntry>,
    pub selection: HashSet<usize>,
    pub anchor: Option<usize>,
    pub cursor: Option<usize>,
    pub zoom: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    pub library_access: Option<Status>,
    pub sheet: Option<Sheet>,
    pub motion: Option<Motion>,
    pub outbox: Vec<Command>,
    by_name: HashMap<String, usize>,
    gone: HashSet<usize>,
    /// What is known to be on disk, and how many writes are still queued, per photo.
    confirmed: Vec<Decision>,
    pending: Vec<u32>,
    writes: HashMap<u64, PendingWrite>,
    next_id: u64,
    open_request: u64,
    revision: u64,
    lists: Lists,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            folder: None,
            loading: false,
            view: View::Sweep,
            queue: VecDeque::new(),
            undo: Vec::new(),
            selection: HashSet::new(),
            anchor: None,
            cursor: None,
            zoom: false,
            status: None,
            error: None,
            library_access: None,
            sheet: None,
            motion: None,
            outbox: Vec::new(),
            by_name: HashMap::new(),
            gone: HashSet::new(),
            confirmed: Vec::new(),
            pending: Vec::new(),
            writes: HashMap::new(),
            next_id: 1,
            open_request: 0,
            revision: 1,
            lists: Lists::default(),
        }
    }
}

impl Store {
    pub fn photos(&self) -> &[Photo] {
        self.folder.as_ref().map_or(&[], |f| &f.photos)
    }
    pub fn is_library(&self) -> bool {
        self.folder.as_ref().is_some_and(|f| f.source == "library")
    }
    pub fn current(&self) -> Option<usize> {
        self.queue.front().copied()
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// Nothing may change while a folder is loading or a confirmation is showing.
    pub fn locked(&self) -> bool {
        self.loading || self.sheet.is_some()
    }

    fn refresh(&mut self) {
        if self.lists.revision == self.revision {
            return;
        }
        let mut lists = Lists {
            revision: self.revision,
            ..Default::default()
        };
        for (index, photo) in self.photos().iter().enumerate() {
            if self.gone.contains(&index) {
                continue;
            }
            match photo.decision {
                Decision::Keep => {
                    lists.keep.push(index);
                    lists.keep_bytes += photo.size;
                }
                Decision::Discard => {
                    lists.discard.push(index);
                    lists.discard_bytes += photo.size;
                }
                Decision::Undecided => lists.undecided += 1,
            }
        }
        self.lists = lists;
    }

    /// `(undecided, keep, discard)`
    pub fn counts(&mut self) -> (usize, usize, usize) {
        self.refresh();
        (
            self.lists.undecided,
            self.lists.keep.len(),
            self.lists.discard.len(),
        )
    }
    pub fn total_bytes(&mut self, decision: Decision) -> u64 {
        self.refresh();
        if decision == Decision::Keep {
            self.lists.keep_bytes
        } else {
            self.lists.discard_bytes
        }
    }
    pub fn list(&mut self, view: View) -> &[usize] {
        self.refresh();
        if view == View::Kept {
            &self.lists.keep
        } else {
            &self.lists.discard
        }
    }

    pub fn total_photos(&self) -> usize {
        self.photos().len() - self.gone.len()
    }

    fn touch(&mut self) {
        self.revision += 1;
    }

    /// Queued behind every earlier write, so what comes back includes all of them.
    pub fn open(&mut self, path: String) {
        if self.sheet.is_some() {
            return;
        }
        self.open_request += 1;
        self.loading = true;
        self.error = None;
        self.library_access = None;
        self.status = None;
        self.outbox.push(Command::Open {
            request: self.open_request,
            path,
        });
    }

    /// Returns true if a new folder was installed, so the caller can drop old images.
    pub fn on_opened(
        &mut self,
        request: u64,
        result: Result<FolderState, String>,
        access: Option<Status>,
    ) -> bool {
        if request != self.open_request {
            return false;
        }
        self.loading = false;
        match result {
            Ok(folder) => {
                self.by_name = folder
                    .photos
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.name.clone(), i))
                    .collect();
                self.confirmed = folder.photos.iter().map(|p| p.decision).collect();
                self.pending = vec![0; folder.photos.len()];
                self.writes.clear();
                self.gone.clear();
                self.queue = folder
                    .photos
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.decision == Decision::Undecided)
                    .map(|(i, _)| i)
                    .collect();
                if folder.state_reset {
                    self.error = Some(
                        "De lagrede valgene for denne mappen var skadet. Alle bildene vises derfor som usortert."
                            .to_owned(),
                    );
                }
                self.folder = Some(folder);
                self.undo.clear();
                self.view = View::Sweep;
                self.zoom = false;
                self.motion = None;
                self.clear_selection();
                self.touch();
                true
            }
            Err(error) => {
                if matches!(access, Some(Status::Denied | Status::Restricted)) {
                    self.library_access = access;
                } else {
                    self.error = Some(error);
                }
                false
            }
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
        self.cursor = None;
    }

    pub fn set_view(&mut self, view: View) {
        if self.folder.is_none() || self.view == view {
            return;
        }
        self.view = view;
        self.zoom = false;
        self.status = None;
        self.clear_selection();
    }

    /// Newly undecided photos go to the front when coming back from undo.
    fn apply(&mut self, photos: &[usize], decision: Decision, front: bool) {
        if let Some(folder) = &mut self.folder {
            for &photo in photos {
                folder.photos[photo].decision = decision;
            }
        }
        if decision == Decision::Undecided {
            let present: HashSet<usize> = self.queue.iter().copied().collect();
            let incoming = photos.iter().copied().filter(|p| !present.contains(p));
            if front {
                for photo in incoming.rev() {
                    self.queue.push_front(photo);
                }
            } else {
                self.queue.extend(incoming);
            }
        } else {
            let leaving: HashSet<usize> = photos.iter().copied().collect();
            self.queue.retain(|p| !leaving.contains(p));
        }
        self.touch();
    }

    fn commit(&mut self, photos: Vec<usize>, decision: Decision, record: bool, front: bool) {
        let Some(folder) = &self.folder else { return };
        if photos.is_empty() {
            return;
        }
        let folder_path = folder.folder.clone();
        let names = photos
            .iter()
            .map(|&p| folder.photos[p].name.clone())
            .collect();
        let versions = photos
            .iter()
            .map(|&p| folder.photos[p].fingerprint)
            .collect();
        let changes = photos
            .iter()
            .map(|&p| (p, folder.photos[p].decision))
            .collect();
        self.apply(&photos, decision, front);
        let id = self.next_id;
        self.next_id += 1;
        if record {
            self.undo.push(UndoEntry {
                id,
                changes,
                next: decision,
            });
        }
        self.error = None;
        self.library_access = None;
        self.status = None;
        for &photo in &photos {
            self.pending[photo] += 1;
        }
        self.writes.insert(
            id,
            PendingWrite {
                photos,
                decision,
                undo: id,
            },
        );
        self.outbox.push(Command::Write {
            token: id,
            folder: folder_path,
            names,
            versions,
            decision,
        });
    }

    pub fn on_written(&mut self, token: u64, result: Result<(), String>) {
        let Some(write) = self.writes.remove(&token) else {
            return;
        };
        let failed = result.is_err();
        if !failed {
            for &photo in &write.photos {
                self.confirmed[photo] = write.decision;
            }
        }
        for &photo in &write.photos {
            self.pending[photo] -= 1;
            // Only the last queued write decides what the photo should show. A failure
            // with newer writes behind it must not overwrite them.
            if failed && self.pending[photo] == 0 {
                let on_disk = self.confirmed[photo];
                if self.photos()[photo].decision != on_disk {
                    self.apply(&[photo], on_disk, true);
                }
            }
        }
        if let Err(error) = result {
            self.undo.retain(|entry| entry.id != write.undo);
            self.prune_selection();
            self.status = None;
            self.error = Some(format!("Kunne ikke lagre valget. {error}"));
        }
    }

    /// The caller has checked that the photo may be judged. Returns the photo that left.
    pub fn decide(&mut self, keep: bool) -> Option<usize> {
        let photo = self.current()?;
        if self.locked() {
            return None;
        }
        let decision = if keep {
            Decision::Keep
        } else {
            Decision::Discard
        };
        self.commit(vec![photo], decision, true, true);
        Some(photo)
    }

    pub fn skip(&mut self) -> Option<usize> {
        if self.queue.len() < 2 || self.locked() {
            return None;
        }
        let photo = self.queue.pop_front()?;
        self.queue.push_back(photo);
        self.status = None;
        Some(photo)
    }

    /// Returns the photos brought back.
    pub fn undo(&mut self) -> Vec<usize> {
        if self.locked() {
            return Vec::new();
        }
        let Some(entry) = self.undo.pop() else {
            return Vec::new();
        };
        let mut groups: Vec<(Decision, Vec<usize>)> = Vec::new();
        for (photo, previous) in entry.changes {
            match groups
                .iter_mut()
                .find(|(decision, _)| *decision == previous)
            {
                Some((_, photos)) => photos.push(photo),
                None => groups.push((previous, vec![photo])),
            }
        }
        if self.view == View::Sweep && entry.next != Decision::Undecided {
            self.motion = Some(if entry.next == Decision::Keep {
                Motion::InFromRight
            } else {
                Motion::InFromLeft
            });
        }
        let mut restored = Vec::new();
        for (decision, photos) in groups {
            restored.extend(photos.iter().copied());
            self.commit(photos, decision, false, true);
        }
        self.prune_selection();
        restored
    }

    pub fn restore_selected(&mut self) {
        if self.view == View::Sweep || self.selection.is_empty() || self.locked() {
            return;
        }
        let view = self.view;
        let before = self.list(view).to_vec();
        let first = before
            .iter()
            .position(|p| self.selection.contains(p))
            .unwrap_or(0);
        let photos: Vec<usize> = before
            .iter()
            .copied()
            .filter(|p| self.selection.contains(p))
            .collect();
        let moved = photos.len();
        self.commit(photos, Decision::Undecided, true, false);
        // Move on to the neighbouring photo, so B can be pressed again without the mouse.
        let after = self.list(view);
        let next = after.get(first.min(after.len().saturating_sub(1))).copied();
        self.clear_selection();
        match next {
            Some(next) => {
                self.selection.insert(next);
                self.anchor = Some(next);
                self.cursor = Some(next);
            }
            None => self.zoom = false,
        }
        self.status = Some(format!("{} lagt tilbake i Sveip.", images(moved)));
    }

    fn prune_selection(&mut self) {
        let wanted = self.view.decision();
        let photos: Vec<Decision> = self.photos().iter().map(|p| p.decision).collect();
        self.selection.retain(|&p| photos[p] == wanted);
        if self.cursor.is_some_and(|c| !self.selection.contains(&c)) {
            self.cursor = None;
        }
        if self.anchor.is_some_and(|a| !self.selection.contains(&a)) {
            self.anchor = None;
        }
    }

    pub fn ask_trash(&mut self) {
        let Some(folder) = self.folder.as_ref().map(|f| f.folder.clone()) else {
            return;
        };
        let marked = self.list(View::Review).to_vec();
        if self.view != View::Review || marked.is_empty() || self.locked() {
            return;
        }
        self.zoom = false;
        let bytes = self.total_bytes(Decision::Discard);
        self.sheet = Some(Sheet::Confirm {
            busy: false,
            folder,
            photos: marked,
            bytes,
        });
    }

    pub fn confirm_trash(&mut self) {
        let Some(Sheet::Confirm {
            busy,
            folder,
            photos,
            ..
        }) = &mut self.sheet
        else {
            return;
        };
        let Some(current) = &self.folder else { return };
        if *busy || *folder != current.folder {
            return;
        }
        *busy = true;
        // Exactly the photos the dialog described, never a recomputed list.
        let names = photos
            .iter()
            .map(|&p| current.photos[p].name.clone())
            .collect();
        let folder = folder.clone();
        self.outbox.push(Command::Trash { folder, names });
    }

    pub fn on_trashed(&mut self, result: Result<TrashResult, String>) {
        let library = self.is_library();
        match result {
            Ok(result) => {
                let gone: HashSet<usize> = result
                    .trashed
                    .iter()
                    .filter_map(|name| self.by_name.get(name).copied())
                    .collect();
                self.gone.extend(gone.iter().copied());
                for entry in &mut self.undo {
                    entry.changes.retain(|(photo, _)| !gone.contains(photo));
                }
                self.undo.retain(|entry| !entry.changes.is_empty());
                self.clear_selection();
                self.touch();
                self.error = result
                    .state_error
                    .map(|e| format!("Bildene ble flyttet, men lagringen av valgene feilet. {e}"));
                let moved = images(result.trashed.len());
                self.status = if result.trashed.is_empty() {
                    None
                } else if library {
                    Some(format!("{moved} flyttet til Nylig slettet."))
                } else {
                    Some(format!(
                        "{moved} flyttet til papirkurven. {} frigjort.",
                        bytes(result.bytes_freed)
                    ))
                };
                self.sheet = (!result.failed.is_empty()).then(|| Sheet::Failures {
                    moved: result.trashed.len(),
                    failed: result
                        .failed
                        .into_iter()
                        .map(|failure| (self.failure_title(&failure.name), failure.error))
                        .collect(),
                });
            }
            Err(error) => {
                self.sheet = None;
                self.status = None;
                self.error = Some(format!("Bildene ble ikke flyttet. {error}"));
            }
        }
    }

    fn failure_title(&self, name: &str) -> String {
        match self.by_name.get(name) {
            Some(&photo) => crate::format::photo_title(&self.photos()[photo]),
            None if self.is_library() => "Ukjent bilde".to_owned(),
            None => name.to_owned(),
        }
    }

    pub fn close_sheet(&mut self) {
        if !matches!(self.sheet, Some(Sheet::Confirm { busy: true, .. })) {
            self.sheet = None;
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{decisions::Fingerprint, library};

    pub fn folder(names: &[&str], decisions: &[Decision]) -> FolderState {
        FolderState {
            folder: "/fake".into(),
            folder_name: "fake".into(),
            state_reset: false,
            source: "folder",
            photos: names
                .iter()
                .enumerate()
                .map(|(i, name)| Photo {
                    name: (*name).to_owned(),
                    path: format!("/fake/{name}"),
                    size: 1000,
                    modified: i as u64,
                    decision: decisions.get(i).copied().unwrap_or_default(),
                    web_safe: true,
                    fingerprint: Fingerprint {
                        len: 1,
                        modified_ms: i as u64,
                        inode: i as u64,
                    },
                })
                .collect(),
        }
    }

    pub fn opened(names: &[&str]) -> Store {
        let mut store = Store::default();
        store.open("/fake".into());
        let Command::Open { request, .. } = store.outbox.remove(0) else {
            panic!()
        };
        assert!(store.on_opened(request, Ok(folder(names, &[])), None));
        store
    }

    fn write_tokens(store: &mut Store) -> Vec<u64> {
        store
            .outbox
            .drain(..)
            .filter_map(|c| match c {
                Command::Write { token, .. } => Some(token),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn deciding_moves_on_and_undo_comes_back() {
        let mut store = opened(&["a", "b"]);
        assert_eq!(store.decide(false), Some(0));
        assert_eq!(store.current(), Some(1));
        assert_eq!(store.counts(), (1, 0, 1));
        assert_eq!(store.undo(), vec![0]);
        assert_eq!(store.current(), Some(0));
        assert_eq!(store.counts(), (2, 0, 0));
    }

    #[test]
    fn a_failed_write_reverts_to_what_is_on_disk() {
        let mut store = opened(&["a"]);
        store.decide(false);
        let token = write_tokens(&mut store)[0];
        store.on_written(token, Err("full disk".into()));
        assert_eq!(store.current(), Some(0));
        assert!(store.error.as_deref().unwrap().contains("full disk"));
        assert!(!store.can_undo());
    }

    #[test]
    fn an_old_failure_does_not_overwrite_a_newer_choice() {
        let mut store = opened(&["a"]);
        store.decide(false);
        store.undo();
        store.decide(true);
        let tokens = write_tokens(&mut store);
        assert_eq!(tokens.len(), 3);
        store.on_written(tokens[0], Err("hiccup".into()));
        store.on_written(tokens[1], Ok(()));
        store.on_written(tokens[2], Ok(()));
        assert_eq!(store.photos()[0].decision, Decision::Keep);
    }

    #[test]
    fn the_last_failure_returns_to_the_last_confirmed_state() {
        let mut store = opened(&["a"]);
        store.decide(false);
        store.undo();
        store.decide(true);
        let tokens = write_tokens(&mut store);
        store.on_written(tokens[0], Ok(()));
        store.on_written(tokens[1], Ok(()));
        store.on_written(tokens[2], Err("no".into()));
        assert_eq!(store.photos()[0].decision, Decision::Undecided);
        assert_eq!(store.current(), Some(0));
    }

    fn ready_to_trash() -> Store {
        let mut store = opened(&["a", "b", "c"]);
        store.decide(false);
        store.decide(false);
        store.outbox.clear();
        store.set_view(View::Review);
        store.ask_trash();
        store
    }

    #[test]
    fn the_confirmation_moves_exactly_what_it_listed_and_only_once() {
        let mut store = ready_to_trash();
        store.confirm_trash();
        store.confirm_trash();
        assert_eq!(store.outbox.len(), 1);
        let Command::Trash { names, .. } = &store.outbox[0] else {
            panic!()
        };
        assert_eq!(names, &["a", "b"]);
    }

    #[test]
    fn nothing_changes_while_a_confirmation_is_open() {
        let mut store = ready_to_trash();
        store.outbox.clear();
        assert_eq!(store.decide(true), None);
        assert!(store.undo().is_empty());
        store.open("/other".into());
        assert!(store.outbox.is_empty());
        assert!(!store.loading);
    }

    #[test]
    fn a_cancelled_system_dialog_closes_the_sheet_and_keeps_the_marks() {
        let mut store = ready_to_trash();
        store.confirm_trash();
        store.on_trashed(Ok(TrashResult {
            trashed: vec![],
            failed: vec![],
            bytes_freed: 0,
            state_error: None,
        }));
        assert!(store.sheet.is_none());
        assert!(store.status.is_none());
        assert_eq!(store.counts().2, 2);
    }

    #[test]
    fn moved_photos_leave_the_lists_and_failures_stay_visible() {
        let mut store = ready_to_trash();
        store.confirm_trash();
        store.on_trashed(Ok(TrashResult {
            trashed: vec!["a".into()],
            failed: vec![crate::trash::Failure {
                name: "b".into(),
                error: "låst".into(),
            }],
            bytes_freed: 1000,
            state_error: None,
        }));
        assert_eq!(store.list(View::Review), &[1]);
        assert!(
            matches!(&store.sheet, Some(Sheet::Failures { moved: 1, failed }) if failed[0].0 == "b")
        );
        assert_eq!(store.total_photos(), 2);
    }

    #[test]
    fn a_failed_trash_says_nothing_moved() {
        let mut store = ready_to_trash();
        store.confirm_trash();
        store.on_trashed(Err("oops".into()));
        assert!(store.sheet.is_none());
        assert_eq!(store.counts().2, 2);
        assert!(
            store
                .error
                .as_deref()
                .unwrap()
                .starts_with("Bildene ble ikke flyttet")
        );
    }

    #[test]
    fn restore_returns_photos_and_selects_the_neighbour() {
        let mut store = opened(&["a", "b", "c"]);
        store.decide(false);
        store.decide(false);
        store.set_view(View::Review);
        store.selection.insert(0);
        store.restore_selected();
        assert_eq!(store.list(View::Review), &[1]);
        assert!(store.selection.contains(&1));
        assert_eq!(store.queue, VecDeque::from([2, 0]));
    }

    #[test]
    fn an_outdated_open_is_ignored() {
        let mut store = opened(&["a"]);
        store.open("/one".into());
        store.open("/two".into());
        assert!(!store.on_opened(1, Ok(folder(&["x"], &[])), None));
        assert_eq!(store.photos()[0].name, "a");
    }

    #[test]
    fn denied_library_access_is_not_shown_as_a_plain_error() {
        let mut store = Store::default();
        store.open(library::LIBRARY_PATH.into());
        store.on_opened(1, Err("no".into()), Some(Status::Denied));
        assert_eq!(store.library_access, Some(Status::Denied));
        assert!(store.error.is_none());
    }

    #[test]
    fn earlier_decisions_reopen_out_of_the_queue() {
        let mut store = Store::default();
        store.open("/fake".into());
        store.on_opened(1, Ok(folder(&["a", "b"], &[Decision::Discard])), None);
        assert_eq!(store.current(), Some(1));
        assert_eq!(store.counts(), (1, 0, 1));
    }
}
