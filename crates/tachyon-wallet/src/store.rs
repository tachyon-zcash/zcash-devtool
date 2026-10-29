//! One SQLite writer lock covers selection and durable reservation of inputs.

use crate::{chain::Block, funding_balance::Snapshot, note::Receipt};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub rpc_url: String,
    pub cookie_file: Option<PathBuf>,
    pub activation: u32,
    pub activation_hash: String,
    pub genesis: String,
    pub network_id: String,
    pub seed: [u8; 32],
    /// Missing on existing wallets, which retain their activation-based history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday_height: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct Pending {
    pub txid: String,
    pub raw: String,
    pub expiry: u32,
    pub source: String,
    pub receipt: Option<Receipt>,
}

pub struct StoredOperation {
    pub id: String,
    pub txid: String,
    pub request: serde_json::Value,
}

pub struct Store {
    conn: Connection,
}

/// Never overwrite receipt files, and create private data with restrictive permissions.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("cannot create private file (it may already exist)")?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

impl Store {
    pub fn open(dir: &Path, initialize: bool) -> Result<Self> {
        if initialize && !dir.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(dir)?;
            }
            #[cfg(not(unix))]
            fs::create_dir_all(dir)?;
        }
        let metadata = fs::symlink_metadata(dir).context("wallet does not exist; run init")?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "wallet must be a real directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "wallet directory must be private (chmod 700)"
            );
        }
        let path = dir.join("tachyon.sqlite");
        if initialize {
            write_private(&path, &[])?;
        }
        ensure!(
            fs::symlink_metadata(&path)?.is_file(),
            "wallet database must be a regular file"
        );
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(Duration::from_secs(1))?;
        conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; BEGIN IMMEDIATE;")?;
        if initialize {
            conn.execute_batch(
                "CREATE TABLE config (id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);
                CREATE TABLE blocks (height INTEGER PRIMARY KEY, data TEXT NOT NULL);
                CREATE TABLE notes (id TEXT PRIMARY KEY, data TEXT NOT NULL);
                CREATE TABLE pending (txid TEXT PRIMARY KEY, data TEXT NOT NULL);",
            )?;
        }
        Ok(Self { conn })
    }
    pub fn commit(&self) -> Result<()> {
        Ok(self.conn.execute_batch("COMMIT")?)
    }
    pub fn set_config(&self, config: &Config) -> Result<()> {
        self.conn.execute(
            "INSERT INTO config VALUES(1, ?)",
            [serde_json::to_string(config)?],
        )?;
        Ok(())
    }
    pub fn config(&self) -> Result<Config> {
        let json: String = self
            .conn
            .query_row("SELECT data FROM config WHERE id=1", [], |r| r.get(0))?;
        let config: Config = serde_json::from_str(&json)?;
        ensure!(config.version == 1, "unsupported wallet database version");
        Ok(config)
    }
    pub fn blocks(&self) -> Result<Vec<Block>> {
        let mut blocks: Vec<Block> = self.read_all("SELECT data FROM blocks ORDER BY height")?;
        // Backfills retain the already downloaded suffix. Until the gap closes,
        // only the contiguous prefix is usable by balance/proof construction.
        if let Some(gap) = blocks.windows(2).position(|pair| {
            pair[0].height.checked_add(1) != Some(pair[1].height)
                || pair[1].previous_block_hash != pair[0].hash
                || pair[1].anchor_before != pair[0].anchor_after
        }) {
            blocks.truncate(gap + 1);
        }
        Ok(blocks)
    }
    pub fn block(&self, height: u32) -> Result<Option<Block>> {
        self.conn
            .query_row("SELECT data FROM blocks WHERE height=?", [height], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .map(|data| Ok(serde_json::from_str(&data)?))
            .transpose()
    }
    pub fn clear_funding_snapshot(&self) -> Result<()> {
        if self.table_exists("funding_snapshot")? {
            self.conn.execute("DELETE FROM funding_snapshot", [])?;
        }
        Ok(())
    }
    pub fn clear_chain(&self) -> Result<()> {
        self.conn.execute("DELETE FROM blocks", [])?;
        self.clear_funding_snapshot()
    }
    pub fn notes(&self) -> Result<Vec<Receipt>> {
        self.read_all("SELECT data FROM notes ORDER BY id")
    }
    pub fn pending(&self) -> Result<Vec<Pending>> {
        self.read_all("SELECT data FROM pending ORDER BY txid")
    }
    pub fn operation(&self, id: &str) -> Result<Option<StoredOperation>> {
        if !self.table_exists("operations")? {
            return Ok(None);
        }
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT txid, request FROM operations WHERE id=?",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(txid, request)| {
            Ok(StoredOperation {
                id: id.into(),
                txid,
                request: serde_json::from_str(&request)?,
            })
        })
        .transpose()
    }
    pub fn operation_ids(&self) -> Result<std::collections::BTreeMap<String, String>> {
        if !self.table_exists("operations")? {
            return Ok(Default::default());
        }
        Ok(self
            .conn
            .prepare("SELECT txid, id FROM operations")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn put_operation(&self, operation: &StoredOperation) -> Result<()> {
        // The ID, signed bytes, input reservation and receipts commit atomically before broadcast.
        // Create lazily so older v1 wallets need no reset or eager migration.
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS operations (
            id TEXT PRIMARY KEY, txid TEXT NOT NULL UNIQUE REFERENCES pending(txid), request TEXT NOT NULL
        );")?;
        self.conn.execute(
            "INSERT INTO operations VALUES(?, ?, ?)",
            params![
                operation.id,
                operation.txid,
                serde_json::to_string(&operation.request)?
            ],
        )?;
        Ok(())
    }
    fn table_exists(&self, table: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            [table],
            |row| row.get(0),
        )?)
    }
    pub fn funding_snapshot(&self) -> Result<Option<Snapshot>> {
        if !self.table_exists("funding_snapshot")? {
            return Ok(None);
        }
        self.conn
            .query_row("SELECT data FROM funding_snapshot WHERE id=1", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .map(|data| Ok(serde_json::from_str(&data)?))
            .transpose()
    }
    pub fn put_funding_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        // Additive, lazy migration: existing v1 wallets remain readable before their next sync.
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS funding_snapshot (id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);"
        )?;
        self.conn.execute(
            "INSERT INTO funding_snapshot VALUES(1, ?) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            [serde_json::to_string(snapshot)?],
        )?;
        Ok(())
    }
    fn read_all<T: serde::de::DeserializeOwned>(&self, sql: &str) -> Result<Vec<T>> {
        self.conn
            .prepare(sql)?
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
    pub fn put_block(&self, block: &Block) -> Result<()> {
        self.conn.execute(
            "INSERT INTO blocks VALUES(?, ?) ON CONFLICT(height) DO UPDATE SET data=excluded.data",
            params![block.height, serde_json::to_string(block)?],
        )?;
        Ok(())
    }
    pub fn rewind(&self, height: u32) -> Result<()> {
        self.conn
            .execute("DELETE FROM blocks WHERE height > ?", [height])?;
        if self
            .funding_snapshot()?
            .is_some_and(|snapshot| snapshot.height > height)
        {
            self.conn.execute("DELETE FROM funding_snapshot", [])?;
        }
        Ok(())
    }
    pub fn put_note(&self, receipt: &Receipt) -> Result<()> {
        let existing: Option<String> = self
            .conn
            .query_row("SELECT data FROM notes WHERE id=?", [receipt.id()?], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(existing) = existing {
            let mut previous: Receipt = serde_json::from_str(&existing)?;
            let mut incoming = receipt.clone();
            let locator = incoming.locator.take().or(previous.locator.take());
            previous.locator = None;
            ensure!(
                previous == incoming,
                "conflicting receipt for this commitment"
            );
            incoming.locator = locator;
            self.conn.execute(
                "UPDATE notes SET data=? WHERE id=?",
                params![serde_json::to_string(&incoming)?, receipt.id()?],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO notes VALUES(?, ?)",
                params![receipt.id()?, serde_json::to_string(receipt)?],
            )?;
        }
        Ok(())
    }
    pub fn reserve(&self, pending: &Pending) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pending VALUES(?, ?)",
            params![pending.txid, serde_json::to_string(pending)?],
        )?;
        Ok(())
    }
}
