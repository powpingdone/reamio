-- Add up migration script here
CREATE TABLE users (
       username_lower TEXT PRIMARY KEY NOT NULL, -- used for uniqueness checking
       username_orig TEXT NOT NULL, -- what is actually displayed to the user
       phc TEXT NOT NULL -- password hash string
) STRICT WITHOUT ROWID;
