/**
 * Fanta - an original chibi companion drawn in SVG, so she stays crisp at any
 * size and can change expression instantly.
 */
export type Mood = "idle" | "happy" | "talking" | "thinking" | "focus" | "alert" | "sleepy";

const HAIR_DARK = "#e2572b";
const LINE = "#4a2a1c";

const EY = 106; // eye center line

function Eye({ x, mood }: { x: number; mood: Mood }) {
  const left = x < 100;
  // Closed, smiling eyes (^ ^) for happy; sleepy droops.
  if (mood === "happy") {
    return <path d={`M${x - 12} ${EY + 4} Q${x} ${EY - 10} ${x + 12} ${EY + 4}`} fill="none" stroke={LINE} strokeWidth={3.6} strokeLinecap="round" />;
  }
  if (mood === "sleepy") {
    return (
      <g>
        <path d={`M${x - 12} ${EY} Q${x} ${EY + 8} ${x + 12} ${EY}`} fill="none" stroke={LINE} strokeWidth={3.2} strokeLinecap="round" />
        <path d={left ? `M${x - 12} ${EY} l-3 -2` : `M${x + 12} ${EY} l3 -2`} stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />
      </g>
    );
  }
  const dy = mood === "thinking" ? -3.5 : 0;
  const dx = mood === "thinking" ? 3 : 0;
  const k = mood === "alert" ? 1.1 : 1;
  return (
    <g className="eye" style={{ transformOrigin: `${x}px ${EY}px` }}>
      <ellipse cx={x} cy={EY} rx={13 * k} ry={15.5 * k} fill="#fff" />
      <ellipse cx={x + dx} cy={EY + 1.5 + dy} rx={10.6} ry={13.6} fill="url(#iris)" />
      <ellipse cx={x + dx} cy={EY + 2.5 + dy} rx={5} ry={6.6} fill="#3a1a0e" />
      {/* iris rim shading under the lid */}
      <path d={`M${x - 10.6 + dx} ${EY - 4 + dy} Q${x + dx} ${EY - 13 + dy} ${x + 10.6 + dx} ${EY - 4 + dy}`} fill="#4a1d0c" opacity={0.45} />
      <circle cx={x - 4 + dx} cy={EY - 5 + dy} r={4} fill="#fff" />
      <circle cx={x + 4.5 + dx} cy={EY + 7 + dy} r={2} fill="#fff" opacity={0.9} />
      <circle cx={x + 5.5 + dx} cy={EY - 6 + dy} r={1.3} fill="#fff" opacity={0.8} />
      {/* upper lash: an even arc with a small flick at the outer corner; thin lower lash */}
      <path d={`M${x - 13.5} ${EY - 5} Q${x} ${EY - 20} ${x + 13.5} ${EY - 5}`} fill="none" stroke={LINE} strokeWidth={3.8} strokeLinecap="round" />
      <path d={left ? `M${x - 13.5} ${EY - 5} q-3 -1 -5 -4` : `M${x + 13.5} ${EY - 5} q3 -1 5 -4`} fill="none" stroke={LINE} strokeWidth={2.6} strokeLinecap="round" />
      <path
        d={`M${x - 7} ${EY + 15.5} Q${x} ${EY + 17.5} ${x + 7} ${EY + 15.5}`}
        fill="none"
        stroke={LINE}
        strokeWidth={1.3}
        strokeLinecap="round"
        opacity={0.55}
      />
    </g>
  );
}

function Mouth({ mood }: { mood: Mood }) {
  switch (mood) {
    case "happy":
      return (
        <g>
          <path d="M92 129 Q100 141 108 129 Z" fill="#c4473b" />
          <path d="M95 133.5 Q100 137 105 133.5" fill="#ff8f86" />
        </g>
      );
    case "talking":
      return <ellipse className="talk" cx={100} cy={132} rx={4.5} ry={4} fill="#c4473b" />;
    case "alert":
      return <ellipse cx={100} cy={133} rx={3.6} ry={4.4} fill="#c4473b" />;
    case "thinking":
      return <path d="M97 132 Q101 130 105 132.5" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    case "focus":
      return <path d="M96 132 L104 132" fill="none" stroke={LINE} strokeWidth={2.2} strokeLinecap="round" />;
    case "sleepy":
      return <ellipse cx={100} cy={133} rx={2.6} ry={2.2} fill="#c4473b" />;
    default:
      return <path d="M94.5 130 Q100 135.5 105.5 130" fill="none" stroke={LINE} strokeWidth={2.4} strokeLinecap="round" />;
  }
}

/** `face` crops to the head, for avatars. */
export default function Character({ mood = "idle", size = 140, face = false }: { mood?: Mood; size?: number; face?: boolean }) {
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
          <stop offset="0" stopColor="#fffaf3" />
          <stop offset="1" stopColor="#f6e3cf" />
        </linearGradient>
        <linearGradient id="skin" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#ffeadb" />
          <stop offset="1" stopColor="#ffdcc8" />
        </linearGradient>
      </defs>

      <ellipse cx={100} cy={214} rx={46} ry={5} fill="#000" opacity={0.12} className="shadow" />

      <g className="body">
        {/* hoodie */}
        <path d="M50 220 C50 182 70 164 100 162 C130 164 150 182 150 220 Z" fill="url(#hoodie)" stroke="#e9cdb2" strokeWidth={1.5} />
        <path d="M74 170 C82 182 118 182 126 170" fill="none" stroke="#e9cdb2" strokeWidth={2} />
        {/* drawstrings */}
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
        {/* blush */}
        <ellipse cx={69} cy={126} rx={9} ry={4.5} fill="#ff7f73" opacity={0.3} />
        <ellipse cx={131} cy={126} rx={9} ry={4.5} fill="#ff7f73" opacity={0.3} />

        <g className="eyes">
          <Eye x={77} mood={mood} />
          <Eye x={123} mood={mood} />
        </g>
        <Mouth mood={mood} />

        {/* bangs */}
        <path
          d="M48 96 C44 58 70 36 100 36 C130 36 156 58 152 96 C146 84 140 74 132 68 C131 79 125 88 117 92 C118 82 115 73 108 67 C104 79 96 88 85 92 C88 82 87 73 83 67 C75 75 64 86 48 96 Z"
          fill="url(#hair)"
        />
        <path d="M100 52 C97 66 98 78 101 88 C104 78 105 66 100 52 Z" fill="url(#hair)" />
        <path d="M83 67 C86 74 86 82 85 90" fill="none" stroke={HAIR_DARK} strokeWidth={1.3} opacity={0.5} />
        <path d="M108 67 C112 74 115 82 116 90" fill="none" stroke={HAIR_DARK} strokeWidth={1.3} opacity={0.5} />
        {/* hair shine */}
        <path d="M70 52 C80 46 92 44 104 45" fill="none" stroke="#fff3d6" strokeWidth={3} strokeLinecap="round" opacity={0.7} />
        {/* side locks */}
        <path d="M50 92 C44 114 46 136 56 154 C60 136 60 116 62 98 Z" fill="url(#hair)" />
        <path d="M150 92 C156 114 154 136 144 154 C140 136 140 116 138 98 Z" fill="url(#hair)" />
        {/* hair clip */}
        <g transform="translate(136 70) rotate(-25)">
          <rect x={-9} y={-3} width={18} height={6} rx={3} fill="#fff6e8" stroke="#e2572b" strokeWidth={1.2} />
          <circle cx={9} cy={0} r={3.4} fill="#7fc8a9" />
        </g>
        {/* ahoge */}
        <path className="ahoge" d="M100 37 C94 22 102 10 114 13 C105 17 102 25 104 37 Z" fill="url(#hair)" />

        {/* headphones while focusing */}
        {mood === "focus" && (
          <g className="headphones">
            <path d="M44 98 C40 36 160 36 156 98" fill="none" stroke="#3b3a4a" strokeWidth={7} strokeLinecap="round" />
            <rect x={34} y={88} width={17} height={30} rx={8} fill="#3b3a4a" />
            <rect x={149} y={88} width={17} height={30} rx={8} fill="#3b3a4a" />
            <rect x={38} y={93} width={6} height={20} rx={3} fill="#ff8a3d" />
            <rect x={156} y={93} width={6} height={20} rx={3} fill="#ff8a3d" />
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
      {mood === "alert" && (
        <g className="alert-mark">
          <path d="M168 20 L166 46" stroke="#e2572b" strokeWidth={6} strokeLinecap="round" />
          <circle cx={165} cy={57} r={3.6} fill="#e2572b" />
        </g>
      )}
      {mood === "sleepy" && (
        <text x={150} y={40} className="zzz" fill="#b07a5a" fontSize={18} fontWeight={800}>
          z
        </text>
      )}
    </svg>
  );
}
