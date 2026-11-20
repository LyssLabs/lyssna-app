import { useEffect, useRef } from "react";
import { listenToLevels } from "./level-bars";

const POINTS = 96;
const TRAILS = 3;
// Frames between each trailing ring's shape and the one in front of it.
const TRAIL_LAG = 5;
const WAVES = [
  { k: 2, speed: 0.55, weight: 1 },
  { k: 3, speed: -0.8, weight: 0.7 },
  { k: 5, speed: 1.3, weight: 0.35 },
];

function outline(radius: number, offsets: number[]) {
  let path = "";
  for (let index = 0; index <= POINTS; index++) {
    const angle = (index / POINTS) * Math.PI * 2;
    const r = radius * (1 + offsets[index % POINTS]);
    const x = 50 + r * Math.cos(angle);
    const y = 50 + r * Math.sin(angle);
    path += `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`;
  }
  return `${path}Z`;
}

/** A ring that bends and swells with the DJ's voice. */
export function DJVoiceRing({ live }: { live: boolean }) {
  const svg = useRef<SVGSVGElement>(null);
  useEffect(() => {
    const paths = Array.from(svg.current?.querySelectorAll("path") ?? []);
    const circle = outline(34, new Array(POINTS).fill(0));
    if (!live) {
      paths.forEach((path) => path.setAttribute("d", circle));
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
    const history: string[] = [];
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
      history.unshift(outline(34 * (1 + level * 0.08), offsets));
      history.length = Math.min(history.length, TRAILS * TRAIL_LAG + 1);
      paths.forEach((path, index) =>
        path.setAttribute(
          "d",
          history[
            Math.min(history.length - 1, (paths.length - 1 - index) * TRAIL_LAG)
          ],
        ),
      );
      frame = requestAnimationFrame(draw);
    });
    return () => {
      stop();
      cancelAnimationFrame(frame);
    };
  }, [live]);
  return (
    <svg
      ref={svg}
      className="dj-voice-ring"
      viewBox="0 0 100 100"
      aria-hidden="true"
    >
      {Array.from({ length: TRAILS + 1 }, (_, index) => TRAILS - index).map(
        (index) => (
          <path
            key={index}
            className={index ? "trail" : "ring"}
            style={index ? { opacity: 0.5 - index * 0.13 } : undefined}
          />
        ),
      )}
    </svg>
  );
}
