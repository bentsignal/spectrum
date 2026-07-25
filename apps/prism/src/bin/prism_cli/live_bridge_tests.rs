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
