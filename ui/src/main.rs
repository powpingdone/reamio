use std::mem::take;

use serde::Deserialize;
use slint::{ModelRc, SharedString, VecModel};

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

        // turn into tree
        //
        // The way this works is that it constructs the tree of artists from the leaves.
        // It collects the tracks [4], then it shoves that into an album [1]. It keeps on collecting tracks
        // and shoving them into albums until the artist mismatches. Once that happens, then it collects
        // the albums and shoves it into the artist [2]. If either of the album id or artist id changes
        // then we've started a new segment end must change the basis `latest` that we're comparing
        // against [3]. Once the loop is done, it may not have added the last artist + album, so just
        // append it.
        let set: VecModel<ArtistRel> = VecModel::default();
        let mut latest = &get[0];
        let mut tracks = vec![];
        let mut albums = vec![];
        for item in get.iter() {
            // [1]
            if item.album_id != latest.album_id {
                albums.push(AlbumRel {
                    assoc_track: ModelRc::new(VecModel::from(take(&mut tracks))),
                    id: latest.album_id,
                    title: SharedString::from(&latest.album_name),
                });
            }

            // [2]
            if item.artist_id != latest.artist_id {
                set.push(ArtistRel {
                    assoc_album: ModelRc::new(VecModel::from(take(&mut albums))),
                    id: latest.artist_id,
                    title: SharedString::from(&latest.artist_name),
                });
            }

            // [3]
            if item.album_id != latest.album_id || item.artist_id != latest.artist_id {
                latest = item;
            }

            // [4]
            tracks.push(TrackRel {
                id: item.track_id,
                title: SharedString::from(&item.track_title),
            });
        }
        
        // [5]
        set.push(ArtistRel {
            assoc_album: ModelRc::new(VecModel::from({
                albums.push(AlbumRel {
                    assoc_track: ModelRc::new(VecModel::from(take(&mut tracks))),
                    id: latest.album_id,
                    title: SharedString::from(&latest.album_name),
                });
                albums
            })),
            id: latest.artist_id,
            title: SharedString::from(&latest.artist_name),
        });

        // update ui
        w_main_window.upgrade_in_event_loop(move |main_window| {
            let tl = main_window.global::<TrackList>();
            tl.set_artists(ModelRc::new(set));
        });
    });

    main_window.run().unwrap();
}
