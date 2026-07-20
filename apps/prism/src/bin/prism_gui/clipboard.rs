use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::*;

const CLIPBOARD_PREFIX: &str = "spectrum-prism-layer:";
const CLIPBOARD_FORMAT: &str = "spectrum.prism.layer-clipboard";
const CLIPBOARD_VERSION: u32 = 1;
const MAX_CLIPBOARD_BYTES: usize = 4 * 1024 * 1024 + 1024;
const COPY_OFFSET: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClipboardOperation {
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ClipboardEnvelope {
    format: String,
    version: u32,
    operation: ClipboardOperation,
    transfer: prism_core::LayerTransfer,
}

impl ClipboardEnvelope {
    fn from_selected(document: &Document, operation: ClipboardOperation) -> Result<Self> {
        Ok(Self {
            format: CLIPBOARD_FORMAT.into(),
            version: CLIPBOARD_VERSION,
            operation,
            transfer: prism_core::LayerTransfer::from_selected(document)?,
        })
    }

    fn encode(&self) -> Result<String> {
        self.validate()?;
        let json = serde_json::to_string(self).context("could not encode Prism clipboard data")?;
        let encoded = format!("{CLIPBOARD_PREFIX}{json}");
        if encoded.len() > MAX_CLIPBOARD_BYTES {
            bail!("Prism clipboard data exceeds the 4 MiB metadata limit");
        }
        Ok(encoded)
    }

    fn decode(value: &str) -> Result<Self> {
        if value.len() > MAX_CLIPBOARD_BYTES {
            bail!("Prism clipboard data exceeds the 4 MiB metadata limit");
        }
        let json = value
            .strip_prefix(CLIPBOARD_PREFIX)
            .context("clipboard does not contain a Prism layer")?;
        let envelope: Self = serde_json::from_str(json).context("invalid Prism clipboard data")?;
        envelope.validate()?;
        Ok(envelope)
    }

    fn validate(&self) -> Result<()> {
        if self.format != CLIPBOARD_FORMAT {
            bail!("unsupported Prism clipboard format {}", self.format);
        }
        if self.version != CLIPBOARD_VERSION {
            bail!(
                "unsupported Prism clipboard version {} (expected {CLIPBOARD_VERSION})",
                self.version
            );
        }
        // Reuse the core envelope validator rather than maintaining a weaker GUI copy.
        let transfer = self.transfer.to_json()?;
        prism_core::LayerTransfer::from_json(&transfer)?;
        Ok(())
    }

    fn into_paste_command(mut self) -> Command {
        if self.operation == ClipboardOperation::Copy {
            self.transfer.layer.name.push_str(" copy");
            self.transfer.layer.transform.x += COPY_OFFSET;
            self.transfer.layer.transform.y += COPY_OFFSET;
        }
        Command::InsertLayer {
            transfer: Box::new(self.transfer),
            index: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardEventKind {
    Copy,
    Cut,
    Paste,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct LayerClipboardState {
    pub(super) workspace_ready: bool,
    pub(super) selection_present: bool,
    pub(super) keyboard_focus: bool,
    pub(super) terminal_visible: bool,
    pub(super) modal_open: bool,
    pub(super) history_visible: bool,
    pub(super) interaction_active: bool,
}

impl LayerClipboardState {
    fn routes_to_canvas(self, event: ClipboardEventKind) -> bool {
        if !self.workspace_ready
            || self.keyboard_focus
            || self.terminal_visible
            || self.modal_open
            || self.history_visible
            || self.interaction_active
        {
            return false;
        }
        event == ClipboardEventKind::Paste || self.selection_present
    }
}

fn clipboard_event(input: &egui::InputState) -> Option<(ClipboardEventKind, Option<String>)> {
    input.events.iter().find_map(|event| match event {
        egui::Event::Copy => Some((ClipboardEventKind::Copy, None)),
        egui::Event::Cut => Some((ClipboardEventKind::Cut, None)),
        egui::Event::Paste(text) => Some((ClipboardEventKind::Paste, Some(text.clone()))),
        _ => None,
    })
}

impl PrismApp {
    pub(super) fn layer_clipboard_state(&self, context: &egui::Context) -> LayerClipboardState {
        LayerClipboardState {
            workspace_ready: self.workspace_initialized,
            selection_present: self.workspace.document.selected.is_some(),
            keyboard_focus: context.egui_wants_keyboard_input()
                || self.inline_text_editor.is_some(),
            terminal_visible: self.terminal.visible(),
            modal_open: self.has_modal_surface(),
            history_visible: self.history.visible,
            interaction_active: self.workspace.interaction_active(),
        }
    }

    pub(super) fn handle_layer_clipboard_events(&mut self, context: &egui::Context) {
        let state = self.layer_clipboard_state(context);
        let Some((event, text)) = context.input(clipboard_event) else {
            return;
        };
        if !state.routes_to_canvas(event) {
            return;
        }
        match event {
            ClipboardEventKind::Copy => {
                self.copy_selected_layer(context, ClipboardOperation::Copy);
            }
            ClipboardEventKind::Cut => {
                self.copy_selected_layer(context, ClipboardOperation::Cut);
            }
            ClipboardEventKind::Paste => {
                let Some(text) = text else {
                    return;
                };
                match ClipboardEnvelope::decode(&text) {
                    Ok(envelope) => {
                        self.execute(envelope.into_paste_command());
                    }
                    Err(error) => {
                        self.status = format!("Could not paste layer: {error:#}");
                        self.status_error = true;
                    }
                }
            }
        }
    }

    fn copy_selected_layer(&mut self, context: &egui::Context, operation: ClipboardOperation) {
        let encoded = ClipboardEnvelope::from_selected(&self.workspace.document, operation)
            .and_then(|envelope| envelope.encode());
        let encoded = match encoded {
            Ok(encoded) => encoded,
            Err(error) => {
                self.status = format!("Could not copy layer: {error:#}");
                self.status_error = true;
                return;
            }
        };

        // Queue the clipboard write before removing a cut layer. The actual document mutation
        // remains the existing durable, one-revision RemoveLayer command.
        context.copy_text(encoded);
        if operation == ClipboardOperation::Cut {
            if let Some(id) = self.workspace.document.selected {
                self.execute(Command::RemoveLayer { id });
            }
        } else {
            self.status = "Copied layer".into();
            self.status_error = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_document() -> Document {
        let mut document = Document::new("Clipboard", 800, 600);
        document.layers.push(Layer {
            id: 7,
            name: "Card".into(),
            transform: Transform {
                x: 32.0,
                y: 48.0,
                ..Default::default()
            },
            kind: LayerKind::Rectangle {
                width: 120,
                height: 80,
                color: [10, 20, 30, 255],
                corner_radius: 8.0,
            },
            ..Default::default()
        });
        document.selected = Some(7);
        document.next_id = 8;
        document
    }

    fn paste_command(value: &str) -> Result<Command> {
        Ok(ClipboardEnvelope::decode(value)?.into_paste_command())
    }

    #[test]
    fn envelope_is_prefixed_versioned_and_distinguishes_copy_from_cut() {
        let document = source_document();
        let copied = ClipboardEnvelope::from_selected(&document, ClipboardOperation::Copy)
            .unwrap()
            .encode()
            .unwrap();
        let cut = ClipboardEnvelope::from_selected(&document, ClipboardOperation::Cut)
            .unwrap()
            .encode()
            .unwrap();
        assert!(copied.starts_with(CLIPBOARD_PREFIX));
        assert_eq!(
            ClipboardEnvelope::decode(&copied).unwrap().operation,
            ClipboardOperation::Copy
        );
        assert_eq!(
            ClipboardEnvelope::decode(&cut).unwrap().operation,
            ClipboardOperation::Cut
        );
    }

    #[test]
    fn event_extraction_returns_the_first_clipboard_event_and_its_paste_text() {
        let input = egui::InputState {
            events: vec![
                egui::Event::Text("ignored".into()),
                egui::Event::Paste("layer payload".into()),
                egui::Event::Copy,
            ],
            ..Default::default()
        };
        assert_eq!(
            clipboard_event(&input),
            Some((ClipboardEventKind::Paste, Some("layer payload".into())))
        );
        assert_eq!(clipboard_event(&egui::InputState::default()), None);
    }

    #[test]
    fn copy_paste_offsets_and_renames_in_exactly_one_revision() {
        let source = Workspace::new(source_document(), None);
        let encoded = ClipboardEnvelope::from_selected(&source.document, ClipboardOperation::Copy)
            .unwrap()
            .encode()
            .unwrap();
        assert!(!source.can_undo());
        let mut destination = Workspace::new(Document::new("Destination", 800, 600), None);
        assert!(!destination.can_undo());
        destination
            .execute(paste_command(&encoded).unwrap())
            .unwrap();
        let pasted = destination.document.layers.last().unwrap();
        assert_eq!(pasted.name, "Card copy");
        assert_eq!(pasted.transform.x, 48.0);
        assert_eq!(pasted.transform.y, 64.0);
        assert!(destination.can_undo());
        destination.execute(Command::Undo).unwrap();
        assert!(destination.document.layers.is_empty());
        assert!(destination.execute(Command::Undo).is_err());
    }

    #[test]
    fn cut_and_cut_paste_each_record_one_revision_without_changing_placement() {
        let document = source_document();
        let encoded = ClipboardEnvelope::from_selected(&document, ClipboardOperation::Cut)
            .unwrap()
            .encode()
            .unwrap();
        let mut source = Workspace::new(document, None);
        source.execute(Command::RemoveLayer { id: 7 }).unwrap();
        assert!(source.document.layers.is_empty());
        source.execute(Command::Undo).unwrap();
        assert_eq!(source.document.layers.len(), 1);
        assert!(source.execute(Command::Undo).is_err());

        let mut destination = Workspace::new(Document::new("Destination", 800, 600), None);
        destination
            .execute(paste_command(&encoded).unwrap())
            .unwrap();
        let pasted = destination.document.layers.last().unwrap();
        assert_eq!(pasted.name, "Card");
        assert_eq!(pasted.transform.x, 32.0);
        assert_eq!(pasted.transform.y, 48.0);
        destination.execute(Command::Undo).unwrap();
        assert!(destination.execute(Command::Undo).is_err());
    }

    #[test]
    fn ordinary_malformed_and_wrong_version_clipboards_are_rejected_atomically() {
        assert!(ClipboardEnvelope::decode("ordinary text").is_err());
        assert!(ClipboardEnvelope::decode("spectrum-prism-layer:{").is_err());

        let before = source_document();
        let mut envelope =
            ClipboardEnvelope::from_selected(&before, ClipboardOperation::Copy).unwrap();
        envelope.version += 1;
        let wrong_version = format!(
            "{CLIPBOARD_PREFIX}{}",
            serde_json::to_string(&envelope).unwrap()
        );
        let workspace = Workspace::new(before.clone(), None);
        assert!(paste_command(&wrong_version).is_err());
        assert_eq!(workspace.document, before);
        assert!(!workspace.can_undo());
    }

    #[test]
    fn focus_modal_terminal_history_and_interactions_keep_events_off_the_canvas() {
        let available = LayerClipboardState {
            workspace_ready: true,
            selection_present: true,
            ..Default::default()
        };
        assert!(available.routes_to_canvas(ClipboardEventKind::Copy));
        assert!(available.routes_to_canvas(ClipboardEventKind::Cut));
        assert!(available.routes_to_canvas(ClipboardEventKind::Paste));
        for unavailable in [
            LayerClipboardState {
                keyboard_focus: true,
                ..available
            },
            LayerClipboardState {
                terminal_visible: true,
                ..available
            },
            LayerClipboardState {
                modal_open: true,
                ..available
            },
            LayerClipboardState {
                history_visible: true,
                ..available
            },
            LayerClipboardState {
                interaction_active: true,
                ..available
            },
        ] {
            assert!(!unavailable.routes_to_canvas(ClipboardEventKind::Copy));
            assert!(!unavailable.routes_to_canvas(ClipboardEventKind::Cut));
            assert!(!unavailable.routes_to_canvas(ClipboardEventKind::Paste));
        }
        assert!(!LayerClipboardState::default().routes_to_canvas(ClipboardEventKind::Paste));
        assert!(
            !LayerClipboardState {
                selection_present: false,
                ..available
            }
            .routes_to_canvas(ClipboardEventKind::Copy)
        );
    }
}
