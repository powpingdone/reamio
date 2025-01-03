use std::{
    collections::HashMap,
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
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use sqlx::prelude::*;

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
        pub id: i64,
        pub title: String,
    }
    #[derive(Serialize)]
    struct Album {
        pub id: i64,
        pub title: String,
        pub tracks: Vec<Track>,
    }
    #[derive(Serialize)]
    struct Artist {
        pub id: i64,
        pub title: String,
        pub albums: Vec<Album>,
    }
    
    let mut albums = vec![];
    let mut arows = sqlx::query("SELECT * FROM album;").fetch(  &mut *db);

    while let Some(row) = arows.next().await {
        let row = row.unwrap();
        let mut album = Album {
            id: row.get("id"),
            title: row.get("name"),
            tracks: vec![],
        };
        // TODO: create subqueries for each album and track
        
        albums.push(album);
    }

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
