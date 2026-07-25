use super::*;

fn temporary_project(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "prism-cli-{label}-{}.prism",
        spectrum_revisions::SessionId::new()
    ))
}

#[test]
fn required_live_semantic_command_never_falls_back_to_direct_mutation() {
    let project = temporary_project("live-required-no-fallback");
    let human_session = SessionId::new();
    let mut human = Workspace::create_durable(
        Document::new("Live routing source", 400, 300),
        &project,
        Actor {
            id: "human:live-routing-test".into(),
            display_name: "Live routing human".into(),
            kind: ActorKind::Human,
        },
        human_session,
    )
    .unwrap();
    human
        .execute(Command::AddRectangle {
            name: None,
            width: 100,
            height: 80,
            color: [255; 4],
            corner_radius: 0.0,
            x: 10.0,
            y: 20.0,
        })
        .unwrap();
    human.save(None).unwrap();
    drop(human);
    let before = Workspace::load_read_only(&project).unwrap();
    let collaboration = Workspace::start_collaboration(
        &project,
        Some(human_session),
        Actor {
            id: "external-agent:live-routing-test".into(),
            display_name: "Live routing test".into(),
            kind: ActorKind::Agent,
        },
        spectrum_revisions::CollaborationMode::Separate,
    )
    .unwrap();

    let error = run(Cli {
        project: project.clone(),
        session: Some(collaboration.agent_session),
        live: Some(CliLiveMode::Required),
        command: CliCommand::RenameDocument {
            name: "must not land directly".into(),
        },
    })
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("no authenticated live Prism binding")
    );
    assert_eq!(
        Workspace::load_read_only(&project).unwrap().name,
        before.name
    );
    std::fs::remove_file(project).unwrap();
}

#[test]
fn required_live_clone_source_without_a_binding_never_mutates_directly() {
    let project = temporary_project("live-required-clone-no-fallback");
    let source = project.with_extension("png");
    let mut pixels = image::RgbaImage::new(8, 8);
    pixels.put_pixel(1, 1, image::Rgba([210, 40, 90, 255]));
    pixels.save(&source).unwrap();
    let canonical_source = std::fs::canonicalize(&source).unwrap();
    let human_session = SessionId::new();
    let mut human = Workspace::create_durable(
        Document::new("Live Clone routing", 8, 8),
        &project,
        Actor {
            id: "human:live-clone-routing".into(),
            display_name: "Live Clone routing human".into(),
            kind: ActorKind::Human,
        },
        human_session,
    )
    .unwrap();
    human
        .execute_batch(vec![
            Command::AddRaster {
                path: canonical_source,
                name: Some("Clone source".into()),
                x: 0.0,
                y: 0.0,
            },
            Command::AddPaintLayer {
                name: Some("Clone destination".into()),
                width: 8,
                height: 8,
            },
        ])
        .unwrap();
    human.save(None).unwrap();
    let history_before = human.history().unwrap().unwrap().revisions.len();
    drop(human);
    let collaboration = Workspace::start_collaboration(
        &project,
        Some(human_session),
        Actor {
            id: "external-agent:live-clone-routing".into(),
            display_name: "Live Clone routing test".into(),
            kind: ActorKind::Agent,
        },
        spectrum_revisions::CollaborationMode::Together,
    )
    .unwrap();

    let error = run(Cli {
        project: project.clone(),
        session: Some(collaboration.agent_session),
        live: Some(CliLiveMode::Required),
        command: CliCommand::Paint(PaintArgs {
            command: paint::PaintCommand::CloneSource {
                id: 1,
                document_x: 1.5,
                document_y: 1.5,
            },
        }),
    })
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("no authenticated live Prism binding")
    );
    let unchanged = Workspace::open_session(&project, collaboration.agent_session).unwrap();
    assert!(unchanged.document.clone_source.is_none());
    assert!(unchanged.document.sampled_sources.is_empty());
    assert_eq!(
        unchanged.history().unwrap().unwrap().revisions.len(),
        history_before
    );
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(project).unwrap();
}

#[test]
fn required_live_mode_keeps_read_only_commands_and_rejects_standalone_creation() {
    let project = temporary_project("live-required-policy");
    run(Cli {
        project: project.clone(),
        session: None,
        live: None,
        command: CliCommand::Init {
            name: "CLI test".into(),
            width: 400,
            height: 300,
            background: "18191dff".into(),
        },
    })
    .unwrap();

    let listed = run(Cli {
        project: project.clone(),
        session: None,
        live: Some(CliLiveMode::Required),
        command: CliCommand::List,
    })
    .unwrap();
    assert_eq!(listed["document"]["name"], "CLI test");

    let output = temporary_project("live-required-init");
    let error = run(Cli {
        project: output.clone(),
        session: None,
        live: Some(CliLiveMode::Required),
        command: CliCommand::Init {
            name: "blocked".into(),
            width: 10,
            height: 10,
            background: "000000ff".into(),
        },
    })
    .unwrap_err();
    assert!(error.to_string().contains("standalone artifact"));
    assert!(!output.exists());

    std::fs::remove_file(project).unwrap();
}
