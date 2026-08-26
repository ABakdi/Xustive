/**
 * A small set of inline line icons.
 *
 * Hand-picked paths rather than an icon library: the search page ships no JavaScript for its
 * results, and an icon package would cost more than the entire entity panel. Each icon is a
 * 24-unit viewBox, 1.75 stroke, `currentColor` — so it takes the colour of the text beside it and
 * reads as a glyph, not a picture. `aria-hidden`: every use sits next to the word it decorates.
 */

const PATHS: Record<string, string> = {
  sparkle: 'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM5 17l.8 2.2L8 20l-2.2.8L5 23l-.8-2.2L2 20l2.2-.8z',
  user: 'M20 21a8 8 0 0 0-16 0M12 13a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9z',
  film: 'M4 4h16v16H4zM4 9h16M4 15h16M9 4v16M15 4v16',
  tv: 'M3 6h18v11H3zM8 21h8M12 17v4',
  pin: 'M12 22s7-6.2 7-12a7 7 0 1 0-14 0c0 5.8 7 12 7 12zM12 12a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z',
  building: 'M4 21V5l8-3v19M12 21V9l8 3v9M8 8h1M8 12h1M8 16h1M16 15h1',
  box: 'M12 2l9 5v10l-9 5-9-5V7zM12 12l9-5M12 12L3 7M12 12v10',
  book: 'M4 4h11a3 3 0 0 1 3 3v13H7a3 3 0 0 0-3 3zM4 4v16M18 20H8',
  music: 'M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM9 18V5l12-2v13',
  calendar: 'M4 5h16v15H4zM4 10h16M8 3v4M16 3v4',
  leaf: 'M5 21c0-8 4-14 14-16-1 10-6 14-14 16zM5 21c3-4 7-7 11-9',
  bulb: 'M9 18h6M10 21h4M12 3a6 6 0 0 0-3.5 10.9c.6.5 1 1.3 1 2.1h5c0-.8.4-1.6 1-2.1A6 6 0 0 0 12 3z',
  cake: 'M4 21h16M5 21v-7a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v7M8 12V9M12 12V9M16 12V9M12 5a1 1 0 1 0 0-.01',
  cross: 'M12 3v18M6 9h12',
  flag: 'M5 21V4M5 4h13l-2.5 4L18 12H5',
  shirt: 'M8 3l4 2 4-2 5 4-3 3-1-1v12H7V9L6 10 3 7z',
  briefcase: 'M3 8h18v12H3zM8 8V5h8v3M3 13h18',
  star: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z',
  clock: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2',
  globe: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM3 12h18M12 3c3 3.5 3 14.5 0 18M12 3c-3 3.5-3 14.5 0 18',
  tag: 'M3 12l9-9h9v9l-9 9zM16.5 7.5a1 1 0 1 0 0-.01',
  people: 'M16 21a5 5 0 0 0-10 0M11 12a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM21 20a4 4 0 0 0-3-3.9M15.5 5.2a3.5 3.5 0 0 1 0 6.6',
  ruler: 'M3 17l14-14 4 4L7 21zM8 8l2 2M11 5l2 2M14 11l2 2M5 11l2 2',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  quote: 'M7 17h4V9H5v4a4 4 0 0 0 2 4zM17 17h4V9h-6v4a4 4 0 0 0 2 4z',
  camera: 'M4 8h4l2-3h4l2 3h4v12H4zM12 17a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z',
  play: 'M6 4l14 8-14 8z',
  check: 'M4 12l5 5L20 7',
  users: 'M17 21v-2a4 4 0 0 0-4-4H7a4 4 0 0 0-4 4v2M10 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM21 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8',
}

export type IconName = keyof typeof PATHS

export function Icon({
  name,
  size = 16,
  className = '',
  style,
}: {
  name: IconName
  size?: number
  className?: string
  style?: React.CSSProperties
}) {
  const d = PATHS[name]
  if (!d) return null
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`inline-block shrink-0 align-[-0.15em] ${className}`.trim()}
      style={style}
    >
      <path d={d} />
    </svg>
  )
}
