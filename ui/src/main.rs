use std::{
    collections::HashSet,
    fs,
    mem::take,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Deserialize;
use slint::{ModelRc, VecModel};

slint::include_modules!();

static WEAK_MAINWINDOW: OnceLock<slint::Weak<MainWindow>> = OnceLock::new();

fn tabledump() {
    #[derive(Deserialize)]
    struct RetRow {
        album_name: String,
        artist_name: String,
        track_title: String,

        album_id: i32,
        artist_id: i32,
        track_id: i32,
    }

    // intermediatary structs before being transfered over to the "rel" structs
    struct ArtistIM {
        id: i32,
        title: String,
        assoc_album: Vec<AlbumIM>,
    }

    struct AlbumIM {
        id: i32,
        title: String,
        assoc_track: Vec<TrackIM>,
    }

    struct TrackIM {
        id: i32,
        title: String,
    }

    impl ArtistIM {
        fn to_rel(self) -> ArtistRel {
            ArtistRel {
                assoc_album: ModelRc::new(VecModel::from(
                    self.assoc_album
                        .into_iter()
                        .map(|x| x.to_rel())
                        .collect::<Vec<_>>(),
                )),
                id: self.id,
                title: self.title.into(),
            }
        }
    }

    impl AlbumIM {
        fn to_rel(self) -> AlbumRel {
            AlbumRel {
                assoc_track: ModelRc::new(VecModel::from(
                    self.assoc_track
                        .into_iter()
                        .map(|x| x.to_rel())
                        .collect::<Vec<_>>(),
                )),
                id: self.id,
                title: self.title.into(),
            }
        }
    }

    impl TrackIM {
        fn to_rel(self) -> TrackRel {
            TrackRel {
                id: self.id,
                title: self.title.into(),
            }
        }
    }

    std::thread::spawn(move || {
        // fetch from server
        let get = ureq::get("http://localhost:8080/api/tabledump")
            .call()
            .unwrap()
            .body_mut()
            .read_json::<Vec<RetRow>>()
            .unwrap();
        if get.len() == 0 {
            return;
        }

        // turn into tree
        //
        // The way this works is that it constructs the tree of artists from the leaves.
        // It collects the tracks [[tcol]], then it shoves that into an album [[addalbum]]. It keeps on collecting tracks
        // and shoving them into albums until the artist mismatches. Once that happens, then it collects
        // the albums and shoves it into the artist [[addartist]]. If either of the album id or artist id changes
        // then we've started a new segment end must change the basis `latest` that we're comparing
        // against [[latestchng]]. Once the loop is done, it will not have added the last artist + album, so just
        // append it [[lastappnd]].
        let mut set: Vec<ArtistIM> = Vec::new();
        let mut latest = &get[0];
        let mut tracks = vec![];
        let mut albums = vec![];
        for item in get.iter() {
            // [[addalbum]]
            if item.album_id != latest.album_id {
                albums.push(AlbumIM {
                    assoc_track: take(&mut tracks),
                    id: latest.album_id,
                    title: latest.album_name.to_owned(),
                });
            }

            // [[addartist]]
            if item.artist_id != latest.artist_id {
                set.push(ArtistIM {
                    assoc_album: take(&mut albums),
                    id: latest.artist_id,
                    title: latest.artist_name.to_owned(),
                });
            }

            // [[latestchng]]
            if item.album_id != latest.album_id || item.artist_id != latest.artist_id {
                latest = item;
            }

            // [[tcol]]
            tracks.push(TrackIM {
                id: item.track_id,
                title: item.track_title.to_owned(),
            });
        }

        // [[lastappnd]]
        set.push(ArtistIM {
            assoc_album: {
                albums.push(AlbumIM {
                    assoc_track: take(&mut tracks),
                    id: latest.album_id,
                    title: latest.album_name.to_owned(),
                });
                albums
            },
            id: latest.artist_id,
            title: latest.artist_name.to_owned(),
        });

        // update ui
        WEAK_MAINWINDOW
            .get()
            .unwrap()
            .upgrade_in_event_loop(move |main_window| {
                let tl = main_window.global::<TrackList>();
                tl.set_artists(ModelRc::new(VecModel::from(
                    set.into_iter().map(|x| x.to_rel()).collect::<Vec<_>>(),
                )));
            })
            .unwrap();
    });
}

fn file_upload() {
    // This LOC blocks. If this is an issue, we could just shove it into
    // another thread (potentially, see rfd's docs)
    let Some(files) = rfd::FileDialog::new().pick_folders() else {
        return;
    };

    // TODO: remove unwrap
    std::thread::spawn(move || {
        /// recursive file tree scanning function for all files in a dir
        ///
        /// Arguments:
        /// - dir_at: &Path
        ///     a directory path to traverse
        fn path_scan(dir_at: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
            let mut hs = Vec::new();
            let dirs = fs::read_dir(dir_at)?;
            for item in dirs {
                // handle err
                let item = item?;

                // file type
                let file_type = item.file_type()?;
                if file_type.is_dir() {
                    // RECURSE
                    let ret_hs = path_scan(&item.path())?;
                    hs.extend(ret_hs);
                } else if file_type.is_file() {
                    hs.push(item.path());
                }
            }
            Ok(hs)
        }

        // base paths used for subtracting root trees
        // see [[sgmtroottrees]] for what happens
        let base_paths = files
            .clone();

        // flatten paths from `path_scan`
        let mut paths = Vec::new();
        for dir_at in files.into_iter() {
            paths.extend(path_scan(&dir_at).unwrap());
        }
       

        // remove root dirs before the actual paths [[sgmtroottrees]]
        //
        // Why do we need this? Long story short the paths taken from read_dir are relative
        // to the path given to it:
        //
        //     read_dir("a/") -> [ "a/b", "a/c" ]
        //     read_dir("/z/b") -> [ "/z/b/c", "/z/b/d" ]
        //     read_dir("C:\a") -> [ "C:\a\b", "C:\a\c"]
        // 
        // This means that the paths from `files` (or `base_paths` in this case) are the head
        // of the dirs and must be stripped so that it can be inserted into the server without
        // friction:
        //
        //     [ "a/b/d", "a/c" ] -> [ "/b/d" "/c" ]
        // 
        // That's what the following code "should" do.
    });
}

fn main() {
    // setup callbacks
    let main_window = MainWindow::new().unwrap();
    main_window
        .global::<TrackList>()
        .on_populate_artists(tabledump);
    main_window
        .global::<TrackList>()
        .on_toggle_upload(file_upload);

    // set weak access
    let w_main_window = main_window.as_weak();
    // drop here because we know that nobody else has set the OnceSync
    drop(WEAK_MAINWINDOW.set(w_main_window));

    // slint entrypoint
    main_window.run().unwrap();
}
