use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::*;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Notes {
    lines: Vec<String>,
    attachments: Vec<PathBuf>,
    selected: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Edit {
    Write(String),
    Attach(PathBuf),
    Select(usize),
    Fail,
    Undo,
    Redo,
}

struct NotesModel;

impl Model for NotesModel {
    type Document = Notes;
    type Command = Edit;
    type Output = String;
    const APPLICATION: &'static str = "spectrum.test-notes";
    const NOUN: &'static str = "note";

    fn apply(document: &mut Notes, command: Edit) -> Result<String> {
        match command {
            Edit::Write(line) => document.lines.push(line),
            Edit::Attach(path) => document.attachments.push(path),
            Edit::Select(index) => document.selected = index,
            Edit::Fail => bail!("this edit fails"),
            Edit::Undo | Edit::Redo => bail!("history is the workspace's"),
        }
        Ok("edited".into())
    }

    fn describe(output: &String) -> String {
        output.clone()
    }

    fn stepped(step: Step) -> String {
        format!("{step:?}")
    }

    fn step(command: &Edit) -> Option<Step> {
        match command {
            Edit::Undo => Some(Step::Undo),
            Edit::Redo => Some(Step::Redo),
            _ => None,
        }
    }

    fn transient(command: &Edit) -> bool {
        matches!(command, Edit::Select(_))
    }

    fn document_files(document: &mut Notes, visit: &mut FileVisitor) -> Result<()> {
        for path in &mut document.attachments {
            visit(path, FileRole::Content)?;
        }
        Ok(())
    }

    fn command_files(command: &mut Edit, visit: &mut FileVisitor) -> Result<()> {
        if let Edit::Attach(path) = command {
            visit(path, FileRole::Content)?;
        }
        Ok(())
    }
}

fn person() -> Actor {
    Actor {
        id: "person:test".into(),
        display_name: "Tester".into(),
        kind: ActorKind::Human,
    }
}

type Notebook = Workspace<NotesModel>;

#[test]
fn edits_and_files_survive_reopening() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let attachment = root.path().join("photo.png");
    std::fs::write(&attachment, b"pixels").unwrap();
    let session = SessionId::new();
    let mut notes = Notebook::create(&path, Notes::default(), person(), session).unwrap();
    notes.execute(Edit::Write("first".into())).unwrap();
    notes.execute(Edit::Attach(attachment.clone())).unwrap();
    // The document refers to its own copy, not the original.
    assert_ne!(notes.document.attachments[0], attachment);
    drop(notes);
    std::fs::remove_file(&attachment).unwrap();
    let reopened = Notebook::open(&path, person(), session).unwrap();
    assert_eq!(reopened.document.lines, ["first"]);
    assert_eq!(
        std::fs::read(&reopened.document.attachments[0]).unwrap(),
        b"pixels"
    );
    assert_eq!(Notebook::read(&path).unwrap().lines, ["first"]);
}

#[test]
fn durable_history_undoes_redoes_and_branches() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let mut notes = Notebook::create(&path, Notes::default(), person(), SessionId::new()).unwrap();
    for line in ["a", "b"] {
        notes.execute(Edit::Write(line.into())).unwrap();
    }
    notes.execute(Edit::Undo).unwrap();
    assert_eq!(notes.document.lines, ["a"]);
    assert!(notes.can_redo());
    notes.execute(Edit::Redo).unwrap();
    assert_eq!(notes.document.lines, ["a", "b"]);
    notes.execute(Edit::Undo).unwrap();
    notes.execute(Edit::Write("c".into())).unwrap();
    // Editing from the past keeps the old future as a branch.
    let history = notes.history().unwrap().unwrap();
    assert_eq!(history.revisions.len(), 4);
    assert_eq!(notes.document.lines, ["a", "c"]);
}

#[test]
fn failed_batches_change_nothing_and_selection_is_not_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let mut notes = Notebook::create(&path, Notes::default(), person(), SessionId::new()).unwrap();
    assert!(
        notes
            .execute_batch(vec![Edit::Write("lost".into()), Edit::Fail])
            .is_err()
    );
    assert!(notes.document.lines.is_empty());
    notes.execute(Edit::Select(3)).unwrap();
    assert_eq!(notes.document.selected, 3);
    assert_eq!(notes.history().unwrap().unwrap().revisions.len(), 1);
}

#[test]
fn memory_documents_undo_without_a_file() {
    let mut notes = Notebook::new(Notes::default());
    notes.execute(Edit::Write("a".into())).unwrap();
    notes.execute(Edit::Undo).unwrap();
    assert!(notes.document.lines.is_empty());
    notes.execute(Edit::Redo).unwrap();
    assert_eq!(notes.document.lines, ["a"]);
    assert!(notes.history().unwrap().is_none());
}

#[test]
fn long_histories_snapshot_and_reopen_exactly() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let session = SessionId::new();
    let mut notes = Notebook::create(&path, Notes::default(), person(), session).unwrap();
    for index in 0..250 {
        notes.execute(Edit::Write(index.to_string())).unwrap();
    }
    let expected = notes.document.clone();
    drop(notes);
    let reopened = Notebook::open(&path, person(), session).unwrap();
    assert_eq!(reopened.document, expected);
}

#[test]
fn other_kinds_of_file_are_refused() {
    struct Other;
    impl Model for Other {
        type Document = Notes;
        type Command = Edit;
        type Output = String;
        const APPLICATION: &'static str = "spectrum.test-other";
        const NOUN: &'static str = "other";
        fn apply(document: &mut Notes, command: Edit) -> Result<String> {
            NotesModel::apply(document, command)
        }
        fn describe(output: &String) -> String {
            output.clone()
        }
        fn stepped(step: Step) -> String {
            format!("{step:?}")
        }
        fn document_files(_: &mut Notes, _: &mut FileVisitor) -> Result<()> {
            Ok(())
        }
        fn command_files(_: &mut Edit, _: &mut FileVisitor) -> Result<()> {
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    Notebook::create(&path, Notes::default(), person(), SessionId::new()).unwrap();
    assert!(Workspace::<Other>::read(&path).is_err());
}

#[test]
fn agents_work_in_their_own_session() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let mut notes = Notebook::create(&path, Notes::default(), person(), SessionId::new()).unwrap();
    notes.execute(Edit::Write("mine".into())).unwrap();
    drop(notes);
    let agent = Actor {
        id: "agent:test".into(),
        display_name: "Agent".into(),
        kind: ActorKind::Agent,
    };
    let collaboration =
        Notebook::start_collaboration(&path, None, agent, CollaborationMode::Separate).unwrap();
    let mut agent = Notebook::open_session(&path, collaboration.agent_session).unwrap();
    assert_eq!(agent.document.lines, ["mine"]);
    agent.execute(Edit::Write("theirs".into())).unwrap();
    assert_eq!(Notebook::read(&path).unwrap().lines, ["mine", "theirs"]);
}

#[test]
fn opening_at_the_newest_builds_on_other_sessions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let (desk, cli) = (SessionId::new(), SessionId::new());
    let mut notes = Notebook::create(&path, Notes::default(), person(), desk).unwrap();
    notes.execute(Edit::Write("desk".into())).unwrap();
    drop(notes);
    let mut agent = Notebook::open_newest(&path, person(), cli).unwrap();
    agent.execute(Edit::Write("agent".into())).unwrap();
    drop(agent);
    let notes = Notebook::open_newest(&path, person(), desk).unwrap();
    assert_eq!(notes.document.lines, ["desk", "agent"]);
    assert!(notes.can_undo());
}

#[test]
fn removing_a_document_removes_its_caches() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let attachment = root.path().join("photo.png");
    std::fs::write(&attachment, b"pixels").unwrap();
    let mut notes = Notebook::create(&path, Notes::default(), person(), SessionId::new()).unwrap();
    notes.execute(Edit::Attach(attachment)).unwrap();
    drop(notes);
    crate::remove(&path).unwrap();
    assert!(!path.exists());
    let cache = root.path().join(".cache");
    let leftover = std::fs::read_dir(&cache)
        .unwrap()
        .chain(std::fs::read_dir(cache.join("files")).unwrap())
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "files" && !name.starts_with('.'))
        .collect::<Vec<_>>();
    assert!(leftover.is_empty(), "{leftover:?}");
}
