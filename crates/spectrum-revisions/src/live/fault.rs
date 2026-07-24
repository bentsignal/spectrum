#[cfg(test)]
use super::RevisionError;
use super::RevisionResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PublishFault {
    SlotSealed,
    CandidateSynced,
    IntentCreated,
    PreExchangeValidated,
    Exchanged,
    SlotWritable,
    MarkerCreated,
    IntentRemoved,
    LinkedSlotUnlinked,
    FullCopyPublished,
    SeedMirrorCreated,
    WorkingPoisonRenamed,
    WorkingPoisonSynced,
    WorkingRecoveryMarkerRenamed,
    WorkingRecoveryMarkerSynced,
    WorkingPoisonRemoved,
    WorkingPoisonRemovalSynced,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum CrashMode {
    Exit,
    Abort,
    Kill,
}

#[cfg(test)]
thread_local! {
    pub(super) static PUBLISH_FAULT: std::cell::Cell<Option<PublishFault>> =
        const { std::cell::Cell::new(None) };
    pub(super) static RECOVERY_FAULT: std::cell::Cell<Option<PublishFault>> =
        const { std::cell::Cell::new(None) };
    pub(super) static PUBLISH_CRASH_MODE: std::cell::Cell<Option<CrashMode>> =
        const { std::cell::Cell::new(None) };
    pub(super) static RECOVERY_CRASH_MODE: std::cell::Cell<Option<CrashMode>> =
        const { std::cell::Cell::new(None) };
    pub(super) static PUBLISH_HARDLINK_ALIAS: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn maybe_publish_fault(point: PublishFault) -> RevisionResult<()> {
    if PUBLISH_FAULT.get() == Some(point) {
        if let Some(mode) = PUBLISH_CRASH_MODE.get() {
            match mode {
                CrashMode::Exit => unsafe { libc::_exit(86) },
                CrashMode::Abort => std::process::abort(),
                CrashMode::Kill => unsafe {
                    libc::raise(libc::SIGKILL);
                    libc::_exit(87);
                },
            }
        }
        return Err(RevisionError::Invalid(format!(
            "injected publication fault at {point:?}"
        )));
    }
    Ok(())
}

#[cfg(not(test))]
pub(super) fn maybe_publish_fault(_point: PublishFault) -> RevisionResult<()> {
    Ok(())
}

#[cfg(test)]
pub(super) fn maybe_recovery_fault(point: PublishFault) -> RevisionResult<()> {
    if RECOVERY_FAULT.get() == Some(point) {
        if let Some(mode) = RECOVERY_CRASH_MODE.get() {
            match mode {
                CrashMode::Exit => unsafe { libc::_exit(86) },
                CrashMode::Abort => std::process::abort(),
                CrashMode::Kill => unsafe {
                    libc::raise(libc::SIGKILL);
                    libc::_exit(87);
                },
            }
        }
        return Err(RevisionError::Invalid(format!(
            "injected working recovery fault at {point:?}"
        )));
    }
    Ok(())
}

#[cfg(not(test))]
pub(super) fn maybe_recovery_fault(_point: PublishFault) -> RevisionResult<()> {
    Ok(())
}

pub(super) fn maybe_seed_fault() -> RevisionResult<()> {
    maybe_publish_fault(PublishFault::SeedMirrorCreated)
}
