use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use lumen_core::{
    LUMEN_LIVE_ACTION_FAMILY, LUMEN_LIVE_ACTION_VERSION, LUMEN_LIVE_APPLICATION, LumenLiveDrain,
    LumenLiveDrainReport, LumenLiveHost, LumenLiveInteractionState, Workspace,
    lumen_live_discovery_root,
};
use spectrum_live_bridge::{
    BindingId, BridgeServer, Capability, DiscoveryDirectory, DiscoveryLease, DiscoveryRecord,
    InstanceId, LocalListener, LocalStream, PROTOCOL_VERSION, ServerConfig,
};
use spectrum_revisions::RevisionId;

const MAX_LIVE_BINDINGS: usize = 1;
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct LumenLiveRegistry {
    directory: DiscoveryDirectory,
    instance_id: InstanceId,
    wake_gui: Arc<dyn Fn() + Send + Sync>,
    bindings: HashMap<u64, LiveBinding>,
}

#[derive(Debug)]
pub(super) struct LiveRegistration {
    pub(super) record: DiscoveryRecord,
    pub(super) retired_binding: Option<BindingId>,
}

#[derive(Debug)]
pub(super) struct LiveRegistrationFailure {
    error: anyhow::Error,
    pub(super) retired_binding: Option<BindingId>,
}

impl std::fmt::Display for LiveRegistrationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for LiveRegistrationFailure {}

struct LiveBinding {
    server: Arc<BridgeServer<LumenLiveHost>>,
    host: Arc<LumenLiveHost>,
    drain: LumenLiveDrain,
    lease: Option<DiscoveryLease>,
    endpoint: spectrum_live_bridge::EndpointAddress,
    stopping: Arc<AtomicBool>,
    accept_worker: Option<thread::JoinHandle<()>>,
    local_interaction: Option<LocalInteraction>,
    next_interaction: u64,
}

struct LocalInteraction {
    id: String,
    started_at: RevisionId,
}

impl LumenLiveRegistry {
    pub(super) fn new(context: eframe::egui::Context) -> anyhow::Result<Self> {
        let root = lumen_live_discovery_root()?;
        Self::at_root_with_wake(root, Arc::new(move || context.request_repaint()))
    }

    #[cfg(test)]
    fn at_root(root: std::path::PathBuf) -> anyhow::Result<Self> {
        Self::at_root_with_wake(root, Arc::new(|| {}))
    }

    fn at_root_with_wake(
        root: std::path::PathBuf,
        wake_gui: Arc<dyn Fn() + Send + Sync>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            directory: DiscoveryDirectory::open(root)?,
            instance_id: InstanceId::new(),
            wake_gui,
            bindings: HashMap::new(),
        })
    }

    pub(super) fn register(
        &mut self,
        tab_id: u64,
        workspace: &Workspace,
    ) -> Result<LiveRegistration, LiveRegistrationFailure> {
        if workspace.live_catalog_identity().is_none() {
            return Err(LiveRegistrationFailure {
                error: anyhow::anyhow!("only durable Lumen workspaces can publish a live binding"),
                retired_binding: None,
            });
        }
        if !self.bindings.contains_key(&tab_id) && self.bindings.len() >= MAX_LIVE_BINDINGS {
            return Err(LiveRegistrationFailure {
                error: anyhow::anyhow!("Lumen live binding limit ({MAX_LIVE_BINDINGS}) reached"),
                retired_binding: None,
            });
        }
        let epoch = self
            .bindings
            .get(&tab_id)
            .and_then(|binding| binding.lease.as_ref())
            .and_then(|lease| lease.record().ok())
            .map_or(1, |record| record.binding_epoch.saturating_add(1));
        let mut binding = match self.start_binding(epoch, workspace) {
            Ok(binding) => binding,
            Err(error) => return Err(self.registration_failure(tab_id, workspace, error)),
        };
        let record = match binding
            .lease
            .as_ref()
            .expect("new live binding must have a discovery lease")
            .record()
        {
            Ok(record) => record,
            Err(error) => {
                binding.shutdown();
                return Err(self.registration_failure(tab_id, workspace, error.into()));
            }
        };
        let mut previous = self.bindings.insert(tab_id, binding);
        let retired_binding = previous.as_ref().map(|binding| binding.host.binding_id());
        if let Some(previous) = &mut previous {
            previous.shutdown();
        }
        Ok(LiveRegistration {
            record,
            retired_binding,
        })
    }

    fn registration_failure(
        &mut self,
        tab_id: u64,
        workspace: &Workspace,
        error: anyhow::Error,
    ) -> LiveRegistrationFailure {
        let incompatible = self
            .bindings
            .get(&tab_id)
            .is_some_and(|binding| !binding.matches_workspace(workspace));
        let retired_binding = incompatible.then(|| self.remove(tab_id)).flatten();
        LiveRegistrationFailure {
            error,
            retired_binding,
        }
    }

    fn start_binding(&mut self, epoch: u64, workspace: &Workspace) -> anyhow::Result<LiveBinding> {
        LiveBinding::start(
            &self.directory,
            self.instance_id,
            epoch,
            workspace,
            Arc::clone(&self.wake_gui),
        )
    }

    pub(super) fn remove(&mut self, tab_id: u64) -> Option<BindingId> {
        if let Some(mut binding) = self.bindings.remove(&tab_id) {
            let binding_id = binding.host.binding_id();
            binding.shutdown();
            return Some(binding_id);
        }
        None
    }

    pub(super) fn drain(
        &mut self,
        tab_id: u64,
        workspace: &mut Workspace,
        interaction: LumenLiveInteractionState,
    ) -> (LumenLiveDrainReport, Option<BindingId>) {
        let Some(binding) = self.bindings.get_mut(&tab_id) else {
            return (LumenLiveDrainReport::default(), None);
        };
        let report = binding.drain.drain(workspace, interaction);
        binding.refresh_discovery();
        let retired = self.retire_if_reopen_required(tab_id, report);
        (report, retired)
    }

    fn retire_if_reopen_required(
        &mut self,
        tab_id: u64,
        report: LumenLiveDrainReport,
    ) -> Option<BindingId> {
        report
            .reopen_required
            .then(|| self.remove(tab_id))
            .flatten()
    }

    pub(super) fn observe(
        &mut self,
        tab_id: u64,
        workspace: &Workspace,
        interaction: LumenLiveInteractionState,
    ) -> anyhow::Result<bool> {
        let Some(binding) = self.bindings.get_mut(&tab_id) else {
            return Ok(false);
        };
        if interaction == LumenLiveInteractionState::Active && binding.local_interaction.is_none() {
            let id = format!(
                "lumen-gui:{}:{}",
                binding.host.binding_id(),
                binding.next_interaction
            );
            binding.next_interaction = binding.next_interaction.saturating_add(1);
            let started_at =
                binding
                    .host
                    .begin_workspace_interaction(workspace, &id, "lumen_gui_gesture")?;
            binding.local_interaction = Some(LocalInteraction { id, started_at });
        }
        let completed = (interaction == LumenLiveInteractionState::Idle)
            .then(|| binding.local_interaction.take())
            .flatten();
        let changed = binding.host.observe_workspace_interaction(
            workspace,
            completed
                .as_ref()
                .map(|interaction| (interaction.id.as_str(), interaction.started_at)),
        )?;
        binding.refresh_discovery();
        Ok(changed)
    }

    pub(super) fn has_pending(&self) -> bool {
        self.bindings
            .values()
            .any(|binding| binding.drain.has_pending())
    }

    #[cfg(test)]
    fn record(&self, key: u64) -> Option<DiscoveryRecord> {
        self.bindings.get(&key)?.lease.as_ref()?.record().ok()
    }

    pub(super) fn shutdown(&mut self) {
        let bindings = std::mem::take(&mut self.bindings);
        for (_, mut binding) in bindings {
            binding.shutdown();
        }
    }
}

impl Drop for LumenLiveRegistry {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl LiveBinding {
    fn matches_workspace(&self, workspace: &Workspace) -> bool {
        let Some((project_id, _, _, _)) = workspace.live_catalog_identity() else {
            return false;
        };
        let Some(path) = workspace.catalog_path.as_deref() else {
            return false;
        };
        let Ok(path) = std::fs::canonicalize(path) else {
            return false;
        };
        self.lease
            .as_ref()
            .and_then(|lease| lease.record().ok())
            .is_some_and(|record| {
                record.project_id == project_id && record.canonical_project_path == path
            })
    }

    fn start(
        directory: &DiscoveryDirectory,
        instance_id: InstanceId,
        binding_epoch: u64,
        workspace: &Workspace,
        wake_gui: Arc<dyn Fn() + Send + Sync>,
    ) -> anyhow::Result<Self> {
        let (project_id, _, _, _) = workspace
            .live_catalog_identity()
            .ok_or_else(|| anyhow::anyhow!("live binding requires a durable workspace"))?;
        let project_path = std::fs::canonicalize(
            workspace
                .catalog_path
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("live project path is missing"))?,
        )?;
        let binding_id = BindingId::new();
        let endpoint = directory.endpoint_for(binding_id);
        let listener = LocalListener::bind(&endpoint)?;
        let capability = Capability::generate()?;
        let record = DiscoveryRecord {
            family: spectrum_live_bridge::DISCOVERY_FAMILY.into(),
            protocol_min: PROTOCOL_VERSION,
            protocol_max: PROTOCOL_VERSION,
            application: LUMEN_LIVE_APPLICATION.into(),
            project_id,
            canonical_project_path: project_path,
            instance_id,
            binding_id,
            binding_epoch,
            endpoint: endpoint.clone(),
            capability_id: capability.id(),
            capability_path: directory.root().join(format!("{binding_id}.capability")),
            process_id: std::process::id(),
            command_versions: BTreeMap::from([(
                LUMEN_LIVE_ACTION_FAMILY.into(),
                (LUMEN_LIVE_ACTION_VERSION, LUMEN_LIVE_ACTION_VERSION),
            )]),
            capabilities: Vec::new(),
            oldest_event_seq: 0,
            newest_event_seq: 0,
            created_unix_millis: 0,
            refreshed_unix_millis: 0,
            expires_unix_millis: 0,
        };
        let published = directory.publish(record, &capability)?;
        let lease = DiscoveryLease::new(published)?;
        let (host, drain) =
            LumenLiveHost::new_with_wake(workspace, binding_id, binding_epoch, wake_gui)?;
        let server = Arc::new(BridgeServer::new(
            ServerConfig {
                application: LUMEN_LIVE_APPLICATION.into(),
                project_id,
                instance_id,
                binding_id,
                binding_epoch,
            },
            capability,
            Arc::clone(&host),
        ));
        host.attach_events(Arc::clone(server.events()))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let accept_worker =
            spawn_accept_worker(listener, Arc::clone(&server), Arc::clone(&stopping))?;
        Ok(Self {
            server,
            host,
            drain,
            lease: Some(lease),
            endpoint,
            stopping,
            accept_worker: Some(accept_worker),
            local_interaction: None,
            next_interaction: 1,
        })
    }

    fn refresh_discovery(&mut self) {
        let (oldest, newest) = self.server.events().range();
        if let Some(lease) = &self.lease {
            let _ = lease.refresh(oldest, newest);
        }
    }

    fn shutdown(&mut self) {
        if let Some(interaction) = self.local_interaction.take() {
            self.host.cancel_workspace_interaction(&interaction.id);
        }
        self.drain.close();
        self.server.close();
        self.stopping.store(true, Ordering::Release);
        let _ = LocalStream::connect(&self.endpoint);
        if let Some(worker) = self.accept_worker.take() {
            let _ = worker.join();
        }
        let deadline = Instant::now() + CONNECTION_DRAIN_TIMEOUT;
        while self.server.active_connection_count() != 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        self.lease.take();
    }
}

fn spawn_accept_worker(
    listener: LocalListener,
    server: Arc<BridgeServer<LumenLiveHost>>,
    stopping: Arc<AtomicBool>,
) -> anyhow::Result<thread::JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name("lumen-live-accept".into())
        .spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let Ok((stream, _peer)) = listener.accept() else {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                let server = Arc::clone(&server);
                let _ = thread::Builder::new()
                    .name("lumen-live-connection".into())
                    .spawn(move || {
                        let _ = server.serve_connection(stream);
                    });
            }
        })?)
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};
    use lumen_core::{Project, Workspace};
    use spectrum_revisions::{Actor, ActorKind, SessionId};

    use super::*;

    fn fixture() -> (tempfile::TempDir, Workspace) {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        RgbaImage::from_pixel(2, 2, Rgba([20, 40, 60, 255]))
            .save(&source)
            .unwrap();
        let mut project = Project::new("Registry fixture");
        project.import(&[source]).unwrap();
        let workspace = Workspace::create_durable(
            project,
            &directory.path().join("fixture.lumen"),
            Actor {
                id: "person:registry".into(),
                display_name: "Registry User".into(),
                kind: ActorKind::Human,
            },
            SessionId::new(),
        )
        .unwrap();
        (directory, workspace)
    }

    #[test]
    fn registration_rotation_move_and_shutdown_revoke_old_bindings() {
        let (directory, mut workspace) = fixture();
        let root = directory.path().join("LiveBridge").join("v2");
        let mut registry = LumenLiveRegistry::at_root(root.clone()).unwrap();
        let first = registry.register(0, &workspace).unwrap().record;
        assert_eq!(first.application, LUMEN_LIVE_APPLICATION);
        assert_eq!(first.protocol_min, PROTOCOL_VERSION);
        assert_eq!(first.protocol_max, PROTOCOL_VERSION);
        assert!(first.capability_path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                first
                    .capability_path
                    .metadata()
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let second = registry.register(0, &workspace).unwrap();
        assert_eq!(second.retired_binding, Some(first.binding_id));
        assert_ne!(second.record.binding_id, first.binding_id);
        assert!(!first.capability_path.exists());

        let moved = directory.path().join("moved.lumen");
        workspace.move_project(&moved).unwrap();
        let third = registry.register(0, &workspace).unwrap();
        assert_eq!(third.retired_binding, Some(second.record.binding_id));
        assert_eq!(
            third.record.canonical_project_path,
            std::fs::canonicalize(moved).unwrap()
        );
        assert!(registry.record(0).is_some());

        let capability = third.record.capability_path.clone();
        registry.shutdown();
        assert!(!capability.exists());
        assert!(
            DiscoveryDirectory::open(root)
                .unwrap()
                .records()
                .unwrap()
                .is_empty()
        );
    }
}
