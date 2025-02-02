-- Add up migration script here
CREATE TABLE dir_tree (
       node INTEGER PRIMARY KEY,
       name BLOB NOT NULL,
       parent INTEGER NULL,
       FOREIGN KEY (parent) REFERENCES (dir_tree) node
) STRICT;

CREATE TABLE track (
      id INTEGER PRIMARY KEY,
      title TEXT NOT NULL,
      dir INTEGER NULL,
      fname BLOB NOT NULL,
      FOREIGN KEY (dir) REFERENCES (dir_tree) node
) STRICT;

CREATE TABLE artist (
       id INTEGER PRIMARY KEY,
       name TEXT NOT NULL
) STRICT;

CREATE TABLE album (
       id INTEGER PRIMARY KEY,
       name TEXT NOT NULL
) STRICT;

CREATE TABLE artist_tracks (
       artist INTEGER NOT NULL,
       track INTEGER NOT NULL,
       PRIMARY KEY (artist, track),
       FOREIGN KEY (artist) REFERENCES (artist) id,
       FOREIGN KEY (track) REFERENCES (track) id
) STRICT, WITHOUT ROWID;

CREATE TABLE album_tracks (
       track INTEGER PRIMARY KEY,
       album INTEGER NOT NULL,
       FOREIGN KEY (track) REFERENCES (track) id,
       FOREIGN KEY (album) REFERENCES (album) id
) STRICT, WITHOUT ROWID;
