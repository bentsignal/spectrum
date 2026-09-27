//! Deleted assets wait in the trash for [`TRASH_DAYS`] before engines purge
//! their content. Trashed assets keep their identity, memberships, and
//! references, so restoring one returns it to its projects and canvases.
//! Purged assets leave a record so canvases can draw a same-sized placeholder.
use crate::{Asset, AssetId, Library, projects::now};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub const TRASH_DAYS: i64 = 30;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS removed(asset TEXT PRIMARY KEY, kind TEXT NOT NULL,
      name TEXT NOT NULL, width INTEGER, height INTEGER, deleted INTEGER NOT NULL,
      purged INTEGER);";

/// Excludes trashed and purged assets from asset queries on alias `a`.
pub(crate) const LIVE: &str = "a.id NOT IN (SELECT asset FROM removed)";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trashed {
    pub asset: Asset,
    /// Unix seconds.
    pub deleted: i64,
    /// Unix seconds after which the asset is purged permanently.
    pub purge_after: i64,
}

/// A trashed or purged asset, as canvases that still use it see it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Removed {
    pub id: AssetId,
    pub kind: String,
    pub name: String,
    /// Pixel size of the image as last rendered, for placeholders.
    pub size: Option<(u32, u32)>,
    pub purged: bool,
}

impl Library {
    /// Moves a live asset to the trash. `size` is its rendered pixel size.
    pub fn trash(&mut self, id: AssetId, size: Option<(u32, u32)>) -> Result<Trashed> {
        let asset = self.get(id)?;
        let deleted = now();
        self.db.execute(
            "INSERT INTO removed VALUES(?1,?2,?3,?4,?5,?6,NULL)",
            params![
                id.to_string(),
                asset.kind,
                asset.name,
                size.map(|s| s.0),
                size.map(|s| s.1),
                deleted
            ],
        )?;
        Ok(Trashed {
            asset,
            deleted,
            purge_after: deleted + TRASH_DAYS * 86_400,
        })
    }

    /// Returns a trashed asset to the library, its projects, and its canvases.
    pub fn restore(&mut self, id: AssetId) -> Result<Asset> {
        let restored = self.db.execute(
            "DELETE FROM removed WHERE asset=?1 AND purged IS NULL",
            [id.to_string()],
        )?;
        if restored == 0 {
            bail!("asset is not in the trash");
        }
        self.get(id)
    }

    /// Trashed assets, most recently deleted first.
    pub fn trashed(&self) -> Result<Vec<Trashed>> {
        let mut statement = self.db.prepare(
            "SELECT asset, deleted FROM removed WHERE purged IS NULL ORDER BY deleted DESC, asset",
        )?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, deleted)| {
                Ok(Trashed {
                    asset: self.lookup(id.parse()?)?,
                    deleted,
                    purge_after: deleted + TRASH_DAYS * 86_400,
                })
            })
            .collect()
    }

    /// Trashed assets whose 30 days have passed at `time`.
    pub fn expired(&self, time: i64) -> Result<Vec<Asset>> {
        Ok(self
            .trashed()?
            .into_iter()
            .filter(|t| t.purge_after <= time)
            .map(|t| t.asset)
            .collect())
    }

    /// The trashed or purged record for an asset, if it was deleted.
    pub fn removed(&self, id: AssetId) -> Result<Option<Removed>> {
        self.db
            .query_row(
                "SELECT kind, name, width, height, purged FROM removed WHERE asset=?1",
                [id.to_string()],
                |r| {
                    let width: Option<u32> = r.get(2)?;
                    let height: Option<u32> = r.get(3)?;
                    Ok(Removed {
                        id,
                        kind: r.get(0)?,
                        name: r.get(1)?,
                        size: width.zip(height),
                        purged: r.get::<_, Option<i64>>(4)?.is_some(),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Whether any indexed asset, including a trashed one, lives in `document`.
    pub fn document_in_use(&self, document: &std::path::Path) -> Result<bool> {
        Ok(self
            .db
            .query_row(
                "SELECT 1 FROM assets WHERE document=?1 LIMIT 1",
                [document.to_string_lossy()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Drops a trashed asset from the index after its engine content is gone.
    /// Its placeholder record remains for canvases that still name it.
    pub fn forget(&mut self, id: AssetId) -> Result<()> {
        if self.removed(id)?.is_none_or(|r| r.purged) {
            bail!("only trashed assets can be purged");
        }
        let id = id.to_string();
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM refs WHERE owner=?1 OR target=?1", [&id])?;
        tx.execute("DELETE FROM project_assets WHERE asset=?1", [&id])?;
        tx.execute("DELETE FROM import_assets WHERE asset=?1", [&id])?;
        tx.execute("DELETE FROM assets WHERE id=?1", [&id])?;
        tx.execute(
            "UPDATE removed SET purged=?2 WHERE asset=?1",
            params![id, now()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Library, TRASH_DAYS};

    #[test]
    fn trash_hides_assets_until_restored_or_purged() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open(tmp.path()).unwrap();
        let path = tmp.path().join("data");
        std::fs::write(&path, b"fixture").unwrap();
        let image = lib.register("image", "image", &path, Some(1)).unwrap();
        let canvas = lib.register("canvas", "canvas", &path, Some(2)).unwrap();
        lib.references(canvas.id, &[("layer".into(), image.id)])
            .unwrap();
        let project = lib.create_project("Trip").unwrap();
        lib.add_to_project(project.id, &[image.id]).unwrap();

        let trashed = lib.trash(image.id, Some((40, 30))).unwrap();
        assert!(lib.trash(image.id, None).is_err());
        assert_eq!(trashed.purge_after - trashed.deleted, TRASH_DAYS * 86_400);
        assert!(lib.get(image.id).is_err());
        assert_eq!(lib.list().unwrap(), vec![canvas.clone()]);
        assert_eq!(lib.project(project.id).unwrap().assets, 0);
        assert!(lib.unassigned().unwrap().iter().all(|a| a.id != image.id));
        // Rescans and canvas indexing keep working while the image is trashed.
        lib.register("image", "image", &path, Some(1)).unwrap();
        lib.references(canvas.id, &[("layer".into(), image.id)])
            .unwrap();
        assert!(lib.expired(trashed.purge_after - 1).unwrap().is_empty());
        assert_eq!(lib.expired(trashed.purge_after).unwrap().len(), 1);

        lib.restore(image.id).unwrap();
        assert_eq!(lib.project_assets(project.id).unwrap(), vec![image.clone()]);
        assert_eq!(lib.dependents(image.id).unwrap(), vec![canvas.id]);
        assert!(lib.restore(image.id).is_err());

        lib.trash(image.id, Some((40, 30))).unwrap();
        lib.forget(image.id).unwrap();
        assert!(lib.trashed().unwrap().is_empty());
        let removed = lib.removed(image.id).unwrap().unwrap();
        assert!(removed.purged);
        assert_eq!(removed.size, Some((40, 30)));
        assert!(lib.restore(image.id).is_err());
        // Canvases that still name a purged image index without it.
        lib.references(canvas.id, &[("layer".into(), image.id)])
            .unwrap();
        assert!(lib.dependents(image.id).unwrap().is_empty());
        assert!(lib.forget(canvas.id).is_err());
    }
}
