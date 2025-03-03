use axum::extract::State;
use serde::Serialize;
use crate::*;

pub fn load_templates() -> Arc<Environment<'static>> {
    let mut ret = Environment::new();
    ret.add_template("base.html", include_str!("templates/base.html.jinja")).unwrap();
    ret.add_template("home.html", include_str!("templates/home.html.jinja")).unwrap();
    Arc::new(ret)
}

pub async fn main_page(State(state): State<ReamioApp<'_>>) -> impl IntoResponse {
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
    let mut rows =
        sqlx::query(
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
        ).fetch(&mut *db);

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
        tracks.push(Track { title: row.get("track") });
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
    return Html(state.jinja.upgrade().unwrap().get_template("home.html").unwrap().render(minijinja::context!{
        artists
    }).unwrap()).into_response();
}

