use super::tests::temporary_project;
use super::*;

#[test]
fn rename_document_cli_changes_metadata_without_changing_the_project_path() {
    let project = temporary_project("rename-document");
    let project_arg = project.to_str().unwrap();
    for arguments in [
        vec!["init", "Original", "--width", "80", "--height", "60"],
        vec!["rename-document", "Campaign"],
    ] {
        let mut cli = vec!["prism", "--project", project_arg];
        cli.extend(arguments);
        run(Cli::try_parse_from(cli).unwrap()).unwrap();
    }
    assert_eq!(
        Workspace::load_read_only(&project).unwrap().name,
        "Campaign"
    );
    assert!(project.exists());
    assert!(!project.with_file_name("Campaign.prism").exists());
    std::fs::remove_file(project).unwrap();
}
