use std::sync::{Arc, Weak};

use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use minijinja::Environment;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

#[derive(Clone)]
pub struct ReamioApp<'a> {
    pub user_db: SqlitePool,
    pub jinja: Weak<Environment<'a>>,
}

async fn main_page<'a>(State(state): State<ReamioApp<'a>>) -> impl IntoResponse {
    Html(
        state
            .jinja
            .upgrade()
            .unwrap()
            .get_template("base.html")
            .unwrap()
            .render(minijinja::context!())
            .unwrap(),
    )
}

fn load_templates() -> Arc<Environment<'static>> {
    let mut ret = Environment::new();
    ret.add_template("base.html", include_str!("templates/base.html.jinja"))
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
    let jinja = load_templates();
    let state = ReamioApp {
        user_db,
        jinja: Arc::downgrade(&jinja),
    };
    let router = Router::new().route("/", get(main_page)).with_state(state);
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap(),
        router,
    )
    .await
    .unwrap();
}
