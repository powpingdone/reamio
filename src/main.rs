use axum::{
    extract::{
        DefaultBodyLimit,
        Multipart,
        State,
    },
    response::{
        Html,
        IntoResponse,
        Redirect,
    },
    routing::{
        get,
        post,
    },
    Router,
};
use futures::StreamExt;
use minijinja::Environment;
use sqlx::{
    pool::PoolConnection,
    prelude::*,
};
use sqlx::{
    sqlite::{
        SqliteConnectOptions,
        SqlitePoolOptions,
    },
    SqlitePool,
};
use std::{
    collections::HashMap,
    mem,
    sync::{
        Arc,
        Weak,
    },
};
use tokio::{
    io::AsyncWriteExt,
    sync::{
        watch,
        RwLock,
    },
};

mod error;
mod pages;

pub use crate::error::*;
pub use crate::pages::*;

type JinjaRef<'a> = Weak<Environment<'a>>;
type MusicDbMapRef = Weak<RwLock<HashMap<String, SqlitePool>>>;
type WakeTx<T> = watch::Sender<T>;
type WakeRx<T> = watch::Receiver<T>;

#[derive(Clone)]
pub struct ReamioApp<'a> {
    pub user_db: SqlitePool,
    pub jinja: JinjaRef<'a>,
    pub music_dbs: MusicDbMapRef,
    pub populate_mdata_waker: WakeTx<PopulateMetadata>,
}

// zero sized type for wakeup
pub struct PopulateMetadata;

// wake on new tracks
async fn task_populate_mdata(mut wake: WakeRx<PopulateMetadata>, user_db: SqlitePool, music_dbs: MusicDbMapRef) {
    // TODO: on task_populate_mdata or other subtask panics, what should we do? is
    // this an architecture issue?
    //
    // this breaks on Err from changed().await when WakeTx has been fully dropped
    while let Ok(()) = wake.changed().await {
        // this realistically _really_ shouldn't fail
        let uploaded_items =
            sqlx::query("SELECT fid, user, orig_path FROM uploaded_files;").fetch_all(&user_db).await.unwrap();
        for row in uploaded_items.into_iter() {
            // serialize
            let user: String = row.get("user");
            let path = row.get::<String, _>("orig_path");
            let fid: i64 = row.get("fid");

            // check for file existence, which is Result::Ok and boolean true
            if !tokio::fs::try_exists(format!("./devdir/temp/{fid}")).await.is_ok_and(|x| x) {
                // TODO maint: clean up uploaded_files that have mismatched files
                continue;
            };

            // spawn a task to add the track 
            let mut music_db = fetch_user_db(music_dbs.clone(), &user).await;
            drop(tokio::spawn({
                let user_db = user_db.clone();
                async move {
                    let ret = music_db.transaction::<_, (), ReamioProcessingErrorInternal>(|txn| {
                        // TODO: actual tagging
                        //
                        // TODO: error handling
                        Box::pin(async move {
                            // insert track mdata
                            //
                            // TODO: ordering is important because this isnt BEGIN IMMEDIATE
                            let album = rand::random::<u64>().to_string();
                            let album_id =
                                sqlx::query("INSERT INTO album (name) VALUES ($1) RETURNING id;")
                                    .bind(album)
                                    .fetch_one(&mut **txn)
                                    .await?
                                    .get::<i64, _>("id");
                            let artist = rand::random::<u64>().to_string();
                            let artist_id =
                                sqlx::query("INSERT INTO artist (name) VALUES ($1) RETURNING id;")
                                    .bind(artist)
                                    .fetch_one(&mut **txn)
                                    .await?
                                    .get::<i64, _>("id");

                            // insert dirs
                            let mut path_split = path.split('/').collect::<Vec<_>>();
                            let Some(fname) = path_split.pop() else {
                                return Err(
                                    ReamioPathError {
                                        msg: "path contains nothing, not even a filename".to_owned(),
                                    }.into(),
                                )
                            };
                            if fname.trim().is_empty() {
                                return Err(
                                    ReamioPathError {
                                        msg: format!("filename \"{fname}\" was trimmed into emptyness"),
                                    }.into(),
                                )
                            }
                            let fname = fname.trim();
                            let path_split = path_split;
                            let parent_dir = {
                                let mut dir = None::<i64>;
                                for frag in path_split {
                                    if frag.trim().is_empty() {
                                        return Err(
                                            ReamioPathError {
                                                msg: format!("folder \"{frag}\" was trimmed into emptyness"),
                                            }.into(),
                                        )
                                    }
                                    let frag = frag.trim();

                                    // list current directory
                                    let ls =
                                        sqlx::query(
                                            r#"
                                            SELECT dir.node, dir.name 
                                            FROM dir_tree JOIN dir ON dir.node = dir_tree.node
                                            WHERE dir_tree.parent IS $1;
                                            "#,
                                        )
                                            .bind(dir)
                                            .fetch_all(&mut **txn)
                                            .await?
                                            .iter()
                                            .map(|x| (x.get("name"), x.get("node")))
                                            .collect::<HashMap<String, i64>>();
                                    if let Some(pt) = ls.get(frag) {
                                        // cd
                                        dir = Some(*pt);
                                    } else {
                                        // mkdir
                                        let pt =
                                            sqlx::query("INSERT INTO dir (name) VALUES ($1) RETURNING node;")
                                                .bind(frag)
                                                .fetch_one(&mut **txn)
                                                .await?
                                                .get::<i64, _>("node");
                                        sqlx::query("INSERT INTO dir_tree (node, parent) VALUES ($1, $2);")
                                            .bind(pt)
                                            .bind(dir)
                                            .execute(&mut **txn)
                                            .await?;

                                        // cd
                                        dir = Some(pt);
                                    }
                                }
                                dir
                            };

                            // insert track
                            let track_name = rand::random::<u64>().to_string();

                            // CHANGING THIS RETURN TYPE HAS CONSEQUENCES
                            let track_id =
                                sqlx::query("INSERT INTO track (title, dir, fname) VALUES ($1, $2, $3) RETURNING id;")
                                    .bind(track_name)
                                    .bind(parent_dir)
                                    .bind(fname)
                                    .fetch_one(&mut **txn)
                                    .await?
                                    .get::<i64, _>("id");

                            // join
                            sqlx::query("INSERT INTO artist_tracks (track, artist) VALUES ($1, $2);")
                                .bind(track_id)
                                .bind(artist_id)
                                .execute(&mut **txn)
                                .await?;
                            sqlx::query("INSERT INTO album_tracks (track, album) VALUES ($1, $2);")
                                .bind(track_id)
                                .bind(album_id)
                                .execute(&mut **txn)
                                .await?;

                            // finally, move file
                            //
                            // note that track_id and fid is secure because it's just a number
                            tokio::fs::rename(format!("./devdir/temp/{fid}"), format!("./devdir/u/{user}/{track_id}")).await?;
                            Ok(())
                        })
                    }).await;
                    if let Err(err) = ret {
                        // TODO: report upload errors to the user
                        println!("while doing upload processing: {err:?}");
                    }

                    // delete upload task after previous txn
                    let ret = sqlx::query("DELETE FROM uploaded_files WHERE fid = $1;").bind(fid).execute(&user_db).await;
                    if let Err(err) = ret {
                        println!("when deleting from uploaded_files: {err:?}");
                    }
                }
            }));
        }
    }
}

async fn fetch_user_db(music_dbs: MusicDbMapRef, user: impl AsRef<str>) -> PoolConnection<sqlx::Sqlite> {
    // TODO: create pools on demand
    let music_db_hold = music_dbs.upgrade().unwrap();
    let music_db = music_db_hold.read().await;
    let db_pool = music_db.get(user.as_ref()).unwrap();
    db_pool.acquire().await.unwrap()
}

async fn upload_track(State(state): State<ReamioApp<'_>>, mut mp: Multipart) -> impl IntoResponse {
    // ingest paths
    state.user_db.acquire().await.unwrap().transaction::<_, _, ReamioWebError>(|txn| {
        Box::pin(async move {
            while let Some(mut mp_field) = mp.next_field().await.unwrap() {
                let Some(path) = mp_field.file_name() else {
                    continue;
                };

                // CHANGING THE RETURN TYPE HAS SECURITY IMPLICATIONS
                let fid: i64 =
                    sqlx::query(
                        "INSERT INTO uploaded_files (orig_path, user, fid) VALUES ($1, $2, NULL) RETURNING fid;",
                    )
                        .bind(path)
                        // TODO: dynamic users
                        .bind("powpingdone")
                        .fetch_one(&mut **txn)
                        .await?
                        .get("fid");

                // write out file
                let mut file = tokio::fs::OpenOptions::new().create(true).write(true)
                    // this is secure because fid is i64, and cannot represent anything other than
                    // [0-9]
                    .open(format!("./devdir/temp/{fid}")).await.unwrap();
                while let Some(chunk) = mp_field.next().await {
                    let chunk = chunk.unwrap();
                    file.write_all(&chunk).await.unwrap();
                }
                file.sync_data().await.unwrap();
                drop(file);
            }

            // wake the mdata
            state.populate_mdata_waker.send(PopulateMetadata).unwrap();
            Ok(())
        })
    }).await.unwrap();

    // TODO maintenace task: on transaction error, the files are left behind. clean
    // them up.
    return Redirect::to("/");
}

#[tokio::main]
async fn main() {
    let user_db =
        SqlitePoolOptions::new()
            .connect_with(SqliteConnectOptions::new().filename("./devdir/user.db").create_if_missing(true))
            .await
            .unwrap();
    sqlx::migrate!("src/migrations/userdb").run(&user_db).await.unwrap();
    sqlx::query(
        "INSERT OR IGNORE INTO users (username_lower, username_orig, phc) VALUES ('powpingdone', 'powpingdone', '');",
    )
        .execute(&user_db)
        .await
        .unwrap();

    // testing db
    let ppd_db =
        SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new().filename("./devdir/u/powpingdone/music.db").create_if_missing(true),
            )
            .await
            .unwrap();
    sqlx::migrate!("src/migrations/per_user").run(&ppd_db).await.unwrap();

    // setup state props
    let jinja = load_templates();
    let w_jinja = Arc::downgrade(&jinja);
    let music_dbs = Arc::new(RwLock::new(HashMap::from([("powpingdone".to_owned(), ppd_db)])));
    let w_music_dbs = Arc::downgrade(&music_dbs);

    // fire tasks
    let (tx_mdata, rx_mdata) = watch::channel(PopulateMetadata);
    let mdata_bg_task = tokio::spawn(task_populate_mdata(rx_mdata, user_db.clone(), w_music_dbs.clone()));

    // run server
    let state = ReamioApp {
        user_db,
        jinja: w_jinja,
        music_dbs: w_music_dbs,
        populate_mdata_waker: tx_mdata,
    };
    let router = Router::new().route("/", get(main_page)).merge(Router::new().route("/upload", post(upload_track))
        // TODO: dynamically enable large uploads via an in-server toggle
        .layer(DefaultBodyLimit::disable())).with_state(state);
    axum::serve(tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap(), router).await.unwrap();

    // cleanup, and drop everything
    drop(jinja);
    drop(music_dbs);
    drop(mdata_bg_task.await);
}
