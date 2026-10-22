//! Selection defaults belong to the Git presentation backend, not row rendering.
use super::actions::Change;
use std::collections::HashSet;
#[derive(Default)]
pub struct FileSelection {
    pub selected: HashSet<String>,
    excluded: HashSet<String>,
}
pub struct SelectionState {
    pub all: bool,
    pub partial: bool,
    pub empty: bool,
}
impl FileSelection {
    pub fn state<'a>(&self, files: impl IntoIterator<Item = &'a Change>) -> SelectionState {
        let (mut total, mut checked) = (0, 0);
        for file in files {
            total += 1;
            checked += usize::from(self.selected.contains(&file.path));
        }
        SelectionState {
            all: total > 0 && total == checked,
            partial: checked > 0 && checked < total,
            empty: total == 0,
        }
    }
    pub fn set_all<'a>(&mut self, files: impl IntoIterator<Item = &'a Change>, checked: bool) {
        for file in files {
            self.set(&file.path, checked);
        }
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.excluded.clear();
    }
    pub fn update(&mut self, files: &[Change]) {
        self.excluded.retain(|p| files.iter().any(|f| &f.path == p));
        self.selected = files
            .iter()
            .filter(|f| !self.excluded.contains(&f.path))
            .map(|f| f.path.clone())
            .collect();
    }
    pub fn set(&mut self, path: &str, checked: bool) {
        if checked {
            self.excluded.remove(path);
            self.selected.insert(path.into());
        } else {
            self.selected.remove(path);
            self.excluded.insert(path.into());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &str) -> Change {
        Change {
            path: path.into(),
            previous: None,
            status: "??".into(),
            directory: false,
        }
    }
    #[test]
    fn bulk_selection_covers_all_groups_and_survives_refresh() {
        let files = vec![file("one"), file("two"), file("three")];
        let mut s = FileSelection::default();
        s.update(&files);
        assert!(s.state(&files).all);
        s.set_all(&files, false);
        assert!(s.selected.is_empty());
        s.update(&files);
        assert!(s.selected.is_empty());
        s.set_all(files[..2].iter(), true);
        assert!(s.state(&files).partial);
        assert!(s.state(files[..2].iter()).all);
        assert!(!s.selected.contains("three"));
        s.set_all(&files, true);
        assert!(s.state(&files).all);
        assert!(s.state([].iter()).empty);
    }
    #[test]
    fn select_new_files_without_reselecting_user_exclusions() {
        let mut s = FileSelection::default();
        s.update(&[file("one"), file("two")]);
        assert_eq!(s.selected.len(), 2);
        s.set("one", false);
        s.update(&[file("one"), file("two"), file("three")]);
        assert!(!s.selected.contains("one"));
        assert!(s.selected.contains("three"));
        s.set("one", true);
        assert!(s.selected.contains("one"));
    }
}
