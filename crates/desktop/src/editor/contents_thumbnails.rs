//! Bounded state for Contents-only thumbnail requests.
use crate::backend::PreviewKind;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

pub(crate) const THUMBNAIL_SIZE: u32 = 64;
const MAX_IMAGES: usize = 256;
const MAX_RGBA_BYTES: usize = 8 * 1024 * 1024;
const MAX_FAILED: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ThumbnailId {
    pub project: String,
    pub id: String,
    pub kind: PreviewKind,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThumbnailState {
    Queued,
    Running,
    Ready,
    Failed,
    Suppressed,
}

#[derive(Clone, Debug)]
struct Entry {
    state: ThumbnailState,
    retry_at: Option<Instant>,
}

/// The visible set is the only queue: off-screen unstarted entries are dropped.
#[derive(Default)]
pub(crate) struct ContentsThumbnails {
    generation: u64,
    project: String,
    entries: HashMap<ThumbnailId, Entry>,
    visible: Vec<ThumbnailId>,
    lru: VecDeque<(ThumbnailId, usize)>,
    failed_lru: VecDeque<ThumbnailId>,
    rgba_bytes: usize,
}

impl ContentsThumbnails {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn visible_snapshot(&self) -> Vec<(String, PreviewKind)> {
        self.visible
            .iter()
            .map(|id| (id.id.clone(), id.kind))
            .collect()
    }

    pub fn invalidate(&mut self, project: Value) {
        self.generation = self.generation.wrapping_add(1);
        self.project = project.to_string();
        self.entries.clear();
        self.visible.clear();
        self.lru.clear();
        self.failed_lru.clear();
        self.rgba_bytes = 0;
    }

    pub fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
        self.lru.clear();
        self.failed_lru.clear();
        self.rgba_bytes = 0;
        self.visible.clear();
    }

    #[allow(clippy::cmp_owned)] // ProjectRef JSON serialization gives the stable full-ref key.
    pub fn set_visible(
        &mut self,
        project: Value,
        ids: Vec<(String, PreviewKind)>,
    ) -> Vec<ThumbnailId> {
        if project.to_string() != self.project {
            self.invalidate(project.clone());
        }
        let project_ref = project.to_string();
        let visible: Vec<_> = ids
            .into_iter()
            .take(256)
            .map(|(id, kind)| ThumbnailId {
                project: project_ref.clone(),
                id,
                kind,
                generation: self.generation,
            })
            .collect();
        let visible_set: HashSet<_> = visible.iter().cloned().collect();
        self.entries.retain(|id, entry| match entry.state {
            ThumbnailState::Queued | ThumbnailState::Suppressed => visible_set.contains(id),
            _ => true,
        });
        for id in &visible {
            self.entries.entry(id.clone()).or_insert(Entry {
                state: ThumbnailState::Queued,
                retry_at: None,
            });
        }
        self.visible = visible;
        self.prune_failed();
        self.next_ready()
    }

    /// A revision bump on the same activation: the state survives
    /// re-keyed to the new revision and generation, entries re-ask to
    /// confirm their content (a matching reply `ready_kept`s without
    /// re-reading its PNG), and the byte budget keeps covering the held
    /// images. Activation changes use `invalidate` instead.
    pub fn revise(&mut self, project: Value) {
        let project_ref = project.to_string();
        self.generation = self.generation.wrapping_add(1);
        self.project = project_ref.clone();
        let rekey = |id: &mut ThumbnailId| {
            id.project = project_ref.clone();
            id.generation = self.generation;
        };
        let mut entries = std::mem::take(&mut self.entries);
        entries = entries
            .drain()
            .map(|(mut id, mut entry)| {
                rekey(&mut id);
                // Failures re-ask too: the new revision may fix what failed.
                entry.state = ThumbnailState::Queued;
                entry.retry_at = None;
                (id, entry)
            })
            .collect();
        self.entries = entries;
        for id in &mut self.visible {
            rekey(id);
        }
        for (id, _) in &mut self.lru {
            rekey(id);
        }
        self.failed_lru.clear();
    }

    /// A re-fetch whose key matched the held image: the entry
    /// re-enters Ready without touching the byte budget — its image
    /// never left it.
    pub fn ready_kept(&mut self, id: &ThumbnailId) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = ThumbnailState::Ready;
            entry.retry_at = None;
        }
        self.touch(id);
    }

    pub fn next_ready(&mut self) -> Vec<ThumbnailId> {
        self.visible
            .iter()
            .filter(|id| {
                self.entries.get(*id).is_some_and(|entry| {
                    entry.state == ThumbnailState::Queued
                        && entry.retry_at.is_none_or(|at| at <= Instant::now())
                })
            })
            .take(1)
            .cloned()
            .collect()
    }

    pub fn mark_running(&mut self, id: &ThumbnailId) -> bool {
        if id.project != self.project || id.generation != self.generation {
            return false;
        }
        if let Some(entry) = self.entries.get_mut(id)
            && entry.state == ThumbnailState::Queued
        {
            entry.state = ThumbnailState::Running;
            return true;
        }
        false
    }

    pub fn busy(&mut self, id: &ThumbnailId) {
        if let Some(entry) = self.entries.get_mut(id)
            && entry.state == ThumbnailState::Running
        {
            entry.state = ThumbnailState::Queued;
            entry.retry_at = Some(Instant::now() + Duration::from_millis(250));
        }
    }

    /// A request discarded before Core submission is not a failure. Keep it
    /// queued only when it remains visible in the latest snapshot.
    pub fn cancelled_before_start(&mut self, id: &ThumbnailId) {
        if !self.visible.contains(id) {
            self.entries.remove(id);
        } else if let Some(entry) = self.entries.get_mut(id)
            && entry.state == ThumbnailState::Running
        {
            entry.state = ThumbnailState::Queued;
            entry.retry_at = None;
        }
    }

    pub fn failed(&mut self, id: &ThumbnailId) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = ThumbnailState::Failed;
            entry.retry_at = None;
            self.failed_lru.retain(|cached| cached != id);
            self.failed_lru.push_back(id.clone());
            self.prune_failed();
        }
    }

    fn prune_failed(&mut self) {
        while self.failed_lru.len() > MAX_FAILED {
            let Some(index) = self
                .failed_lru
                .iter()
                .position(|candidate| !self.visible.contains(candidate))
            else {
                break;
            };
            if let Some(oldest) = self.failed_lru.remove(index)
                && self
                    .entries
                    .get(&oldest)
                    .is_some_and(|entry| entry.state == ThumbnailState::Failed)
            {
                self.entries.remove(&oldest);
            }
        }
    }

    pub fn ready(&mut self, id: &ThumbnailId, bytes: usize) -> Option<Vec<ThumbnailId>> {
        if id.project != self.project
            || id.generation != self.generation
            || !self.entries.contains_key(id)
        {
            return None;
        }
        // A kept image being replaced after a revise still owns one LRU
        // slot; retire it before the replacement takes its place so the
        // budget never counts both.
        if let Some(index) = self.lru.iter().position(|(cached, _)| cached == id) {
            let (_, prior) = self.lru.remove(index).expect("index was just found");
            self.rgba_bytes = self.rgba_bytes.saturating_sub(prior);
        }
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = ThumbnailState::Ready;
        }
        self.lru.push_back((id.clone(), bytes));
        self.rgba_bytes = self.rgba_bytes.saturating_add(bytes);
        let mut evicted = Vec::new();
        while self.lru.len() > MAX_IMAGES || self.rgba_bytes > MAX_RGBA_BYTES {
            let index = self
                .lru
                .iter()
                .position(|(item, _)| !self.visible.contains(item))
                .unwrap_or(0);
            if let Some((item, size)) = self.lru.remove(index) {
                self.rgba_bytes = self.rgba_bytes.saturating_sub(size);
                if self.visible.contains(&item) {
                    if let Some(entry) = self.entries.get_mut(&item) {
                        entry.state = ThumbnailState::Suppressed;
                    }
                } else {
                    self.entries.remove(&item);
                }
                evicted.push(item);
            } else {
                break;
            }
        }
        Some(evicted)
    }

    pub fn state(&self, id: &ThumbnailId) -> Option<ThumbnailState> {
        self.entries.get(id).map(|entry| entry.state)
    }

    pub fn touch(&mut self, id: &ThumbnailId) {
        if let Some(index) = self.lru.iter().position(|(cached, _)| cached == id)
            && let Some(item) = self.lru.remove(index)
        {
            self.lru.push_back(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project(revision: u64) -> Value {
        serde_json::json!({"project_id":"p","activation":"a","revision":revision})
    }

    #[test]
    fn only_visible_items_are_queued_and_duplicate_ids_are_deduplicated() {
        let mut state = ContentsThumbnails::default();
        let next = state.set_visible(
            project(1),
            vec![
                ("a".into(), PreviewKind::Texture),
                ("a".into(), PreviewKind::Texture),
                ("b".into(), PreviewKind::Material),
            ],
        );
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].id, "a");
        assert_eq!(state.visible.len(), 3);
        let stale = state.visible[2].clone();
        state.set_visible(project(1), vec![("b".into(), PreviewKind::Material)]);
        assert_eq!(state.state(&stale), Some(ThumbnailState::Queued));
        assert!(state.entries.keys().all(|id| id.id == "b"));
    }

    #[test]
    fn cancelled_before_start_removes_offscreen_running_item_without_failure() {
        let mut state = ContentsThumbnails::default();
        let a = state
            .set_visible(project(1), vec![("A".into(), PreviewKind::Texture)])
            .remove(0);
        assert!(state.mark_running(&a));
        let b = state
            .set_visible(project(1), vec![("B".into(), PreviewKind::Material)])
            .remove(0);
        state.cancelled_before_start(&a);
        assert_eq!(state.state(&a), None);
        assert_eq!(state.state(&b), Some(ThumbnailState::Queued));
        assert_eq!(state.next_ready(), vec![b]);
    }

    #[test]
    fn busy_is_backed_off_and_failure_is_not_retried_by_scroll() {
        let mut state = ContentsThumbnails::default();
        let id = state
            .set_visible(project(1), vec![("a".into(), PreviewKind::Texture)])
            .remove(0);
        assert!(state.mark_running(&id));
        state.busy(&id);
        assert!(state.next_ready().is_empty());
        assert_eq!(state.state(&id), Some(ThumbnailState::Queued));
        state.failed(&id);
        state.set_visible(project(1), vec![("a".into(), PreviewKind::Texture)]);
        assert_eq!(state.state(&id), Some(ThumbnailState::Failed));
        assert!(state.next_ready().is_empty());
    }

    #[test]
    fn project_revision_and_refresh_make_old_completions_stale() {
        let mut state = ContentsThumbnails::default();
        let old = state
            .set_visible(project(1), vec![("a".into(), PreviewKind::Texture)])
            .remove(0);
        state.set_visible(project(2), vec![("a".into(), PreviewKind::Texture)]);
        assert!(state.ready(&old, 64 * 64 * 4).is_none());
        state.refresh();
        assert_eq!(state.generation(), old.generation + 2);
    }

    #[test]
    fn ready_cache_obeys_image_and_byte_budgets() {
        let mut state = ContentsThumbnails::default();
        for base in (0..400).step_by(100) {
            state.set_visible(
                project(1),
                (base..base + 100)
                    .map(|i| (format!("{i}"), PreviewKind::Texture))
                    .collect(),
            );
            let ids = state.visible.clone();
            for id in &ids {
                state.mark_running(id);
                state.ready(id, 64 * 64 * 4);
            }
        }
        assert!(state.lru.len() <= MAX_IMAGES);
        assert!(state.rgba_bytes <= MAX_RGBA_BYTES);
        assert_eq!(
            state.state(&state.lru.front().unwrap().0),
            Some(ThumbnailState::Ready)
        );
        assert!(
            state
                .entries
                .keys()
                .all(|id| id.id.parse::<usize>().unwrap_or(usize::MAX) >= 144)
        );
    }

    #[test]
    fn failed_metadata_is_bounded_while_scroll_keeps_recent_failures() {
        let mut state = ContentsThumbnails::default();
        for base in (0..10_000).step_by(100) {
            state.set_visible(
                project(1),
                (base..base + 100)
                    .map(|i| (format!("{i}"), PreviewKind::Texture))
                    .collect(),
            );
            for id in state.visible.clone() {
                state.mark_running(&id);
                state.failed(&id);
            }
        }
        assert!(state.failed_lru.len() <= MAX_FAILED);
        assert!(state.entries.len() <= MAX_FAILED);
        assert!(state.entries.iter().all(|(id, entry)| {
            entry.state != ThumbnailState::Failed || state.failed_lru.contains(id)
        }));
        assert!(
            (9_900..10_000).all(|i| {
                state.state(&state.visible[i - 9_900]) == Some(ThumbnailState::Failed)
            })
        );
    }

    #[test]
    fn ready_lru_evicts_nonvisible_items_before_visible_images() {
        let mut state = ContentsThumbnails::default();
        let mut ids = Vec::new();
        for i in 0..MAX_IMAGES {
            let id = state
                .set_visible(project(1), vec![(format!("{i}"), PreviewKind::Texture)])
                .remove(0);
            state.mark_running(&id);
            state.ready(&id, 64 * 64 * 4);
            ids.push(id);
        }
        let mut visible = vec![("0".into(), PreviewKind::Texture)];
        visible.extend((2..MAX_IMAGES).map(|i| (i.to_string(), PreviewKind::Texture)));
        visible.push((MAX_IMAGES.to_string(), PreviewKind::Texture));
        let new = state
            .set_visible(project(1), visible)
            .into_iter()
            .next()
            .unwrap();
        state.mark_running(&new);
        let evicted = state.ready(&new, 64 * 64 * 4).unwrap();
        assert!(!evicted.contains(&ids[0]));
        assert!(evicted.contains(&ids[1]));
        assert_eq!(state.state(&ids[0]), Some(ThumbnailState::Ready));
        assert_eq!(state.state(&ids[1]), None);
    }

    #[test]
    fn revision_requeues_entries_under_the_new_id_and_keeps_the_budget() {
        let mut state = ContentsThumbnails::default();
        let old = state
            .set_visible(project(1), vec![("a".into(), PreviewKind::Texture)])
            .remove(0);
        assert!(state.mark_running(&old));
        state.ready(&old, 64 * 64 * 4);
        // The activation holds: only the revision moves, so entries stay
        // instead of the full `invalidate` drop.
        state.revise(project(2));
        let revised = ThumbnailId {
            project: project(2).to_string(),
            id: "a".into(),
            kind: PreviewKind::Texture,
            generation: state.generation(),
        };
        assert_eq!(state.state(&old), None, "the old revision's key is gone");
        assert_eq!(
            state.state(&revised),
            Some(ThumbnailState::Queued),
            "a held entry re-asks instead of dropping"
        );
        assert_eq!(state.next_ready(), vec![revised.clone()]);
        assert_eq!(state.lru.len(), 1, "the image's budget slot survives");
        assert_eq!(state.rgba_bytes, 64 * 64 * 4);
        assert!(state.lru[0].0 == revised);
    }

    #[test]
    fn ready_kept_reenters_ready_without_touching_the_byte_budget() {
        let mut state = ContentsThumbnails::default();
        let old = state
            .set_visible(project(1), vec![("a".into(), PreviewKind::Texture)])
            .remove(0);
        state.mark_running(&old);
        state.ready(&old, 64 * 64 * 4);
        state.revise(project(2));
        let revised = state.next_ready().remove(0);
        assert!(state.mark_running(&revised));
        // The re-fetch answered the same content key: the image stays and
        // the entry re-enters Ready without re-adding its bytes.
        state.ready_kept(&revised);
        assert_eq!(state.state(&revised), Some(ThumbnailState::Ready));
        assert_eq!(state.lru.len(), 1);
        assert_eq!(state.rgba_bytes, 64 * 64 * 4);
        assert!(state.next_ready().is_empty());
    }

    #[test]
    fn replacing_a_kept_image_recounts_its_bytes_once() {
        let mut state = ContentsThumbnails::default();
        let old = state
            .set_visible(project(1), vec![("a".into(), PreviewKind::Texture)])
            .remove(0);
        state.mark_running(&old);
        state.ready(&old, 64 * 64 * 4);
        state.revise(project(2));
        let revised = state.next_ready().remove(0);
        state.mark_running(&revised);
        // The content key changed: the replacement must not count the
        // kept image's bytes a second time.
        state.ready(&revised, 64 * 64 * 4);
        assert_eq!(state.lru.iter().filter(|(id, _)| *id == revised).count(), 1);
        assert_eq!(state.rgba_bytes, 64 * 64 * 4);
    }

    #[test]
    fn activation_still_drops_everything_after_a_revise() {
        let mut state = ContentsThumbnails::default();
        let old = state
            .set_visible(project(1), vec![("a".into(), PreviewKind::Texture)])
            .remove(0);
        state.mark_running(&old);
        state.ready(&old, 64 * 64 * 4);
        state.revise(project(2));
        let revised_id = state.visible[0].clone();
        state.invalidate(project(3));
        assert_eq!(state.entries.len(), 0);
        assert_eq!(state.lru.len(), 0);
        assert_eq!(state.rgba_bytes, 0);
        assert_eq!(state.state(&revised_id), None);
    }
}
