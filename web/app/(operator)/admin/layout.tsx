import type { Metadata } from 'next'

import { AdminSidebar } from '@/components/admin/AdminSidebar'
import { CommandPalette } from '@/components/admin/CommandPalette'

export const metadata: Metadata = {
  title: 'Xustive admin',
  robots: { index: false, follow: false },
}

/**
 * The operator console shell. A two-column grid — sidebar and content — the same shape the Rust
 * process used to render, now a set of ordinary Next.js pages that read the `/api/v1/admin/*` JSON
 * API. Deliberately plainer than the search UI: it is a tool, always LTR and in English.
 *
 * One column below `md`, where the sidebar becomes a scrolling strip along the top: a fixed
 * 190 px column on a 390 px phone left 170 px for the content and wrapped every heading to one
 * word per line ([[UI - Responsive]] §0).
 */
export default function AdminLayout({ children }: { children: React.ReactNode }) {
  return (
    <div
      dir="ltr"
      className="mx-auto grid max-w-[1400px] grid-cols-1 gap-4 px-[var(--pad)] pb-16 pt-4 md:grid-cols-[190px_minmax(0,1fr)] md:gap-8 md:pt-6"
      style={{ color: 'var(--fg)' }}
    >
      <AdminSidebar />
        <CommandPalette />
      <main className="min-w-0">{children}</main>
    </div>
  )
}
