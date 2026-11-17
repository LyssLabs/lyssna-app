import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { rpc } from "./bridge";
import type { PlayerRequest } from "./types";

type Anchor = {
  position: number;
  at: number;
  playing: boolean;
  rate: number;
  end: number;
};

export type CreatorPlayer = {
  active: boolean;
  playing: boolean;
  position: number;
  rate: number;
  anchor: Anchor;
  read: (at?: number) => number;
  play: (from?: number, to?: number) => void;
  pause: () => void;
  toggle: () => void;
  seek: (ms: number) => void;
  setRate: (rate: number) => void;
  setVolume: (percent: number) => void;
};

/** Where an anchor's clock stands at `at`, never past its end. */
export function anchorRead(anchor: Anchor, at: number) {
  if (!anchor.playing) return anchor.position;
  return Math.min(
    anchor.end,
    anchor.position + Math.max(0, at - anchor.at) * anchor.rate,
  );
}

/** While the TTML Creator is open over Spotify Soloist, Lyssna plays the
 * song itself from its decoded copy: Soloist cannot change speed, and its
 * position arrives only when the song or play state changes. */
export function useCreatorPlayer({
  active,
  uri,
  durationMs,
  volume,
  onFailed,
}: {
  active: boolean;
  uri?: string;
  durationMs: number;
  volume: number;
  onFailed: (error: unknown) => void;
}): CreatorPlayer {
  const [anchor, setAnchorState] = useState<Anchor>({
    position: 0,
    at: 0,
    playing: false,
    rate: 1,
    end: durationMs,
  });
  const anchorRef = useRef(anchor);
  const setAnchor = useCallback((next: Anchor) => {
    anchorRef.current = next;
    setAnchorState(next);
  }, []);
  const asked = useRef(0);
  const live = useRef({ uri, durationMs, volume, onFailed });
  live.current = { uri, durationMs, volume, onFailed };

  const read = useCallback(
    (at?: number) => anchorRead(anchorRef.current, at ?? performance.now()),
    [],
  );
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (!anchor.playing) return;
    let frame = 0;
    const tick = (time: number) => {
      setNow(time);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [anchor]);

  const start = useCallback(
    (from: number, rate: number, to?: number) => {
      const { uri, durationMs, volume } = live.current;
      if (!uri) return;
      const position = Math.max(0, Math.min(from, durationMs));
      const end = Math.min(to ?? durationMs, durationMs);
      const request = ++asked.current;
      setAnchor({ position, at: performance.now(), playing: false, rate, end });
      rpc("creator_play", {
        uri,
        fromMs: position,
        toMs: to === undefined ? null : end,
        speed: Math.round(rate * 100),
        volume: Math.round(volume),
      })
        .then(() => {
          if (request !== asked.current) return;
          setAnchor({
            position,
            at: performance.now(),
            playing: true,
            rate,
            end,
          });
        })
        .catch((error) => {
          if (request !== asked.current) return;
          live.current.onFailed(error);
        });
    },
    [setAnchor],
  );

  const pause = useCallback(() => {
    asked.current++;
    const at = performance.now();
    setAnchor({
      ...anchorRef.current,
      position: anchorRead(anchorRef.current, at),
      at,
      playing: false,
      end: live.current.durationMs,
    });
    void rpc("stop_clip").catch(() => undefined);
  }, [setAnchor]);

  const play = useCallback(
    (from?: number, to?: number) => {
      const current = anchorRef.current;
      let at = from ?? read();
      if (from === undefined && at >= live.current.durationMs - 50) at = 0;
      start(at, current.rate, to);
    },
    [read, start],
  );

  const toggle = useCallback(() => {
    if (anchorRef.current.playing) pause();
    else play();
  }, [pause, play]);

  const seek = useCallback(
    (ms: number) => {
      const current = anchorRef.current;
      if (current.playing) start(ms, current.rate);
      else
        setAnchor({
          ...current,
          position: Math.max(0, Math.min(ms, live.current.durationMs)),
          at: performance.now(),
          end: live.current.durationMs,
        });
    },
    [start, setAnchor],
  );

  const setRate = useCallback(
    (rate: number) => {
      const current = anchorRef.current;
      if (current.playing) start(read(), rate, current.end);
      else setAnchor({ ...current, rate });
    },
    [read, start, setAnchor],
  );

  const setVolume = useCallback((percent: number) => {
    void rpc("creator_volume", { volume: Math.round(percent) }).catch(
      () => undefined,
    );
  }, []);

  // Stops where the song or the asked span ends, as the audio does.
  useEffect(() => {
    if (!anchor.playing) return;
    const left =
      (anchor.end - anchorRead(anchor, performance.now())) / anchor.rate;
    const timer = window.setTimeout(
      () => {
        if (anchorRef.current !== anchor) return;
        asked.current++;
        setAnchor({
          ...anchor,
          position: anchor.end,
          at: performance.now(),
          playing: false,
          end: live.current.durationMs,
        });
      },
      Math.max(0, left),
    );
    return () => clearTimeout(timer);
  }, [anchor, setAnchor]);

  useEffect(() => {
    if (active) return;
    asked.current++;
    if (anchorRef.current.playing)
      setAnchor({ ...anchorRef.current, playing: false });
    void rpc("stop_clip").catch(() => undefined);
  }, [active, setAnchor]);
  // Deferred so React's development remount does not cut the audio.
  const stopping = useRef(0);
  useEffect(() => {
    clearTimeout(stopping.current);
    return () => {
      stopping.current = window.setTimeout(
        () => void rpc("stop_clip").catch(() => undefined),
      );
    };
  }, []);

  return useMemo(
    () => ({
      active,
      playing: anchor.playing,
      position: anchorRead(anchor, Math.max(now, anchor.at)),
      rate: anchor.rate,
      anchor,
      read,
      play,
      pause,
      toggle,
      seek,
      setRate,
      setVolume,
    }),
    [active, anchor, now, read, play, pause, toggle, seek, setRate, setVolume],
  );
}

/** Lets App's player commands and Space reach the creator's player while it
 * stands in for Soloist. Returns true when the request was handled. */
export const creatorTransport: {
  current: null | {
    handle: (request: PlayerRequest) => boolean;
    toggle: () => void;
  };
  passing: boolean;
} = { current: null, passing: false };

/** Sends a request past the creator's player to the real one. */
export function direct(
  onPlayer: (request: PlayerRequest) => void,
  request: PlayerRequest,
) {
  creatorTransport.passing = true;
  try {
    onPlayer(request);
  } finally {
    creatorTransport.passing = false;
  }
}
