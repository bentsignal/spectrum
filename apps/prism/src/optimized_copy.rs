use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use spectrum_fonts::{
    FontSubsetEngine, HarfBuzzSubsetEngine, SubsetRequest,
    UnicodeVariationSequence as SubsetVariationSequence,
};
use spectrum_revisions::{
    AppendRevision, Asset, AssetId, NewProject, Payload, Revision, RevisionId, RevisionStore,
};

use crate::{
    Command, Document, FontAsset, FontEmbeddingPermission, LayerKind, VerifiedFontSource,
    font_usage::analyze_font_usage_with_source,
};

use super::{
    APPLICATION_ID, AssetReference, OPERATIONS_FAMILY, PreparedSnapshot, PrismCompatibility,
    decode_snapshot, media_type, readonly_font, validate_operations_version,
};

#[path = "optimized_copy_validation.rs"]
mod validation;
use validation::{validate_copy_store, validate_render_parity};

const MAX_OPTIMIZED_FONT_COUNT: usize = 64;
const MAX_OPTIMIZED_FONT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OPTIMIZED_REVISIONS: usize = 100_000;
const MAX_OPERATION_PAYLOAD_BYTES: usize = 128 * 1024 * 1024;
const MAX_COMMANDS_PER_REVISION: u32 = 1_000_000;
const MAX_REVISION_ASSET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OptimizedCopyFont {
    pub source_content_hash: String,
    pub content_hash: String,
    pub source_bytes: u64,
    pub subset_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OptimizedCopyReport {
    pub source: PathBuf,
    pub output: PathBuf,
    pub revisions: usize,
    pub fonts: Vec<OptimizedCopyFont>,
    pub source_bytes: u64,
    pub output_bytes: u64,
    pub saved_bytes: u64,
}

struct FontRequirement {
    template: FontAsset,
    source: VerifiedFontSource,
    media_type: String,
    extension: String,
    codepoints: BTreeSet<u32>,
    variation_sequences: BTreeSet<(u32, u32)>,
    shaping_samples: BTreeSet<Vec<u32>>,
}

struct RewrittenFont {
    asset: Asset,
    path: PathBuf,
    source_hash: String,
    content_hash: String,
    source_bytes: u64,
    subset_bytes: u64,
}

pub fn create_optimized_font_copy(source: &Path, output: &Path) -> Result<OptimizedCopyReport> {
    create_optimized_font_copy_before_publish(source, output, || Ok(()))
}

fn create_optimized_font_copy_before_publish(
    source: &Path,
    output: &Path,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<OptimizedCopyReport> {
    let (source, output) = checked_sibling_paths(source, output)?;
    let source_identity = SourceIdentity::read(&source)?;
    let source_bytes = source.metadata()?.len();
    let source_store = RevisionStore::open_read_only(&source)?;
    source_store.verify_integrity()?;
    let (revisions, track) = linear_history(&source_store, &source)?;
    preflight_operations(&source_store, &revisions)?;

    let mut document = root_document(&source_store, revisions[0].id)?;
    let mut requirements = HashMap::new();
    collect_font_requirements(&source_store, &document, &mut requirements)?;
    for revision in &revisions[1..] {
        let (_, commands) = operation_batch(&source_store, revision)?;
        replay_commands(&source_store, &mut document, commands)?;
        validate_source_exact_snapshot(&source_store, revision.id, &document)?;
        collect_font_requirements(&source_store, &document, &mut requirements)?;
    }
    let rewritten_fonts = subset_fonts(requirements)?;

    let temporary = temporary_path(&output);
    let mut cleanup = TemporaryProject::create(temporary.clone())?;
    let mut document = root_document(&source_store, revisions[0].id)?;
    rewrite_document_fonts(&mut document, &rewritten_fonts)?;
    let mut root_assets = referenced_document_assets(&source_store, &document, &rewritten_fonts)?;
    let root_snapshot = PreparedSnapshot::portable(&document)?;
    root_assets.extend(root_snapshot.assets);
    let root = &revisions[0];
    let (mut destination, destination_info) = RevisionStore::create(
        &temporary,
        NewProject {
            application_id: APPLICATION_ID.into(),
            application_version: root.application_version.clone(),
            actor: root.actor.clone(),
            session_id: root.session_id,
            root_label: root.label.clone(),
            track_kind: track.kind,
            track_label: track.label,
            initial_snapshots: vec![root_snapshot.payload],
            assets: deduplicate_assets(root_assets)?,
        },
    )?;
    let mut destination_parent = destination_info.root_revision;

    let mut source_document = root_document(&source_store, revisions[0].id)?;
    let mut final_rewritten_document = document;
    for revision in &revisions[1..] {
        let (operation_payload, commands) = operation_batch(&source_store, revision)?;
        replay_commands(&source_store, &mut source_document, commands.clone())?;
        let mut rewritten_commands = commands;
        rewrite_command_fonts(&mut rewritten_commands, &rewritten_fonts)?;
        let mut rewritten_document = source_document.clone();
        rewrite_document_fonts(&mut rewritten_document, &rewritten_fonts)?;
        let snapshot = PreparedSnapshot::portable(&rewritten_document)?;
        let mut assets =
            referenced_command_assets(&source_store, &rewritten_commands, &rewritten_fonts)?;
        assets.extend(referenced_document_assets(
            &source_store,
            &rewritten_document,
            &rewritten_fonts,
        )?);
        assets.extend(snapshot.assets);
        let session = destination.resume_session(
            revision.session_id,
            revision.actor.clone(),
            destination_parent,
        )?;
        if session.cursor != destination_parent {
            destination.move_session(revision.session_id, session.cursor, destination_parent)?;
        }
        let appended = destination.append(AppendRevision {
            track_id: destination_info.default_track_id,
            session_id: revision.session_id,
            expected_parent: destination_parent,
            application_version: revision.application_version.clone(),
            label: revision.label.clone(),
            command_count: revision.command_count,
            operation_payloads: vec![Payload::new(
                operation_payload.encoding,
                serde_json::to_vec(&rewritten_commands)?,
            )],
            snapshots: vec![snapshot.payload],
            assets: deduplicate_assets(assets)?,
        })?;
        destination_parent = appended.id;
        final_rewritten_document = rewritten_document;
    }
    reset_author_sessions_to_tip(
        &mut destination,
        destination_info.default_track_id,
        destination_parent,
    )?;
    destination.verify_integrity()?;
    validate_copy_store(&destination, revisions.len())?;
    validate_render_parity(
        &source_store,
        &source_document,
        &destination,
        &final_rewritten_document,
        &output,
    )?;
    destination.checkpoint()?;
    drop(destination);
    let reopened = RevisionStore::open_read_only(&temporary)
        .context("could not reopen the private optimized copy")?;
    reopened.verify_integrity()?;
    validate_copy_store(&reopened, revisions.len())?;
    drop(reopened);
    let output_bytes = temporary.metadata()?.len();
    if output_bytes >= source_bytes {
        bail!("optimized copy does not reduce the source project size");
    }

    before_publish()?;
    source_identity.require_unchanged(&source)?;
    publish_optimized_copy(&temporary, &output, &mut cleanup, |source, destination| {
        spectrum_revisions::publish_noreplace(source, destination)
    })?;
    cleanup.disarm();
    let mut fonts = rewritten_fonts
        .values()
        .map(|font| OptimizedCopyFont {
            source_content_hash: font.source_hash.clone(),
            content_hash: font.content_hash.clone(),
            source_bytes: font.source_bytes,
            subset_bytes: font.subset_bytes,
        })
        .collect::<Vec<_>>();
    fonts.sort_by(|left, right| left.source_content_hash.cmp(&right.source_content_hash));
    Ok(OptimizedCopyReport {
        source,
        output,
        revisions: revisions.len(),
        fonts,
        source_bytes,
        output_bytes,
        saved_bytes: source_bytes.saturating_sub(output_bytes),
    })
}

fn publish_optimized_copy(
    temporary: &Path,
    output: &Path,
    cleanup: &mut TemporaryProject,
    publish: impl FnOnce(&Path, &Path) -> spectrum_revisions::RevisionResult<()>,
) -> Result<()> {
    if let Err(error) = publish(temporary, output) {
        if matches!(
            error,
            spectrum_revisions::RevisionError::PublishedButNotSynced { .. }
        ) {
            cleanup.disarm();
            return Err(anyhow::Error::new(error).context(format!(
                "optimized copy exists at {}, but its directory durability sync failed",
                output.display()
            )));
        }
        return Err(anyhow::Error::new(error).context(format!(
            "could not publish optimized copy {}",
            output.display()
        )));
    }
    Ok(())
}

fn reset_author_sessions_to_tip(
    store: &mut RevisionStore,
    track: spectrum_revisions::TrackId,
    tip: RevisionId,
) -> Result<()> {
    for session in store.sessions_on_track(track)? {
        if session.cursor != tip {
            store.move_session(session.id, session.cursor, tip)?;
        }
    }
    Ok(())
}

fn checked_sibling_paths(source: &Path, output: &Path) -> Result<(PathBuf, PathBuf)> {
    let source = fs::canonicalize(source)
        .with_context(|| format!("could not resolve source project {}", source.display()))?;
    if !source.metadata()?.is_file() {
        bail!("source Prism project is not a regular file");
    }
    let output = if output.is_absolute() {
        output.to_owned()
    } else {
        std::env::current_dir()?.join(output)
    };
    let output_name = output
        .file_name()
        .filter(|name| !name.is_empty())
        .context("optimized copy destination requires a file name")?;
    let output_parent = fs::canonicalize(
        output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .context("could not resolve optimized copy destination directory")?;
    let output = output_parent.join(output_name);
    match fs::symlink_metadata(&output) {
        Ok(_) => bail!(
            "refusing to overwrite optimized copy destination {}",
            output.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok((source, output))
}

fn linear_history(
    store: &RevisionStore,
    source: &Path,
) -> Result<(Vec<Revision>, spectrum_revisions::Track)> {
    let info = store.project_info()?;
    if info.application_id != APPLICATION_ID {
        bail!(
            "{} is a {} project, not a Prism project",
            source.display(),
            info.application_id
        );
    }
    let tracks = store.tracks()?;
    if tracks.len() != 1 {
        bail!("optimized copy requires exactly one revision track");
    }
    let track = tracks.into_iter().next().unwrap();
    if track.id != info.default_track_id || track.root_revision != info.root_revision {
        bail!("Prism default track metadata is inconsistent");
    }
    let revisions = store.revisions_for_track(track.id)?;
    if revisions.len() > MAX_OPTIMIZED_REVISIONS {
        bail!("optimized copy exceeds the 100,000-revision history limit");
    }
    let by_id = revisions
        .iter()
        .map(|revision| (revision.id, revision))
        .collect::<HashMap<_, _>>();
    let root = by_id
        .get(&track.root_revision)
        .context("Prism root revision is missing")?;
    if root.parent_id.is_some() || root.command_count != 0 {
        bail!("Prism root revision is not an empty history root");
    }
    if store
        .compatible_operation_payload(root.id, &PrismCompatibility)?
        .is_some()
    {
        bail!("Prism root revision unexpectedly contains operations");
    }
    let mut children = HashMap::<RevisionId, Vec<RevisionId>>::new();
    for revision in &revisions {
        if revision.id == track.root_revision {
            continue;
        }
        let parent = revision
            .parent_id
            .context("non-root Prism revision has no parent")?;
        if !by_id.contains_key(&parent) {
            bail!("Prism revision history contains an external parent");
        }
        children.entry(parent).or_default().push(revision.id);
    }
    if children.values().any(|children| children.len() != 1) {
        bail!("optimized copy requires linear history without branches");
    }
    let mut ordered = Vec::with_capacity(revisions.len());
    let mut cursor = track.root_revision;
    loop {
        ordered.push((*by_id.get(&cursor).unwrap()).clone());
        let Some(next) = children.get(&cursor).and_then(|children| children.first()) else {
            break;
        };
        cursor = *next;
    }
    if ordered.len() != revisions.len() {
        bail!("Prism revision history is disconnected");
    }
    if store.most_recent_cursor_for_track(track.id)? != ordered.last().unwrap().id {
        bail!("optimized copy requires the active project cursor to be at the linear history tip");
    }
    Ok((ordered, track))
}

fn preflight_operations(store: &RevisionStore, revisions: &[Revision]) -> Result<()> {
    for revision in &revisions[1..] {
        operation_batch(store, revision)?;
    }
    Ok(())
}

fn root_document(store: &RevisionStore, root: RevisionId) -> Result<Document> {
    let plan = store.replay_plan(root, &PrismCompatibility)?;
    if plan.snapshot_revision != root || !plan.steps.is_empty() {
        bail!("Prism root revision has no compatible exact snapshot");
    }
    let mut document: Document = serde_json::from_slice(&decode_snapshot(&plan.snapshot)?)
        .context("invalid Prism root snapshot")?;
    readonly_font::hydrate_read_only_legacy_font_permissions(store, &mut document)?;
    document.migrate()?;
    Ok(document)
}

fn validate_source_exact_snapshot(
    store: &RevisionStore,
    revision: RevisionId,
    replayed: &Document,
) -> Result<()> {
    let plan = store.replay_plan(revision, &PrismCompatibility)?;
    if plan.snapshot_revision != revision || !plan.steps.is_empty() {
        return Ok(());
    }
    let mut exact: Document = serde_json::from_slice(&decode_snapshot(&plan.snapshot)?)
        .context("invalid exact Prism source snapshot")?;
    readonly_font::hydrate_read_only_legacy_font_permissions(store, &mut exact)?;
    exact.migrate()?;
    if &exact != replayed {
        bail!("source Prism revision {revision} operations do not reproduce its exact snapshot");
    }
    Ok(())
}

fn operation_batch(store: &RevisionStore, revision: &Revision) -> Result<(Payload, Vec<Command>)> {
    if revision.command_count > MAX_COMMANDS_PER_REVISION {
        bail!(
            "revision {} exceeds the 1,000,000-command limit",
            revision.id
        );
    }
    let payload = store
        .compatible_operation_payload(revision.id, &PrismCompatibility)?
        .with_context(|| {
            format!(
                "revision {} has no compatible operation payload",
                revision.id
            )
        })?;
    if payload.encoding.family != OPERATIONS_FAMILY {
        bail!("revision {} operation family is invalid", revision.id);
    }
    if payload.bytes.len() > MAX_OPERATION_PAYLOAD_BYTES {
        bail!(
            "revision {} exceeds the 128 MiB operation payload limit",
            revision.id
        );
    }
    let commands: Vec<Command> = serde_json::from_slice(&payload.bytes)
        .with_context(|| format!("revision {} has invalid Prism operations", revision.id))?;
    validate_operations_version(&commands, payload.encoding.version)?;
    if revision.command_count != commands.len().try_into().unwrap_or(u32::MAX) {
        bail!(
            "revision {} command count does not match its payload",
            revision.id
        );
    }
    Ok((payload, commands))
}

fn replay_commands(
    store: &RevisionStore,
    document: &mut Document,
    commands: Vec<Command>,
) -> Result<()> {
    for command in commands {
        readonly_font::replay_command(store, document, command)?;
    }
    document.migrate()?;
    Ok(())
}

fn collect_font_requirements(
    store: &RevisionStore,
    document: &Document,
    requirements: &mut HashMap<String, FontRequirement>,
) -> Result<()> {
    for font in &document.font_assets {
        let reference =
            AssetReference::parse(&font.path).context("font is not an embedded project asset")?;
        if reference.id.to_string() != font.content_hash {
            bail!("font path does not match its content identity");
        }
        let asset = store
            .asset_record(reference.id)?
            .with_context(|| format!("embedded Prism font asset {} is missing", reference.id))?;
        validate_asset_identity(&asset, reference.id, &reference.extension)?;
        let source = font.verify_embedded_bytes(asset.bytes)?;
        let analysis = analyze_font_usage_with_source(document, font.id, source.bytes())?;
        if !analysis.usage.unpaired_variation_selectors.is_empty() {
            bail!("font {} has unpaired Unicode variation selectors", font.id);
        }
        if !analysis.missing_codepoints.is_empty()
            || !analysis.missing_variation_sequences.is_empty()
        {
            bail!(
                "font {} does not cover its historical text repertoire",
                font.id
            );
        }
        if !analysis.embedding_metadata_allows_subsetting {
            bail!("font {} embedding metadata forbids subsetting", font.id);
        }
        let requirement = requirements
            .entry(font.content_hash.clone())
            .or_insert_with(|| FontRequirement {
                template: font.clone(),
                source: source.clone(),
                media_type: asset.media_type,
                extension: reference.extension,
                codepoints: BTreeSet::new(),
                variation_sequences: BTreeSet::new(),
                shaping_samples: BTreeSet::new(),
            });
        if requirement.source.bytes() != source.bytes() {
            bail!("font content identity resolves to inconsistent source bytes");
        }
        requirement.codepoints.extend(analysis.usage.codepoints);
        requirement.variation_sequences.extend(
            analysis
                .usage
                .variation_sequences
                .into_iter()
                .map(|sequence| (sequence.base_codepoint, sequence.selector_codepoint)),
        );
        requirement
            .shaping_samples
            .extend(shaping_samples(document, font.id));
    }
    Ok(())
}

fn shaping_samples(document: &Document, font_id: u64) -> impl Iterator<Item = Vec<u32>> + '_ {
    document.layers.iter().flat_map(move |layer| {
        let text = match &layer.kind {
            LayerKind::Text {
                text, typography, ..
            } if typography.font_id == Some(font_id) => Some(text.as_str()),
            _ => None,
        };
        text.into_iter().flat_map(|text| {
            text.split('\n')
                .map(|line| line.chars().map(u32::from).collect::<Vec<_>>())
                .filter(|sample| !sample.is_empty())
        })
    })
}

fn subset_fonts(
    requirements: HashMap<String, FontRequirement>,
) -> Result<HashMap<String, RewrittenFont>> {
    if requirements.is_empty() {
        bail!("Prism project has no embedded fonts to optimize");
    }
    if requirements.len() > MAX_OPTIMIZED_FONT_COUNT {
        bail!("optimized copy exceeds the 64-font aggregate limit");
    }
    let aggregate_bytes = requirements
        .values()
        .try_fold(0_u64, |total, requirement| {
            let bytes = u64::try_from(requirement.source.len()).unwrap_or(u64::MAX);
            total.checked_add(bytes).context("font byte total overflow")
        })?;
    if aggregate_bytes > MAX_OPTIMIZED_FONT_BYTES {
        bail!("optimized copy exceeds the 256 MiB aggregate font limit");
    }
    let mut rewritten = HashMap::with_capacity(requirements.len());
    let mut subset_sources = HashMap::with_capacity(requirements.len());
    for (source_hash, requirement) in requirements {
        if requirement.codepoints.is_empty() && requirement.variation_sequences.is_empty() {
            bail!(
                "font {} has no historical text repertoire to subset",
                requirement.template.id
            );
        }
        let request = SubsetRequest::new(
            0,
            requirement.codepoints,
            requirement.variation_sequences.into_iter().map(
                |(base_codepoint, selector_codepoint)| SubsetVariationSequence {
                    base_codepoint,
                    selector_codepoint,
                },
            ),
        )
        .with_shaping_samples(requirement.shaping_samples);
        let artifact = HarfBuzzSubsetEngine
            .subset(requirement.source.bytes(), &request)
            .with_context(|| format!("could not subset embedded font {source_hash}"))?;
        if artifact.subset_bytes >= artifact.source_bytes {
            bail!("font {source_hash} subset does not reduce its source bytes");
        }
        let asset = Asset::new(requirement.media_type, artifact.bytes);
        let content_hash = asset.id.to_string();
        require_unique_subset_content(&mut subset_sources, &source_hash, &content_hash)?;
        let verified = VerifiedFontSource::from_embedded_bytes(asset.bytes.clone(), &content_hash)?;
        verify_subset_metadata(&requirement.template, &verified)?;
        let path = AssetReference {
            id: asset.id,
            extension: requirement.extension,
        }
        .path();
        rewritten.insert(
            source_hash.clone(),
            RewrittenFont {
                asset,
                path,
                source_hash,
                content_hash,
                source_bytes: artifact.source_bytes.try_into().unwrap_or(u64::MAX),
                subset_bytes: artifact.subset_bytes.try_into().unwrap_or(u64::MAX),
            },
        );
    }
    Ok(rewritten)
}

fn require_unique_subset_content(
    subset_sources: &mut HashMap<String, String>,
    source_hash: &str,
    subset_hash: &str,
) -> Result<()> {
    if let Some(existing_source) = subset_sources.get(subset_hash)
        && existing_source != source_hash
    {
        let mut sources = [existing_source.as_str(), source_hash];
        sources.sort_unstable();
        bail!(
            "distinct source fonts {} and {} converge to subset content hash {}; optimized copy cannot preserve font identity",
            sources[0],
            sources[1],
            subset_hash
        );
    }
    subset_sources.insert(subset_hash.to_owned(), source_hash.to_owned());
    Ok(())
}

fn verify_subset_metadata(template: &FontAsset, subset: &VerifiedFontSource) -> Result<()> {
    if subset.family != template.family
        || subset.style != template.style
        || subset.weight != template.weight
        || subset.slant != template.slant
        || subset.embedding_permission() != template.embedding_permission
        || subset.subset_allowed() != template.subset_allowed
    {
        bail!("subset font metadata does not match its immutable source");
    }
    if subset.embedding_permission() == FontEmbeddingPermission::LegacyUnknown {
        bail!("subset font embedding permission is unresolved");
    }
    Ok(())
}

fn rewrite_document_fonts(
    document: &mut Document,
    fonts: &HashMap<String, RewrittenFont>,
) -> Result<()> {
    for font in &mut document.font_assets {
        let rewritten = fonts
            .get(&font.content_hash)
            .with_context(|| format!("font {} has no optimized replacement", font.id))?;
        font.content_hash = rewritten.content_hash.clone();
        font.path = rewritten.path.clone();
        font.original_path = None;
    }
    Ok(())
}

fn rewrite_command_fonts(
    commands: &mut [Command],
    fonts: &HashMap<String, RewrittenFont>,
) -> Result<()> {
    for command in commands {
        match command {
            Command::ImportFont { path, .. } => {
                let reference =
                    AssetReference::parse(path).context("font import is not a project asset")?;
                let rewritten = fonts
                    .get(&reference.id.to_string())
                    .context("font import has no optimized replacement")?;
                *path = rewritten.path.clone();
            }
            Command::InsertLayer { transfer, .. } => {
                if let Some(font) = &mut transfer.font_asset {
                    let reference = AssetReference::parse(&font.path)
                        .context("transferred font is not a project asset")?;
                    if reference.id.to_string() != font.content_hash {
                        bail!("transferred font path does not match its content identity");
                    }
                    let rewritten = fonts
                        .get(&font.content_hash)
                        .context("transferred font has no optimized replacement")?;
                    font.content_hash = rewritten.content_hash.clone();
                    font.path = rewritten.path.clone();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn referenced_document_assets(
    store: &RevisionStore,
    document: &Document,
    fonts: &HashMap<String, RewrittenFont>,
) -> Result<Vec<Asset>> {
    let mut assets = Vec::new();
    for layer in &document.layers {
        if let LayerKind::Raster { path, .. } = &layer.kind {
            assets.push(source_asset(store, path)?);
        }
    }
    for font in &document.font_assets {
        let rewritten = fonts
            .values()
            .find(|rewritten| rewritten.content_hash == font.content_hash)
            .context("rewritten font asset is missing")?;
        assets.push(rewritten.asset.clone());
    }
    Ok(assets)
}

fn referenced_command_assets(
    store: &RevisionStore,
    commands: &[Command],
    fonts: &HashMap<String, RewrittenFont>,
) -> Result<Vec<Asset>> {
    let mut assets = Vec::new();
    for command in commands {
        match command {
            Command::AddRaster { path, .. } | Command::RasterizeShape { path, .. } => {
                assets.push(source_asset(store, path)?);
            }
            Command::ImportFont { path, .. } => {
                assets.push(rewritten_asset(path, fonts)?);
            }
            Command::InsertLayer { transfer, .. } => {
                if let LayerKind::Raster { path, .. } = &transfer.layer.kind {
                    assets.push(source_asset(store, path)?);
                }
                if let Some(font) = &transfer.font_asset {
                    assets.push(rewritten_asset(&font.path, fonts)?);
                }
            }
            _ => {}
        }
    }
    Ok(assets)
}

fn source_asset(store: &RevisionStore, path: &Path) -> Result<Asset> {
    let reference =
        AssetReference::parse(path).context("history references a non-project raster asset")?;
    let asset = store
        .asset_record(reference.id)?
        .with_context(|| format!("embedded Prism asset {} is missing", reference.id))?;
    validate_asset_identity(&asset, reference.id, &reference.extension)?;
    Ok(asset)
}

fn rewritten_asset(path: &Path, fonts: &HashMap<String, RewrittenFont>) -> Result<Asset> {
    let reference =
        AssetReference::parse(path).context("history references a non-project font asset")?;
    fonts
        .values()
        .find(|font| font.asset.id == reference.id)
        .map(|font| font.asset.clone())
        .context("rewritten font asset is missing")
}

fn validate_asset_identity(asset: &Asset, expected: AssetId, extension: &str) -> Result<()> {
    if AssetId::for_bytes(&asset.bytes) != expected || asset.id != expected {
        bail!("embedded Prism asset {expected} does not match its content identity");
    }
    if asset.media_type != media_type(extension) {
        bail!("embedded Prism asset {expected} has an invalid media type");
    }
    Ok(())
}

fn deduplicate_assets(assets: Vec<Asset>) -> Result<Vec<Asset>> {
    let mut unique = HashMap::<AssetId, Asset>::new();
    for asset in assets {
        unique.entry(asset.id).or_insert(asset);
    }
    let mut assets = unique.into_values().collect::<Vec<_>>();
    assets.sort_by_key(|asset| asset.id.to_string());
    let total = assets.iter().try_fold(0_u64, |total, asset| {
        total
            .checked_add(u64::try_from(asset.bytes.len()).unwrap_or(u64::MAX))
            .context("asset byte total overflow")
    })?;
    if total > MAX_REVISION_ASSET_BYTES {
        bail!("optimized copy revision exceeds the 2 GiB reachable-asset limit");
    }
    Ok(assets)
}

fn temporary_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("optimized.prism");
    output.with_file_name(format!(
        ".{name}.{}.optimized-copy.tmp",
        spectrum_revisions::SessionId::new()
    ))
}

struct TemporaryProject {
    path: PathBuf,
    armed: bool,
}

impl TemporaryProject {
    fn create(path: PathBuf) -> Result<Self> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "could not reserve private optimized copy {}",
                    path.display()
                )
            })?;
        Ok(Self { path, armed: true })
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TemporaryProject {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(PathBuf::from(format!("{}-wal", self.path.display())));
        let _ = fs::remove_file(PathBuf::from(format!("{}-shm", self.path.display())));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceIdentity {
    length: u64,
    modified: Option<std::time::SystemTime>,
    sha256: [u8; 32],
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl SourceIdentity {
    fn read(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)?;
        let mut file = fs::File::open(path)?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            sha256: digest.finalize().into(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }

    fn require_unchanged(&self, path: &Path) -> Result<()> {
        if &Self::read(path)? != self {
            bail!("source Prism project changed during optimized copy");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "optimized_copy_tests.rs"]
mod tests;
