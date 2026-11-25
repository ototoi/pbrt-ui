//! What the Inspector says under one edit field: the core's reason for rejecting
//! an edit, or that another edit changed
//! the value while it was being typed. Pure so it can be tested.
//!
//! Fields are named by a form id that survives revisions (`parameter:<target>:
//! <name>`, `transform:<operation>`, `label:<id>`); the forms themselves are
//! rebuilt whenever the revision or edition changes.
use std::collections::{HashMap, HashSet};

pub const CHANGED_ELSEWHERE: &str = "Another edit changed this value";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditFeedback {
    /// Forms whose edit was sent and not answered yet.
    pending: HashSet<String>,
    /// Forms whose own edit was applied; their next rebuild is expected.
    applied: HashSet<String>,
    /// Rejected forms to rebuild from the core's values.
    reset: HashSet<String>,
    messages: HashMap<String, String>,
}

impl EditFeedback {
    /// A form sent its edit; an older message no longer applies.
    pub fn submitted(&mut self, form: &str) {
        self.pending.insert(form.to_owned());
        self.messages.remove(form);
    }

    pub fn applied(&mut self, form: &str) {
        self.pending.remove(form);
        self.applied.insert(form.to_owned());
        self.messages.remove(form);
    }

    /// The core refused the edit: show why and put the field back.
    pub fn rejected(&mut self, form: &str, reason: &str) {
        self.pending.remove(form);
        self.reset.insert(form.to_owned());
        self.messages.insert(form.to_owned(), reason.to_owned());
    }

    /// A field was rebuilt for a new revision. Its own edit explains the
    /// change; otherwise unsent typing was overwritten by another edit.
    pub fn replaced(&mut self, form: &str, was_modified: bool) {
        if self.applied.remove(form) || self.pending.contains(form) {
            return;
        }
        if was_modified {
            self.messages
                .insert(form.to_owned(), CHANGED_ELSEWHERE.to_owned());
        }
    }

    /// Whether a rejected field must be rebuilt now (consumed once).
    pub fn take_reset(&mut self, form: &str) -> bool {
        self.reset.remove(form)
    }

    pub fn is_pending(&self, form: &str) -> bool {
        self.pending.contains(form)
    }

    pub fn message(&self, form: &str) -> Option<&str> {
        self.messages.get(form).map(String::as_str)
    }

    /// Another selection or document: nothing carries over.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejected_edit_explains_itself_and_resets_the_field() {
        let mut feedback = EditFeedback::default();
        feedback.submitted("parameter:n:radius");
        feedback.rejected("parameter:n:radius", "radius must be positive");
        assert_eq!(
            feedback.message("parameter:n:radius"),
            Some("radius must be positive")
        );
        assert!(feedback.take_reset("parameter:n:radius"));
        assert!(!feedback.take_reset("parameter:n:radius"));
        feedback.submitted("parameter:n:radius");
        assert_eq!(feedback.message("parameter:n:radius"), None);
    }

    #[test]
    fn the_fields_own_edit_rebuilds_it_quietly() {
        let mut feedback = EditFeedback::default();
        feedback.submitted("transform:t");
        feedback.applied("transform:t");
        feedback.replaced("transform:t", true);
        assert_eq!(feedback.message("transform:t"), None);
        // The rebuild before the answer arrives is quiet too.
        feedback.submitted("label:n");
        feedback.replaced("label:n", true);
        assert_eq!(feedback.message("label:n"), None);
    }

    #[test]
    fn another_edit_over_unsent_typing_is_reported() {
        let mut feedback = EditFeedback::default();
        feedback.replaced("parameter:n:radius", false);
        assert_eq!(feedback.message("parameter:n:radius"), None);
        feedback.replaced("parameter:n:radius", true);
        assert_eq!(
            feedback.message("parameter:n:radius"),
            Some(CHANGED_ELSEWHERE)
        );
        // Only the first rebuild after an own edit is expected.
        feedback.submitted("transform:t");
        feedback.applied("transform:t");
        feedback.replaced("transform:t", false);
        feedback.replaced("transform:t", true);
        assert_eq!(feedback.message("transform:t"), Some(CHANGED_ELSEWHERE));
        feedback.clear();
        assert_eq!(feedback.message("transform:t"), None);
    }
}
