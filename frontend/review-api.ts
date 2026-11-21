import { rpc } from "./bridge";
import type { Track } from "./types";

export type ReviewLane = "priority" | "backlog";
export type ReviewAction = "approve" | "reject" | "request-changes";
export type RejectReason =
  "low_effort" | "wrong_song" | "duplicate" | "spam" | "other";

export const REJECT_REASONS: [RejectReason, string][] = [
  ["low_effort", "Low effort"],
  ["wrong_song", "Wrong song"],
  ["duplicate", "Duplicate"],
  ["spam", "Spam"],
  ["other", "Other"],
];
export const GRADES: [1 | 2 | 3, string][] = [
  [3, "Clean"],
  [2, "Minor fixes"],
  [1, "Heavy fixes"],
];

export interface ReviewLink {
  platform: string;
  external_id: string;
}

export interface ReviewSubmission {
  id: string;
  song_id: string;
  song_title: string;
  kind: "new" | "edit";
  base_revision_id?: string;
  head_revision_id?: string;
  submitter_id: string;
  submitter_name?: string;
  submitter_tier: number;
  status: string;
  proposed_links: ReviewLink[];
  major_rewrite: boolean;
  light_review: boolean;
  priority: boolean;
  submitted_at: string;
  decision_note?: string;
  head_score: number;
  head_votes: number;
}

export interface ReviewComment {
  id: string;
  author_id: string;
  author_name?: string;
  body: string;
  created_at: string;
}

interface ReviewSong {
  id: string;
  title: string;
  artists?: string[];
  links?: ReviewLink[];
}

const get = <T>(path: string) =>
  rpc<T>("lyssna_review", { method: "get", path });
const post = <T>(path: string, body: object = {}) =>
  rpc<T>("lyssna_review", { method: "post", path, body });

export const reviewQueue = (lane: ReviewLane, limit = 50) =>
  get<ReviewSubmission[]>(`/v1/queue?lane=${lane}&limit=${limit}`);
export const reviewSubmission = (id: string) =>
  get<ReviewSubmission>(`/v1/submissions/${id}`);
export const reviewComments = (id: string) =>
  get<ReviewComment[]>(`/v1/submissions/${id}/comments`);
export const addReviewComment = (id: string, body: string) =>
  post<ReviewComment>(`/v1/submissions/${id}/comments`, { body });
export const revisionTtml = (id: string) =>
  get<string>(`/v1/revisions/${id}/ttml`);
export const decideReview = (
  id: string,
  action: ReviewAction,
  decision: { grade?: number; reason?: RejectReason; note?: string },
) => post<ReviewSubmission>(`/v1/submissions/${id}/${action}`, decision);

/** The Spotify track a submission is for: the song's own link, or for a new
 *  song, the one its submitter proposed. */
export async function reviewTrackUri(submission: ReviewSubmission) {
  const spotify = (links?: ReviewLink[]) =>
    links?.find(
      (link) =>
        link.platform === "spotify" &&
        /^[A-Za-z0-9]{22}$/.test(link.external_id),
    )?.external_id;
  const proposed = spotify(submission.proposed_links);
  if (proposed) return `spotify:track:${proposed}`;
  const song = await get<ReviewSong>(`/v1/songs/${submission.song_id}`).catch(
    () => undefined,
  );
  const linked = spotify(song?.links);
  return linked ? `spotify:track:${linked}` : undefined;
}

const songs = new Map<string, Promise<Track | undefined>>();
/** The Spotify song a submission is for, with its artists and cover. */
export function reviewSong(submission: ReviewSubmission) {
  let song = songs.get(submission.id);
  if (!song) {
    song = reviewTrackUri(submission)
      .then((uri) => (uri ? rpc<Track>("resolve_track", { uri }) : undefined))
      .catch(() => undefined);
    songs.set(submission.id, song);
  }
  return song;
}

/** Approved lyrics go live, so the app drops its saved answers for the song. */
export async function reviewPublished(submission: ReviewSubmission) {
  const track = await reviewSong(submission);
  if (track) await rpc("lyssna_published", { track }).catch(() => {});
}

export const REVIEWER_ROLES = ["reviewer", "moderator", "admin"];
export const canReview = (role?: string | null) =>
  !!role && REVIEWER_ROLES.includes(role);

export interface DiffLine {
  text: string;
  startMs?: number;
}
export type DiffRow =
  | { kind: "same"; text: string; retimed: boolean }
  | { kind: "added"; text: string }
  | { kind: "removed"; text: string };

// A line counts as retimed when its start moves more than this.
const RETIMED_MS = 40;

/** What an edit changes line by line: lines matched by their words, and
 *  matched lines flagged when their timing moved. */
export function diffLines(base: DiffLine[], head: DiffLine[]): DiffRow[] {
  const key = (line: DiffLine) => line.text.trim().toLowerCase();
  const rows = base.length;
  const cols = head.length;
  const lengths = Array.from({ length: rows + 1 }, () =>
    new Array<number>(cols + 1).fill(0),
  );
  for (let row = rows - 1; row >= 0; row--)
    for (let col = cols - 1; col >= 0; col--)
      lengths[row][col] =
        key(base[row]) === key(head[col])
          ? lengths[row + 1][col + 1] + 1
          : Math.max(lengths[row + 1][col], lengths[row][col + 1]);
  const out: DiffRow[] = [];
  let row = 0;
  let col = 0;
  while (row < rows && col < cols) {
    if (key(base[row]) === key(head[col])) {
      const before = base[row].startMs;
      const after = head[col].startMs;
      out.push({
        kind: "same",
        text: head[col].text,
        retimed:
          before !== undefined &&
          after !== undefined &&
          Math.abs(before - after) > RETIMED_MS,
      });
      row++;
      col++;
    } else if (lengths[row + 1][col] >= lengths[row][col + 1])
      out.push({ kind: "removed", text: base[row++].text });
    else out.push({ kind: "added", text: head[col++].text });
  }
  while (row < rows) out.push({ kind: "removed", text: base[row++].text });
  while (col < cols) out.push({ kind: "added", text: head[col++].text });
  return out;
}
