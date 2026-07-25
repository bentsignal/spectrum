use super::*;

#[test]
fn post_rename_sync_failure_reports_that_the_destination_exists() {
    let directory = std::env::temp_dir().join(format!(
        "prism-optimized-sync-failure-{}",
        spectrum_revisions::SessionId::new()
    ));
    fs::create_dir_all(&directory).unwrap();
    let temporary = directory.join("private.prism");
    let output = directory.join("published.prism");
    let mut cleanup = TemporaryProject::create(temporary.clone()).unwrap();
    fs::write(&temporary, b"published bytes").unwrap();

    let error = publish_optimized_copy(&temporary, &output, &mut cleanup, |source, destination| {
        fs::rename(source, destination)?;
        Err(spectrum_revisions::RevisionError::PublishedButNotSynced {
            destination: destination.to_owned(),
            source: std::io::Error::other("injected sync failure"),
        })
    })
    .unwrap_err();

    assert!(error.to_string().contains("optimized copy exists at"));
    drop(cleanup);
    assert_eq!(fs::read(output).unwrap(), b"published bytes");
    fs::remove_dir_all(directory).unwrap();
}
