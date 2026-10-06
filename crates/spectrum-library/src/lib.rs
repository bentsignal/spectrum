//! The library index: every asset's identity, kind, name, and document,
//! the typed references between assets, projects, import batches, and the
//! trash. Engines own what is inside each document.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub use uuid::Uuid as AssetId;

mod projects;
mod trash;
pub use projects::{BatchId, ImportBatch, Project, ProjectId};
pub use trash::{Removed, TRASH_DAYS, Trashed};

/// What an asset is, which decides the editors that open it and what it
/// can use.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Image,
    Canvas,
    Video,
    Audio,
    Music,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Canvas => "canvas",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Music => "music",
        }
    }

    /// Whether an asset of this kind can use one of `target`'s: canvases
    /// use images; video uses images, canvases, audio, video, and music.
    pub fn accepts(self, target: AssetKind) -> bool {
        use AssetKind::*;
        matches!(
            (self, target),
            (Canvas, Image) | (Video, Image | Canvas | Audio | Video | Music)
        )
    }
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for AssetKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "image" => Self::Image,
            "canvas" => Self::Canvas,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "music" => Self::Music,
            other => bail!("{other} is not a kind of asset"),
        })
    }
}

/// An asset in the library: its identity, kind, name, and the document
/// that holds it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub id: AssetId,
    pub kind: AssetKind,
    pub name: String,
    /// The asset's document, relative to the library root.
    pub document: PathBuf,
}

pub struct Library {
    root: PathBuf,
    db: Connection,
}
impl Library {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let root = std::fs::canonicalize(root)?;
        let db = Connection::open(root.join("library.sqlite"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS assets(id TEXT PRIMARY KEY, kind TEXT NOT NULL,
              name TEXT NOT NULL, document TEXT NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS refs(owner TEXT NOT NULL REFERENCES assets(id),
              slot TEXT NOT NULL, target TEXT NOT NULL REFERENCES assets(id),
              PRIMARY KEY(owner,slot));",
        )?;
        db.execute_batch(projects::SCHEMA)?;
        db.execute_batch(trash::SCHEMA)?;
        Ok(Self { root, db })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Adds the asset held by `path`, a document in the library.
    pub fn register(&self, kind: AssetKind, name: &str, path: &Path) -> Result<Asset> {
        let path = std::fs::canonicalize(path)?;
        let document = path
            .strip_prefix(&self.root)
            .context("document must belong to the library")?;
        let id = AssetId::new_v4();
        self.db.execute(
            "INSERT INTO assets VALUES(?1,?2,?3,?4)",
            params![
                id.to_string(),
                kind.as_str(),
                name,
                document.to_string_lossy()
            ],
        )?;
        self.lookup(id)
    }
    /// Renames an asset.
    pub fn rename(&self, id: AssetId, name: &str) -> Result<Asset> {
        let name = name.trim();
        if name.is_empty() {
            bail!("asset name cannot be empty");
        }
        self.get(id)?;
        self.db.execute(
            "UPDATE assets SET name=?2 WHERE id=?1",
            params![id.to_string(), name],
        )?;
        self.get(id)
    }
    /// A live asset. Trashed assets are not returned.
    pub fn get(&self, id: AssetId) -> Result<Asset> {
        let asset = self.lookup(id)?;
        if self.removed(id)?.is_some() {
            bail!("asset is in the trash");
        }
        Ok(asset)
    }
    /// An indexed asset, including one in the trash.
    pub fn lookup(&self, id: AssetId) -> Result<Asset> {
        self.db
            .query_row(
                "SELECT id,kind,name,document FROM assets WHERE id=?1",
                [id.to_string()],
                asset_row,
            )
            .optional()?
            .map(asset_from_row)
            .context("asset not found")?
    }
    /// Live assets, excluding the trash.
    pub fn list(&self) -> Result<Vec<Asset>> {
        let mut statement = self.db.prepare(&format!(
            "SELECT id,kind,name,document FROM assets a WHERE {} ORDER BY name,id",
            trash::LIVE
        ))?;
        let rows = statement.query_map([], asset_row)?;
        rows.map(|row| asset_from_row(row?)).collect()
    }
    pub fn path(&self, asset: &Asset) -> Result<PathBuf> {
        let path = std::fs::canonicalize(self.root.join(&asset.document))?;
        if !path.starts_with(&self.root) {
            bail!("asset document escaped library");
        }
        Ok(path)
    }
    /// Replaces the dependency set atomically, rejecting cycles and invalid uses.
    /// Links to purged assets are skipped; their owners draw placeholders.
    pub fn references(&mut self, owner: AssetId, links: &[(String, AssetId)]) -> Result<()> {
        let kind = self.lookup(owner)?.kind;
        let mut kept = Vec::with_capacity(links.len());
        for (slot, target) in links {
            if self.removed(*target)?.is_some_and(|r| r.purged) {
                continue;
            }
            let target_kind = self.lookup(*target)?.kind;
            if !kind.accepts(target_kind) {
                bail!("{kind} cannot reference {target_kind}");
            }
            kept.push((slot.clone(), *target));
        }
        let links = kept.as_slice();
        let current: Vec<(String, AssetId)> = {
            let mut statement = self
                .db
                .prepare("SELECT slot,target FROM refs WHERE owner=?1 ORDER BY slot")?;
            statement
                .query_map([owner.to_string()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .map(|row| {
                    let (slot, id) = row?;
                    Ok((slot, id.parse()?))
                })
                .collect::<Result<_>>()?
        };
        let mut expected = links.to_vec();
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        if current == expected {
            return Ok(());
        }
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM refs WHERE owner=?1", [owner.to_string()])?;
        for (slot, target) in links {
            tx.execute(
                "INSERT INTO refs VALUES(?1,?2,?3)",
                params![owner.to_string(), slot, target.to_string()],
            )?;
        }
        let cycle = tx.query_row("WITH RECURSIVE reachable(id) AS (
            SELECT target FROM refs WHERE owner=?1 UNION SELECT refs.target FROM refs JOIN reachable ON refs.owner=reachable.id)
            SELECT 1 FROM reachable WHERE id=?1 LIMIT 1", [owner.to_string()], |_| Ok(())).optional()?.is_some();
        if cycle {
            bail!("asset references cannot form a cycle");
        }
        tx.commit()?;
        Ok(())
    }
    pub fn dependents(&self, id: AssetId) -> Result<Vec<AssetId>> {
        let mut statement = self.db.prepare("WITH RECURSIVE affected(id) AS (
            SELECT owner FROM refs WHERE target=?1 UNION SELECT refs.owner FROM refs JOIN affected ON refs.target=affected.id)
            SELECT id FROM affected ORDER BY id")?;
        statement
            .query_map([id.to_string()], |r| r.get::<_, String>(0))?
            .map(|r| Ok(r?.parse()?))
            .collect()
    }
}
type AssetRow = (String, String, String, String);

fn asset_row(r: &rusqlite::Row) -> rusqlite::Result<AssetRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}

fn asset_from_row((id, kind, name, document): AssetRow) -> Result<Asset> {
    Ok(Asset {
        id: id.parse()?,
        kind: kind.parse()?,
        name,
        document: document.into(),
    })
}

/// Spectrum's per-user data directory: the library, caches, and live-bridge
/// discovery live under it.
pub fn data_root() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = PathBuf::from(std::env::var_os("APPDATA").context("APPDATA is unavailable")?)
        .join("Spectrum/data");
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable")?)
        .join("Library/Application Support/Spectrum");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("data directory is unavailable")?
        .join("spectrum");
    Ok(base)
}

/// The library: `SPECTRUM_LIBRARY` if set, or `Library` in the data root.
pub fn default_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SPECTRUM_LIBRARY") {
        return Ok(path.into());
    }
    Ok(data_root()?.join("Library"))
}

/// Rebuildable caches, such as revision snapshots and derived rasters.
pub fn cache_root() -> Result<PathBuf> {
    Ok(data_root()?.join("Caches"))
}

/// A document file for tests.
#[cfg(test)]
pub(crate) fn test_document(root: &Path, number: u32) -> PathBuf {
    let path = root.join(format!("document-{number}.spectrum"));
    std::fs::write(&path, b"fixture").unwrap();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_are_typed_transitive_and_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open(tmp.path()).unwrap();
        let image = lib
            .register(
                crate::AssetKind::Image,
                "image",
                &crate::test_document(tmp.path(), 1),
            )
            .unwrap();
        let canvas = lib
            .register(
                crate::AssetKind::Canvas,
                "canvas",
                &crate::test_document(tmp.path(), 2),
            )
            .unwrap();
        let video = lib
            .register(
                crate::AssetKind::Video,
                "video",
                &crate::test_document(tmp.path(), 3),
            )
            .unwrap();
        lib.references(canvas.id, &[("layer".into(), image.id)])
            .unwrap();
        lib.references(video.id, &[("clip".into(), canvas.id)])
            .unwrap();
        assert_eq!(lib.dependents(image.id).unwrap().len(), 2);
        assert!(
            lib.references(canvas.id, &[("bad".into(), video.id)])
                .is_err()
        );
        assert_eq!(lib.dependents(image.id).unwrap().len(), 2);
        assert!(
            lib.references(video.id, &[("cycle".into(), video.id)])
                .is_err()
        );
        assert_eq!(lib.dependents(image.id).unwrap().len(), 2);
        assert_eq!(lib.rename(image.id, " renamed ").unwrap().name, "renamed");
        assert!(lib.rename(image.id, "  ").is_err());
    }
}
