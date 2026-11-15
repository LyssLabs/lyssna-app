import type { Track } from "./types";

/** Songs Lyssna already shows, by URI, to name queue songs Spotify Soloist
 * sends bare until their details arrive. */
export function knownTracks(lists: (Track[] | undefined)[]) {
  const known = new Map<string, Track>();
  for (const list of lists)
    for (const track of list ?? [])
      if (track.name && track.artists.length) known.set(track.uri, track);
  return known;
}

export function fillFromKnown(track: Track, known: Map<string, Track>): Track {
  if (track.name && track.artists.length && track.album.images.length)
    return track;
  const full = known.get(track.uri);
  if (!full) return track;
  return {
    ...track,
    name: track.name || full.name,
    artists: track.artists.length ? track.artists : full.artists,
    album: track.album.images.length ? track.album : full.album,
    duration_ms: track.duration_ms || full.duration_ms,
  };
}
