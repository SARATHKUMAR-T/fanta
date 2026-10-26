/**
 * Fanta - an original chibi companion drawn in SVG, so she stays crisp at any
 * size and can change expression instantly.
 */
export type Mood =
  | "idle"
  | "happy"
  | "talking"
  | "thinking"
  | "focus"
  | "alert"
  | "sleepy"
  | "surprised"
  | "proud"
  | "love"
  | "sad"
  | "wink"
  | "listening"
  | "worried"
  | "excited"
  | "shy"
  | "caring"
  | "pout"
  | "laugh";

const HAIR_DARK = "#e2572b";
/** The hoodie is the theme colour, oklch(28.3% 0.141 291.089), lit from above. */
const HOODIE_TOP = "#4c1d95";
const HOODIE = "#2e1065";
const HOODIE_EDGE = "#1f0a4d";
const LINE = "#4a2a1c";
const EY = 106; // eye center line

type EyeStyle = "open" | "closed-happy" | "closed-sleepy" | "heart";

function eyeStyle(mood: Mood, left: boolean): EyeStyle {
  switch (mood) {
    case "happy":
    case "proud":
    case "laugh":
      return "closed-happy";
    case "wink":
      return left ? "open" : "closed-happy";
    case "sleepy":
      return "closed-sleepy";
    case "love":
      return "heart";
    default:
      return "open";
  }
}

/** Where the pupils look, and how wide the eyes open. */
function gaze(mood: Mood): { dx: number; dy: number; wide: number; pupil: number } {
  switch (mood) {
    case "thinking":
      return { dx: 3, dy: -3.5, wide: 1, pupil: 1 };
    case "listening":
      return { dx: -2.5, dy: 2.5, wide: 1, pupil: 1 };
    case "sad":
      return { dx: 0, dy: 3, wide: 1, pupil: 1.02 };
    case "surprised":
      return { dx: 0, dy: 0, wide: 1.15, pupil: 0.68 };
    case "alert":
      return { dx: 0, dy: 0, wide: 1.1, pupil: 0.82 };
    case "worried":
      return { dx: -1.5, dy: 1, wide: 1, pupil: 0.9 };
    case "excited":
      return { dx: 0, dy: -1, wide: 1.12, pupil: 1.08 };
    case "shy":
      return { dx: -3.5, dy: 3.5, wide: 0.96, pupil: 1 };
    case "caring":
      return { dx: 0, dy: 1.5, wide: 1, pupil: 1.06 };
    case "pout":
      return { dx: 3.5, dy: 0.5, wide: 1, pupil: 0.95 };
    default:
      return { dx: 0, dy: 0, wide: 1, pupil: 1 };
  }
}

/**
 * Upper-lid line for moods that show emotion through the eyelids, as heights
 * (relative to the eye centre) at the outer and inner corners. The eye above
 * the line is covered with skin, so the eye itself changes shape.
 */
function lid(mood: Mood): { outer: number; inner: number } | null {
  switch (mood) {
    case "focus":
      return { outer: -5, inner: -6 }; // flat, half-closed: determined
    case "thinking":
      return { outer: -9, inner: -8 }; // slight squint
    case "worried":
      return { outer: -5, inner: -12 }; // inner corners pulled up
    case "sad":
      return { outer: -1, inner: -11 }; // drooping outward
    case "caring":
      return { outer: -8, inner: -11 }; // soft, a little sorry for you
    case "pout":
      return { outer: -9, inner: -4 }; // inner corners down: grumpy
    default:
      return null;
  }
}

/** A four-point star, for sparkling eyes. */
function star(cx: number, cy: number, r: number) {
  const k = r * 0.22;
  return `M${cx} ${cy - r} Q${cx + k} ${cy - k} ${cx + r} ${cy} Q${cx + k} ${cy + k} ${cx} ${cy + r} Q${cx - k} ${cy + k} ${cx - r} ${cy} Q${cx - k} ${cy - k} ${cx} ${cy - r} Z`;
}

function Eye({ x, mood, look }: { x: number; mood: Mood; look: { x: number; y: number } }) {
  const left = x < 100;
  const style = eyeStyle(mood, left);
  if (style === "closed-happy") {
    return <path d={`M${x - 12} ${EY + 4} Q${x} ${EY - 10} ${x + 12} ${EY + 4}`} fill="none" stroke={LINE} strokeWidth={3.6} strokeLinecap="round" />;
  }
  if (style === "closed-sleepy") {
    return (
      <g>
        <path d={`M${x - 12} ${EY} Q${x} ${EY + 8} ${x + 12} ${EY}`} fill="none" stroke={LINE} strokeWidth={3.2} strokeLinecap="round" />
        <path d={left ? `M${x - 12} ${EY} l-3 -2` : `M${x + 12} ${EY} l3 -2`} stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />
      </g>
    );
  }
  if (style === "heart") {
    return (
      <path
        className="heart-eye"
        d={`M${x} ${EY + 10} C${x - 16} ${EY} ${x - 12} ${EY - 12} ${x} ${EY - 5} C${x + 12} ${EY - 12} ${x + 16} ${EY} ${x} ${EY + 10} Z`}
        fill="#ff5a6e"
        stroke="#c93a4f"
        strokeWidth={1.5}
        style={{ transformOrigin: `${x}px ${EY}px` }}
      />
    );
  }
  const { dx, dy, wide: k, pupil } = gaze(mood);
  const l = lid(mood);
  const starry = mood === "surprised" || mood === "excited";
  // Lid corners: the outer corner is on the left for the left eye, mirrored for the right.
  const [ly1, ly2] = l ? (left ? [EY + l.outer, EY + l.inner] : [EY + l.inner, EY + l.outer]) : [0, 0];
  return (
    <g className="eye" style={{ transformOrigin: `${x}px ${EY}px` }}>
      <ellipse cx={x} cy={EY} rx={13 * k} ry={15.5 * k} fill="#fff" />
      <g transform={`translate(${look.x} ${look.y})`}>
      <g className="pupils">
        <ellipse cx={x + dx} cy={EY + 1.5 + dy} rx={10.6 * pupil} ry={13.6 * pupil} fill="url(#iris)" />
        <ellipse cx={x + dx} cy={EY + 2.5 + dy} rx={5 * pupil} ry={6.6 * pupil} fill="#3a1a0e" />
        <path
          d={`M${x - 10.6 * pupil + dx} ${EY - 4 + dy} Q${x + dx} ${EY - 13 * pupil + dy} ${x + 10.6 * pupil + dx} ${EY - 4 + dy}`}
          fill="#4a1d0c"
          opacity={0.45}
        />
        {starry ? (
          <>
            <path className="eye-star" d={star(x - 3 + dx, EY - 4 + dy, 8)} fill="#fff" style={{ transformOrigin: `${x - 3 + dx}px ${EY - 4 + dy}px` }} />
            <path d={star(x + 5 + dx, EY + 6 + dy, 3.5)} fill="#fff" opacity={0.9} />
          </>
        ) : (
          <circle cx={x - 4 + dx} cy={EY - 5 + dy} r={4} fill="#fff" />
        )}
        <circle cx={x + 4.5 + dx} cy={EY + 7 + dy} r={mood === "sad" ? 2.8 : 2} fill="#fff" opacity={0.9} className={mood === "sad" ? "wobble" : undefined} />
        <circle cx={x + 5.5 + dx} cy={EY - 6 + dy} r={1.3} fill="#fff" opacity={0.8} />
      </g>
      </g>
      {l ? (
        <>
          {/* skin-coloured lid covering the top of the eye, with the lash along its edge */}
          <path d={`M${x - 17} ${EY - 22} L${x + 17} ${EY - 22} L${x + 17} ${ly2} L${x - 17} ${ly1} Z`} fill="url(#skin)" />
          <path d={`M${x - 13.5} ${ly1 + (left ? 1 : 0)} L${x + 13.5} ${ly2 + (left ? 0 : 1)}`} fill="none" stroke={LINE} strokeWidth={3.8} strokeLinecap="round" />
        </>
      ) : (
        <>
          {/* upper lash: an even arc with a small flick at the outer corner */}
          <path d={`M${x - 13.5} ${EY - 5} Q${x} ${EY - 20} ${x + 13.5} ${EY - 5}`} fill="none" stroke={LINE} strokeWidth={3.8} strokeLinecap="round" />
          <path d={left ? `M${x - 13.5} ${EY - 5} q-3 -1 -5 -4` : `M${x + 13.5} ${EY - 5} q3 -1 5 -4`} fill="none" stroke={LINE} strokeWidth={2.6} strokeLinecap="round" />
        </>
      )}
      <path d={`M${x - 7} ${EY + 15.5} Q${x} ${EY + 17.5} ${x + 7} ${EY + 15.5}`} fill="none" stroke={LINE} strokeWidth={1.3} strokeLinecap="round" opacity={0.55} />
    </g>
  );
}

/** Eyebrows carry most of the emotion; drawn over the bangs, half-transparent, anime style. */
function Brows({ mood }: { mood: Mood }) {
  // [outer y, middle y, inner y] for the left brow; the right brow mirrors it.
  const shape: Record<Mood, [number, number, number]> = {
    idle: [85, 80, 84],
    happy: [83, 77, 82],
    proud: [83, 77, 82],
    love: [83, 77, 82],
    wink: [83, 77, 82],
    talking: [84, 79, 83],
    surprised: [79, 72, 77],
    alert: [80, 74, 78],
    sad: [86, 81, 78],
    worried: [85, 80, 77],
    thinking: [82, 79, 85],
    focus: [83, 81, 85],
    listening: [84, 78, 82],
    sleepy: [87, 84, 86],
    excited: [80, 73, 78],
    shy: [85, 80, 79],
    caring: [85, 80, 78],
    pout: [82, 82, 88],
    laugh: [83, 76, 81],
  };
  const [o, m, i] = shape[mood].map((y) => y - 4);
  return (
    <g className="brows" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" opacity={0.75}>
      <path d={`M65 ${o} Q76 ${m} 87 ${i}`} />
      <path d={`M135 ${o} Q124 ${m} 113 ${i}`} />
    </g>
  );
}

function Mouth({ mood }: { mood: Mood }) {
  switch (mood) {
    case "happy":
    case "proud":
      return (
        <g>
          <path d="M92 129 Q100 141 108 129 Z" fill="#c4473b" />
          <path d="M95 133.5 Q100 137 105 133.5" fill="#ff8f86" />
        </g>
      );
    case "excited":
    case "laugh":
      return (
        <g className={mood === "laugh" ? "giggle" : undefined}>
          <path d="M90 128 Q100 145 110 128 Z" fill="#c4473b" />
          <path d="M94 135 Q100 140.5 106 135" fill="#ff8f86" />
        </g>
      );
    case "shy":
      return <path d="M95.5 131.5 q2.25 1.8 4.5 0 q2.25 1.8 4.5 0" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    case "caring":
      return <path d="M94 130.5 Q100 135 106 130.5" fill="none" stroke={LINE} strokeWidth={2.3} strokeLinecap="round" />;
    case "pout":
      return <path d="M97 134 Q100 130.8 103 134" fill="none" stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />;
    case "wink":
      return (
        <g>
          <path d="M93.5 129.5 Q100 137 106.5 129.5" fill="none" stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />
          <path d="M101 133 q3 4.5 5.5 0" fill="#ff7f86" />
        </g>
      );
    case "love":
      return <path d="M92.5 130.5 q3.75 3.5 7.5 0 q3.75 3.5 7.5 0" fill="none" stroke={LINE} strokeWidth={2.3} strokeLinecap="round" />;
    case "talking":
      return <ellipse className="talk" cx={100} cy={132} rx={4.5} ry={4} fill="#c4473b" />;
    case "alert":
      return <ellipse cx={100} cy={133} rx={3.6} ry={4.4} fill="#c4473b" />;
    case "surprised":
      return <ellipse cx={100} cy={134} rx={4.6} ry={5.8} fill="#c4473b" />;
    case "sad":
      return <path d="M94.5 135 Q100 129.5 105.5 135" fill="none" stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />;
    case "worried":
      return <path d="M93.5 133 q3.25 -2.5 6.5 0 q3.25 2.5 6.5 0" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    case "thinking":
      return <path d="M97 132 Q101 130 105 132.5" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    case "focus":
      return <path d="M96 132 L104 132" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    case "sleepy":
      return <ellipse cx={100} cy={133} rx={2.6} ry={2.2} fill="#c4473b" />;
    case "listening":
      return <path d="M96 131 Q100 134 104 131" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    default:
      return <path d="M94.5 130 Q100 135.5 105.5 130" fill="none" stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />;
  }
}

function Sparkle({ x, y, s, delay }: { x: number; y: number; s: number; delay: number }) {
  return (
    <path
      className="sparkle"
      style={{ animationDelay: `${delay}s`, transformOrigin: `${x}px ${y}px` }}
      d={`M${x} ${y - s} Q${x + s * 0.18} ${y - s * 0.18} ${x + s} ${y} Q${x + s * 0.18} ${y + s * 0.18} ${x} ${y + s} Q${x - s * 0.18} ${y + s * 0.18} ${x - s} ${y} Q${x - s * 0.18} ${y - s * 0.18} ${x} ${y - s} Z`}
      fill="#ffcf4a"
    />
  );
}

function FloatHeart({ x, y, delay }: { x: number; y: number; delay: number }) {
  return (
    <path
      className="float-heart"
      style={{ animationDelay: `${delay}s` }}
      d={`M${x} ${y + 7} C${x - 11} ${y} ${x - 8} ${y - 8} ${x} ${y - 3} C${x + 8} ${y - 8} ${x + 11} ${y} ${x} ${y + 7} Z`}
      fill="#ff6b81"
    />
  );
}

/** The little cross-popping vein of anime annoyance. */
function Vein() {
  return (
    <g className="vein" fill="none" stroke="#e8453c" strokeWidth={2.6} strokeLinecap="round" transform="translate(150 56)">
      <path d="M-7 -2 q5 0 5 -5" />
      <path d="M2 -7 q0 5 5 5" />
      <path d="M7 2 q-5 0 -5 5" />
      <path d="M-2 7 q0 -5 -5 -5" />
    </g>
  );
}

/**
 * `face` crops to the head, for avatars. `look` nudges the pupils (about ±3)
 * toward something, like the mouse pointer.
 */
export default function Character({
  mood = "idle",
  size = 140,
  face = false,
  look = { x: 0, y: 0 },
}: {
  mood?: Mood;
  size?: number;
  face?: boolean;
  look?: { x: number; y: number };
}) {
  const blush =
    mood === "shy" ? 0.8 : mood === "love" || mood === "proud" || mood === "wink" || mood === "pout" ? 0.5 : mood === "excited" || mood === "caring" ? 0.42 : mood === "sad" ? 0.15 : 0.3;
  return (
    <svg
      className={`character mood-${mood}`}
      viewBox={face ? "34 22 132 132" : "0 0 200 220"}
      width={size}
      height={face ? size : size * 1.1}
      aria-label={`Fanta (${mood})`}
    >
      <defs>
        <linearGradient id="hair" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#ffb054" />
          <stop offset="0.55" stopColor="#ff8a3d" />
          <stop offset="1" stopColor={HAIR_DARK} />
        </linearGradient>
        <radialGradient id="iris" cx="0.5" cy="0.75" r="0.75">
          <stop offset="0" stopColor="#f4a63a" />
          <stop offset="0.55" stopColor="#b8571f" />
          <stop offset="1" stopColor="#5a2614" />
        </radialGradient>
        <linearGradient id="hoodie" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={HOODIE_TOP} />
          <stop offset="1" stopColor={HOODIE} />
        </linearGradient>
        <linearGradient id="skin" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#ffeadb" />
          <stop offset="1" stopColor="#ffdcc8" />
        </linearGradient>
      </defs>

      <ellipse cx={100} cy={214} rx={46} ry={5} fill="#000" opacity={0.12} className="shadow" />

      <g className="body">
        {/* hoodie */}
        <path d="M50 220 C50 182 70 164 100 162 C130 164 150 182 150 220 Z" fill="url(#hoodie)" stroke={HOODIE_EDGE} strokeWidth={1.5} />
        <path d="M74 170 C82 182 118 182 126 170" fill="none" stroke="#7c3aed" strokeWidth={2} />
        <path d="M90 178 L88 200" stroke="#ff8a3d" strokeWidth={2.4} strokeLinecap="round" />
        <path d="M110 178 L112 200" stroke="#ff8a3d" strokeWidth={2.4} strokeLinecap="round" />
        <circle cx={88} cy={202} r={2.6} fill="#e2572b" />
        <circle cx={112} cy={202} r={2.6} fill="#e2572b" />
        {/* little orange-slice pin */}
        <g transform="translate(128 194)">
          <circle r={6.5} fill="#ffb054" stroke="#e2572b" strokeWidth={1.5} />
          <path d="M0 0 L0 -6 M0 0 L5.2 3 M0 0 L-5.2 3" stroke="#fff6e8" strokeWidth={1.3} />
        </g>
        <rect x={93} y={150} width={14} height={16} rx={6} fill="#ffd9c4" />
      </g>

      <g className="head">
        {/* back hair */}
        <path
          d="M40 104 C34 52 68 26 100 26 C132 26 166 52 160 104 C162 134 158 156 150 172 C144 160 140 150 139 138 C141 124 141 112 139 104 L61 104 C59 112 59 124 61 138 C60 150 56 160 50 172 C42 156 38 134 40 104 Z"
          fill="url(#hair)"
        />
        {/* face */}
        <path d="M50 100 C50 64 74 46 100 46 C126 46 150 64 150 100 C150 132 128 153 100 155 C72 153 50 132 50 100 Z" fill="url(#skin)" />
        <ellipse cx={69} cy={126} rx={9} ry={4.5} fill="#ff7f73" opacity={blush} />
        <ellipse cx={131} cy={126} rx={9} ry={4.5} fill="#ff7f73" opacity={blush} />
        {(mood === "love" || mood === "proud" || mood === "shy") && (
          <g stroke="#ff7f73" strokeWidth={1.2} strokeLinecap="round" opacity={0.7}>
            <path d="M64 124 l-2 4 M69 124 l-2 4 M74 124 l-2 4" />
            <path d="M126 124 l-2 4 M131 124 l-2 4 M136 124 l-2 4" />
          </g>
        )}

        {/* keyed by mood so each new expression eases in instead of snapping */}
        <g key={mood} className="face-pop">
          <g className="eyes">
            <Eye x={77} mood={mood} look={look} />
            <Eye x={123} mood={mood} look={look} />
          </g>
          <Mouth mood={mood} />
        </g>

        {/* bangs */}
        <path
          d="M48 96 C44 58 70 36 100 36 C130 36 156 58 152 96 C146 84 140 74 132 68 C131 79 125 88 117 92 C118 82 115 73 108 67 C104 79 96 88 85 92 C88 82 87 73 83 67 C75 75 64 86 48 96 Z"
          fill="url(#hair)"
        />
        <path d="M100 52 C97 66 98 78 101 88 C104 78 105 66 100 52 Z" fill="url(#hair)" />
        <path d="M83 67 C86 74 86 82 85 90" fill="none" stroke={HAIR_DARK} strokeWidth={1.3} opacity={0.5} />
        <path d="M108 67 C112 74 115 82 116 90" fill="none" stroke={HAIR_DARK} strokeWidth={1.3} opacity={0.5} />
        <path d="M70 52 C80 46 92 44 104 45" fill="none" stroke="#fff3d6" strokeWidth={3} strokeLinecap="round" opacity={0.7} />
        <Brows mood={mood} />
        {/* side locks */}
        <path d="M50 92 C44 114 46 136 56 154 C60 136 60 116 62 98 Z" fill="url(#hair)" />
        <path d="M150 92 C156 114 154 136 144 154 C140 136 140 116 138 98 Z" fill="url(#hair)" />
        {/* hair clip */}
        <g transform="translate(136 70) rotate(-25)">
          <rect x={-9} y={-3} width={18} height={6} rx={3} fill="#fff6e8" stroke="#e2572b" strokeWidth={1.2} />
          <circle cx={9} cy={0} r={3.4} fill="#7fc8a9" />
        </g>
        <path className="ahoge" d="M100 37 C94 22 102 10 114 13 C105 17 102 25 104 37 Z" fill="url(#hair)" />

        {mood === "focus" && (
          <g className="headphones">
            <path d="M44 98 C40 36 160 36 156 98" fill="none" stroke="#3b3a4a" strokeWidth={7} strokeLinecap="round" />
            <rect x={34} y={88} width={17} height={30} rx={8} fill="#3b3a4a" />
            <rect x={149} y={88} width={17} height={30} rx={8} fill="#3b3a4a" />
            <rect x={38} y={93} width={6} height={20} rx={3} fill="#ff8a3d" />
            <rect x={156} y={93} width={6} height={20} rx={3} fill="#ff8a3d" />
          </g>
        )}
        {mood === "sad" && <path className="tear" d="M66 118 q-4 7 0 9.5 q4 -2.5 0 -9.5 Z" fill="#8fd0ff" stroke="#5aa9e6" strokeWidth={0.8} />}
        {mood === "pout" && <ellipse className="puff" cx={131} cy={124} rx={10} ry={7} fill="#ffb3a6" opacity={0.55} />}
        {mood === "laugh" && <path className="joy-tear" d="M137 104 q4 5 1 8 q-4 -1 -1 -8 Z" fill="#a8dcff" stroke="#6bb6ea" strokeWidth={0.8} />}
        {mood === "worried" && <path className="sweat" d="M146 74 q-6 10 0 13 q6 -3 0 -13 Z" fill="#a8dcff" stroke="#6bb6ea" strokeWidth={1} />}
        {mood === "listening" && (
          <g className="listen-waves" fill="none" stroke="#ff8a3d" strokeWidth={2} strokeLinecap="round">
            <path d="M28 96 q-5 8 0 16" />
            <path d="M21 91 q-8 13 0 26" opacity={0.6} />
          </g>
        )}
      </g>

      {mood === "thinking" && (
        <g className="thought" fill="#fff" stroke="#e9cdb2" strokeWidth={1.5}>
          <circle cx={160} cy={46} r={4} />
          <circle cx={172} cy={32} r={6} />
          <circle cx={186} cy={14} r={8} />
        </g>
      )}
      {(mood === "alert" || mood === "surprised") && (
        <g className="alert-mark">
          <path d="M168 20 L166 46" stroke="#e2572b" strokeWidth={6} strokeLinecap="round" />
          <circle cx={165} cy={57} r={3.6} fill="#e2572b" />
          {mood === "surprised" && <path d="M181 26 L177 44" stroke="#ff8a3d" strokeWidth={4.5} strokeLinecap="round" />}
        </g>
      )}
      {mood === "sleepy" && (
        <text x={150} y={40} className="zzz" fill="#b07a5a" fontSize={18} fontWeight={800}>
          z
        </text>
      )}
      {mood === "proud" && (
        <g>
          <Sparkle x={30} y={52} s={9} delay={0} />
          <Sparkle x={172} y={40} s={11} delay={0.3} />
          <Sparkle x={184} y={110} s={7} delay={0.6} />
          <Sparkle x={20} y={110} s={6} delay={0.45} />
        </g>
      )}
      {mood === "love" && (
        <g>
          <FloatHeart x={162} y={56} delay={0} />
          <FloatHeart x={36} y={64} delay={0.5} />
          <FloatHeart x={178} y={90} delay={1} />
        </g>
      )}
      {mood === "wink" && <Sparkle x={152} y={92} s={7} delay={0} />}
      {mood === "pout" && <Vein />}
      {mood === "excited" && (
        <g>
          <Sparkle x={26} y={60} s={10} delay={0} />
          <Sparkle x={176} y={36} s={12} delay={0.2} />
          <Sparkle x={186} y={96} s={7} delay={0.4} />
          <Sparkle x={16} y={118} s={7} delay={0.55} />
        </g>
      )}
      {mood === "caring" && <FloatHeart x={166} y={60} delay={0} />}
    </svg>
  );
}
