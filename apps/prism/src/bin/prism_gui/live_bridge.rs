use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use prism_core::{
    PRISM_LIVE_ACTION_FAMILY, PRISM_LIVE_ACTION_VERSION, PRISM_LIVE_APPLICATION, PrismLiveDrain,
    PrismLiveDrainReport, PrismLiveHost, PrismLiveInteractionState, Workspace,
    prism_live_discovery_root,
};
use spectrum_live_bridge::{
    BindingId, BridgeServer, Capability, DiscoveryDirectory, DiscoveryLease, DiscoveryRecord,
    InstanceId, LocalListener, LocalStream, PROTOCOL_VERSION, ServerConfig,
};
use spectrum_revisions::RevisionId;

const MAX_LIVE_BINDINGS: usize = 32;
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct PrismLiveRegistry {
    directory: DiscoveryDirectory,
    instance_id: InstanceId,
    wake_gui: Arc<dyn Fn() + Send + Sync>,
    bindings: HashMap<u64, LiveBinding>,
    round_robin: usize,
    #[cfg(test)]
    fail_next_start: bool,
}

pub(super) struct LiveRegistration {
    pub(super) record: DiscoveryRecord,
    pub(super) retired_binding: Option<BindingId>,
}

struct LiveBinding {
    server: Arc<BridgeServer<PrismLiveHost>>,
    host: Arc<PrismLiveHost>,
    drain: PrismLiveDrain,
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

impl PrismLiveRegistry {
    pub(super) fn new(context: eframe::egui::Context) -> anyhow::Result<Self> {
        let root = prism_live_discovery_root()?;
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
            round_robin: 0,
            #[cfg(test)]
            fail_next_start: false,
        })
    }

    pub(super) fn register(
        &mut self,
        tab_id: u64,
        workspace: &Workspace,
    ) -> anyhow::Result<LiveRegistration> {
        if workspace.live_state()?.is_none() {
            anyhow::bail!("only durable Prism workspaces can publish a live binding");
        }
        if !self.bindings.contains_key(&tab_id) && self.bindings.len() >= MAX_LIVE_BINDINGS {
            anyhow::bail!("Prism live binding limit ({MAX_LIVE_BINDINGS}) reached");
        }
        let epoch = self
            .bindings
            .get(&tab_id)
            .and_then(|binding| binding.lease.as_ref())
            .and_then(|lease| lease.record().ok())
            .map_or(1, |record| record.binding_epoch.saturating_add(1));
        let mut binding = self.start_binding(epoch, workspace)?;
        let record = match binding
            .lease
            .as_ref()
            .expect("new live binding must have a discovery lease")
            .record()
        {
            Ok(record) => record,
            Err(error) => {
                binding.shutdown();
                return Err(error.into());
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

    fn start_binding(&mut self, epoch: u64, workspace: &Workspace) -> anyhow::Result<LiveBinding> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_start) {
            anyhow::bail!("injected live binding start failure");
        }
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
        interaction: PrismLiveInteractionState,
    ) -> PrismLiveDrainReport {
        let Some(binding) = self.bindings.get_mut(&tab_id) else {
            return PrismLiveDrainReport::default();
        };
        let report = binding.drain.drain(workspace, interaction);
        binding.refresh_discovery();
        report
    }

    pub(super) fn observe(
        &mut self,
        tab_id: u64,
        workspace: &Workspace,
        interaction: PrismLiveInteractionState,
    ) -> anyhow::Result<bool> {
        let Some(binding) = self.bindings.get_mut(&tab_id) else {
            return Ok(false);
        };
        if interaction == PrismLiveInteractionState::Active && binding.local_interaction.is_none() {
            let id = format!(
                "prism-gui:{}:{}",
                binding.host.binding_id(),
                binding.next_interaction
            );
            binding.next_interaction = binding.next_interaction.saturating_add(1);
            let started_at =
                binding
                    .host
                    .begin_workspace_interaction(workspace, &id, "prism_gui_gesture")?;
            binding.local_interaction = Some(LocalInteraction { id, started_at });
        }
        let completed = (interaction == PrismLiveInteractionState::Idle)
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

    pub(super) fn ordered_tabs(&mut self, tabs: &[u64]) -> Vec<u64> {
        if tabs.is_empty() {
            return Vec::new();
        }
        let start = self.round_robin % tabs.len();
        self.round_robin = (start + 1) % tabs.len();
        tabs[start..]
            .iter()
            .chain(&tabs[..start])
            .copied()
            .collect()
    }

    pub(super) fn has_pending(&self) -> bool {
        self.bindings
            .values()
            .any(|binding| binding.drain.has_pending())
    }

    pub(super) fn record(&self, tab_id: u64) -> Option<DiscoveryRecord> {
        self.bindings.get(&tab_id)?.lease.as_ref()?.record().ok()
    }

    #[cfg(test)]
    fn fail_next_start(&mut self) {
        self.fail_next_start = true;
    }

    pub(super) fn shutdown(&mut self) {
        let bindings = std::mem::take(&mut self.bindings);
        for (_, mut binding) in bindings {
            binding.shutdown();
        }
    }
}

impl Drop for PrismLiveRegistry {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl LiveBinding {
    fn start(
        directory: &DiscoveryDirectory,
        instance_id: InstanceId,
        binding_epoch: u64,
        workspace: &Workspace,
        wake_gui: Arc<dyn Fn() + Send + Sync>,
    ) -> anyhow::Result<Self> {
        let live = workspace
            .live_state()?
            .ok_or_else(|| anyhow::anyhow!("live binding requires a durable workspace"))?;
        let project_path = std::fs::canonicalize(
            workspace
                .project_path
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
            application: PRISM_LIVE_APPLICATION.into(),
            project_id: live.project_id,
            canonical_project_path: project_path,
            instance_id,
            binding_id,
            binding_epoch,
            endpoint: endpoint.clone(),
            capability_id: capability.id(),
            capability_path: directory.root().join(format!("{binding_id}.capability")),
            process_id: std::process::id(),
            command_versions: BTreeMap::from([(
                PRISM_LIVE_ACTION_FAMILY.into(),
                (PRISM_LIVE_ACTION_VERSION, PRISM_LIVE_ACTION_VERSION),
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
            PrismLiveHost::new_with_wake(workspace, binding_id, binding_epoch, wake_gui)?;
        let server = Arc::new(BridgeServer::new(
            ServerConfig {
                application: PRISM_LIVE_APPLICATION.into(),
                project_id: live.project_id,
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
    server: Arc<BridgeServer<PrismLiveHost>>,
    stopping: Arc<AtomicBool>,
) -> anyhow::Result<thread::JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name("prism-live-accept".into())
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
                    .name("prism-live-connection".into())
                    .spawn(move || {
                        let _ = server.serve_connection(stream);
                    });
            }
        })?)
}

#[cfg(test)]
mod tests {
    use spectrum_live_bridge::{BridgeClient, BridgeEventKind, ClientConfig, ServerMessage};
    use spectrum_revisions::{Actor, ActorKind, SessionId};

    use super::*;

    #[test]
    fn tab_lifecycle_rotates_and_removes_authenticated_discovery() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("discovery");
        let project = temporary.path().join("project.prism");
        let mut workspace = Workspace::create_durable(
            prism_core::Document::new("Live", 64, 64),
            &project,
            Actor {
                id: "human:test".into(),
                display_name: "Test Human".into(),
                kind: ActorKind::Human,
            },
            SessionId::new(),
        )
        .unwrap();
        let mut registry = PrismLiveRegistry::at_root(root).unwrap();
        let first = registry.register(7, &workspace).unwrap().record;
        assert_eq!(registry.directory.records().unwrap().len(), 1);
        let second_registration = registry.register(7, &workspace).unwrap();
        assert_eq!(second_registration.retired_binding, Some(first.binding_id));
        let second = second_registration.record;
        assert_ne!(first.binding_id, second.binding_id);
        assert!(second.binding_epoch > first.binding_epoch);
        assert_eq!(registry.directory.records().unwrap().len(), 1);
        let moved = temporary.path().join("moved.prism");
        workspace.move_project(&moved).unwrap();
        let moved_registration = registry.register(7, &workspace).unwrap();
        assert_eq!(moved_registration.retired_binding, Some(second.binding_id));
        let moved_record = moved_registration.record;
        assert_ne!(second.binding_id, moved_record.binding_id);
        assert!(moved_record.binding_epoch > second.binding_epoch);
        assert_eq!(
            moved_record.canonical_project_path,
            std::fs::canonicalize(&moved).unwrap()
        );

        let other_project = temporary.path().join("other.prism");
        let other = Workspace::create_durable(
            prism_core::Document::new("Other", 64, 64),
            &other_project,
            Actor {
                id: "human:other".into(),
                display_name: "Other Human".into(),
                kind: ActorKind::Human,
            },
            SessionId::new(),
        )
        .unwrap();
        let other_registration = registry.register(8, &other).unwrap();
        assert_eq!(other_registration.retired_binding, None);
        let other_record = other_registration.record;
        assert_eq!(registry.directory.records().unwrap().len(), 2);
        registry.remove(8);
        let remaining = registry.directory.records().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].binding_id, moved_record.binding_id);
        assert_ne!(other_record.binding_id, moved_record.binding_id);

        let capability = registry.directory.load_capability(&moved_record).unwrap();
        let mut client = BridgeClient::connect(
            &ClientConfig::local(moved_record.endpoint.clone()),
            &capability,
        )
        .unwrap();
        client.subscribe(moved_record.newest_event_seq).unwrap();
        assert_eq!(
            registry
                .bindings
                .get(&7)
                .unwrap()
                .server
                .active_connection_count(),
            1
        );
        registry.remove(7);
        assert!(registry.directory.records().unwrap().is_empty());
        assert!(matches!(
            client.read_subscription_message().unwrap(),
            ServerMessage::Event(spectrum_live_bridge::BridgeEvent {
                event: BridgeEventKind::ProjectClosed,
                ..
            })
        ));
        assert!(client.read_subscription_message().is_err());
    }

    #[test]
    fn replacement_start_failure_preserves_the_old_authenticated_binding() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("discovery");
        let project = temporary.path().join("project.prism");
        let workspace = Workspace::create_durable(
            prism_core::Document::new("Live", 64, 64),
            &project,
            Actor {
                id: "human:test".into(),
                display_name: "Test Human".into(),
                kind: ActorKind::Human,
            },
            SessionId::new(),
        )
        .unwrap();
        let mut registry = PrismLiveRegistry::at_root(root).unwrap();
        let original = registry.register(7, &workspace).unwrap().record;
        let capability = registry.directory.load_capability(&original).unwrap();

        registry.fail_next_start();
        assert!(registry.register(7, &workspace).is_err());
        let surviving = registry.record(7).unwrap();
        assert_eq!(surviving.binding_id, original.binding_id);
        assert_eq!(surviving.binding_epoch, original.binding_epoch);
        let records = registry.directory.records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].binding_id, original.binding_id);

        let mut client = BridgeClient::connect(
            &ClientConfig::local(surviving.endpoint.clone()),
            &capability,
        )
        .unwrap();
        client.ping(7).unwrap();
        let snapshot = client.subscribe(surviving.newest_event_seq).unwrap();
        assert_eq!(snapshot.binding_id, original.binding_id);
        assert_eq!(snapshot.binding_epoch, original.binding_epoch);
        assert_eq!(
            registry
                .bindings
                .get(&7)
                .unwrap()
                .server
                .active_connection_count(),
            1
        );
    }
}
