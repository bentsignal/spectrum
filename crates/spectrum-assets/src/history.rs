//! An asset's history beyond moving through it: names on revisions, and
//! small previews of any revision for a history view.
use super::{Service, save_atomically};
use anyhow::{Result, bail};
use spectrum_canvas::CanvasModel;
use spectrum_document::{RevisionId, SessionId, Workspace};
use spectrum_image::ImageModel;
use spectrum_library::{AssetId, AssetKind};
use std::path::PathBuf;

impl Service {
    /// Names a revision of an asset, or with `None` clears its name. A name
    /// marks a checkpoint and names the branch it is on; it is not an edit,
    /// so no one moves and no revision is added.
    pub fn name_revision(
        &mut self,
        id: AssetId,
        revision: RevisionId,
        name: Option<&str>,
    ) -> Result<()> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => Workspace::<ImageModel>::name_revision(&path, revision, name),
            AssetKind::Canvas => Workspace::<CanvasModel>::name_revision(&path, revision, name),
            AssetKind::Video => {
                Workspace::<spectrum_video::VideoModel>::name_revision(&path, revision, name)
            }
            kind => bail!("{kind} assets have no history yet"),
        }
    }

    /// The person's session in this library, which agents start from.
    pub fn person_session(&self) -> SessionId {
        self.person
    }

    /// Where `session` (a person's or an agent's) is in an asset's history.
    pub fn session_place(&self, id: AssetId, session: SessionId) -> Result<RevisionId> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => Workspace::<ImageModel>::cursor_of(&path, session),
            AssetKind::Canvas => Workspace::<CanvasModel>::cursor_of(&path, session),
            AssetKind::Video => Workspace::<spectrum_video::VideoModel>::cursor_of(&path, session),
            kind => bail!("{kind} assets have no history yet"),
        }
    }

    /// Names the branch holding `revision`, or with `None` clears its name.
    /// The name goes on the revision that began the branch, so it names the
    /// branch however long it grows; other names along it are cleared.
    pub fn name_branch(
        &mut self,
        id: AssetId,
        revision: RevisionId,
        name: Option<&str>,
    ) -> Result<RevisionId> {
        let history = self.history(id)?;
        let branch = history.branch_of(revision);
        let start = branch[0];
        for (named, _) in &history.names {
            if *named != start && branch.contains(named) {
                self.name_revision(id, *named, None)?;
            }
        }
        self.name_revision(id, start, name)?;
        Ok(start)
    }

    /// A render of an asset at one revision, no larger than `max` pixels on
    /// its long edge, made without moving anyone and kept with the other
    /// previews. A revision never changes, so it renders once (a canvas's
    /// linked images show as they are now).
    pub fn revision_preview(&self, id: AssetId, revision: RevisionId, max: u32) -> Result<PathBuf> {
        let asset = self.library.get(id)?;
        let document = self.library.path(&asset)?;
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("{id}-revision-{revision}-{max}.png"));
        if path.exists() {
            return Ok(path);
        }
        match asset.kind {
            AssetKind::Image => {
                let image = Workspace::<ImageModel>::document_at(&document, revision)?;
                let rendered = spectrum_image::engine::render_thumbnail(&image, max)?;
                save_atomically(&rendered, &path)?;
            }
            AssetKind::Canvas => {
                let mut doc = Workspace::<CanvasModel>::document_at(&document, revision)?;
                self.resolve_small(&mut doc, Some((max * 2).max(256)), 1.0)?;
                spectrum_canvas::export_document_sized(&doc, &path, 90, Some(max))?;
            }
            AssetKind::Video => {
                let video =
                    Workspace::<spectrum_video::VideoModel>::document_at(&document, revision)?;
                save_atomically(
                    &image::DynamicImage::ImageRgba8(self.video_poster(&video, max)?),
                    &path,
                )?;
            }
            kind => bail!("{kind} assets have no history yet"),
        }
        Ok(path)
    }
}
