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
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use sqlx::{pool::PoolConnection, prelude::*};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{RwLock, watch},
};

mod error;

pub use crate::error::*;

type MusicDbMapRef = Weak<RwLock<HashMap<String, SqlitePool>>>;
type WakeTx<T> = watch::Sender<T>;
type WakeRx<T> = watch::Receiver<T>;

#[derive(Clone)]
pub struct ReamioApp {
    pub user_db: SqlitePool,
    pub music_dbs: MusicDbMapRef,
    pub populate_mdata_waker: WakeTx<PopulateMetadata>,
}

// zero sized type for wakeup
pub struct PopulateMetadata;

// wake on new tracks
async fn task_populate_mdata(
    mut wake: WakeRx<PopulateMetadata>,
    user_db: SqlitePool,
    music_dbs: MusicDbMapRef,
) {
    // TODO: on task_populate_mdata or other subtask panics, what should we do? is
    // this an architecture issue?
    //
    // this breaks on Err from changed().await when WakeTx has been fully dropped
    while let Ok(()) = wake.changed().await {
        // this realistically _really_ shouldn't fail
        let uploaded_items = sqlx::query("SELECT fid, user, orig_path FROM uploaded_files;")
            .fetch_all(&user_db)
            .await
            .unwrap();
        for row in uploaded_items.into_iter() {
            // serialize
            let user: String = row.get("user");
            let path = row.get::<String, _>("orig_path");
            let fid: i64 = row.get("fid");

            // check for file existence, which is Result::Ok and boolean true
            if !tokio::fs::try_exists(format!("./devdir/temp/{fid}"))
                .await
                .is_ok_and(|x| x)
            {
                // TODO: maint: clean up uploaded_files that have mismatched files
                continue;
            };

            // spawn a task to add the track
            //
            // TODO: make this spawn as an async task
            let mut music_db = fetch_users_music_db(music_dbs.clone(), &user).await;
            let poss_txn = music_db.begin_with("IMMEDIATE").await;
            match poss_txn {
                Err(err) => {
                    println!("while getting db transaction connection: {err:?}");
                    return;
                }
                Ok(txn) => {
                    let ret = task_populate_mdata_userdb_proccessing(txn, path, user, fid).await;
                    if let Err(err) = ret {
                        // TODO: report upload errors to the user
                        println!("while doing upload processing: {err:?}");
                    }
                }
            }

            // delete upload task after previous txn
            let ret = sqlx::query("DELETE FROM uploaded_files WHERE fid = $1;")
                .bind(fid)
                .execute(&user_db)
                .await;
            if let Err(err) = ret {
                println!("when deleting from uploaded_files: {err:?}");
            }
        }
    }
}

// subtask function as part of the above function of the same prefix.
// processes tags and inserts them into the music db, after moving the file to the u/ dir
async fn task_populate_mdata_userdb_proccessing(
    mut txn: sqlx::Transaction<'_, sqlx::Sqlite>,
    path: String,
    user: String,
    fid: i64,
) -> Result<(), ReamioProcessingErrorInternal> {
    // TODO: actual tagging
    //
    // step 1: insert track mdata
    let album = rand::random::<u64>().to_string();
    let album_id = sqlx::query("INSERT INTO album (name) VALUES ($1) RETURNING id;")
        .bind(album)
        .fetch_one(&mut *txn)
        .await?
        .get::<i64, _>("id");
    let artist = rand::random::<u64>().to_string();
    let artist_id = sqlx::query("INSERT INTO artist (name) VALUES ($1) RETURNING id;")
        .bind(artist)
        .fetch_one(&mut *txn)
        .await?
        .get::<i64, _>("id");

    // step 2: process requested path
    let mut path_split = path.split('/').collect::<Vec<_>>();
    let Some(filename) = path_split.pop() else {
        return Err(ReamioPathError {
            msg: "path contains nothing, not even a filename".to_owned(),
        }
        .into());
    };
    if filename.trim().is_empty() {
        return Err(ReamioPathError {
            msg: format!("filename \"{filename}\" was trimmed into emptyness"),
        }
        .into());
    }
    let filename = filename.trim();

    // step 3: navigate to dir in database
    let parent_dir = {
        let mut dir = None::<i64>;
        for frag in path_split {
            if frag.trim().is_empty() {
                return Err(ReamioPathError {
                    msg: format!("folder \"{frag}\" was trimmed into emptyness"),
                }
                .into());
            }
            let frag = frag.trim();

            // list current directory
            let ls = sqlx::query(
                "SELECT dir.node, dir.name
                     FROM dir_tree JOIN dir ON dir.node = dir_tree.node
                     WHERE dir_tree.parent IS $1;",
            )
            .bind(dir)
            .fetch_all(&mut *txn)
            .await?
            .iter()
            .map(|x| (x.get("name"), x.get("node")))
            .collect::<HashMap<String, i64>>();

            // mkdir or cd to that dir
            if let Some(pt) = ls.get(frag) {
                // cd
                dir = Some(*pt);
            } else {
                // mkdir
                let pt = sqlx::query("INSERT INTO dir (name) VALUES ($1) RETURNING node;")
                    .bind(frag)
                    .fetch_one(&mut *txn)
                    .await?
                    .get::<i64, _>("node");
                sqlx::query("INSERT INTO dir_tree (node, parent) VALUES ($1, $2);")
                    .bind(pt)
                    .bind(dir)
                    .execute(&mut *txn)
                    .await?;

                // cd
                dir = Some(pt);
            }
        }
        dir
    };

    // TODO: tagging
    //
    // step 4: insert track with dir
    let track_name = rand::random::<u64>().to_string();
    // CHANGING THIS RETURN TYPE HAS CONSEQUENCES
    let track_id =
        sqlx::query("INSERT INTO track (title, dir, fname) VALUES ($1, $2, $3) RETURNING id;")
            .bind(track_name)
            .bind(parent_dir)
            .bind(filename)
            .fetch_one(&mut *txn)
            .await?
            .get::<i64, _>("id");

    // step 5: join track with album and artist
    sqlx::query("INSERT INTO artist_tracks (track, artist) VALUES ($1, $2);")
        .bind(track_id)
        .bind(artist_id)
        .execute(&mut *txn)
        .await?;
    sqlx::query("INSERT INTO album_tracks (track, album) VALUES ($1, $2);")
        .bind(track_id)
        .bind(album_id)
        .execute(&mut *txn)
        .await?;

    // step 6: finally, move file
    //
    // note that track_id and fid is secure because it's just a number
    tokio::fs::rename(
        format!("./devdir/temp/{fid}"),
        format!("./devdir/u/{user}/{track_id}"),
    )
    .await?;

    txn.commit().await?;
    Ok(())
}

async fn fetch_users_music_db(
    music_dbs: MusicDbMapRef,
    user: impl AsRef<str>,
) -> PoolConnection<sqlx::Sqlite> {
    // TODO: create pools on demand, user management
    let music_db_hold = music_dbs.upgrade().unwrap();
    let music_db = music_db_hold.read().await;
    let db_pool = music_db.get(user.as_ref()).unwrap();
    db_pool.acquire().await.unwrap()
}

#[derive(Deserialize)]
struct UploadArgs {
    pub path: String, // expected param: a set of strings seperated by '/'
}

#[derive(Serialize)]
struct UploadReturn {
    written: usize,
}

/// Ingest track. This does not process any tracks, only writes them to disk
/// and notifies the actual processor that there are tracks to process.
async fn upload_track(
    State(state): State<ReamioApp>,
    Query(UploadArgs { path }): Query<UploadArgs>,
    body: Body,
) -> Result<Json<UploadReturn>, ReamioWebError> {
    let mut txn = state.user_db.begin_with("IMMEDIATE").await?;

    // CHANGING THE RETURN TYPE HAS SECURITY IMPLICATIONS
    //
    // now, why is it i64 and not something more generic, like a Uuid? long story short:
    // Uuid is not natively supported by sqlite and I needed a quick id impl so
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

    // write out file
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        // this is secure because fid is i64 and cannot represent anything other than [0-9]*
        .open(format!("./devdir/temp/{fid}"))
        .await
        .unwrap();
    let mut body = body.into_data_stream();
    let mut size_acc = 0;
    while let Some(mut chunk) = body.try_next().await? {
        while chunk.has_remaining() {
            size_acc += file.write_buf(&mut chunk).await.unwrap();
        }
    }
    file.sync_data().await.unwrap();
    drop(file);

    // op is good
    txn.commit().await?;

    // wake the mdata
    //
    // TODO maintenace task: on transaction error, the files are left behind. clean
    // them up.
    state.populate_mdata_waker.send(PopulateMetadata).unwrap();
    Ok(Json(UploadReturn { written: size_acc }))
}

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

#[tokio::main]
async fn main() {
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

    // fire tasks
    let (tx_mdata, rx_mdata) = watch::channel(PopulateMetadata);
    let mdata_bg_task = tokio::spawn(task_populate_mdata(
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
        // api stuffs
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
