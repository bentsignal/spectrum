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

fn agent() -> Actor {
    Actor {
        id: "agent:test".into(),
        display_name: "Agent".into(),
        kind: ActorKind::Agent,
    }
}

#[test]
fn an_agent_branches_from_the_person_and_never_moves_them() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let me = SessionId::new();
    let mut notes = Notebook::create(&path, Notes::default(), person(), me).unwrap();
    notes.execute(Edit::Write("mine".into())).unwrap();
    let start = notes.revision().unwrap();
    drop(notes);

    let collaboration =
        Notebook::start_collaboration(&path, Some(me), agent(), CollaborationMode::Separate)
            .unwrap();
    assert_eq!(collaboration.base_revision, start);
    let mut helper = Notebook::open_session(&path, collaboration.agent_session).unwrap();
    assert_eq!(helper.document.lines, ["mine"]);
    helper.execute(Edit::Write("agent".into())).unwrap();
    drop(helper);

    // The person still sees, and keeps working on, their own line.
    let (seen, at) = Notebook::read_session(&path, me).unwrap();
    assert_eq!((seen.lines, at), (vec!["mine".to_string()], start));
    let mut notes = Notebook::open(&path, person(), me).unwrap();
    notes.execute(Edit::Write("more".into())).unwrap();
    assert_eq!(notes.document.lines, ["mine", "more"]);
    // Both futures remain, branching from where the agent started.
    let history = Notebook::history_of(&path, me).unwrap();
    let children = history
        .revisions
        .iter()
        .filter(|revision| revision.parent_id == Some(start))
        .count();
    assert_eq!(children, 2);
    drop(notes);

    // Jumping to the agent's work and editing there branches from it.
    let (agent_view, agent_at) =
        Notebook::read_session(&path, collaboration.agent_session).unwrap();
    assert_eq!(agent_view.lines, ["mine", "agent"]);
    let mut notes = Notebook::open_at(&path, person(), me, Some(agent_at)).unwrap();
    notes.execute(Edit::Write("after agent".into())).unwrap();
    assert_eq!(notes.document.lines, ["mine", "agent", "after agent"]);
    let (helper_view, _) = Notebook::read_session(&path, collaboration.agent_session).unwrap();
    assert_eq!(helper_view.lines, ["mine", "agent"]);
}

#[test]
fn a_person_follows_an_agent_together_until_they_edit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.spectrum");
    let me = SessionId::new();
    let notes = Notebook::create(&path, Notes::default(), person(), me).unwrap();
    drop(notes);
    let collaboration =
        Notebook::start_collaboration(&path, Some(me), agent(), CollaborationMode::Together)
            .unwrap();
    let mut helper = Notebook::open_session(&path, collaboration.agent_session).unwrap();
    helper.execute(Edit::Write("one".into())).unwrap();
    assert!(matches!(
        Notebook::follow(&path, me).unwrap(),
        CollaborationSync::Advanced { .. }
    ));
    assert_eq!(Notebook::read_session(&path, me).unwrap().0.lines, ["one"]);
    helper.execute(Edit::Write("two".into())).unwrap();
    Notebook::follow(&path, me).unwrap();
    assert_eq!(
        Notebook::read_session(&path, me).unwrap().0.lines,
        ["one", "two"]
    );

    // The person's own edit ends the following; later agent work stays apart.
    let mut notes = Notebook::open(&path, person(), me).unwrap();
    notes.execute(Edit::Write("mine".into())).unwrap();
    drop(notes);
    helper.execute(Edit::Write("three".into())).unwrap();
    assert!(matches!(
        Notebook::follow(&path, me).unwrap(),
        CollaborationSync::Split(_)
    ));
    assert_eq!(
        Notebook::read_session(&path, me).unwrap().0.lines,
        ["one", "two", "mine"]
    );
    assert!(Notebook::following(&path, me).unwrap().is_none());
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

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn a_copied_library_opens_and_keeps_saving() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let path = original.join("notes.spectrum");
    let session = SessionId::new();
    let mut notes = Notebook::create(&path, Notes::default(), person(), session).unwrap();
    for line in ["a", "b", "c"] {
        notes.execute(Edit::Write(line.into())).unwrap();
    }
    drop(notes);
    // A backup restored elsewhere carries its caches along.
    let copy = root.path().join("copy");
    copy_tree(&original, &copy);
    let path = copy.join("notes.spectrum");
    let mut notes = Notebook::open(&path, person(), session).unwrap();
    assert_eq!(notes.document.lines, ["a", "b", "c"]);
    notes.execute(Edit::Write("d".into())).unwrap();
    drop(notes);
    std::fs::remove_dir_all(copy.join(".cache")).unwrap();
    assert_eq!(Notebook::read(&path).unwrap().lines, ["a", "b", "c", "d"]);
}
