import { useEffect, useRef } from "react";
import { listenToLevels } from "./level-bars";

const POINTS = 96;
const RADIUS = 26;
// Where the DJ cover draws its two rings.
const RING = { x: 49, y: 51 };
const ECHO = { x: 54, y: 48 };
// Frames the echo's shape trails the green ring's.
const ECHO_LAG = 8;
const WAVES = [
  { k: 2, speed: 0.55, weight: 1 },
  { k: 3, speed: -0.8, weight: 0.7 },
  { k: 5, speed: 1.3, weight: 0.35 },
];

function outline(
  centre: { x: number; y: number },
  radius: number,
  offsets: number[],
) {
  let path = "";
  for (let index = 0; index <= POINTS; index++) {
    const angle = (index / POINTS) * Math.PI * 2;
    const r = radius * (1 + offsets[index % POINTS]);
    const x = centre.x + r * Math.cos(angle);
    const y = centre.y + r * Math.sin(angle);
    path += `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`;
  }
  return `${path}Z`;
}

/** The DJ cover's rings, bending and swelling with the DJ's voice. */
export function DJVoiceRing({ live }: { live: boolean }) {
  const echo = useRef<SVGPathElement>(null);
  const ring = useRef<SVGPathElement>(null);
  useEffect(() => {
    const round = new Array(POINTS).fill(0);
    if (!live) {
      echo.current?.setAttribute("d", outline(ECHO, RADIUS, round));
      ring.current?.setAttribute("d", outline(RING, RADIUS, round));
      return;
    }
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
    const history: { offsets: number[]; radius: number }[] = [];
    let frame = requestAnimationFrame(function draw(now) {
      const step = Math.min(0.05, (now - last) / 1000);
      last = now;
      level += (target - level) * (target > level ? 0.45 : 0.12);
      time += step * (0.6 + level * 1.8);
      const bend = 0.025 + level * 0.11;
      const offsets = Array.from({ length: POINTS }, (_, index) => {
        const angle = (index / POINTS) * Math.PI * 2;
        return (
          (bend *
            WAVES.reduce(
              (sum, wave) =>
                sum +
                wave.weight * Math.sin(wave.k * angle + time * wave.speed * 3),
              0,
            )) /
          2
        );
      });
      history.unshift({ offsets, radius: RADIUS * (1 + level * 0.1) });
      history.length = Math.min(history.length, ECHO_LAG + 1);
      const behind = history[history.length - 1];
      ring.current?.setAttribute(
        "d",
        outline(RING, history[0].radius, offsets),
      );
      echo.current?.setAttribute(
        "d",
        outline(ECHO, behind.radius, behind.offsets),
      );
      frame = requestAnimationFrame(draw);
    });
    return () => {
      stop();
      cancelAnimationFrame(frame);
    };
  }, [live]);
  return (
    <svg className="dj-voice-ring" viewBox="12 12 80 80" aria-hidden="true">
      <path ref={echo} className="echo" />
      <path ref={ring} className="ring" />
    </svg>
  );
}
