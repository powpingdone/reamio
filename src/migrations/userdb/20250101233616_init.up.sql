-- Add up migration script here
CREATE TABLE users (
       username_lower TEXT PRIMARY KEY NOT NULL, -- used for uniqueness checking
       username_orig TEXT NOT NULL, -- what is actually displayed to the user
       phc TEXT NOT NULL -- password hash string
) STRICT, WITHOUT ROWID;

CREATE TABLE uploaded_files (
       orig_path TEXT NOT NULL, -- the original path submitted 
       user TEXT NOT NULL, -- file that has been assigned to user 
       fid INTEGER PRIMARY KEY, -- unique id that references the job
       FOREIGN KEY (user) REFERENCES users(username_lower) 
) STRICT, WITHOUT ROWID;
