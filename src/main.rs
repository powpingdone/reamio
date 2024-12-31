use axum::Router;
use minijinja::Environment;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

#[derive(Clone)]
pub struct ReamioApp<'a> {
    pub user_db: SqlitePool,
    pub jinja: Environment<'a>,
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
    let jinja = Environment::new();
    let state = ReamioApp {
        user_db,
        jinja,
    };
    let router = Router::new().with_state(state);
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap(),
        router,
    )
    .await
    .unwrap();
}
