import { describe, expect, it } from "vitest";
import { canReview, diffLines } from "./review-api";

describe("diffLines", () => {
  it("marks added, removed and retimed lines", () => {
    const rows = diffLines(
      [
        { text: "Hello there", startMs: 1000 },
        { text: "Old line", startMs: 2000 },
        { text: "Same time", startMs: 3000 },
      ],
      [
        { text: "hello there ", startMs: 1200 },
        { text: "Same time", startMs: 3010 },
        { text: "New line", startMs: 4000 },
      ],
    );
    expect(rows).toEqual([
      { kind: "same", text: "hello there ", retimed: true },
      { kind: "removed", text: "Old line" },
      { kind: "same", text: "Same time", retimed: false },
      { kind: "added", text: "New line" },
    ]);
  });

  it("handles an empty side", () => {
    expect(diffLines([], [{ text: "a" }])).toEqual([
      { kind: "added", text: "a" },
    ]);
    expect(diffLines([{ text: "a" }], [])).toEqual([
      { kind: "removed", text: "a" },
    ]);
  });
});

describe("canReview", () => {
  it("allows reviewers and above", () => {
    expect(canReview("reviewer")).toBe(true);
    expect(canReview("admin")).toBe(true);
    expect(canReview("uploader")).toBe(false);
    expect(canReview(null)).toBe(false);
  });
});
