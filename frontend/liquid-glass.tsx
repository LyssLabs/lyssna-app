import { memo } from "react";
import { Ambience } from "./ambience";
import { useMotionCover } from "./motion-cover";
import type { Track } from "./types";

/**
 * The playing track's warped, moving artwork behind the whole window, the
 * same ambience the lyrics page uses but warped harder and faster, so the
 * glass panels have moving colour to refract. It animates while playing and
 * holds still when paused or when motion is reduced.
 */
export const GlassBackdrop = memo(function GlassBackdrop({
  track,
  playing,
  reduceMotion,
  animatedCovers,
}: {
  track?: Track;
  playing: boolean;
  reduceMotion: boolean;
  animatedCovers: boolean;
}) {
  const motion = useMotionCover(track, animatedCovers && !reduceMotion);
  return (
    <div className="glass-backdrop" aria-hidden="true">
      <Ambience
        track={track}
        animate={playing}
        reduceMotion={reduceMotion}
        warp={1.6}
        speed={2.4}
        saturation={1.5}
        shade={false}
        followCanvas="window"
        motion={motion}
      />
      <div className="glass-backdrop-shade" />
    </div>
  );
});
