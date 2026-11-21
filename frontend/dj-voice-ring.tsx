import { useEffect, useRef } from "react";
import { listenToLevels } from "./level-bars";

const POINTS = 120;
const RADIUS = 26;
const CENTRE = { x: 50, y: 50 };
// Spotify's DJ only moves the ring's upper right, at about 1:30 on a clock
// face (SVG angles run clockwise from 3 o'clock).
const FOCUS = -Math.PI / 4;
// Frames each blue copy trails the green ring by, nearest first.
const ECHO_LAGS = [5, 10];

/** How much of the voice reaches each part of the ring: all of it at the
 *  upper right, fading to none by the left and bottom. */
function reach(angle: number) {
  const facing = Math.cos(angle - FOCUS);
  return facing > 0 ? facing ** 2.5 : 0;
}

function outline(radius: (angle: number) => number) {
  let path = "";
  for (let index = 0; index <= POINTS; index++) {
    const angle = (index / POINTS) * Math.PI * 2;
    const r = radius(angle);
    const x = CENTRE.x + r * Math.cos(angle);
    const y = CENTRE.y + r * Math.sin(angle);
    path += `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`;
  }
  return `${path}Z`;
}

/** The ring's radius at each angle for a voice level and moment: the upper
 *  right swells into a soft corner with a pinch that drifts along it. */
function shape(level: number, time: number, spread = 0) {
  const pinchAt = FOCUS - 0.3 + Math.sin(time * 0.9) * 0.2;
  const swell = 0.2 + Math.sin(time * 1.7) * 0.04;
  return (angle: number) => {
    const fromPinch = Math.atan2(
      Math.sin(angle - pinchAt),
      Math.cos(angle - pinchAt),
    );
    const pinch = Math.exp(-((fromPinch / 0.28) ** 2));
    return (
      RADIUS *
      (1 +
        level * reach(angle) * (swell + spread - pinch * (swell * 0.7 + 0.04)))
    );
  };
}

const ROUND = outline(() => RADIUS);

/** The DJ ring: a still green circle whose upper right bends with the DJ's
 *  voice, two blue copies trailing behind the bend. */
export function DJVoiceRing({ live }: { live: boolean }) {
  const ring = useRef<SVGPathElement>(null);
  const echoes = useRef<(SVGPathElement | null)[]>([]);
  useEffect(() => {
    const paths = [ring.current, ...echoes.current];
    for (const path of paths) path?.setAttribute("d", ROUND);
    if (!live) return;
    let target = 0;
    let peak = 0.4;
    const stop = listenToLevels(([low, mid, high]) => {
      const voice = low * 0.25 + mid * 0.55 + high * 0.2;
      // Voices vary in loudness; follow this one's recent peak.
      peak = Math.max(voice, peak * 0.996, 0.4);
      target = Math.min(1, Math.max(0, (voice / peak - 0.3) / 0.7));
    });
    let level = 0;
    let time = 0;
    let last = performance.now();
    const history: { level: number; time: number }[] = [];
    let frame = requestAnimationFrame(function draw(now) {
      const step = Math.min(0.05, (now - last) / 1000);
      last = now;
      level += (target - level) * (target > level ? 0.35 : 0.1);
      time += step * (0.8 + level * 1.6);
      history.unshift({ level, time });
      history.length = Math.min(history.length, ECHO_LAGS.at(-1)! + 1);
      const current = shape(level, time);
      ring.current?.setAttribute("d", outline(current));
      // A copy never dips inside the green ring, or it would show through
      // the ring's hole while the voice rises.
      ECHO_LAGS.forEach((lag, index) => {
        const past = history[Math.min(lag, history.length - 1)];
        const then = shape(past.level, past.time, 0.1 * (index + 1));
        echoes.current[index]?.setAttribute(
          "d",
          outline((angle) => Math.max(then(angle), current(angle))),
        );
      });
      frame = requestAnimationFrame(draw);
    });
    return () => {
      stop();
      cancelAnimationFrame(frame);
    };
  }, [live]);
  return (
    <svg className="dj-voice-ring" viewBox="10 10 80 80" aria-hidden="true">
      {ECHO_LAGS.map((lag, index) => (
        <path
          key={lag}
          ref={(path) => {
            echoes.current[index] = path;
          }}
          className={`echo echo-${index + 1}`}
        />
      )).reverse()}
      <path ref={ring} className="ring" />
    </svg>
  );
}
