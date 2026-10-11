use crate::{models::*, platform::display_path};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::HashSet, path::Path};

pub struct Index {
    connection: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if !(0..=2).contains(&version) {
            bail!(
                "索引格式版本 {version} 高于此程序支持的版本 2；请使用更新的程序，不会覆盖现有索引"
            );
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS items (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, source TEXT NOT NULL,
                game_dir TEXT, name TEXT NOT NULL, search TEXT NOT NULL,
                size INTEGER, time_ms INTEGER, ownership TEXT NOT NULL,
                available INTEGER NOT NULL, generation INTEGER NOT NULL, json TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS items_kind_size ON items(kind,size);
             CREATE INDEX IF NOT EXISTS items_kind_time ON items(kind,time_ms);
             CREATE INDEX IF NOT EXISTS items_game ON items(game_dir);
             CREATE INDEX IF NOT EXISTS items_source ON items(source);
             CREATE TABLE IF NOT EXISTS annotations (
                item_id TEXT PRIMARY KEY, favorite INTEGER NOT NULL DEFAULT 0,
                tags_json TEXT NOT NULL DEFAULT '[]', search TEXT NOT NULL DEFAULT '', last_opened_ms INTEGER);
             CREATE TABLE IF NOT EXISTS annotation_tags (
                item_id TEXT NOT NULL, tag TEXT NOT NULL, tag_key TEXT NOT NULL,
                PRIMARY KEY(item_id,tag_key));
             CREATE INDEX IF NOT EXISTS annotation_tags_key ON annotation_tags(tag_key,item_id);
             CREATE INDEX IF NOT EXISTS annotations_recent ON annotations(last_opened_ms);
             PRAGMA user_version=2;"
        )?;
        transaction.commit()?;
        Ok(Self { connection })
    }

    pub fn load_settings(&self) -> Result<Settings> {
        let json: Option<String> = self
            .connection
            .query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
            .optional()?;
        Ok(match json {
            Some(json) => serde_json::from_str(&json)?,
            None => Settings::default(),
        })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    pub fn upsert(&self, item: &Item, generation: i64) -> Result<()> {
        let content = item
            .content
            .as_ref()
            .map(|content| {
                format!(
                    "{} {} {} {} {} {}",
                    content.description.as_deref().unwrap_or(""),
                    content.declared_id.as_deref().unwrap_or(""),
                    content.declared_version.as_deref().unwrap_or(""),
                    content.authors.join(" "),
                    content.loaders.join(" "),
                    content.compatibility.join(" ")
                )
            })
            .unwrap_or_default();
        let search = format!(
            "{} {} {} {} {} {} {} {}",
            item.name,
            item.folder,
            display_path(&item.path),
            display_path(&item.root),
            item.ownership,
            item.version_id.as_deref().unwrap_or(""),
            item.game_version.as_deref().unwrap_or(""),
            content
        )
        .to_lowercase();
        // User annotations live in separate tables and cannot be replaced by a refresh.
        let mut scanned = item.clone();
        scanned.favorite = false;
        scanned.tags.clear();
        scanned.last_opened_ms = None;
        self.connection.execute(
            "INSERT INTO items(id,kind,source,game_dir,name,search,size,time_ms,ownership,available,generation,json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,source=excluded.source,game_dir=excluded.game_dir,
                name=excluded.name,search=excluded.search,size=excluded.size,time_ms=excluded.time_ms,
                ownership=excluded.ownership,available=excluded.available,generation=excluded.generation,json=excluded.json",
            params![item.id, item.kind.as_str(), item.source, item.game_dir.as_ref().map(|p| display_path(p)),
                item.name, search, item.size.map(|n| n.min(i64::MAX as u64) as i64), item.last_played_ms.or(item.modified_ms),
                item.ownership, item.available, generation, serde_json::to_string(&scanned)?],
        )?;
        Ok(())
    }

    pub fn annotate(&self, id: &str, favorite: bool, tags: &[String]) -> Result<()> {
        self.require_item(id)?;
        let mut seen = HashSet::new();
        let tags: Vec<String> = tags
            .iter()
            .map(|tag| tag.trim())
            .filter(|tag| !tag.is_empty())
            .filter(|tag| seen.insert(tag.to_lowercase()))
            .map(str::to_owned)
            .collect();
        if tags.len() > 64 || tags.iter().any(|tag| tag.chars().count() > 128) {
            bail!("标签最多 64 个，每个标签最多 128 个字符");
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO annotations(item_id,favorite,tags_json,search) VALUES(?1,?2,?3,?4)
             ON CONFLICT(item_id) DO UPDATE SET favorite=excluded.favorite,tags_json=excluded.tags_json,search=excluded.search",
            params![id, favorite, serde_json::to_string(&tags)?, tags.join(" ").to_lowercase()],
        )?;
        transaction.execute("DELETE FROM annotation_tags WHERE item_id=?1", [id])?;
        for tag in &tags {
            transaction.execute(
                "INSERT INTO annotation_tags(item_id,tag,tag_key) VALUES(?1,?2,?3)",
                params![id, tag, tag.to_lowercase()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_opened(&self, id: &str) -> Result<()> {
        self.require_item(id)?;
        self.connection.execute(
            "INSERT INTO annotations(item_id,last_opened_ms) VALUES(?1,?2)
             ON CONFLICT(item_id) DO UPDATE SET last_opened_ms=excluded.last_opened_ms",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    fn require_item(&self, id: &str) -> Result<()> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM items WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if !exists {
            bail!("索引条目不存在：{id}");
        }
        Ok(())
    }

    pub fn tags(&self) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare("SELECT MIN(t.tag) FROM annotation_tags t INNER JOIN items i ON i.id=t.item_id GROUP BY t.tag_key ORDER BY t.tag_key")?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn discovered(&mut self, items: &[Item], generation: i64) -> Result<()> {
        // Existing cache stays visible during refresh; only new items start with unknown sizes.
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            for item in items {
                let touched = self.connection.execute(
                    "UPDATE items SET generation=?1 WHERE id=?2",
                    params![generation, item.id],
                )?;
                if touched == 0 {
                    self.upsert(item, generation)?;
                }
            }
            Ok(())
        })();
        if result.is_ok() {
            self.connection.execute_batch("COMMIT")?;
        } else {
            self.connection.execute_batch("ROLLBACK")?;
        }
        result
    }

    pub fn finish_source(&self, source: &str, generation: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM items WHERE source=?1 AND generation<>?2",
            params![source, generation],
        )?;
        Ok(())
    }

    pub fn mark_unavailable(&self, source: &str) -> Result<()> {
        let items = self.query(&Query {
            source: Some(source.into()),
            ..Default::default()
        })?;
        for mut item in items {
            item.available = false;
            item.metadata_status =
                "缓存条目：来源目录不可访问；保留历史索引，重新连接后刷新".into();
            self.upsert(&item, 0)?;
        }
        Ok(())
    }

    pub fn remove_source(&self, source: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM items WHERE source=?1", [source])?;
        Ok(())
    }

    pub fn remove_items(&self, ids: &[String]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        for id in ids {
            transaction.execute("DELETE FROM items WHERE id=?1", [id])?;
        }
        // Keep annotations so a restored file at the same indexed path retains its labels.
        transaction.commit()?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Item>> {
        let row: Option<(String, bool, String, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT i.json,COALESCE(a.favorite,0),COALESCE(a.tags_json,'[]'),a.last_opened_ms
             FROM items i LEFT JOIN annotations a ON a.item_id=i.id WHERE i.id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(merge_annotations).transpose()
    }

    pub fn query(&self, query: &Query) -> Result<Vec<Item>> {
        // Escape LIKE metacharacters: '%' and '_' typed by users remain literal.
        let term = query
            .text
            .to_lowercase()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{term}%");
        let order = match query.sort {
            Sort::Name => "i.name COLLATE NOCASE ASC",
            Sort::SizeDesc => "i.size DESC",
            Sort::SizeAsc => "i.size IS NULL ASC, i.size ASC",
            Sort::TimeDesc => "i.time_ms DESC",
            Sort::TimeAsc => "i.time_ms IS NULL ASC, i.time_ms ASC",
            Sort::Ownership => "i.ownership COLLATE NOCASE ASC, i.name COLLATE NOCASE ASC",
            Sort::Recent => "a.last_opened_ms IS NULL ASC, a.last_opened_ms DESC",
        };
        let sql = format!(
            "SELECT i.json,COALESCE(a.favorite,0),COALESCE(a.tags_json,'[]'),a.last_opened_ms
             FROM items i LEFT JOIN annotations a ON a.item_id=i.id
             WHERE (i.search LIKE ?1 ESCAPE '\\' OR COALESCE(a.search,'') LIKE ?1 ESCAPE '\\')
             AND (?2 IS NULL OR i.kind=?2) AND (?3 IS NULL OR i.game_dir=?3) AND (?4 IS NULL OR i.source=?4)
             AND (?5=0 OR a.favorite=1)
             AND (?6 IS NULL OR EXISTS(SELECT 1 FROM annotation_tags t WHERE t.item_id=i.id AND t.tag_key=?6))
             AND (?7=0 OR a.last_opened_ms IS NOT NULL) ORDER BY {order},i.id ASC"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                pattern,
                query.kind.as_ref().map(ItemKind::as_str),
                query.game_dir,
                query.source,
                query.favorite_only,
                query.tag.as_ref().map(|t| t.trim().to_lowercase()),
                query.recent_only
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )?;
        rows.map(|row| merge_annotations(row?)).collect()
    }

    pub fn stats(&self) -> Result<Stats> {
        let count = |kind: &str| -> Result<u64> {
            Ok(self.connection.query_row(
                "SELECT COUNT(*) FROM items WHERE kind=?1",
                [kind],
                |r| r.get::<_, i64>(0),
            )? as u64)
        };
        Ok(Stats {
            worlds: count("world")?,
            versions: count("version")?,
            instances: count("instance")?,
            resource_packs: count("resource_pack")?,
            mods: count("mod")?,
            shader_packs: count("shader_pack")?,
            data_packs: count("data_pack")?,
            world_bytes: self.connection.query_row(
                "SELECT COALESCE(SUM(size),0) FROM items WHERE kind='world'",
                [],
                |r| r.get::<_, i64>(0),
            )? as u64,
        })
    }
}

fn merge_annotations(
    (json, favorite, tags, opened): (String, bool, String, Option<i64>),
) -> Result<Item> {
    let mut item: Item = serde_json::from_str(&json)?;
    item.favorite = favorite;
    item.tags = serde_json::from_str(&tags)?;
    item.last_opened_ms = opened;
    Ok(item)
}
