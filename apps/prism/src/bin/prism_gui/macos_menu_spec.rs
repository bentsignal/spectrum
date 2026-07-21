#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(isize)]
pub(super) enum NativeMenuAction {
    NewDocument = 44_001,
    OpenDocument,
    MoveProject,
    Export,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Deselect,
    ToggleHistory,
    ToggleTerminal,
    FitCanvas,
    ZoomIn,
    ZoomOut,
}

impl NativeMenuAction {
    pub(super) const ALL: [Self; 16] = [
        Self::NewDocument,
        Self::OpenDocument,
        Self::MoveProject,
        Self::Export,
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
        Self::Deselect,
        Self::ToggleHistory,
        Self::ToggleTerminal,
        Self::FitCanvas,
        Self::ZoomIn,
        Self::ZoomOut,
    ];

    pub(super) fn from_tag(tag: isize) -> Option<Self> {
        Self::ALL.into_iter().find(|action| *action as isize == tag)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum NativeMenuSection {
    File,
    Edit,
    View,
}

impl NativeMenuSection {
    pub(super) const fn title(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum KeyModifiers {
    Command,
    CommandShift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ActionKeyEquivalent {
    pub(super) key: &'static str,
    pub(super) modifiers: KeyModifiers,
}

impl ActionKeyEquivalent {
    pub(super) const fn command(key: &'static str) -> Self {
        Self {
            key,
            modifiers: KeyModifiers::Command,
        }
    }

    const fn command_shift(key: &'static str) -> Self {
        Self {
            key,
            modifiers: KeyModifiers::CommandShift,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ActionMenuItemSpec {
    pub(super) section: NativeMenuSection,
    pub(super) title: &'static str,
    pub(super) action: NativeMenuAction,
    pub(super) equivalent: Option<ActionKeyEquivalent>,
    pub(super) separator_before: bool,
}

const fn action_spec(
    section: NativeMenuSection,
    title: &'static str,
    action: NativeMenuAction,
    equivalent: Option<ActionKeyEquivalent>,
    separator_before: bool,
) -> ActionMenuItemSpec {
    ActionMenuItemSpec {
        section,
        title,
        action,
        equivalent,
        separator_before,
    }
}

pub(super) const ACTION_MENU_ITEMS: [ActionMenuItemSpec; 16] = [
    action_spec(
        NativeMenuSection::File,
        "New Document",
        NativeMenuAction::NewDocument,
        Some(ActionKeyEquivalent::command("n")),
        false,
    ),
    action_spec(
        NativeMenuSection::File,
        "Open…",
        NativeMenuAction::OpenDocument,
        Some(ActionKeyEquivalent::command("o")),
        false,
    ),
    action_spec(
        NativeMenuSection::File,
        "Move Project…",
        NativeMenuAction::MoveProject,
        None,
        true,
    ),
    action_spec(
        NativeMenuSection::File,
        "Export…",
        NativeMenuAction::Export,
        Some(ActionKeyEquivalent::command("e")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Undo",
        NativeMenuAction::Undo,
        Some(ActionKeyEquivalent::command("z")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Redo",
        NativeMenuAction::Redo,
        Some(ActionKeyEquivalent::command_shift("z")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Cut",
        NativeMenuAction::Cut,
        Some(ActionKeyEquivalent::command("x")),
        true,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Copy",
        NativeMenuAction::Copy,
        Some(ActionKeyEquivalent::command("c")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Paste",
        NativeMenuAction::Paste,
        Some(ActionKeyEquivalent::command("v")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Select All",
        NativeMenuAction::SelectAll,
        Some(ActionKeyEquivalent::command("a")),
        true,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Deselect",
        NativeMenuAction::Deselect,
        Some(ActionKeyEquivalent::command("d")),
        false,
    ),
    action_spec(
        NativeMenuSection::View,
        "Show History",
        NativeMenuAction::ToggleHistory,
        Some(ActionKeyEquivalent::command_shift("h")),
        false,
    ),
    action_spec(
        NativeMenuSection::View,
        "Show Terminal",
        NativeMenuAction::ToggleTerminal,
        Some(ActionKeyEquivalent::command("j")),
        false,
    ),
    action_spec(
        NativeMenuSection::View,
        "Fit Canvas",
        NativeMenuAction::FitCanvas,
        Some(ActionKeyEquivalent::command("0")),
        true,
    ),
    action_spec(
        NativeMenuSection::View,
        "Zoom In",
        NativeMenuAction::ZoomIn,
        Some(ActionKeyEquivalent::command("+")),
        false,
    ),
    action_spec(
        NativeMenuSection::View,
        "Zoom Out",
        NativeMenuAction::ZoomOut,
        Some(ActionKeyEquivalent::command("-")),
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn action_spec_has_unique_tags_and_key_equivalents() {
        let tags: HashSet<_> = ACTION_MENU_ITEMS
            .iter()
            .map(|spec| spec.action as isize)
            .collect();
        assert_eq!(tags.len(), NativeMenuAction::ALL.len());
        assert_eq!(ACTION_MENU_ITEMS.len(), NativeMenuAction::ALL.len());

        let equivalents: Vec<_> = ACTION_MENU_ITEMS
            .iter()
            .filter_map(|spec| spec.equivalent)
            .collect();
        assert_eq!(
            equivalents.iter().copied().collect::<HashSet<_>>().len(),
            equivalents.len()
        );
    }

    #[test]
    fn file_export_uses_command_e() {
        let export = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::Export)
            .unwrap();
        assert_eq!(export.section, NativeMenuSection::File);
        assert_eq!(export.title, "Export…");
        assert_eq!(export.equivalent, Some(ActionKeyEquivalent::command("e")));
    }

    #[test]
    fn selection_actions_use_standard_edit_menu_chords() {
        let select_all = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::SelectAll)
            .unwrap();
        let deselect = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::Deselect)
            .unwrap();
        assert_eq!(select_all.section, NativeMenuSection::Edit);
        assert_eq!(
            select_all.equivalent,
            Some(ActionKeyEquivalent::command("a"))
        );
        assert_eq!(deselect.section, NativeMenuSection::Edit);
        assert_eq!(deselect.equivalent, Some(ActionKeyEquivalent::command("d")));
    }
}
