use std::{
    collections::HashMap,
    mem,
    sync::{Arc, RwLock, Weak},
};

use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures::StreamExt;
use minijinja::Environment;
use serde::Serialize;
use sqlx::prelude::*;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

#[derive(Clone)]
pub struct ReamioApp<'a> {
    pub user_db: SqlitePool,
    pub jinja: Weak<Environment<'a>>,
    pub music_dbs: Weak<RwLock<HashMap<String, SqlitePool>>>,
}

async fn main_page<'a>(State(state): State<ReamioApp<'a>>) -> impl IntoResponse {
    let music_db_hold = state.music_dbs.upgrade().unwrap();
    let music_db = music_db_hold.read().unwrap();
    let db_pool = music_db.get("powpingdone").unwrap();
    let mut db = db_pool.acquire().await.unwrap();

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
    JOIN album_track ON track.id = album_track.track
    JOIN album ON album_track.album = album.id
    GROUP BY ar_id, al_id
    ORDER BY artist, album;"#,
    )
    .fetch(&mut *db);

    // init structs
    let mut artists: Vec<Artist> = vec![];
    let mut ar_id = -1_i64;
    let mut al_id: i64;
    let mut albums = vec![];
    let mut tracks = vec![];
    let mut ar_t: String;
    let mut al_t: String;

    // extractor
    while let Some(row) = rows.next().await {
        let row = row.unwrap();
        if row.get::<i64, _>("ar_id") != ar_id {
            if ar_id == -1 {
                // init states
                ar_t = row.get("artist");
                ar_id = row.get("ar_id");
                al_t = row.get("album");
                al_id = row.get("al_id");
            } else {
                // artist completed
            }
        }
    }
    if ar_id == -1 {
        // no rows found, die
        return "".into_response();
    }

    // construct final artist
    artists.push(Artist {
        title: ar_t,
        albums: {
            let mut album = mem::take(&mut albums);
            album.push(Album {
                title: al_t,
                tracks,
            });
            album
        },
    });

    Html(
        state
            .jinja
            .upgrade()
            .unwrap()
            .get_template("home.html")
            .unwrap()
            .render(minijinja::context!())
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
    let jinja = load_templates();
    let music_dbs = Arc::new(RwLock::new(HashMap::new()));
    let state = ReamioApp {
        user_db,
        jinja: Arc::downgrade(&jinja),
        music_dbs: Arc::downgrade(&music_dbs),
    };
    let router = Router::new().route("/", get(main_page)).with_state(state);
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap(),
        router,
    )
    .await
    .unwrap();
}
