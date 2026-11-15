import { describe, expect, it } from "vitest";
import { fillFromKnown, knownTracks } from "./queue-fill";
import type { Track } from "./types";

const song = (uri: string, name = "", artist = ""): Track => ({
  uri,
  name,
  artists: artist
    ? [{ name: artist, uri: "", id: "" } as Track["artists"][0]]
    : [],
  album: {
    name: "",
    uri: "",
    images: name ? [{ url: "a" }] : [],
  } as unknown as Track["album"],
  duration_ms: 0,
  is_local: false,
});

describe("queue songs sent bare", () => {
  it("take their name, artists and artwork from songs already shown", () => {
    const known = knownTracks([
      [song("spotify:track:a", "Spawn!", "Evan Fong")],
    ]);
    const filled = fillFromKnown(song("spotify:track:a"), known);
    expect(filled.name).toBe("Spawn!");
    expect(filled.artists[0].name).toBe("Evan Fong");
    expect(filled.album.images).toHaveLength(1);
  });
  it("stay as sent when nothing shown matches", () => {
    const bare = song("spotify:track:b");
    expect(fillFromKnown(bare, knownTracks([]))).toBe(bare);
  });
});
