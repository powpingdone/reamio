use std::mem::take;

use serde::Deserialize;
use slint::{ModelRc, VecModel};

slint::include_modules!();

fn main() {
    let main_window = MainWindow::new().unwrap();
    let w_main_window = main_window.as_weak();

    std::thread::spawn(move || {
        #[derive(Deserialize)]
        struct RetRow {
            album_name: String,
            artist_name: String,
            track_title: String,

            album_id: i32,
            artist_id: i32,
            track_id: i32,
        }

        let get = ureq::get("http://localhost:8080/api/tabledump")
            .call()
            .unwrap()
            .body_mut()
            .read_json::<Vec<RetRow>>()
            .unwrap();
        if get.len() == 0 {
            return;
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

        // turn into tree
        //
        // The way this works is that it constructs the tree of artists from the leaves.
        // It collects the tracks [4], then it shoves that into an album [1]. It keeps on collecting tracks
        // and shoving them into albums until the artist mismatches. Once that happens, then it collects
        // the albums and shoves it into the artist [2]. If either of the album id or artist id changes
        // then we've started a new segment end must change the basis `latest` that we're comparing
        // against [3]. Once the loop is done, it will not have added the last artist + album, so just
        // append it [5].
        let mut set: Vec<ArtistIM> = Vec::new();
        let mut latest = &get[0];
        let mut tracks = vec![];
        let mut albums = vec![];
        for item in get.iter() {
            // [1]
            if item.album_id != latest.album_id {
                albums.push(AlbumIM {
                    assoc_track: take(&mut tracks),
                    id: latest.album_id,
                    title: latest.album_name.to_owned(),
                });
            }

            // [2]
            if item.artist_id != latest.artist_id {
                set.push(ArtistIM {
                    assoc_album: take(&mut albums),
                    id: latest.artist_id,
                    title: latest.artist_name.to_owned(),
                });
            }

            // [3]
            if item.album_id != latest.album_id || item.artist_id != latest.artist_id {
                latest = item;
            }

            // [4]
            tracks.push(TrackIM {
                id: item.track_id,
                title: item.track_title.to_owned(),
            });
        }

        // [5]
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
        w_main_window
            .upgrade_in_event_loop(move |main_window| {
                let tl = main_window.global::<TrackList>();
                tl.set_artists(ModelRc::new(VecModel::from(
                    set.into_iter().map(|x| x.to_rel()).collect::<Vec<_>>(),
                )));
            })
            .unwrap();
    });

    main_window.run().unwrap();
}
