//! Projects group library assets without owning them, and import batches record
//! which assets arrived together. Removing an asset from a project never
//! deletes it; assets in no project remain in the library as unassigned.
use crate::{Asset, AssetId, Library};
use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub type ProjectId = uuid::Uuid;
pub type BatchId = uuid::Uuid;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,
      name TEXT NOT NULL UNIQUE COLLATE NOCASE, created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS project_assets(
      project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
      asset TEXT NOT NULL REFERENCES assets(id), added INTEGER NOT NULL,
      PRIMARY KEY(project,asset));
    CREATE TABLE IF NOT EXISTS imports(id TEXT PRIMARY KEY, created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS import_assets(asset TEXT PRIMARY KEY REFERENCES assets(id),
      batch TEXT NOT NULL REFERENCES imports(id));";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Unix seconds.
    pub created: i64,
    /// Number of member assets.
    pub assets: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportBatch {
    pub id: BatchId,
    /// Unix seconds.
    pub created: i64,
    pub assets: Vec<AssetId>,
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn project_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        bail!("project name cannot be empty");
    }
    Ok(name)
}

impl Library {
    fn unique_name(&self, name: &str, except: Option<ProjectId>) -> Result<()> {
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM projects WHERE name=?1 COLLATE NOCASE",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_some_and(|id| Some(id) != except.map(|e| e.to_string())) {
            bail!("a project named \"{name}\" already exists");
        }
        Ok(())
    }

    pub fn create_project(&self, name: &str) -> Result<Project> {
        let name = project_name(name)?;
        self.unique_name(name, None)?;
        let id = ProjectId::new_v4();
        self.db.execute(
            "INSERT INTO projects VALUES(?1,?2,?3)",
            params![id.to_string(), name, now()],
        )?;
        self.project(id)
    }

    pub fn rename_project(&self, id: ProjectId, name: &str) -> Result<Project> {
        let name = project_name(name)?;
        self.project(id)?;
        self.unique_name(name, Some(id))?;
        self.db.execute(
            "UPDATE projects SET name=?2 WHERE id=?1",
            params![id.to_string(), name],
        )?;
        self.project(id)
    }

    /// Deletes the project and its memberships. Its assets stay in the library.
    pub fn delete_project(&self, id: ProjectId) -> Result<()> {
        self.project(id)?;
        self.db
            .execute("DELETE FROM projects WHERE id=?1", [id.to_string()])?;
        Ok(())
    }

    pub fn project(&self, id: ProjectId) -> Result<Project> {
        self.projects()?
            .into_iter()
            .find(|p| p.id == id)
            .context("project not found")
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut statement = self.db.prepare(
            "SELECT p.id, p.name, p.created, COUNT(m.asset) FROM projects p
            LEFT JOIN project_assets m ON m.project=p.id
              AND m.asset NOT IN (SELECT asset FROM removed)
            GROUP BY p.id ORDER BY p.name COLLATE NOCASE, p.id",
        )?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, created, assets) = row?;
            Ok(Project {
                id: id.parse()?,
                name,
                created,
                assets: assets as usize,
            })
        })
        .collect()
    }

    /// Adds assets to a project. Assets already in it keep their original order.
    pub fn add_to_project(&mut self, project: ProjectId, assets: &[AssetId]) -> Result<()> {
        self.project(project)?;
        for asset in assets {
            self.get(*asset)?;
        }
        let added = now();
        let tx = self.db.transaction()?;
        for asset in assets {
            tx.execute(
                "INSERT OR IGNORE INTO project_assets VALUES(?1,?2,?3)",
                params![project.to_string(), asset.to_string(), added],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes assets from a project only. They remain in the library.
    pub fn remove_from_project(&mut self, project: ProjectId, assets: &[AssetId]) -> Result<()> {
        self.project(project)?;
        let tx = self.db.transaction()?;
        for asset in assets {
            tx.execute(
                "DELETE FROM project_assets WHERE project=?1 AND asset=?2",
                params![project.to_string(), asset.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A project's assets, most recently added first.
    pub fn project_assets(&self, project: ProjectId) -> Result<Vec<Asset>> {
        self.project(project)?;
        self.assets_where(
            &format!(
                "JOIN project_assets m ON m.asset=a.id WHERE m.project=?1 AND {}
                ORDER BY m.added DESC, a.name, a.id",
                crate::trash::LIVE
            ),
            [project.to_string()],
        )
    }

    /// When each asset joined a project, in Unix seconds.
    pub fn project_added(&self, project: ProjectId) -> Result<Vec<(AssetId, i64)>> {
        let mut statement = self
            .db
            .prepare("SELECT asset, added FROM project_assets WHERE project=?1")?;
        let rows = statement
            .query_map([project.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, added)| Ok((id.parse()?, added)))
            .collect()
    }

    /// Assets that belong to no project.
    pub fn unassigned(&self) -> Result<Vec<Asset>> {
        self.assets_where(
            &format!(
                "WHERE {} AND NOT EXISTS (SELECT 1 FROM project_assets m WHERE m.asset=a.id)
                ORDER BY a.name, a.id",
                crate::trash::LIVE
            ),
            [],
        )
    }

    /// Projects that contain an asset.
    pub fn asset_projects(&self, asset: AssetId) -> Result<Vec<Project>> {
        let mut statement = self
            .db
            .prepare("SELECT project FROM project_assets WHERE asset=?1")?;
        let ids = statement
            .query_map([asset.to_string()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(self
            .projects()?
            .into_iter()
            .filter(|p| ids.contains(&p.id.to_string()))
            .collect())
    }

    /// Records assets that were imported together.
    pub fn record_import(&mut self, assets: &[AssetId]) -> Result<ImportBatch> {
        for asset in assets {
            self.get(*asset)?;
        }
        let id = BatchId::new_v4();
        let created = now();
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO imports VALUES(?1,?2)",
            params![id.to_string(), created],
        )?;
        for asset in assets {
            tx.execute(
                "INSERT INTO import_assets VALUES(?1,?2)",
                params![asset.to_string(), id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(ImportBatch {
            id,
            created,
            assets: assets.to_vec(),
        })
    }

    /// Import batches, newest first.
    pub fn imports(&self) -> Result<Vec<ImportBatch>> {
        let mut statement = self.db.prepare(
            "SELECT i.id, i.created, m.asset FROM imports i
            LEFT JOIN import_assets m ON m.batch=i.id ORDER BY i.created DESC, i.id, m.asset",
        )?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut batches: Vec<ImportBatch> = Vec::new();
        for row in rows {
            let (id, created, asset) = row?;
            let id: BatchId = id.parse()?;
            if batches.last().is_none_or(|b| b.id != id) {
                batches.push(ImportBatch {
                    id,
                    created,
                    assets: Vec::new(),
                });
            }
            if let Some(asset) = asset {
                batches.last_mut().unwrap().assets.push(asset.parse()?);
            }
        }
        Ok(batches)
    }

    fn assets_where(&self, clause: &str, params: impl rusqlite::Params) -> Result<Vec<Asset>> {
        let mut statement = self.db.prepare(&format!(
            "SELECT a.id,a.kind,a.name,a.document FROM assets a {clause}"
        ))?;
        let rows = statement.query_map(params, crate::asset_row)?;
        rows.map(|row| crate::asset_from_row(row?)).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::Library;

    #[test]
    fn projects_group_assets_without_owning_them() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open(tmp.path()).unwrap();
        let a = lib
            .register(
                crate::AssetKind::Image,
                "a",
                &crate::test_document(tmp.path(), 1),
            )
            .unwrap();
        let b = lib
            .register(
                crate::AssetKind::Image,
                "b",
                &crate::test_document(tmp.path(), 2),
            )
            .unwrap();
        assert_eq!(lib.unassigned().unwrap().len(), 2);

        let trip = lib.create_project("Trip").unwrap();
        assert!(lib.create_project(" trip ").is_err());
        assert!(lib.create_project("  ").is_err());
        let other = lib.create_project("Other").unwrap();
        lib.add_to_project(trip.id, &[a.id, b.id]).unwrap();
        lib.add_to_project(other.id, &[a.id]).unwrap();
        lib.add_to_project(trip.id, &[a.id]).unwrap();
        assert_eq!(lib.project(trip.id).unwrap().assets, 2);
        assert_eq!(lib.asset_projects(a.id).unwrap().len(), 2);
        assert_eq!(lib.project_added(trip.id).unwrap().len(), 2);
        assert!(lib.unassigned().unwrap().is_empty());

        lib.remove_from_project(trip.id, &[b.id]).unwrap();
        assert_eq!(lib.project_assets(trip.id).unwrap(), vec![a.clone()]);
        assert_eq!(lib.unassigned().unwrap(), vec![b.clone()]);
        assert_eq!(lib.list().unwrap().len(), 2);

        assert!(lib.rename_project(other.id, "TRIP").is_err());
        assert_eq!(lib.rename_project(trip.id, "trip").unwrap().name, "trip");
        lib.delete_project(other.id).unwrap();
        assert_eq!(lib.projects().unwrap().len(), 1);
        assert_eq!(lib.asset_projects(a.id).unwrap().len(), 1);
        assert!(lib.add_to_project(other.id, &[a.id]).is_err());
    }

    #[test]
    fn import_batches_record_assets_that_arrived_together() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open(tmp.path()).unwrap();
        let a = lib
            .register(
                crate::AssetKind::Image,
                "a",
                &crate::test_document(tmp.path(), 1),
            )
            .unwrap();
        let b = lib
            .register(
                crate::AssetKind::Image,
                "b",
                &crate::test_document(tmp.path(), 2),
            )
            .unwrap();
        let c = lib
            .register(
                crate::AssetKind::Image,
                "c",
                &crate::test_document(tmp.path(), 3),
            )
            .unwrap();
        let first = lib.record_import(&[a.id, b.id]).unwrap();
        let second = lib.record_import(&[c.id]).unwrap();
        assert!(lib.record_import(&[a.id]).is_err());
        let batches = lib.imports().unwrap();
        assert_eq!(batches.len(), 2);
        let find = |id| batches.iter().find(|b| b.id == id).unwrap();
        let mut assets = find(first.id).assets.clone();
        assets.sort();
        let mut expected = vec![a.id, b.id];
        expected.sort();
        assert_eq!(assets, expected);
        assert_eq!(find(second.id).assets, vec![c.id]);
    }
}
