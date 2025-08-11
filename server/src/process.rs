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
            let path: String = row.get("orig_path");
            let fid: i64 = row.get("fid");

            // The following span weirdness is due to async fun stuff. Long story short,
            // because of Poll::Pending spans will be created incorrectly per async futures
            // pinning the span, causing it to not drop normally. Instead, instrument the future
            // that comes from each function. The problem is the `continue` following this
            // statement at [[exchk]]. Because we cannot `continue` from inside an async
            // future, manually span the following statements.
            let rowp_span = error_span!("row processing", user, path, fid);
            let _rowp_enter = rowp_span.enter();
            trace!("serialized row");

            // [[exchk]] check for file nonexistence, which is Err or Ok and false
            if !tokio::fs::try_exists(format!("./devdir/temp/{fid}"))
                .in_current_span()
                .await
                .is_ok_and(|x| x)
            {
                // TODO: maint: clean up uploaded_files that have mismatched files
                warn!(fid, "fid does not exist in temp dir");
                continue;
            };

            // add the track, scoped properly
            //
            // TODO: make this spawn as an async task
            {
                let mut music_db = fetch_users_music_db(music_dbs.clone(), &user).await;
                let user_db = &user_db;

                async move {
                    let poss_txn = music_db.begin_with("BEGIN IMMEDIATE").await;
                    match poss_txn {
                        Err(err) => {
                            error!("while getting db transaction connection: {}", err);
                            return;
                        }
                        Ok(txn) => {
                            let ret =
                                task_populate_mdata_userdb_proccessing(txn, path, user, fid).await;
                            if let Err(err) = ret {
                                // TODO: report upload errors to the user
                                error!("while doing upload processing: {:?}", err);
                            }
                        }
                    }

                    // delete upload task after previous txn
                    let ret = sqlx::query("DELETE FROM uploaded_files WHERE fid = $1;")
                        .bind(fid)
                        .execute(user_db)
                        .await;
                    if let Err(err) = ret {
                        error!("when deleting from uploaded_files: {err:?}");
                    }
                }
                .in_current_span()
                .await
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
    debug!(?tags, "tags fetched");

    // step 2: insert track mdata
    //
    // TODO: support multiple Album/Artist bindings
    //
    // TODO: actually support inserting into the same Album/Artist
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
    debug!(album_id, "album processed");
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
    debug!(artist_id, "artist processed");

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
    let mut path_split = path.split('/').skip(1).collect::<Vec<_>>();
    trace!(?path_split, "path split up");
    // this unwrap is fine because of [[ptie]]
    let filename = path_split.pop().unwrap();
    if filename.trim().is_empty() {
        return Err(ReamioPathError {
            msg: format!("filename \"{filename}\" was trimmed into emptyness"),
        }
        .into());
    }
    let filename = filename.trim();
    debug!(?path_split, "final filename generated");

    // step 4: navigate to dir in database
    let parent_dir = {
        let mut dir = None::<i64>;
        for frag in path_split {
            trace!(dir, "cd to {} dir at", frag);
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
                trace!("cd'ing to {pt}");
                dir = Some(pt);
            } else {
                // mkdir
                trace!("dir {frag} does not exist, generating");
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
                debug!("dir {frag} did not exist under {dir:?}, now exists at {pt}");

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
    debug!("track id {track_id} created");

    // step 6: join track with album and artist
    if artist_id.is_some() {
        debug!("binding {track_id} to {artist_id:?}");
        sqlx::query("INSERT INTO artist_tracks (track, artist) VALUES ($1, $2);")
            .bind(track_id)
            .bind(artist_id)
            .execute(&mut *txn)
            .await?;
    }
    if album_id.is_some() {
        debug!("binding {track_id} to {album_id:?}");
        sqlx::query("INSERT INTO album_tracks (track, album) VALUES ($1, $2);")
            .bind(track_id)
            .bind(album_id)
            .execute(&mut *txn)
            .await?;
    }

    // step 7: finally, move file
    //
    // note that track_id and fid is secure because it's just a number
    let from = format!("./devdir/temp/{fid}");
    let to = format!("./devdir/u/{user}/{track_id}");
    trace!("doing user movement {from} -> {to}");
    tokio::fs::rename(from, to).await?;

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
