// Small stroke icons (24px grid), drawn to match the rounded UI.
type P = { size?: number };

function Svg({ size = 20, children }: P & { children: React.ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      {children}
    </svg>
  );
}

export const ChatIcon = (p: P) => (
  <Svg {...p}>
    <path d="M20 12a8 8 0 0 1-11.6 7.1L4 20l1-4.1A8 8 0 1 1 20 12Z" />
  </Svg>
);
export const TasksIcon = (p: P) => (
  <Svg {...p}>
    <rect x="4" y="4" width="16" height="16" rx="4" />
    <path d="m8.5 12 2.5 2.5 4.5-5" />
  </Svg>
);
export const InboxIcon = (p: P) => (
  <Svg {...p}>
    <path d="M4 13h4l1.5 2.5h5L16 13h4" />
    <path d="M5.5 6.5 4 13v5a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5l-1.5-6.5A2 2 0 0 0 16.6 5H7.4a2 2 0 0 0-1.9 1.5Z" />
  </Svg>
);
export const CalendarIcon = (p: P) => (
  <Svg {...p}>
    <rect x="4" y="5" width="16" height="15" rx="3" />
    <path d="M4 10h16M9 3v4M15 3v4" />
  </Svg>
);
export const GearIcon = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
  </Svg>
);
export const CloseIcon = (p: P) => (
  <Svg {...p}>
    <path d="M6 6l12 12M18 6 6 18" />
  </Svg>
);
export const SendIcon = (p: P) => (
  <Svg {...p}>
    <path d="M5 12h13M13 6l6 6-6 6" />
  </Svg>
);
export const PlayIcon = (p: P) => (
  <Svg {...p}>
    <path d="M8 5.5v13l11-6.5-11-6.5Z" fill="currentColor" />
  </Svg>
);
export const StopIcon = (p: P) => (
  <Svg {...p}>
    <rect x="6.5" y="6.5" width="11" height="11" rx="2.5" fill="currentColor" />
  </Svg>
);
export const ShieldIcon = (p: P) => (
  <Svg {...p}>
    <path d="M12 3 5 6v5c0 4.4 3 8.3 7 10 4-1.7 7-5.6 7-10V6l-7-3Z" />
    <path d="m9 12 2 2 4-4" />
  </Svg>
);
export const PlugIcon = (p: P) => (
  <Svg {...p}>
    <path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0V8ZM12 17v4" />
  </Svg>
);
export const TrashIcon = (p: P) => (
  <Svg {...p}>
    <path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3" />
  </Svg>
);
export const RefreshIcon = (p: P) => (
  <Svg {...p}>
    <path d="M20 12a8 8 0 1 1-2.3-5.7L20 8.6M20 4v4.6h-4.6" />
  </Svg>
);
export const MicIcon = (p: P) => (
  <Svg {...p}>
    <rect x="9" y="3" width="6" height="11" rx="3" />
    <path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21" />
  </Svg>
);
export const SpeakerIcon = (p: P) => (
  <Svg {...p}>
    <path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5H4Z" />
    <path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11" />
  </Svg>
);
