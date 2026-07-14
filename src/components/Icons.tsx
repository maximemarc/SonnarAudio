/**
 * Minimal inline SVG icons (Sonar-style channel glyphs). `currentColor`
 * everywhere so the accent tints them for free.
 */

const S = {
  width: 15,
  height: 15,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export const GamepadIcon = () => (
  <svg {...S}>
    <path d="M6 12h4M8 10v4M15 11h.01M18 13h.01" />
    <path d="M17.5 6.5h-11a5 5 0 0 0-5 5.5c.2 2.5 1 5.5 3 5.5 1.6 0 2.3-1.3 3-2.5h9c.7 1.2 1.4 2.5 3 2.5 2 0 2.8-3 3-5.5a5 5 0 0 0-5-5.5Z" />
  </svg>
);

export const ChatIcon = () => (
  <svg {...S}>
    <path d="M21 12a8 8 0 0 1-8 8H4l2.5-2.7A8 8 0 1 1 21 12Z" />
  </svg>
);

export const MusicIcon = () => (
  <svg {...S}>
    <path d="M9 18V5l12-2v13" />
    <circle cx="6" cy="18" r="3" />
    <circle cx="18" cy="16" r="3" />
  </svg>
);

export const MicIcon = () => (
  <svg {...S}>
    <rect x="9" y="2" width="6" height="12" rx="3" />
    <path d="M5 10a7 7 0 0 0 14 0M12 17v4" />
  </svg>
);

export const WaveIcon = () => (
  <svg {...S}>
    <path d="M2 12h3l3-8 4 16 3-8h3" />
  </svg>
);

export const MasterIcon = () => (
  <svg {...S}>
    <path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3" />
    <path d="M2 14h4M10 8h4M18 16h4" />
  </svg>
);

export const SpeakerIcon = () => (
  <svg {...S}>
    <path d="M11 5 6 9H2v6h4l5 4V5Z" />
    <path d="M15.5 8.5a5 5 0 0 1 0 7M19 5a9 9 0 0 1 0 14" />
  </svg>
);

export const HeadphonesIcon = () => (
  <svg {...S}>
    <path d="M3 18v-6a9 9 0 0 1 18 0v6" />
    <path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3v5ZM3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3v5Z" />
  </svg>
);

export const SpeakerOffIcon = () => (
  <svg {...S}>
    <path d="M11 5 6 9H2v6h4l5 4V5Z" />
    <path d="m22 9-6 6M16 9l6 6" />
  </svg>
);

export const LinkIcon = () => (
  <svg {...S}>
    <path d="M9.5 14.5 14.5 9.5" />
    <path d="M8 16a3.5 3.5 0 0 1 0-5l2-2a3.5 3.5 0 0 1 5 5" />
    <path d="M16 8a3.5 3.5 0 0 1 0 5l-2 2a3.5 3.5 0 0 1-5-5" />
  </svg>
);
