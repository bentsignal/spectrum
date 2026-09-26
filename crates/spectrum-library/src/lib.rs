//! Editable library identities and typed references, independent of editor engines.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub use uuid::Uuid as AssetId;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub id: AssetId,
    pub kind: String,
    pub name: String,
    /// Engine-owned durable document, relative to the library root.
    pub document: PathBuf,
    pub item: Option<u64>,
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
              name TEXT NOT NULL, document TEXT NOT NULL, item INTEGER NOT NULL,
              UNIQUE(document,item));
            CREATE TABLE IF NOT EXISTS refs(owner TEXT NOT NULL REFERENCES assets(id),
              slot TEXT NOT NULL, target TEXT NOT NULL REFERENCES assets(id),
              PRIMARY KEY(owner,slot));",
        )?;
        Ok(Self { root, db })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn register(
        &self,
        kind: &str,
        name: &str,
        path: &Path,
        item: Option<u64>,
    ) -> Result<Asset> {
        if kind.is_empty()
            || !kind
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        {
            bail!("asset type must be a nonempty identifier");
        }
        let path = std::fs::canonicalize(path)?;
        let document = path
            .strip_prefix(&self.root)
            .context("document must belong to the library")?;
        let item_key = item.map(i64::try_from).transpose()?.unwrap_or(-1);
        self.db.execute(
            "INSERT INTO assets VALUES(?1,?2,?3,?4,?5)
            ON CONFLICT(document,item) DO UPDATE SET name=excluded.name WHERE assets.name != excluded.name AND assets.kind = excluded.kind",
            params![
                AssetId::new_v4().to_string(),
                kind,
                name,
                document.to_string_lossy(),
                item_key
            ],
        )?;
        let id: String = self.db.query_row(
            "SELECT id FROM assets WHERE document=?1 AND item=?2",
            params![document.to_string_lossy(), item_key],
            |r| r.get(0),
        )?;
        let asset = self.get(id.parse()?)?;
        if asset.kind != kind {
            bail!("asset type cannot change");
        }
        Ok(asset)
    }
    pub fn get(&self, id: AssetId) -> Result<Asset> {
        self.list()?
            .into_iter()
            .find(|a| a.id == id)
            .context("asset not found")
    }
    pub fn list(&self) -> Result<Vec<Asset>> {
        let mut statement = self
            .db
            .prepare("SELECT id,kind,name,document,item FROM assets ORDER BY name,id")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, kind, name, document, item) = row?;
            Ok(Asset {
                id: id.parse()?,
                kind,
                name,
                document: document.into(),
                item: if item < 0 { None } else { Some(item as u64) },
            })
        })
        .collect()
    }
    pub fn path(&self, asset: &Asset) -> Result<PathBuf> {
        let path = std::fs::canonicalize(self.root.join(&asset.document))?;
        if !path.starts_with(&self.root) {
            bail!("asset document escaped library");
        }
        Ok(path)
    }
    /// Replaces the dependency set atomically, rejecting cycles and invalid uses.
    pub fn references(&mut self, owner: AssetId, links: &[(String, AssetId)]) -> Result<()> {
        let kind = self.get(owner)?.kind;
        for (_, target) in links {
            let target_kind = self.get(*target)?.kind;
            if !accepts(&kind, &target_kind) {
                bail!("{kind} cannot reference {target_kind}");
            }
        }
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
/// Compatibility is explicit. New types can be stored before an editor supports them.
pub fn accepts(owner: &str, target: &str) -> bool {
    matches!(
        (owner, target),
        ("canvas", "image") | ("video", "image" | "canvas" | "audio" | "video" | "music")
    )
}

/// Shared app-managed location used by the CLI and both editor adapters.
pub fn default_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SPECTRUM_LIBRARY") {
        return Ok(path.into());
    }
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
    Ok(base.join("Library"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_are_typed_transitive_and_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open(tmp.path()).unwrap();
        let path = tmp.path().join("data");
        std::fs::write(&path, b"fixture").unwrap();
        let image = lib.register("image", "image", &path, Some(1)).unwrap();
        let canvas = lib.register("canvas", "canvas", &path, Some(2)).unwrap();
        let video = lib.register("video", "video", &path, Some(3)).unwrap();
        let future = lib
            .register("spectrum.future-music", "score", &path, Some(4))
            .unwrap();
        assert_eq!(lib.get(future.id).unwrap().kind, "spectrum.future-music");
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
        assert_eq!(
            lib.register("image", "renamed", &path, Some(1)).unwrap().id,
            image.id
        );
    }
}
