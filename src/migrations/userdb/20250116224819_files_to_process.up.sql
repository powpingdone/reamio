CREATE TABLE uploaded_files (
       path TEXT NOT NULL,
       user TEXT NOT NULL,
       PRIMARY KEY(path, user),
       FOREIGN KEY (user) REFERENCES users(username_lower) 
) STRICT, WITHOUT ROWID;
