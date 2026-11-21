import { expect, it } from "vitest";
import { emptyState } from "./state";
import {
  clearTemporaryLoad,
  markTemporaryLoad,
  reviewerSettings,
} from "./ttml-reviewer";

const custom = {
  ...emptyState.settings,
  lyric_size: 60,
  space_gravity: true,
  lyric_offset_ms: 300,
  lyric_song_offsets: { "spotify:track:a": 200 },
  lyrics_up_next: true,
};

it("leaves settings alone when reviewing is off", () => {
  const settings = { ...custom, ttml_reviewer: "off" as const };
  expect(
    reviewerSettings(settings, "spotify:track:a", "Local TTML", true),
  ).toBe(settings);
});

it("resets display settings only for a temporary load of the playing song", () => {
  const settings = { ...custom, ttml_reviewer: "temporary" as const };
  markTemporaryLoad("spotify:track:a");
  const reviewed = reviewerSettings(settings, "spotify:track:a", "Local TTML");
  expect(reviewed.lyric_size).toBe(emptyState.settings.lyric_size);
  expect(reviewed.space_gravity).toBe(false);
  expect(reviewed.lyric_offset_ms).toBe(0);
  expect(reviewed.lyric_song_offsets).toEqual({});
  expect(reviewed.lyrics_up_next).toBe(false);
  expect(reviewerSettings(settings, "spotify:track:b", "Local TTML")).toBe(
    settings,
  );
  expect(reviewerSettings(settings, "spotify:track:a", "lyssna")).toBe(
    settings,
  );
  clearTemporaryLoad("spotify:track:a");
  expect(reviewerSettings(settings, "spotify:track:a", "Local TTML")).toBe(
    settings,
  );
  expect(
    reviewerSettings(settings, "spotify:track:a", undefined, true)
      .space_gravity,
  ).toBe(false);
});

it("covers every loaded TTML when set to always", () => {
  const settings = { ...custom, ttml_reviewer: "always" as const };
  expect(
    reviewerSettings(settings, "spotify:track:c", "Local TTML").space_gravity,
  ).toBe(false);
  expect(reviewerSettings(settings, "spotify:track:c", "apple_music")).toBe(
    settings,
  );
});
