import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  CircleCheck,
  CircleX,
  MessageSquareWarning,
  Play,
  RefreshCw,
  SquarePen,
  X,
} from "lucide-react";
import type { Lyrics, Track } from "./types";
import { Cover } from "./components";
import {
  GRADES,
  REJECT_REASONS,
  addReviewComment,
  decideReview,
  diffLines,
  reviewComments,
  reviewQueue,
  reviewSong,
  reviewTrackUri,
  revisionTtml,
  type DiffRow,
  type RejectReason,
  type ReviewComment,
  type ReviewLane,
  type ReviewSubmission,
} from "./review-api";

export interface ReviewOpen {
  submission: ReviewSubmission;
  uri: string;
  ttml: string;
}

const ago = (iso: string) => {
  const minutes = Math.max(0, (Date.now() - Date.parse(iso)) / 60000);
  if (minutes < 60) return `${Math.round(minutes)}m ago`;
  if (minutes < 60 * 24) return `${Math.round(minutes / 60)}h ago`;
  return `${Math.round(minutes / 60 / 24)}d ago`;
};
const who = (submission: ReviewSubmission) =>
  submission.submitter_name ? `@${submission.submitter_name}` : "Deleted user";

/** Waiting submissions by lane; picking one hands it to `onPick`. */
export function ReviewQueue({
  onPick,
  onClose,
  refresh = 0,
}: {
  onPick: (submission: ReviewSubmission) => void;
  onClose?: () => void;
  refresh?: number;
}) {
  const [lane, setLane] = useState<ReviewLane>("priority");
  const [items, setItems] = useState<ReviewSubmission[] | null>(null);
  const [failed, setFailed] = useState<string>();
  const load = useCallback(() => {
    setFailed(undefined);
    setItems(null);
    reviewQueue(lane).then(setItems, (error) => {
      setItems([]);
      setFailed(String(error));
    });
  }, [lane]);
  useEffect(load, [load, refresh]);
  return (
    <>
      <header className="review-head">
        <h3>Review</h3>
        <button onClick={load} title="Refresh" aria-label="Refresh queue">
          <RefreshCw size={15} />
        </button>
        {onClose && (
          <button onClick={onClose} aria-label="Close review">
            <X size={16} />
          </button>
        )}
      </header>
      <div className="review-lanes" role="tablist">
        {(
          [
            ["priority", "Priority"],
            ["backlog", "Backlog"],
          ] as const
        ).map(([value, label]) => (
          <button
            key={value}
            role="tab"
            aria-selected={lane === value}
            className={lane === value ? "active" : ""}
            onClick={() => setLane(value)}
          >
            {label}
          </button>
        ))}
      </div>
      {failed ? (
        <p className="review-empty">{failed}</p>
      ) : items === null ? (
        <p className="review-empty">Loading…</p>
      ) : !items.length ? (
        <p className="review-empty">Nothing waiting here.</p>
      ) : (
        <ul className="review-list">
          {items.map((item) => (
            <li key={item.id}>
              <button onClick={() => onPick(item)}>
                <ReviewSongRow submission={item} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function ReviewSongRow({ submission }: { submission: ReviewSubmission }) {
  const [song, setSong] = useState<Track | null>();
  useEffect(() => {
    let live = true;
    void reviewSong(submission).then((found) => live && setSong(found ?? null));
    return () => {
      live = false;
    };
  }, [submission]);
  return (
    <>
      <Cover images={song?.album.images ?? []} className="review-cover" />
      <span className="review-song">
        <strong>{song?.name ?? submission.song_title}</strong>
        {song && (
          <span>{song.artists.map((artist) => artist.name).join(", ")}</span>
        )}
        <small>
          {submission.kind === "edit" ? "Edit" : "New"} · {who(submission)} ·
          Tier {submission.submitter_tier} · {ago(submission.submitted_at)}
          {submission.major_rewrite
            ? " · Major rewrite"
            : submission.light_review
              ? " · Light review"
              : ""}
        </small>
      </span>
    </>
  );
}

/** The TTML Creator's review side panel: a submission's changes, comments
 *  and decision, or the queue to pick another. */
export function ReviewPanel({
  initial,
  playing,
  editing,
  editorTtml,
  onPlay,
  onOpen,
  onParse,
  onSubmitFix,
  onDecided,
  notify,
  onClose,
}: {
  /** Picked on the Review page, so it opens in the editor at once. */
  initial?: ReviewSubmission;
  playing?: Track;
  /** The submission loaded in the editor and the TTML it opened with. */
  editing?: { id: string; baseline: string };
  editorTtml: () => string;
  onPlay: (uri: string) => void;
  onOpen: (open: ReviewOpen) => void;
  onParse: (ttml: string) => Promise<Lyrics>;
  onSubmitFix: (ttml: string) => Promise<void>;
  onDecided: () => void;
  notify: (message: string, error?: boolean) => void;
  onClose: () => void;
}) {
  const [picked, setPicked] = useState(initial);
  const [refresh, setRefresh] = useState(0);
  useEffect(() => setPicked(initial), [initial]);
  return (
    <aside className="review-panel" aria-label="Review">
      {picked ? (
        <ReviewDetail
          key={picked.id}
          submission={picked}
          autoOpen={picked === initial}
          playing={playing}
          editing={editing?.id === picked.id ? editing : undefined}
          editorTtml={editorTtml}
          onPlay={onPlay}
          onOpen={onOpen}
          onParse={onParse}
          onSubmitFix={onSubmitFix}
          notify={notify}
          onBack={() => setPicked(undefined)}
          onDecided={() => {
            setPicked(undefined);
            setRefresh((count) => count + 1);
            onDecided();
          }}
        />
      ) : (
        <ReviewQueue onPick={setPicked} onClose={onClose} refresh={refresh} />
      )}
    </aside>
  );
}

function ReviewDetail({
  submission,
  autoOpen,
  playing,
  editing,
  editorTtml,
  onPlay,
  onOpen,
  onParse,
  onSubmitFix,
  notify,
  onBack,
  onDecided,
}: {
  submission: ReviewSubmission;
  autoOpen: boolean;
  playing?: Track;
  editing?: { id: string; baseline: string };
  editorTtml: () => string;
  onPlay: (uri: string) => void;
  onOpen: (open: ReviewOpen) => void;
  onParse: (ttml: string) => Promise<Lyrics>;
  onSubmitFix: (ttml: string) => Promise<void>;
  notify: (message: string, error?: boolean) => void;
  onBack: () => void;
  onDecided: () => void;
}) {
  const [uri, setUri] = useState<string | null>();
  const [ttml, setTtml] = useState<string>();
  const [diff, setDiff] = useState<DiffRow[] | null>();
  const [comments, setComments] = useState<ReviewComment[]>();
  const [reply, setReply] = useState("");
  const [grade, setGrade] = useState<1 | 2 | 3>(3);
  const [note, setNote] = useState("");
  const [reason, setReason] = useState<RejectReason>("low_effort");
  const [busy, setBusy] = useState(false);
  const id = submission.id;
  // Callers pass fresh callbacks each render; the fetches below run once.
  const parse = useRef(onParse);
  parse.current = onParse;
  const tell = useRef(notify);
  tell.current = notify;
  useEffect(() => {
    let live = true;
    void reviewTrackUri(submission).then(
      (found) => live && setUri(found ?? null),
    );
    if (submission.head_revision_id)
      revisionTtml(submission.head_revision_id).then(
        (text) => live && setTtml(text),
        (error) => live && tell.current(String(error), true),
      );
    reviewComments(id).then(
      (list) => live && setComments(list),
      () => live && setComments([]),
    );
    return () => {
      live = false;
    };
  }, [submission, id]);
  useEffect(() => {
    if (!ttml || submission.kind !== "edit" || !submission.base_revision_id)
      return setDiff(null);
    let live = true;
    const lines = (lyrics: Lyrics) =>
      lyrics.lines
        .filter((line) => !line.background)
        .map((line) => ({ text: line.text, startMs: line.start_ms }));
    revisionTtml(submission.base_revision_id)
      .then((base) => Promise.all([parse.current(base), parse.current(ttml)]))
      .then(
        ([before, after]) =>
          live && setDiff(diffLines(lines(before), lines(after))),
        () => live && setDiff(null),
      );
    return () => {
      live = false;
    };
  }, [ttml, submission]);
  const open = useRef(onOpen);
  open.current = onOpen;
  const opened = useRef(false);
  useEffect(() => {
    if (!autoOpen || opened.current || !uri || !ttml) return;
    opened.current = true;
    open.current({ submission, uri, ttml });
  }, [autoOpen, uri, ttml, submission]);

  const playingIt = !!uri && playing?.uri === uri;
  const fixed = !!editing && editorTtml() !== editing.baseline;
  const decide = async (
    action: "approve" | "reject" | "request-changes",
    then?: () => Promise<void>,
  ) => {
    if (busy) return;
    setBusy(true);
    try {
      await decideReview(id, action, {
        grade: action === "approve" ? grade : undefined,
        reason: action === "reject" ? reason : undefined,
        note: note.trim() || undefined,
      });
      await then?.();
      notify(
        action === "approve"
          ? `Approved ${submission.song_title}.`
          : action === "reject"
            ? `Rejected ${submission.song_title}.`
            : `Asked for changes to ${submission.song_title}.`,
      );
      onDecided();
    } catch (error) {
      notify(String(error), true);
    } finally {
      setBusy(false);
    }
  };
  const send = async () => {
    const body = reply.trim();
    if (!body) return;
    try {
      const comment = await addReviewComment(id, body);
      setComments((list) => [...(list ?? []), comment]);
      setReply("");
    } catch (error) {
      notify(String(error), true);
    }
  };
  const changed = diff?.filter((row) => row.kind !== "same" || row.retimed);

  return (
    <div className="review-detail">
      <header className="review-head">
        <button onClick={onBack} aria-label="Back to the queue">
          <ArrowLeft size={16} />
        </button>
        <h3 title={submission.song_title}>{submission.song_title}</h3>
      </header>
      <p className="review-meta">
        {submission.kind === "edit" ? "Edit" : "New lyrics"} by{" "}
        {who(submission)} · Tier {submission.submitter_tier} ·{" "}
        {ago(submission.submitted_at)}
      </p>
      <div className="review-buttons">
        <button
          disabled={!uri}
          title={uri === null ? "No Spotify link for this song" : undefined}
          onClick={() => uri && onPlay(uri)}
        >
          <Play size={14} /> {playingIt ? "Playing" : "Play song"}
        </button>
        <button
          disabled={!uri || !ttml}
          onClick={() => uri && ttml && onOpen({ submission, uri, ttml })}
        >
          <SquarePen size={14} />{" "}
          {editing ? "Opened in editor" : "Open in editor"}
        </button>
      </div>
      {submission.kind === "edit" && (
        <section className="review-section">
          <h4>Changes</h4>
          {diff === undefined ? (
            <p className="review-empty">Comparing…</p>
          ) : diff === null ? (
            <p className="review-empty">
              Couldn't compare with the live lyrics.
            </p>
          ) : !changed?.length ? (
            <p className="review-empty">
              No line changes; only details differ.
            </p>
          ) : (
            <ol className="review-diff">
              {diff.map((row, index) =>
                row.kind === "same" && !row.retimed ? null : (
                  <li
                    key={index}
                    data-kind={row.kind === "same" ? "retimed" : row.kind}
                  >
                    {row.text || "(empty line)"}
                  </li>
                ),
              )}
            </ol>
          )}
        </section>
      )}
      <section className="review-section">
        <h4>Comments</h4>
        {comments === undefined ? (
          <p className="review-empty">Loading…</p>
        ) : !comments.length ? (
          <p className="review-empty">No comments yet.</p>
        ) : (
          <ul className="review-comments">
            {comments.map((comment) => (
              <li key={comment.id}>
                <strong>
                  {comment.author_name
                    ? `@${comment.author_name}`
                    : "Deleted user"}
                </strong>
                <p>{comment.body}</p>
              </li>
            ))}
          </ul>
        )}
        <div className="review-reply">
          <input
            value={reply}
            placeholder="Comment to the submitter"
            aria-label="Comment"
            maxLength={2000}
            onChange={(event) => setReply(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && void send()}
          />
          <button onClick={() => void send()} disabled={!reply.trim()}>
            Send
          </button>
        </div>
      </section>
      <section className="review-section review-decide">
        <h4>Decision</h4>
        <textarea
          value={note}
          placeholder="Note for the submitter (needed to ask for changes)"
          aria-label="Decision note"
          maxLength={2000}
          onChange={(event) => setNote(event.target.value)}
        />
        <div className="review-choice" role="radiogroup" aria-label="Quality">
          {GRADES.map(([value, label]) => (
            <button
              key={value}
              role="radio"
              aria-checked={grade === value}
              className={grade === value ? "active" : ""}
              onClick={() => setGrade(value)}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="review-buttons">
          <button
            className="primary"
            disabled={busy}
            onClick={() => void decide("approve")}
          >
            <CircleCheck size={14} /> Approve
          </button>
          <button
            disabled={busy || !fixed}
            title={
              fixed
                ? "Approve, then submit your corrections as an edit credited to you"
                : "Open it in the editor and make corrections first"
            }
            onClick={() =>
              void decide("approve", () => onSubmitFix(editorTtml()))
            }
          >
            <SquarePen size={14} /> Fix and approve
          </button>
        </div>
        <div className="review-buttons">
          <button
            disabled={busy || !note.trim()}
            title={
              note.trim() ? undefined : "Write a note saying what to change"
            }
            onClick={() => void decide("request-changes")}
          >
            <MessageSquareWarning size={14} /> Ask for changes
          </button>
        </div>
        <div
          className="review-choice"
          role="radiogroup"
          aria-label="Reject reason"
        >
          {REJECT_REASONS.map(([value, label]) => (
            <button
              key={value}
              role="radio"
              aria-checked={reason === value}
              className={reason === value ? "active" : ""}
              onClick={() => setReason(value)}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="review-buttons">
          <button
            className="danger"
            disabled={busy}
            onClick={() => void decide("reject")}
          >
            <CircleX size={14} /> Reject
          </button>
        </div>
      </section>
    </div>
  );
}
