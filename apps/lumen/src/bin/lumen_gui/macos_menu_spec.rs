#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(isize)]
pub(super) enum NativeMenuAction {
    NewCatalog = 43_001,
    OpenCatalog,
    ImportPhotos,
    MoveCatalog,
    ExportPhotos,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    ToggleTerminal,
    ToggleWorkspaceView,
    PreviousPhoto,
    NextPhoto,
    ToggleHistory,
    FitPhoto,
    ZoomIn,
    ZoomOut,
}

impl NativeMenuAction {
    pub(super) const ALL: [Self; 19] = [
        Self::NewCatalog,
        Self::OpenCatalog,
        Self::ImportPhotos,
        Self::MoveCatalog,
        Self::ExportPhotos,
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
        Self::ToggleTerminal,
        Self::ToggleWorkspaceView,
        Self::PreviousPhoto,
        Self::NextPhoto,
        Self::ToggleHistory,
        Self::FitPhoto,
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
    const fn command(key: &'static str) -> Self {
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

pub(super) const ACTION_MENU_ITEMS: [ActionMenuItemSpec; 19] = [
    action_spec(
        NativeMenuSection::File,
        "New Shoot",
        NativeMenuAction::NewCatalog,
        Some(ActionKeyEquivalent::command("n")),
        false,
    ),
    action_spec(
        NativeMenuSection::File,
        "Open Catalog…",
        NativeMenuAction::OpenCatalog,
        Some(ActionKeyEquivalent::command("o")),
        false,
    ),
    action_spec(
        NativeMenuSection::File,
        "Move Catalog…",
        NativeMenuAction::MoveCatalog,
        Some(ActionKeyEquivalent::command_shift("m")),
        false,
    ),
    action_spec(
        NativeMenuSection::File,
        "Import Photos…",
        NativeMenuAction::ImportPhotos,
        Some(ActionKeyEquivalent::command("i")),
        true,
    ),
    action_spec(
        NativeMenuSection::File,
        "Export Photos…",
        NativeMenuAction::ExportPhotos,
        Some(ActionKeyEquivalent::command("e")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Undo Last Edit",
        NativeMenuAction::Undo,
        Some(ActionKeyEquivalent::command("z")),
        false,
    ),
    action_spec(
        NativeMenuSection::Edit,
        "Redo Last Edit",
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
        NativeMenuSection::View,
        "Show Terminal",
        NativeMenuAction::ToggleTerminal,
        Some(ActionKeyEquivalent::command("j")),
        false,
    ),
    action_spec(
        NativeMenuSection::View,
        "Show Catalog",
        NativeMenuAction::ToggleWorkspaceView,
        Some(ActionKeyEquivalent::command("l")),
        true,
    ),
    action_spec(
        NativeMenuSection::View,
        "Previous Photo",
        NativeMenuAction::PreviousPhoto,
        Some(ActionKeyEquivalent::command("[")),
        true,
    ),
    action_spec(
        NativeMenuSection::View,
        "Next Photo",
        NativeMenuAction::NextPhoto,
        Some(ActionKeyEquivalent::command("]")),
        false,
    ),
    action_spec(
        NativeMenuSection::View,
        "Show History",
        NativeMenuAction::ToggleHistory,
        Some(ActionKeyEquivalent::command("h")),
        true,
    ),
    action_spec(
        NativeMenuSection::View,
        "Fit Photo",
        NativeMenuAction::FitPhoto,
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
    fn action_spec_has_unique_tags_and_shortcuts() {
        let tags: HashSet<_> = ACTION_MENU_ITEMS
            .iter()
            .map(|spec| spec.action as isize)
            .collect();
        assert_eq!(tags.len(), NativeMenuAction::ALL.len());
        let shortcuts: Vec<_> = ACTION_MENU_ITEMS
            .iter()
            .filter_map(|spec| spec.equivalent)
            .collect();
        assert_eq!(
            shortcuts.iter().copied().collect::<HashSet<_>>().len(),
            shortcuts.len()
        );
    }

    #[test]
    fn catalog_and_photo_actions_live_in_expected_native_menus() {
        let new_shoot = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::NewCatalog)
            .unwrap();
        let import = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::ImportPhotos)
            .unwrap();
        let move_catalog = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::MoveCatalog)
            .unwrap();
        let undo = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::Undo)
            .unwrap();
        let select_all = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::SelectAll)
            .unwrap();
        let workspace_view = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::ToggleWorkspaceView)
            .unwrap();
        assert_eq!(import.section, NativeMenuSection::File);
        assert_eq!(import.equivalent, Some(ActionKeyEquivalent::command("i")));
        assert!(import.separator_before);
        assert_eq!(
            move_catalog.equivalent,
            Some(ActionKeyEquivalent::command_shift("m"))
        );
        assert!(!move_catalog.separator_before);
        let move_index = ACTION_MENU_ITEMS
            .iter()
            .position(|spec| spec.action == NativeMenuAction::MoveCatalog)
            .unwrap();
        let import_index = ACTION_MENU_ITEMS
            .iter()
            .position(|spec| spec.action == NativeMenuAction::ImportPhotos)
            .unwrap();
        assert_eq!(move_index + 1, import_index);
        assert_eq!(new_shoot.title, "New Shoot");
        assert_eq!(
            new_shoot.equivalent,
            Some(ActionKeyEquivalent::command("n"))
        );
        assert_eq!(undo.section, NativeMenuSection::Edit);
        assert_eq!(select_all.section, NativeMenuSection::Edit);
        assert_eq!(
            select_all.equivalent,
            Some(ActionKeyEquivalent::command("a"))
        );
        assert_eq!(workspace_view.section, NativeMenuSection::View);
        assert_eq!(workspace_view.title, "Show Catalog");
        assert_eq!(
            workspace_view.equivalent,
            Some(ActionKeyEquivalent::command("l"))
        );
    }

    #[test]
    fn history_keeps_the_lumen_command_h_shortcut() {
        let history = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::ToggleHistory)
            .unwrap();
        assert_eq!(history.equivalent, Some(ActionKeyEquivalent::command("h")));
    }

    #[test]
    fn terminal_uses_native_command_j() {
        let terminal = ACTION_MENU_ITEMS
            .iter()
            .find(|spec| spec.action == NativeMenuAction::ToggleTerminal)
            .unwrap();
        assert_eq!(terminal.section, NativeMenuSection::View);
        assert_eq!(terminal.equivalent, Some(ActionKeyEquivalent::command("j")));
    }
}
