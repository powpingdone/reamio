-- Add up migration script here
CREATE TABLE dir (
       node INTEGER PRIMARY KEY, -- uid of the dir
       name TEXT NOT NULL -- dir display name
) STRICT;

CREATE TABLE dir_tree (
       -- there is a way to create a symlink loop, but it would most likely require editing the db
       node INTEGER UNIQUE NOT NULL,  -- child dir
       parent INTEGER NULL, -- parent dir, NULL means root
       FOREIGN KEY (parent) REFERENCES dir (node)
) STRICT;

CREATE INDEX dir_tree_parent ON dir_tree (parent);

CREATE TABLE track (
      id INTEGER PRIMARY KEY,
      title TEXT NOT NULL,
      dir INTEGER NULL,
      fname TEXT NOT NULL,
      FOREIGN KEY (dir) REFERENCES dir (node)
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
       FOREIGN KEY (artist) REFERENCES artist (id),
       FOREIGN KEY (track) REFERENCES track (id)
) STRICT, WITHOUT ROWID;

CREATE TABLE album_tracks (
       track INTEGER PRIMARY KEY,
       album INTEGER NOT NULL,
       FOREIGN KEY (track) REFERENCES track (id),
       FOREIGN KEY (album) REFERENCES album (id)
) STRICT, WITHOUT ROWID;
