// import old shell history from mcfly!
// automatically hoover up all that we can find

//
// An example sqlite query for mcfly data:
//
//id|cmd|cmd_tpl|session_id|when_run|exit_code|selected|dir\old_dir
//
//
// SELECT
//   commands.id,
//   commands.when_run,
//   commands.exit_code,
//   commands.cmd,
//   commands.dir
// FROM commands;
//
// CREATE TABLE commands(id INTEGER PRIMARY KEY AUTOINCREMENT,
//                       cmd TEXT NOT NULL, cmd_tpl TEXT,
//                       session_id TEXT NOT NULL,
//                       when_run INTEGER NOT NULL,
//                       exit_code INTEGER NOT NULL,
//                       selected INTEGER NOT NULL,
//                       dir TEXT,
//                       old_dir TEXT);
//

use std::convert::TryInto;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{prelude::*, Utc};
use directories::UserDirs;
use eyre::{eyre, Result};
use sqlx::{sqlite::SqlitePool, Pool};

use super::Importer;
use crate::history::History;
use crate::import::Loader;

#[derive(sqlx::FromRow, Debug)]
pub struct McFlyEntryCount {
    pub count: usize,
}

#[derive(sqlx::FromRow, Debug)]
pub struct McFlyEntry {
    pub id: i64,
    pub when_run: NaiveDateTime,
    pub exit_code: i64,
    pub cmd: String,
    pub dir: String,
}

impl From<McFlyEntry> for History {
    fn from(mcfly_item: McFlyEntry) -> Self {
        let dt = NaiveDateTime::from_timestamp(mcfly_item.when_run.timestamp(), mcfly_item.id.try_into().unwrap()); // try to use id as nanosecs of timestamp
        History::new(
            DateTime::from_utc(dt, Utc), // must assume UTC?
            mcfly_item.cmd,
            mcfly_item.dir,
            mcfly_item.exit_code,
            0, // assume 0, we have no way of knowing :(
            None,
            None,
        )
    }
}

#[derive(Debug)]
pub struct McFly {
    entries: Vec<McFlyEntry>,
}

/// Read db at given file, return vector of entries.
async fn hist_from_db(dbpath: PathBuf) -> Result<Vec<McFlyEntry>> {
    let pool = SqlitePool::connect(dbpath.to_str().unwrap()).await?;
    hist_from_db_conn(pool).await
}

async fn hist_from_db_conn(pool: Pool<sqlx::Sqlite>) -> Result<Vec<McFlyEntry>> {
    let query = "select commands.id, commands.when_run, commands.exit_code, commands.cmd, commands.dir from commands order by commands.id";
    let myflydb_vec: Vec<McFlyEntry> = sqlx::query_as::<_, McFlyEntry>(query)
        .fetch_all(&pool)
        .await?;
    Ok(myflydb_vec)
}

impl McFly {
    pub fn path_candidate() -> PathBuf {
        // TODO: This needs work - mcfly has multiple default paths
        let user_dirs = UserDirs::new().unwrap(); // should catch error here?
        let home_dir = user_dirs.home_dir();
        std::env::var("MCFLY_HISTORY_FILE")
            .as_ref()
            .map(|x| Path::new(x).to_path_buf())
            .unwrap_or_else(|_err| home_dir.join(".mcfly/history.db"))
    }
    pub fn path() -> Result<PathBuf> {
        let mcfly_path = McFly::path_candidate();
        if mcfly_path.exists() {
            Ok(mcfly_path)
        } else {
            Err(eyre!(
                "Could not find history file. Try setting $MCFLY_HISTORY_FILE"
            ))
        }
    }
}

#[async_trait]
impl Importer for McFly {
    const NAME: &'static str = "mcfly";

    /// Creates a new McFly and populates the history based on the pre-populated data
    /// structure.
    async fn new() -> Result<Self> {
        let dbpath = McFly::path()?;
        let mcfly_entry_vec = hist_from_db(dbpath).await?;
        Ok(Self {
            entries: mcfly_entry_vec,
        })
    }
    async fn entries(&mut self) -> Result<usize> {
        Ok(self.entries.len())
    }
    async fn load(self, h: &mut impl Loader) -> Result<()> {
        for i in self.entries {
            h.push(i.into()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::env;
    #[tokio::test(flavor = "multi_thread")]
    async fn test_env_vars() {
        let test_env_db = "nonstd-mcfly-history.db";
        let key = "MCFLY_HISTORY_FILE";
        env::set_var(key, test_env_db);

        // test the env got set
        assert_eq!(env::var(key).unwrap(), test_env_db.to_string());

        // test histdb returns the proper db from previous step
        let path = McFly::path_candidate();
        assert_eq!(path.to_str().unwrap(), test_env_db);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_import() {
        let pool: SqlitePool = SqlitePoolOptions::new()
            .min_connections(2)
            .connect(":memory:")
            .await
            .unwrap();

        // sql dump directly from a test database.
        // let db_sql = r#"
        // PRAGMA foreign_keys=OFF;
        // BEGIN TRANSACTION;
        // CREATE TABLE commands (id integer primary key autoincrement, argv text, unique(argv) on conflict ignore);
        // INSERT INTO commands VALUES(1,'pwd');
        // INSERT INTO commands VALUES(2,'curl google.com');
        // INSERT INTO commands VALUES(3,'bash');
        // CREATE TABLE places   (id integer primary key autoincrement, host text, dir text, unique(host, dir) on conflict ignore);
        // INSERT INTO places VALUES(1,'mbp16.local','/home/noyez');
        // CREATE TABLE history  (id integer primary key autoincrement,
        //                        session int,
        //                        command_id int references commands (id),
        //                        place_id int references places (id),
        //                        exit_status int,
        //                        start_time int,
        //                        duration int);
        // INSERT INTO history VALUES(1,0,1,1,0,1651497918,1);
        // INSERT INTO history VALUES(2,0,2,1,0,1651497923,1);
        // INSERT INTO history VALUES(3,0,3,1,NULL,1651497930,NULL);
        // DELETE FROM sqlite_sequence;
        // INSERT INTO sqlite_sequence VALUES('commands',3);
        // INSERT INTO sqlite_sequence VALUES('places',3);
        // INSERT INTO sqlite_sequence VALUES('history',3);
        // CREATE INDEX hist_time on history(start_time);
        // CREATE INDEX place_dir on places(dir);
        // CREATE INDEX place_host on places(host);
        // CREATE INDEX history_command_place on history(command_id, place_id);
        // COMMIT; "#;
        let db_sql = r#"
        BEGIN TRANSACTION;
        CREATE TABLE IF NOT EXISTS schema_versions(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                version INTEGER NOT NULL,
                when_run INTEGER NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS schema_versions_index ON schema_versions (version);
        INSERT INTO schema_versions (version, when_run) VALUES (3, strftime('%s','now'));
        COMMIT;
        "#;

        sqlx::query(db_sql).execute(&pool).await.unwrap();

        // test mcfly iterator
        let hist_vec = hist_from_db_conn(pool).await.unwrap();
        let mcfly = McFly { entries: hist_vec };

        println!("h: {:#?}", mcfly.entries);
        println!("counter: {:?}", mcfly.entries.len());
        for i in mcfly.entries {
            println!("{:?}", i);
        }
    }
}
