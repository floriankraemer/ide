//! Project tree sort direction: translation only, `project-model` owns the
//! rule. Persisting the choice is `ui-shell`'s job (ADR-0002) — this crate
//! has no settings dependency.

use project_model::SortOrder;

use crate::AppSession;

impl AppSession {
    /// Folders always lead; this only covers the name comparison within
    /// each group.
    pub fn tree_sort_order(&self) -> SortOrder {
        self.project.sort_order()
    }

    /// Change the tree's sort direction and re-order the open tree in place.
    pub fn set_tree_sort_order(&mut self, order: SortOrder) {
        self.project.set_sort_order(order);
    }
}
