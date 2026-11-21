/**
 * The section names Genius uses, each with the names Genius pages in other
 * languages give it. A tag in any of them is saved under the English name,
 * as Apple Music's TTML does.
 */
const PARTS: Record<string, string[]> = {
  Intro: [
    "intro",
    "introducción",
    "introdução",
    "introduzione",
    "introduction",
    "イントロ",
    "인트로",
  ],
  Verse: [
    "verse",
    "verso",
    "estrofa",
    "strofa",
    "strophe",
    "couplet",
    "vers",
    "ヴァース",
    "バース",
    "벌스",
  ],
  "Pre-Chorus": [
    "pre-chorus",
    "prechorus",
    "pre-estribillo",
    "pré-refrão",
    "pre-refrão",
    "pré-refrain",
    "pre-refrain",
    "pre-coro",
    "pre-ritornello",
    "prä-refrain",
    "pre-hook",
    "プリコーラス",
    "프리코러스",
  ],
  Chorus: [
    "chorus",
    "estribillo",
    "coro",
    "refrão",
    "ritornello",
    "ritornell",
    "kehrreim",
    "サビ",
    "コーラス",
    "코러스",
    "후렴",
  ],
  "Post-Chorus": [
    "post-chorus",
    "postchorus",
    "post-estribillo",
    "pós-refrão",
    "post-refrain",
    "post-coro",
    "post-hook",
    "ポストコーラス",
    "포스트코러스",
  ],
  Hook: ["hook", "gancho"],
  Refrain: ["refrain"],
  Bridge: ["bridge", "puente", "ponte", "pont", "brücke", "ブリッジ", "브릿지"],
  Interlude: [
    "interlude",
    "interludio",
    "interlúdio",
    "zwischenspiel",
    "間奏",
    "간주",
  ],
  Instrumental: ["instrumental", "instrumentale", "instrumental break"],
  Solo: ["solo", "guitar solo"],
  Breakdown: ["breakdown"],
  Drop: ["drop"],
  "Build-Up": ["build-up", "build up", "buildup", "build"],
  Outro: ["outro", "final", "アウトロ", "아웃트로"],
  Skit: ["skit"],
  Spoken: ["spoken", "spoken word"],
};
const NAMES = new Map(
  Object.entries(PARTS).flatMap(([name, aliases]) =>
    aliases.map((alias) => [alias, name] as const),
  ),
);

/** The Genius section a header names, like "Verse" for "Verso 2: Ana",
 *  or undefined for anything else. */
export function songPart(header: string): string | undefined {
  const key = header
    .split(":")[0]
    .replace(/\s*\(.*?\)\s*/g, " ")
    .replace(/\s+\d+$/, "")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
  return NAMES.get(key);
}

export const SONG_PARTS = Object.keys(PARTS);
