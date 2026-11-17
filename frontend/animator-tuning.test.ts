import { describe, expect, it } from "vitest";
import {
  ANIMATOR_DEFAULTS,
  animatorValues,
  choosePreset,
  DEFAULT_PRESET_ID,
  deletePreset,
  idleScale,
  importCode,
  presetOf,
  type AnimatorPreset,
  loadPreset,
  randomize,
  savePreset,
  setValue,
  shareCode,
  visibleParameters,
} from "./animator-tuning";
import { MOTION, tuneMotion } from "./lyric-motion";

describe("animator presets", () => {
  it("plays Lyssna (Default) unless Legacy or Custom is chosen", () => {
    expect(presetOf({})).toBe("Lyssna");
    // Saved before Lyssna (Default) existed, "Default" meant the default.
    expect(presetOf({ preset: "Default" as AnimatorPreset })).toBe("Lyssna");
    expect(animatorValues({})).toEqual(ANIMATOR_DEFAULTS);
    expect(animatorValues({}).wordPeakScale).toBe(1.0505);
    // Unsung words keep the spacing sung ones have.
    expect(idleScale({})).toBe(1);
    expect(idleScale({ preset: "Custom" })).toBe(1);
    // Lyssna and Legacy ignore Custom values.
    expect(
      animatorValues({ preset: "Lyssna", custom: { wordPeakScale: 2 } })
        .wordPeakScale,
    ).toBe(1.0505);
    expect(
      animatorValues({ preset: "Legacy", custom: { wordPeakScale: 2 } })
        .wordPeakScale,
    ).toBe(1.075);
  });
  it("copies the playing values the first time Custom is chosen", () => {
    const custom = choosePreset({ preset: "Lyssna" }, "Custom");
    expect(custom.custom).toEqual(ANIMATOR_DEFAULTS);
    expect(
      choosePreset({ preset: "Legacy" }, "Custom").custom?.wordPeakScale,
    ).toBe(1.075);
    const kept = choosePreset(
      { preset: "Legacy", custom: { glowStrength: 2 } },
      "Custom",
    );
    expect(kept.custom).toEqual({ glowStrength: 2 });
  });
  it("switches to Custom on edit and clamps typed values", () => {
    const edited = setValue({ preset: "Lyssna" }, "glowStrength", 1e9);
    expect(edited.preset).toBe("Custom");
    expect(animatorValues(edited).glowStrength).toBe(100);
    expect(animatorValues(edited).wordPeakScale).toBe(1.0505);
  });
  it("saves, overwrites by name, loads and deletes presets", () => {
    let settings = setValue({}, "scaleDamping", 0.9);
    settings = savePreset(settings, "Mine");
    const id = settings.selected!;
    settings = savePreset(setValue(settings, "scaleDamping", 0.3), "mine");
    expect(settings.saved).toHaveLength(1);
    expect(settings.saved![0].values.scaleDamping).toBe(0.3);
    settings = loadPreset({ ...settings, custom: {} }, id);
    expect(settings.custom?.scaleDamping).toBe(0.3);
    expect(deletePreset(settings, id)).toMatchObject({
      saved: [],
      selected: null,
    });
  });
  it("loads the built-in Default entry as Lyssna (Default)'s values", () => {
    const loaded = loadPreset(
      setValue({}, "glowStrength", 3),
      DEFAULT_PRESET_ID,
    );
    expect(loaded).toMatchObject({ preset: "Custom", selected: "default" });
    expect(loaded.custom).toEqual(ANIMATOR_DEFAULTS);
  });
  it("round-trips SLP1 share codes and renames duplicates", () => {
    const code = shareCode("Bouncy", { verticalDamping: 0.2 });
    expect(code).toMatch(/^SLP1:Bouncy\|1\.0505,1\.175,/);
    const first = importCode({}, code);
    expect(first.name).toBe("Bouncy");
    expect(first.settings.custom?.verticalDamping).toBe(0.2);
    const second = importCode(first.settings, code);
    expect(second.name).toBe("Bouncy (2)");
    expect(() => importCode({}, "SLP1:x|1,2")).toThrow(/missing/);
    expect(() => importCode({}, "hello")).toThrow(/Not an animator preset/);
  });
  it("reads the fork's legacy SLAP1 codes", () => {
    const json = JSON.stringify({ name: "Old", values: { glowStrength: 2 } });
    const code = `SLAP1:${btoa(json).replace(/=+$/, "")}`;
    expect(importCode({}, code).settings.custom?.glowStrength).toBe(2);
  });
  it("randomizes shown parameters on slider steps", () => {
    const ids = visibleParameters("Basic").map((item) => item.id);
    expect(ids).toHaveLength(5);
    expect(visibleParameters("Advanced")).toHaveLength(11);
    const random = randomize({}, ids, () => 0.999999);
    expect(random.preset).toBe("Custom");
    expect(random.custom?.wordPeakScale).toBe(2);
    expect(random.custom?.scaleFrequency).toBe(0.88);
  });
  it("matches the fork's Lyssna preset curve for curve", () => {
    // The fork's rebuildAnimatorTuning with its Lyssna preset and default
    // values, outside Simple mode: idle 1 on word and letter scale, peaks
    // 1.0505 and 1.175 at 0.7, upstream lift and glow, and its springs.
    tuneMotion({ preset: "Lyssna" });
    expect(MOTION.idleScale).toBe(1);
    for (const [at, value] of [
      [0, 1],
      [0.7, 1.0505],
      [1, 1],
    ])
      expect(MOTION.scale(at)).toBeCloseTo(value, 10);
    for (const [at, value] of [
      [0, 1],
      [0.7, 1.175],
      [1, 1],
    ])
      expect(MOTION.letterScale(at)).toBeCloseTo(value, 10);
    for (const [at, value] of [
      [0, 1 / 100],
      [0.9, -(1 / 60)],
      [1, 0],
    ])
      expect(MOTION.lift(at)).toBeCloseTo(value, 10);
    for (const [at, value] of [
      [0, 0],
      [0.15, 1],
      [0.6, 1],
      [1, 0],
    ])
      expect(MOTION.glow(at)).toBeCloseTo(value, 10);
    expect(MOTION.springs).toEqual({
      scale: [0.88, 0.64],
      lift: [1.45, 0.4],
      glow: [1.18, 0.56],
    });
    expect(MOTION.blurPerLine).toBe(1.25);
  });
  it("tunes the motion curves", () => {
    tuneMotion({ preset: "Legacy" });
    expect(MOTION.scale(0)).toBeCloseTo(1);
    expect(MOTION.scale(0.7)).toBeCloseTo(1.075);
    tuneMotion(setValue({}, "wordPeakScale", 1.2));
    expect(MOTION.scale(0)).toBeCloseTo(1);
    expect(MOTION.scale(0.7)).toBeCloseTo(1.2);
    tuneMotion(setValue({}, "blurStrength", 2));
    expect(MOTION.blurPerLine).toBeCloseTo(2.5);
    // Stronger motion rises further; unsung text stays level with the line.
    tuneMotion(setValue({}, "verticalStrength", 15));
    expect(MOTION.lift(0)).toBeCloseTo(0.01);
    expect(MOTION.lift(0.9)).toBeCloseTo(-0.25);
    tuneMotion({});
  });
});
