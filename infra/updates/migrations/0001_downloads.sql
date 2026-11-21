-- One row per day, platform, kind and version. No IPs or other visitor data.
CREATE TABLE downloads (
  day TEXT NOT NULL,
  platform TEXT NOT NULL,
  kind TEXT NOT NULL,
  version TEXT NOT NULL,
  count INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, platform, kind, version)
);
