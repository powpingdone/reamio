use id3::TagLike;

use crate::prelude::*;
use std::{collections::HashMap, path::Path};

// wake on new tracks
#[tracing::instrument(skip(wake))]
pub async fn task_populate_mdata(
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
#[tracing::instrument]
async fn task_populate_mdata_userdb_proccessing(
    mut txn: sqlx::Transaction<'_, sqlx::Sqlite>,
    path: String,
    user: String,
    fid: i64,
) -> Result<(), ReamioProcessingErrorInternal> {
    // step 1: get tags
    let mut tags = extract_tags(fid)?;

    // step 2: insert track mdata
    //
    // TODO: support multiple Album/Artist bindings
    let album_id = match tags.remove("album") {
        Some(album) => Some(
            sqlx::query("INSERT INTO album (name) VALUES ($1) RETURNING id;")
                .bind(String::from_utf8(album).unwrap())
                .fetch_one(&mut *txn)
                .await?
                .get::<i64, _>("id"),
        ),
        None => None,
    };
    let artist_id = match tags.remove("artist") {
        Some(artist) => Some(
            sqlx::query("INSERT INTO artist (name) VALUES ($1) RETURNING id;")
                .bind(String::from_utf8(artist).unwrap())
                .fetch_one(&mut *txn)
                .await?
                .get::<i64, _>("id"),
        ),
        None => None,
    };

    // step 3: process requested path
    if !path.chars().next().is_some_and(|x| x == '/') {
        return Err(ReamioPathError {
            msg: "the path is not absolute".to_owned(),
        }
        .into());
    }
    // [[ptie]]
    if path.trim().is_empty() {
        return Err(ReamioPathError {
            msg: "path contains nothing, not even a filename".to_owned(),
        }
        .into());
    }
    let mut path_split = path.split('/').collect::<Vec<_>>();
    // this unwrap is fine because of [[ptie]]
    let filename = path_split.pop().unwrap();
    if filename.trim().is_empty() {
        return Err(ReamioPathError {
            msg: format!("filename \"{filename}\" was trimmed into emptyness"),
        }
        .into());
    }
    let filename = filename.trim();

    // step 4: navigate to dir in database
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
                "SELECT dir.node
                     FROM dir_tree JOIN dir ON dir.node = dir_tree.node
                     WHERE dir_tree.parent IS $1
                           AND dir.name IS $2;",
            )
            .bind(dir)
            .bind(frag)
            .fetch_optional(&mut *txn)
            .await?
            .and_then(|x| x.try_get::<i64, _>("node").ok());

            // mkdir or cd to that dir
            if let Some(pt) = ls {
                // cd
                dir = Some(pt);
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
    // step 5: insert track with dir
    let track_name = match tags.remove("track") {
        Some(x) => String::from_utf8(x).unwrap(),
        None => filename.to_owned(),
    };
    // CHANGING THIS RETURN TYPE HAS CONSEQUENCES
    let track_id =
        sqlx::query("INSERT INTO track (title, dir, fname) VALUES ($1, $2, $3) RETURNING id;")
            .bind(track_name)
            .bind(parent_dir)
            .bind(filename)
            .fetch_one(&mut *txn)
            .await?
            .get::<i64, _>("id");

    // step 6: join track with album and artist
    if artist_id.is_some() {
        sqlx::query("INSERT INTO artist_tracks (track, artist) VALUES ($1, $2);")
            .bind(track_id)
            .bind(artist_id)
            .execute(&mut *txn)
            .await?;
    }
    if album_id.is_some() {
        sqlx::query("INSERT INTO album_tracks (track, album) VALUES ($1, $2);")
            .bind(track_id)
            .bind(album_id)
            .execute(&mut *txn)
            .await?;
    }

    // step 7: finally, move file
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

#[tracing::instrument]
fn extract_tags(fid: i64) -> Result<HashMap<String, Vec<u8>>, ReamioProcessingErrorInternal> {
    let path = format!("./devdir/temp/{fid}");
    let path = Path::new(&path);

    let readers: Vec<Box<dyn TagReader>> =
        vec![Box::new(ID3TagReader), Box::new(MetaFlacTagReader)];
    for reader in readers {
        match reader.is_candidate(path)? {
            Some(x) if x => return Ok(reader.tags_parse(path)?),
            Some(_) => continue,
            None => {
                // unsupported is_candidate
                if let Ok(map) = reader.tags_parse(path) {
                    return Ok(map);
                }
            }
        }
    }

    return Ok(HashMap::new());
}

trait TagReader {
    fn is_candidate(&self, path: &Path) -> Result<Option<bool>, ReamioProcessingErrorInternal>;

    fn tags_parse(
        &self,
        path: &Path,
    ) -> Result<HashMap<String, Vec<u8>>, ReamioProcessingErrorInternal>;
}

/// ID3TagReader reads the tags from "MPEG" files (along with mp3, wav, aiff).
#[derive(Debug)]
struct ID3TagReader;

impl TagReader for ID3TagReader {
    #[tracing::instrument]
    fn is_candidate(&self, path: &Path) -> Result<Option<bool>, ReamioProcessingErrorInternal> {
        let file = std::fs::File::open(path)?;
        id3::Tag::is_candidate(file)
            .map(|x| Some(x))
            .map_err(ReamioProcessingErrorInternal::from)
    }

    #[tracing::instrument]
    fn tags_parse(
        &self,
        path: &Path,
    ) -> Result<HashMap<String, Vec<u8>>, ReamioProcessingErrorInternal> {
        let tag = id3::Tag::read_from_path(path)?;
        let mut hmap = HashMap::new();

        if let Some(x) = tag.title() {
            hmap.insert("title".to_owned(), x.bytes().collect());
        }
        if let Some(x) = tag.artist() {
            hmap.insert("artist".to_owned(), x.bytes().collect());
        }
        if let Some(x) = tag.album() {
            hmap.insert("album".to_owned(), x.bytes().collect());
        }
        Ok(hmap)
    }
}

/// MetaFlacTagReader reads tags from vorbis containers (ogg, flac)
#[derive(Debug)]
struct MetaFlacTagReader;

impl TagReader for MetaFlacTagReader {
    #[tracing::instrument]
    fn is_candidate(&self, path: &Path) -> Result<Option<bool>, ReamioProcessingErrorInternal> {
        let mut file = std::fs::File::open(path)?;
        Ok(Some(metaflac::Tag::is_candidate(&mut file)))
    }

    #[tracing::instrument]
    fn tags_parse(
        &self,
        path: &Path,
    ) -> Result<HashMap<String, Vec<u8>>, ReamioProcessingErrorInternal> {
        let tag = metaflac::Tag::read_from_path(path)?;
        let mut hmap = HashMap::new();
        for block in tag.get_blocks(metaflac::BlockType::VorbisComment) {
            let metaflac::Block::VorbisComment(vc) = block else {
                // wtf happened here
                continue;
            };

            if let Some(x) = vc.title()
                && let Some(x) = x.get(0)
            {
                hmap.insert("title".to_owned(), x.bytes().collect());
            }
            if let Some(x) = vc.artist()
                && let Some(x) = x.get(0)
            {
                hmap.insert("artist".to_owned(), x.bytes().collect());
            }
            if let Some(x) = vc.album()
                && let Some(x) = x.get(0)
            {
                hmap.insert("album".to_owned(), x.bytes().collect());
            }
        }
        Ok(hmap)
    }
}
