const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.8,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export function Logo({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="9" cy="12" r="5" {...stroke} strokeWidth={2} />
      <circle cx="15" cy="12" r="5" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function NewChatIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6" {...stroke} />
      <path d="M17.5 3.5a2.1 2.1 0 0 1 3 3L13 14l-4 1 1-4Z" {...stroke} />
    </svg>
  );
}

export function ArrowUpIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 19V5M5 12l7-7 7 7" {...stroke} strokeWidth={2.2} />
    </svg>
  );
}

export function StopIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="5" y="5" width="14" height="14" rx="2.5" fill="currentColor" />
    </svg>
  );
}

export function MenuIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 7h16M4 12h16M4 17h10" {...stroke} />
    </svg>
  );
}

export function ChevronIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
      <path d="m6 9 6 6 6-6" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function CheckIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <path d="m5 12 5 5 9-10" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function ArchiveIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="4" width="18" height="5" rx="1.5" {...stroke} />
      <path d="M5 9v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9M10 13h4" {...stroke} />
    </svg>
  );
}

export function GlobeIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="9" {...stroke} />
      <path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3Z" {...stroke} />
    </svg>
  );
}

export function MicIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="9" y="3" width="6" height="11" rx="3" {...stroke} />
      <path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21" {...stroke} />
    </svg>
  );
}

export function SpeakerIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4Z" {...stroke} />
      <path d="M15.5 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11" {...stroke} />
    </svg>
  );
}

export function MicOffIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M15 10.5V6a3 3 0 0 0-5.7-1.3M9 9v2a3 3 0 0 0 4.6 2.5M5.5 11a6.5 6.5 0 0 0 10.4 5.2M18.5 11a6.4 6.4 0 0 1-.6 2.7M12 17.5V21M4 4l16 16" {...stroke} />
    </svg>
  );
}

export function CloseIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 6l12 12M18 6 6 18" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function VoiceIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function ImageIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" {...stroke} />
      <circle cx="9" cy="10" r="1.6" {...stroke} />
      <path d="m5 18 4.5-4.5 3 3 2.5-2.5 4 4" {...stroke} />
    </svg>
  );
}

export function DownloadIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 4v11M7.5 11l4.5 4.5 4.5-4.5M5 19.5h14" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function FolderIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" {...stroke} />
    </svg>
  );
}

export function BackIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d="m14.5 6-6 6 6 6" {...stroke} strokeWidth={2} />
    </svg>
  );
}

export function CopyIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="8.5" y="8.5" width="11" height="11" rx="2.5" {...stroke} />
      <path d="M15.5 5.5v-.5a2 2 0 0 0-2-2h-8a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h.5" {...stroke} />
    </svg>
  );
}
