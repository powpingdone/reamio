use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use minijinja::Environment;
use serde::Serialize;
use sqlx::{pool::PoolConnection, prelude::*};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::{
    collections::HashMap,
    mem,
    sync::{Arc, Weak},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{watch, RwLock},
};

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
async fn task_populate_mdata(
    mut wake: WakeRx<PopulateMetadata>,
    user_db: SqlitePool,
    music_dbs: MusicDbMapRef,
) {
    // this breaks on Err from changed().await when WakeTx has been fully dropped
    while let Ok(()) = wake.changed().await {
        let uploaded_items = sqlx::query("SELECT fid, user, orig_path FROM uploaded_files;")
            .fetch_all(&user_db)
            .await
            .unwrap();
        for row in uploaded_items.into_iter() {
            // serialize
            let user: String = row.get("user");
            let path = row.get::<String, _>("orig_path");
            let fid: i64 = row.get("fid");

            // check for file existence
            if !tokio::fs::try_exists(format!("./devdir/temp/{fid}"))
                .await
                .is_ok_and(
                    |x| x,
                    // is true
                )
            {
                // TODO maint: clean up uploaded_files that have mismatched files
                continue;
            };

            // spawn a task to add the thing
            let mut music_db = fetch_user_db(music_dbs.clone(), &user).await;
            drop(tokio::spawn({
                let user_db = user_db.clone();
                async move {
                    println!("{:?}", music_db.transaction::<_, (), sqlx::Error>(|txn| {
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
                                panic!("item must have fname")
                            };
                            let path_split = path_split;
                            let parent_dir = {
                                let mut dir = None::<i64>;
                                for frag in path_split {
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
                            tokio::fs::rename(format!("./devdir/temp/{fid}"), format!("./devdir/u/{user}/{track_id}")).await.unwrap();
                            Ok(())
                        })
                    }).await);

                    // delete upload task after previous txn
                    sqlx::query("DELETE FROM uploaded_files WHERE fid = $1;")
                        .bind(fid)
                        .execute(&user_db)
                        .await
                        .unwrap();
                }
            }));
        }
    }
}

async fn fetch_user_db(
    music_dbs: MusicDbMapRef,
    user: impl AsRef<str>,
) -> PoolConnection<sqlx::Sqlite> {
    // TODO: create pools on demand
    let music_db_hold = music_dbs.upgrade().unwrap();
    let music_db = music_db_hold.read().await;
    let db_pool = music_db.get(user.as_ref()).unwrap();
    db_pool.acquire().await.unwrap()
}

async fn upload_track(State(state): State<ReamioApp<'_>>, mut mp: Multipart) -> impl IntoResponse {
    // ingest paths
    state.user_db.acquire().await.unwrap().transaction::<_, _, sqlx::Error>(|txn| {
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

async fn main_page(State(state): State<ReamioApp<'_>>) -> impl IntoResponse {
    let mut db = fetch_user_db(state.music_dbs, "powpingdone").await;

    // structs for jinja
    #[derive(Serialize)]
    struct Track {
        pub title: String,
    }

    #[derive(Serialize)]
    struct Album {
        pub title: String,
        pub tracks: Vec<Track>,
    }

    #[derive(Serialize)]
    struct Artist {
        pub title: String,
        pub albums: Vec<Album>,
    }

    // mega query
    let mut rows = sqlx::query(
        r#"
    SELECT
      artist.name AS artist,
      track.title AS track,
      album.name AS album,
      artist.id AS ar_id,
      album.id AS al_id
    FROM artist
    JOIN artist_tracks ON artist_tracks.artist = artist.id
    JOIN track ON artist_tracks.track = track.id
    JOIN album_tracks ON track.id = album_tracks.track
    JOIN album ON album_tracks.album = album.id
    ORDER BY artist, album;"#,
    )
    .fetch(&mut *db);

    // init structs
    let mut artists: Vec<Artist> = vec![];
    let mut ar_id = -1_i64;
    let mut al_id = -1_i64;
    let mut albums = vec![];
    let mut tracks = vec![];
    let mut ar_t = String::new();
    let mut al_t = String::new();

    // extractor
    while let Some(row) = rows.next().await {
        let row = row.unwrap();
        if row.get::<i64, _>("ar_id") != ar_id {
            if ar_id != -1 {
                // artist was actually completed, add to list
                albums.push(Album {
                    title: mem::take(&mut al_t),
                    tracks: mem::take(&mut tracks),
                });
                artists.push(Artist {
                    title: mem::take(&mut ar_t),
                    albums: mem::take(&mut albums),
                });
            }

            // init (new) states
            ar_t = row.get("artist");
            ar_id = row.get("ar_id");
            al_t = row.get("album");
            al_id = row.get("al_id");
        } else if row.get::<i64, _>("al_id") != al_id {
            // add completed album
            albums.push(Album {
                title: mem::take(&mut al_t),
                tracks: mem::take(&mut tracks),
            });

            // new album
            al_t = row.get("album");
            al_id = row.get("al_id");
        }
        tracks.push(Track {
            title: row.get("track"),
        });
    }
    if ar_id != -1 {
        // construct final artist
        artists.push(Artist {
            title: ar_t,
            albums: {
                albums.push(Album {
                    title: al_t,
                    tracks,
                });
                albums
            },
        });
    }

    // render
    return Html(
        state
            .jinja
            .upgrade()
            .unwrap()
            .get_template("home.html")
            .unwrap()
            .render(minijinja::context! {
                artists
            })
            .unwrap(),
    )
    .into_response();
}

fn load_templates() -> Arc<Environment<'static>> {
    let mut ret = Environment::new();
    ret.add_template("base.html", include_str!("templates/base.html.jinja"))
        .unwrap();
    ret.add_template("home.html", include_str!("templates/home.html.jinja"))
        .unwrap();
    Arc::new(ret)
}

#[tokio::main]
async fn main() {
    let user_db = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename("./devdir/user.db")
                .create_if_missing(true),
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
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::migrate!("src/migrations/per_user")
        .run(&ppd_db)
        .await
        .unwrap();

    // setup state props
    let jinja = load_templates();
    let w_jinja = Arc::downgrade(&jinja);
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
        jinja: w_jinja,
        music_dbs: w_music_dbs,
        populate_mdata_waker: tx_mdata,
    };
    let router = Router::new()
        .route("/", get(main_page))
        .merge(
            Router::new()
                .route("/upload", post(upload_track))
                // TODO: dynamically enable large uploads via an in-server toggle
                .layer(DefaultBodyLimit::disable()),
        )
        .with_state(state);
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap(),
        router,
    )
    .await
    .unwrap();

    // cleanup, and drop everything
    drop(jinja);
    drop(music_dbs);
    drop(mdata_bg_task.await);
}
