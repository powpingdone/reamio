use std::{
    collections::HashMap,
    mem,
    sync::{Arc, Weak},
};

use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    response::{Html, IntoResponse},
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
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ReamioApp<'a> {
    pub user_db: SqlitePool,
    pub jinja: Weak<Environment<'a>>,
    pub music_dbs: Weak<RwLock<HashMap<String, SqlitePool>>>,
}

async fn fetch_user_db(
    music_dbs: Weak<RwLock<HashMap<String, SqlitePool>>>,
    user: impl AsRef<str>,
) -> PoolConnection<sqlx::Sqlite> {
    let music_db_hold = music_dbs.upgrade().unwrap();
    let music_db = music_db_hold.read().await;
    let db_pool = music_db.get(user.as_ref()).unwrap();
    db_pool.acquire().await.unwrap()
}

async fn upload_track(State(state): State<ReamioApp<'_>>, mut mp: Multipart) -> impl IntoResponse {
    let mut db = fetch_user_db(state.music_dbs, "powpingdone").await;
    while let Some(mut mp_field) = mp.next_field().await.unwrap() {
        let path = mp_field.file_name();
        // TODO: path injection protection
        while let Some(chunk) = mp_field.next().await {
            let chunk = chunk.unwrap();
            
        } 
        todo!()
    }
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
    Html(
        state
            .jinja
            .upgrade()
            .unwrap()
            .get_template("home.html")
            .unwrap()
            .render(minijinja::context! { artists })
            .unwrap(),
    )
    .into_response()
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

    // testing db
    let ppd_db = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename("./devdir/powpingdone/music.db")
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::migrate!("src/migrations/per_user")
        .run(&ppd_db)
        .await
        .unwrap();

    // setup state
    let jinja = load_templates();
    let music_dbs = Arc::new(RwLock::new(HashMap::from([(
        "powpingdone".to_owned(),
        ppd_db,
    )])));
    let state = ReamioApp {
        user_db,
        jinja: Arc::downgrade(&jinja),
        music_dbs: Arc::downgrade(&music_dbs),
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
}
