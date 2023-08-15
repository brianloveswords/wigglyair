CREATE TABLE tracks(
    path TEXT NOT NULL,
    last_modified TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    track_length INTEGER NOT NULL,
    sample_rate INTEGER NOT NULL,
    channels INTEGER NOT NULL,
    album TEXT NOT NULL,
    artist TEXT NOT NULL,
    title TEXT NOT NULL,
    album_artist TEXT NOT NULL,
    track INTEGER NOT NULL,
    UNIQUE(path)
);
