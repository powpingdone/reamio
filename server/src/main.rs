use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Query, State},
    response::IntoResponse,
    routing::{get, post},
};
use bytes::Buf;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::AsyncWriteExt,
    sync::{RwLock, watch},
};

mod error;
mod prelude;
mod process;

use crate::prelude::*;

#[derive(Clone)]
pub struct ReamioApp {
    pub user_db: SqlitePool,
    pub music_dbs: MusicDbMapRef,
    pub populate_mdata_waker: WakeTx<PopulateMetadata>,
}

impl std::fmt::Debug for ReamioApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReamioApp")
            .field("user_db", &self.user_db)
            .field("music_dbs", &self.music_dbs)
            .finish_non_exhaustive()
    }
}

#[tracing::instrument]
pub async fn fetch_users_music_db<U>(
    music_dbs: MusicDbMapRef,
    user: U,
) -> PoolConnection<sqlx::Sqlite>
where
    U: AsRef<str> + std::fmt::Debug,
{
    // TODO: create pools on demand, user management
    let music_db_hold = music_dbs.upgrade().unwrap();
    let music_db = music_db_hold.read().await;
    let db_pool = music_db.get(user.as_ref()).unwrap();
    db_pool.acquire().await.unwrap()
}

/// [[UploadArgs]]
/// Query arguments for a function
///
/// Fields:
/// - pub path: String
///     Path relative to the root of the user dir that the file will be uploaded to.
///     The path represented follows the following semantics:
///     
///     + The path is valid utf-8 (ie: String::from_utf8).
///     + Each item is seperated by a '/'.
///     + The path starts with '/', indicating the root of the dir.
///     + There is no preceding items for the first '/'.
///     + All items before the last item is a folder.
///     + The last item is the file name.
#[derive(Deserialize)]
struct UploadArgs {
    // TODO: Newtype this into something like "ReamioPath" with checks
    pub path: Option<String>,
}

#[derive(Serialize)]
struct UploadReturn {
    written: usize,
}

/// Ingest track. This does not process any tracks, only writes them to disk
/// and notifies the actual processor that there are tracks to process.
///
/// Path: /api/upload?path={}
///
/// Arguments:
/// - State(state): State<ReamioApp>
///     Server state. [[ReamioApp]].
/// - Query(UploadArgs { path }): Query<UploadArgs>
///     Query args. See [[UploadArgs]] for a description of the query parameters for this function.
/// - body: Body,
///     Body of HTTP request. This is the item to be uploaded in it's entirety.
#[tracing::instrument]
async fn upload_track(
    State(state): State<ReamioApp>,
    Query(UploadArgs { path }): Query<UploadArgs>,
    body: Body,
) -> Result<Json<UploadReturn>, ReamioWebError> {
    let Some(path) = path else {
        trace!("no path was attached to the query");
        return Ok(Json(UploadReturn {written: 0}));
    };
    debug!("Uploading path {}", path);

    // begin transaction
    let mut txn = state.user_db.begin_with("BEGIN IMMEDIATE").await?;
    trace!(transaction = ?txn);

    // CHANGING THE RETURN TYPE HAS SECURITY IMPLICATIONS
    //
    // now, why is it i64 and not something more generic, like a Uuid? long story short:
    // Uuid is a external module in sqlite and I dont want to actually load a module right now
    // TODO: change this to a uuid and possibly make this path safe
    let fid: i64 = sqlx::query(
        "INSERT INTO uploaded_files (orig_path, user, fid) VALUES ($1, $2, NULL) RETURNING fid;",
    )
    .bind(path)
    // TODO: dynamic users
    .bind("powpingdone")
    .fetch_one(&mut *txn)
    .await?
    .get("fid");
    trace!(fid);

    // write out file
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        // this is secure because fid is i64 and cannot represent anything other than [0-9]*
        .open(format!("./devdir/temp/{fid}"))
        .await
        .unwrap();
    trace!("file opened");

    let mut body = body.into_data_stream();
    let mut size_acc = 0;
    while let Some(mut chunk) = body.try_next().await? {
        while chunk.has_remaining() {
            size_acc += file.write_buf(&mut chunk).await.unwrap();
        }
    }
    file.sync_data().await.unwrap();
    drop(file);
    debug!(size_acc, fid, "file written");

    // operation is good
    txn.commit().await?;
    trace!(fid, "transaction finished");

    // wake the mdata
    //
    // TODO maintenace task: on transaction error, the files are left behind. clean
    // them up.
    state.populate_mdata_waker.send(PopulateMetadata).unwrap();
    trace!("sent waker for task");
    Ok(Json(UploadReturn { written: size_acc }))
}

/// Dump table for display.
#[tracing::instrument]
async fn get_artist_album_track(State(state): State<ReamioApp>) -> impl IntoResponse {
    #[derive(Serialize, sqlx::FromRow, Debug)]
    struct RetRow {
        album_name: String,
        artist_name: String,
        track_title: String,

        album_id: i64,
        artist_id: i64,
        track_id: i64,
    }

    // TODO user handling
    let mut db = fetch_users_music_db(state.music_dbs, "powpingdone").await;
    Ok::<_, error::ReamioWebError>(Json(
        sqlx::query_as::<_, RetRow>(
            "SELECT
               album.id AS album_id,
               album.name AS album_name,
               artist.id AS artist_id,
               artist.name AS artist_name,
               track.id AS track_id,
               track.title AS track_title
           FROM album
           JOIN album_tracks ON album.id = album_tracks.album
           JOIN track ON album_tracks.track = track.id
           JOIN artist_tracks ON track.id = artist_tracks.track
           JOIN artist ON artist_tracks.artist = artist.id
           GROUP BY artist.id, album.id, track.id;",
        )
        .fetch_all(&mut *db)
        .await?,
    ))
}

#[tracing::instrument]
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .pretty()
        .init();

    let user_db = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename("./devdir/user.db")
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    sqlx::migrate!("src/migrations/userdb")
        .run(&user_db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT OR IGNORE INTO users (username_lower, username_orig, phc) VALUES ('powpingdone', 'powpingdone', '');",
    )
        .execute(&user_db)
        .await
        .unwrap();

    // testing db
    let ppd_db = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename("./devdir/u/powpingdone/music.db")
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    sqlx::migrate!("src/migrations/per_user")
        .run(&ppd_db)
        .await
        .unwrap();

    // setup state props
    let music_dbs = Arc::new(RwLock::new(HashMap::from([(
        "powpingdone".to_owned(),
        ppd_db,
    )])));
    let w_music_dbs = Arc::downgrade(&music_dbs);

    // fire background tasks
    let (tx_mdata, rx_mdata) = watch::channel(PopulateMetadata);
    let mdata_bg_task = tokio::spawn(process::task_populate_mdata(
        rx_mdata,
        user_db.clone(),
        w_music_dbs.clone(),
    ));

    // run server
    let state = ReamioApp {
        user_db,
        music_dbs: w_music_dbs,
        populate_mdata_waker: tx_mdata,
    };
    let router = Router::new()
        .nest(
            "/api",
            Router::new()
                .route("/tabledump", get(get_artist_album_track))
                .merge(
                    Router::new()
                        .route("/upload", post(upload_track))
                        // TODO: dynamically enable large uploads via an in-server toggle
                        .layer(DefaultBodyLimit::disable()),
                ),
        )
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap(),
        router,
    )
    .await
    .unwrap();

    // cleanup, and drop everything
    drop(music_dbs);
    drop(mdata_bg_task.await);
}
